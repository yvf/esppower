//! `NetCtl` + `ThreadDiag` + `WirelessDiag` + `NetChangeNotif` adapter over
//! openthread — the commissioning-critical piece.
//!
//! During Matter commissioning the commissioner sends the Thread operational
//! dataset (AddOrUpdateThreadNetwork) and then ConnectNetwork → `connect()`,
//! where we apply the dataset to openthread and wait until the node attaches.

use embassy_time::{Duration, Instant, Timer};

use openthread::{DeviceRole, OpenThread};

use rs_matter_stack::matter::dm::clusters::net_comm::{
    NetCtl, NetCtlError, NetworkScanInfo, NetworkType, WirelessCreds,
};
use rs_matter_stack::matter::dm::clusters::thread_diag::ThreadDiag;
use rs_matter_stack::matter::dm::clusters::wifi_diag::WirelessDiag;
use rs_matter_stack::matter::dm::networks::NetChangeNotif;
use rs_matter_stack::matter::error::{Error, ErrorCode};
use rs_matter_stack::matter::utils::sync::DynBase;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_POLL: Duration = Duration::from_secs(1);

fn is_attached(ot: &OpenThread) -> bool {
    matches!(
        ot.net_status().role,
        DeviceRole::Child | DeviceRole::Router | DeviceRole::Leader
    )
}

/// Log the identifying fields of a Thread operational dataset (MeshCoP TLVs) so an
/// attach failure can be cross-checked against the live network. Decodes only the
/// non-secret TLVs — channel (0x00), PAN ID (0x01), extended PAN ID (0x02), network
/// name (0x03). The network key (0x05) and PSKc (0x04) are deliberately NOT logged.
fn log_dataset_summary(tlv: &[u8]) {
    let mut i = 0;
    while i + 2 <= tlv.len() {
        let ty = tlv[i];
        let len = tlv[i + 1] as usize;
        let val = tlv.get(i + 2..i + 2 + len);
        let Some(val) = val else { break };
        match ty {
            0x00 if val.len() == 3 => {
                // page(1) || channel(2, big-endian)
                let ch = u16::from_be_bytes([val[1], val[2]]);
                log::info!("[matter] dataset: channel = {ch}");
            }
            0x01 if val.len() == 2 => {
                log::info!(
                    "[matter] dataset: pan_id = 0x{:04x}",
                    u16::from_be_bytes([val[0], val[1]])
                );
            }
            0x02 if val.len() == 8 => {
                log::info!("[matter] dataset: ext_pan_id = {val:02x?}");
            }
            0x03 => {
                if let Ok(name) = core::str::from_utf8(val) {
                    log::info!("[matter] dataset: network_name = {name:?}");
                }
            }
            _ => {}
        }
        i += 2 + len;
    }
}

/// `NetCtl`/`ThreadDiag` over an OpenThread instance.
pub struct OtNetCtl<'a> {
    ot: OpenThread<'a>,
}

impl<'a> OtNetCtl<'a> {
    pub const fn new(ot: OpenThread<'a>) -> Self {
        Self { ot }
    }
}

impl NetCtl for OtNetCtl<'_> {
    fn net_type(&self) -> NetworkType {
        NetworkType::Thread
    }

    /// Apple Home (and chip-tool) commission Thread by pushing the operational
    /// dataset directly, so we report no scan results and let `connect` apply it.
    async fn scan<F>(&self, _network: Option<&[u8]>, _f: F) -> Result<(), NetCtlError>
    where
        F: FnMut(&NetworkScanInfo) -> Result<(), Error>,
    {
        Ok(())
    }

    async fn connect(&self, creds: &WirelessCreds<'_>) -> Result<(), NetCtlError> {
        let WirelessCreds::Thread { dataset_tlv } = creds else {
            return Err(NetCtlError::Other(ErrorCode::InvalidData.into()));
        };

        log::info!(
            "[matter] OtNetCtl::connect — applying Thread dataset ({} TLV bytes)",
            dataset_tlv.len()
        );
        log_dataset_summary(dataset_tlv);

        self.ot
            .set_active_dataset_tlv(dataset_tlv)
            .map_err(|_| NetCtlError::Other(ErrorCode::ConstraintError.into()))?;
        self.ot
            .enable_ipv6(true)
            .map_err(|_| NetCtlError::Other(ErrorCode::NoNetworkInterface.into()))?;
        self.ot
            .enable_thread(true)
            .map_err(|_| NetCtlError::Other(ErrorCode::NoNetworkInterface.into()))?;

        let started = Instant::now();
        loop {
            if is_attached(&self.ot) {
                return Ok(());
            }
            if started.elapsed() > CONNECT_TIMEOUT {
                return Err(NetCtlError::AuthFailure);
            }
            Timer::after(CONNECT_POLL).await;
        }
    }
}

impl NetChangeNotif for OtNetCtl<'_> {
    async fn wait_changed(&self) {
        self.ot.wait_changed().await;
    }
}

impl DynBase for OtNetCtl<'_> {}

impl WirelessDiag for OtNetCtl<'_> {
    fn connected(&self) -> Result<bool, Error> {
        Ok(is_attached(&self.ot))
    }
}

// Thread diagnostics — all defaults (report unknown) suffice for commissioning;
// channel/pan-id/role can be filled from `ot.net_status()` later if needed.
impl ThreadDiag for OtNetCtl<'_> {}

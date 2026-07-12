//! `NetifDiag` + `NetChangeNotif` adapter over openthread.
//!
//! Reports the single Thread interface (addresses from `ot.ipv6_addrs`,
//! operational once attached) and wakes on `ot.wait_changed()`.
//! Template: esp-idf-matter `netif.rs` (EspMatterNetif).

use core::net::{Ipv4Addr, Ipv6Addr};

use openthread::{DeviceRole, OpenThread};

use rs_matter_stack::matter::dm::clusters::gen_diag::{InterfaceTypeEnum, NetifDiag, NetifInfo};
use rs_matter_stack::matter::dm::networks::NetChangeNotif;
use rs_matter_stack::matter::error::Error;
use rs_matter_stack::matter::utils::sync::DynBase;

/// Max IPv6 addresses we report for the Thread interface (link-local, mesh-local,
/// OMR, etc.). Thread devices rarely exceed a handful.
const MAX_IPV6: usize = 8;

/// `NetifDiag`/`NetChangeNotif` over an OpenThread instance.
pub struct OtNetif<'a> {
    ot: OpenThread<'a>,
    eui64: [u8; 8],
}

impl<'a> OtNetif<'a> {
    pub const fn new(ot: OpenThread<'a>, eui64: [u8; 8]) -> Self {
        Self { ot, eui64 }
    }
}

impl DynBase for OtNetif<'_> {}

impl NetifDiag for OtNetif<'_> {
    fn netifs(&self, f: &mut dyn FnMut(&NetifInfo) -> Result<(), Error>) -> Result<(), Error> {
        let mut ipv6_addrs = heapless::Vec::<Ipv6Addr, MAX_IPV6>::new();
        // ipv6_addrs rarely errors; on error we just report what we gathered.
        let _ = self.ot.ipv6_addrs(|addr| {
            if let Some((ip, _prefix)) = addr {
                if ip != Ipv6Addr::UNSPECIFIED {
                    let _ = ipv6_addrs.push(ip);
                }
            }
            Ok(())
        });

        let operational = matches!(
            self.ot.net_status().role,
            DeviceRole::Child | DeviceRole::Router | DeviceRole::Leader
        );

        let info = NetifInfo {
            name: "ot",
            operational,
            offprem_svc_reachable_ipv4: None,
            offprem_svc_reachable_ipv6: None,
            hw_addr: &self.eui64,
            ipv4_addrs: &[] as &[Ipv4Addr], // Thread is IPv6-only
            ipv6_addrs: &ipv6_addrs,
            netif_type: InterfaceTypeEnum::Thread,
            netif_index: 0,
        };

        f(&info)
    }
}

impl NetChangeNotif for OtNetif<'_> {
    async fn wait_changed(&self) {
        self.ot.wait_changed().await;
    }
}

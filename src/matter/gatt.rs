//! `GattPeripheral` adapter: Matter BLE commissioning (BTP) over trouble.
//!
//! Owns the BLE stack lifecycle: builds the trouble Host, advertises the Matter
//! BTP service (0xFFF6), and on each connection runs two pumps —
//!  - **incoming**: GATT writes to C1 → `Btp::process_incoming`,
//!  - **outgoing**: `Btp::process_outgoing` → C2 indications.

use embassy_futures::select::{select, Either};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};

// heapless 0.9 — trouble's version, the one its `AsGatt for Vec<u8,N>` is on.
use heapless_09::Vec as HVec;

use trouble_host::prelude::*;

use rs_matter_stack::ble::GattPeripheral;
use rs_matter_stack::matter::error::{Error, ErrorCode};
use rs_matter_stack::matter::transport::network::btp::{AdvData, Btp};
use rs_matter_stack::matter::transport::network::BtAddr;

/// BTP segment buffer. Bounded by the ATT MTU of the `mtu-255` packet pool.
const BTP_BUF: usize = 247;
const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2; // signalling + ATT

// Matter BTP GATT service + characteristics (UUIDs per the Matter Core spec /
// `rs_matter::transport::network::btp::gatt`).
#[gatt_server]
struct BtpServer {
    btp: BtpService,
}

#[gatt_service(uuid = "0000FFF6-0000-1000-8000-00805F9B34FB")]
struct BtpService {
    /// C1: commissioner → device (write).
    #[characteristic(uuid = "18EE2EF5-263D-4559-959F-4F9C429F9D11", write)]
    c1: HVec<u8, BTP_BUF>,
    /// C2: device → commissioner (indicate). `Vec` so the indication carries the
    /// exact BTP segment length (a fixed array would pad with garbage and corrupt
    /// BTP framing).
    #[characteristic(uuid = "18EE2EF5-263D-4559-959F-4F9C429F9D12", indicate)]
    c2: HVec<u8, BTP_BUF>,
}

fn to_err<E: core::fmt::Debug>(e: E) -> Error {
    // BTP/BLE failures are a transport problem, not a missing-netif one. Mapping to
    // BtpError keeps the surfaced error honest (it previously masqueraded as
    // NoNetworkInterface, which sent debugging in the wrong direction).
    log::warn!("[matter] OtGattPeripheral error: {e:?}");
    ErrorCode::BtpError.into()
}

/// `GattPeripheral` over a trouble BLE controller.
pub struct OtGattPeripheral<C: Controller> {
    controller: Option<C>,
    address: [u8; 6],
}

impl<C: Controller> OtGattPeripheral<C> {
    pub const fn new(controller: C, address: [u8; 6]) -> Self {
        Self {
            controller: Some(controller),
            address,
        }
    }

    /// Construct WITHOUT a BLE controller, for the operational (already-commissioned)
    /// path where the non-concurrent stack skips `run()` (BLE commissioning). No BLE
    /// stack is initialized, so no BLE controller task is spun up — important on a
    /// stack restart, where re-initializing BLE every time leaks the controller's
    /// (heap-allocated) task stack and eventually exhausts memory. If `run()` were ever
    /// called on such an instance (it is not, while commissioned) it fails cleanly with
    /// `InvalidData` rather than advertising.
    pub const fn without_controller(address: [u8; 6]) -> Self {
        Self {
            controller: None,
            address,
        }
    }
}

impl<C: Controller> GattPeripheral for OtGattPeripheral<C> {
    async fn run(
        &mut self,
        btp: &Btp,
        service_name: &str,
        service_adv: &AdvData,
    ) -> Result<(), Error> {
        log::info!("[matter] OtGattPeripheral::run starting (service '{service_name}')");

        // Enable relaxed MTU negotiation. Without it, rs-matter falls back to the 23-byte
        // MIN_MTU on *any* mismatch between our GATT MTU (251) and the commissioner's
        // proposed BTP MTU — and those practically never match, so every session ran at
        // MTU 23. Relaxed mode instead uses min(peer, ours, MAX_MTU=247). Paired with the
        // handshake ATT_MTU=0 substitution in `serve_conn`, this lifts BTP to ~247.
        btp.set_relaxed_mtu_nego(true);

        // BLE is used once, for the commissioning window; `run` loops internally.
        let controller = self
            .controller
            .take()
            .ok_or_else(|| Error::from(ErrorCode::InvalidData))?;
        let address = Address::random(self.address);

        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
            HostResources::new();
        let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
        let Host {
            mut peripheral,
            runner,
            ..
        } = stack.build();

        let server = BtpServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: service_name,
            appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
        }))
        .map_err(to_err)?;

        // The Matter BTP advertisement payload (Flags AD1 + 0xFFF6 service data AD2).
        let mut adv = [0u8; 31];
        let mut adv_len = 0;
        for b in service_adv.iter() {
            if adv_len >= adv.len() {
                break;
            }
            adv[adv_len] = b;
            adv_len += 1;
        }

        let runner_fut = async {
            let mut runner = runner;
            loop {
                if let Err(e) = runner.run().await {
                    return to_err(e);
                }
            }
        };

        let serve_fut = async {
            loop {
                match advertise_btp(&mut peripheral, &server, &adv[..adv_len]).await {
                    Ok(conn) => {
                        serve_conn(&server, btp, &conn).await;
                        // Drop any half-open BTP session before re-advertising.
                        btp.reset();
                    }
                    Err(_e) => {}
                }
            }
        };

        match select(runner_fut, serve_fut).await {
            Either::First(e) => Err(e),
            Either::Second(()) => Ok(()),
        }
    }
}

async fn advertise_btp<'a, 'b, C: Controller>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BtpServer<'a>,
    adv_data: &[u8],
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    // Default advertising interval (160 ms). We commission in non-concurrent mode (see the
    // `stack.run(...)` note in stack.rs): BLE runs ONLY while the device is un-commissioned,
    // with Thread not yet attached, so there is no BLE↔802.15.4 coexistence contention to
    // avoid here — fast advertising just means quicker discovery by the commissioner.
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data,
                scan_data: &[],
            },
        )
        .await?;
    log::info!("[matter] BLE advertising Matter BTP service (0xFFF6) — waiting for commissioner");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    log::info!("[matter] BLE commissioner connected — running BTP session");
    Ok(conn)
}

async fn serve_conn<P: PacketPool>(
    server: &BtpServer<'_>,
    btp: &Btp,
    conn: &GattConnection<'_, '_, P>,
) {
    let addr = BtAddr(conn.raw().peer_address().into_inner());

    // NB: read `att_mtu()` *fresh* at each BTP op, never cache it. At connect time it
    // is the 23-byte ATT default; the trouble runner only raises it (to min(247, peer))
    // when the peer's ExchangeMTU completes. Since the runner processes ACL packets in
    // wire order, that exchange lands before the first C1 write reaches us — but a value
    // latched at connect would pin BTP to 23 and stall commissioning (Apple → "Accessory
    // Not Found").

    // C2 indications must be enabled by the peer (a CCCD write) before we send anything:
    // trouble's `indicate()` *silently drops* the packet and still returns `Ok` when the
    // subscription isn't active (see `Characteristic::indicate` → `should_indicate`). The
    // BlueZ chip-tool writes the BTP handshake to C1 *before* subscribing to C2, so firing
    // the handshake-response indication the instant the C1 write lands races the CCCD
    // write and gets dropped — the commissioner then times out waiting for the response.
    // This signal latches once the peer enables C2 indications; the outgoing pump waits on
    // it before its first send. Both pumps run cooperatively under one `select`, so a
    // NoopRawMutex is sufficient.
    let subscribed: Signal<NoopRawMutex, ()> = Signal::new();
    let c2_cccd_handle = server.btp.c2.cccd_handle;

    let incoming = async {
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    log::info!("[matter] BTP incoming: peer disconnected (reason {reason:?})");
                    break;
                }
                GattConnectionEvent::Gatt { event } => {
                    if let GattEvent::Write(w) = &event {
                        if w.handle() == server.btp.c1.handle {
                            let mtu = conn.raw().att_mtu();
                            let d = w.data();
                            log::info!(
                                "[matter] BTP C1 write: {} bytes (att_mtu={})",
                                d.len(),
                                mtu
                            );
                            // The BTP handshake request is a 9-byte segment with the H
                            // (handshake) flag in bit 6 of byte 0; its bytes 6..8 carry the
                            // commissioner's proposed ATT_MTU (little-endian). Per the Matter
                            // spec a value of 0 means "use the GATT-negotiated ATT MTU". With
                            // relaxed MTU negotiation enabled (see `run`), rs-matter takes
                            // min(req.mtu, gatt_mtu, MAX_MTU) — but req.mtu==0 would underflow
                            // to a bogus MTU. So when the field is 0 we substitute our real
                            // ATT MTU into a patched copy before handing it to rs-matter. This
                            // lifts BTP off the 23-byte floor (which segmented the ~670-byte
                            // AttestationResponse into ~40 windowed pieces) up to ~247.
                            let mut patched: HVec<u8, BTP_BUF> = HVec::new();
                            let d = if d.len() >= 8 && d.first().is_some_and(|b| b & 0x40 != 0) {
                                let cli_mtu = u16::from_le_bytes([d[6], d[7]]);
                                log::info!(
                                    "[matter] BTP handshake req: {d:02x?} → client ATT_MTU field = {cli_mtu}"
                                );
                                if cli_mtu == 0 {
                                    let _ = patched.extend_from_slice(d);
                                    let sub = mtu.to_le_bytes();
                                    patched[6] = sub[0];
                                    patched[7] = sub[1];
                                    log::info!(
                                        "[matter] BTP handshake ATT_MTU=0 → substituting {mtu}"
                                    );
                                    &patched[..]
                                } else {
                                    d
                                }
                            } else {
                                d
                            };
                            if let Err(e) = btp.process_incoming(Some(mtu), addr, d) {
                                log::warn!("[matter] BTP process_incoming error: {e:?}");
                            }
                        } else if Some(w.handle()) == c2_cccd_handle {
                            // CCCD write: bit 1 (0x02) enables indications. `accept()` below
                            // records the subscription in the server; signal so the outgoing
                            // pump may start sending.
                            let enabled = w.data().first().is_some_and(|b| b & 0x02 != 0);
                            log::info!("[matter] BTP C2 CCCD write: indications enabled={enabled}");
                            if enabled {
                                // Accept first so the server's should_indicate() is true
                                // before the outgoing pump wakes.
                                match event.accept() {
                                    Ok(reply) => reply.send().await,
                                    Err(e) => log::warn!("[matter] BTP CCCD accept error: {e:?}"),
                                }
                                subscribed.signal(());
                                continue;
                            }
                        }
                    }
                    // Accept must fire so the peer gets its ATT write/read response;
                    // log a failure rather than swallowing it.
                    match event.accept() {
                        Ok(reply) => reply.send().await,
                        Err(e) => log::warn!("[matter] BTP event.accept error: {e:?}"),
                    }
                }
                _ => {}
            }
        }
        log::info!("[matter] BTP incoming pump exited");
    };

    let outgoing = async {
        // Wait until the peer has enabled C2 indications, else the handshake response is
        // silently dropped (see the `subscribed` note above).
        subscribed.wait().await;
        log::info!("[matter] BTP C2 subscribed — starting outgoing pump");
        let mut buf = [0u8; BTP_BUF];
        loop {
            btp.wait_outgoing().await;
            let mtu = conn.raw().att_mtu();
            match btp.process_outgoing(Some(mtu), &mut buf) {
                Ok(0) => continue,
                Ok(len) => {
                    let mut value: HVec<u8, BTP_BUF> = HVec::new();
                    let _ = value.extend_from_slice(&buf[..len]);
                    log::info!("[matter] BTP C2 indicate: {len} bytes (att_mtu={mtu})");
                    if let Err(e) = server.btp.c2.indicate(conn, &value).await {
                        log::warn!("[matter] BTP c2.indicate error: {e:?}");
                    }
                }
                Err(e) => {
                    log::warn!("[matter] BTP process_outgoing error: {e:?}");
                    break;
                }
            }
        }
        log::info!("[matter] BTP outgoing pump exited");
    };

    select(incoming, outgoing).await;
    log::info!("[matter] serve_conn returning — BTP session over");
}

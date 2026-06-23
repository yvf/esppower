//! `GattPeripheral` adapter: Matter BLE commissioning (BTP) over trouble.
//!
//! Owns the BLE stack lifecycle: builds the trouble Host, advertises the Matter
//! BTP service (0xFFF6), and on each connection runs two pumps —
//!  - **incoming**: GATT writes to C1 → `Btp::process_incoming`,
//!  - **outgoing**: `Btp::process_outgoing` → C2 indications.
//! Template: esp-idf-matter `ble.rs` (EspBtpGattPeripheral) for the pump shape;
//! the GATT-server half is trouble (reused from the Phase-3 BLE code).

use embassy_futures::select::{select, Either};

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

fn to_err<E>(_e: E) -> Error {
    ErrorCode::NoNetworkInterface.into()
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
}

impl<C: Controller> GattPeripheral for OtGattPeripheral<C> {
    async fn run(
        &mut self,
        btp: &Btp,
        service_name: &str,
        service_adv: &AdvData,
    ) -> Result<(), Error> {
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
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data,
                scan_data: &[],
            },
        )
        .await?;
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    Ok(conn)
}

async fn serve_conn<P: PacketPool>(
    server: &BtpServer<'_>,
    btp: &Btp,
    conn: &GattConnection<'_, '_, P>,
) {
    let mtu = conn.raw().att_mtu();
    let addr = BtAddr(conn.raw().peer_address().into_inner());

    let incoming = async {
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { .. } => break,
                GattConnectionEvent::Gatt { event } => {
                    if let GattEvent::Write(w) = &event {
                        if w.handle() == server.btp.c1.handle {
                            let _ = btp.process_incoming(Some(mtu), addr, w.data());
                        }
                    }
                    let _ = event.accept();
                }
                _ => {}
            }
        }
    };

    let outgoing = async {
        let mut buf = [0u8; BTP_BUF];
        loop {
            btp.wait_outgoing().await;
            match btp.process_outgoing(Some(mtu), &mut buf) {
                Ok(0) => continue,
                Ok(len) => {
                    let mut value: HVec<u8, BTP_BUF> = HVec::new();
                    let _ = value.extend_from_slice(&buf[..len]);
                    let _ = server.btp.c2.indicate(conn, &value).await;
                }
                Err(_) => break,
            }
        }
    };

    select(incoming, outgoing).await;
}

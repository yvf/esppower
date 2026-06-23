//! RCD Reset Controller — no-std (esp-hal) rewrite, ESP32-H2.
//!
//! Phase 3: Thread (OpenThread) AND BLE (trouble) running concurrently on the
//! shared H2 radio (the coexistence Matter needs — commission over BLE, operate
//! over Thread). BLE currently advertises a placeholder GATT service; Phase 4
//! replaces it with the Matter BTP service and wires the rs-matter-stack glue.
//! See docs/no-std-plan.md.

#![no_std]
#![no_main]

use core::net::Ipv6Addr;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
use log::{info, warn};

use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use esp_radio::ieee802154::Ieee802154;

use openthread::esp::EspRadio;
use openthread::{
    OpenThread, OtResources, OtRngCore, OtSrpResources, OtUdpResources, SimpleRamSettings,
};

use trouble_host::prelude::*;

use esp_backtrace as _;
use esp_println as _;
use tinyrlibc as _;

esp_bootloader_esp_idf::esp_app_desc!();

macro_rules! mk_static {
    ($t:ty) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        STATIC_CELL.uninit()
    }};
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        STATIC_CELL.uninit().write($val)
    }};
}

// ─── Thread (OpenThread) sizing ──────────────────────────────────────────────
const UDP_MAX_SOCKETS: usize = 2;
const UDP_SOCKETS_BUF: usize = 1280;
const SRP_MAX_SERVICES: usize = 2;
const SRP_SERVICE_BUF: usize = 300;

/// Thread operational dataset (hex TLV). Standalone test only — under Matter the
/// commissioner supplies this over BLE.
const THREAD_DATASET: &str = if let Some(d) = option_env!("THREAD_DATASET") {
    d
} else {
    "0e080000000000010000000300000b35060004001fffe002083a90e3a319a904940708fd1fa298dbd1e3290510fe0458f7db96354eaa6041b880ea9c0f030f4f70656e5468726561642d35386431010258d10410888f813c61972446ab616ee3c556a5910c0402a0f7f8"
};

// ─── BLE (trouble) sizing ────────────────────────────────────────────────────
const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2; // signalling + ATT

/// Placeholder GATT server (replaced by the Matter BTP service in Phase 4).
#[gatt_server]
struct Server {
    battery: BatteryService,
}

#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    level: u8,
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_alloc::heap_allocator!(size: 64 * 1024);
    esp_println::logger::init_logger_from_env();
    info!("RCD no-std (Thread + BLE coex) starting…");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(
        timg0.timer0,
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT)
            .software_interrupt0,
    );

    // ── Thread ────────────────────────────────────────────────────────────────
    let rng = mk_static!(Rng, Rng::new());
    let mut ieee_eui64 = [0u8; 8];
    rng.fill_bytes(&mut ieee_eui64);

    let ot_resources = mk_static!(OtResources, OtResources::new());
    let ot_udp_resources =
        mk_static!(OtUdpResources<UDP_MAX_SOCKETS, UDP_SOCKETS_BUF>, OtUdpResources::new());
    let ot_srp_resources =
        mk_static!(OtSrpResources<SRP_MAX_SERVICES, SRP_SERVICE_BUF>, OtSrpResources::new());
    let ot_settings_buf = mk_static!([u8; 1024], [0; 1024]);
    let ot_settings = mk_static!(SimpleRamSettings, SimpleRamSettings::new(ot_settings_buf));

    let ot = OpenThread::new_with_udp_srp(
        ieee_eui64,
        rng,
        ot_settings,
        ot_resources,
        ot_udp_resources,
        ot_srp_resources,
    )
    .unwrap();

    spawner.spawn(
        run_ot(
            ot.clone(),
            EspRadio::new(Ieee802154::new(peripherals.IEEE802154)),
        )
        .unwrap(),
    );

    ot.srp_autostart().unwrap();
    ot.set_active_dataset_tlv_hexstr(THREAD_DATASET).unwrap();
    ot.enable_ipv6(true).unwrap();
    ot.enable_thread(true).unwrap();
    info!("Thread enabled");

    // ── BLE ─────────────────────────────────────────────────────────────────────
    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 1> = ExternalController::new(connector);

    info!("Thread + BLE up; advertising and reporting Thread status…");

    // Run both transports concurrently on the shared radio.
    join(thread_status_loop(ot), ble_peripheral(controller)).await;
}

// ─── Thread tasks ────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn run_ot(ot: OpenThread<'static>, radio: EspRadio<'static>) -> ! {
    ot.run(radio).await
}

async fn thread_status_loop(ot: OpenThread<'static>) {
    loop {
        Timer::after(Duration::from_secs(5)).await;
        let status = ot.net_status();
        let mut n = 0;
        ot.ipv6_addrs(|addr| {
            if let Some((ip, _prefix)) = addr {
                if ip != Ipv6Addr::UNSPECIFIED {
                    n += 1;
                }
            }
            Ok(())
        })
        .unwrap();
        info!(
            "Thread: role={:?} ext_pan_id={:?} ip6={} | {} addr(s), free heap = {} B",
            status.role, status.ext_pan_id, status.ip6_enabled, n, esp_alloc::HEAP.free()
        );
    }
}

// ─── BLE peripheral (trouble) ────────────────────────────────────────────────

async fn ble_peripheral<C: Controller>(controller: C) {
    let address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "RCD-Reset",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    let _ = join(ble_runner(runner), async {
        loop {
            match advertise("RCD-Reset", &mut peripheral, &server).await {
                Ok(conn) => {
                    info!("[ble] connected");
                    let _ = gatt_events(&server, &conn).await;
                    info!("[ble] disconnected");
                }
                Err(e) => warn!("[ble] advertise error: {e:?}"),
            }
        }
    })
    .await;
}

async fn ble_runner<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            warn!("[ble] runner error: {e:?}");
        }
    }
}

async fn advertise<'a, 'b, C: Controller>(
    name: &'a str,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b Server<'a>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv_data = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut adv_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    Ok(conn)
}

async fn gatt_events<P: PacketPool>(
    _server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), trouble_host::Error> {
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { .. } => break,
            GattConnectionEvent::Gatt { event } => {
                // Placeholder: accept everything. Phase 4 routes C1/C2 into Btp.
                let _ = event.accept();
            }
            _ => {}
        }
    }
    Ok(())
}

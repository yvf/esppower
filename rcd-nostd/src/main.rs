//! RCD Reset Controller — no-std (esp-hal) rewrite, ESP32-H2.
//!
//! Phase 3: Thread (OpenThread) AND BLE (trouble) running concurrently on the
//! shared H2 radio (the coexistence Matter needs — commission over BLE, operate
//! over Thread). BLE currently advertises a placeholder GATT service; Phase 4
//! replaces it with the Matter BTP service and wires the rs-matter-stack glue.
//! See docs/no-std-plan.md.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use log::{info, warn};

use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use esp_radio::ieee802154::Ieee802154;

use openthread::esp::EspRadio;
use openthread::{
    OpenThread, OtResources, OtRngCore, OtSrpResources, OtUdpResources, SimpleRamSettings,
};

use trouble_host::prelude::ExternalController;

use esp_backtrace as _;
use esp_println as _;
use tinyrlibc as _;

esp_bootloader_esp_idf::esp_app_desc!();

mod matter; // Phase 4b transport glue (built incrementally; not yet wired in)

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

    // Bring the IPv6 interface up (link-local) so the Matter operational stack has
    // a netif to initialize against during BLE commissioning. Thread itself is NOT
    // attached here — the commissioner supplies the operational dataset over BLE and
    // `matter::OtNetCtl` applies it (set_active_dataset_tlv + enable_thread) then.
    ot.enable_ipv6(true).unwrap();

    // ── BLE ─────────────────────────────────────────────────────────────────────
    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 1> = ExternalController::new(connector);

    info!("Starting Matter (commission over BLE, operate over Thread)…");
    if let Err(e) = matter::run_matter(ot, controller, ieee_eui64, Rng::new()).await {
        warn!("Matter stack exited: {e:?}");
    }
}

// ─── Thread tasks ────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn run_ot(ot: OpenThread<'static>, radio: EspRadio<'static>) -> ! {
    ot.run(radio).await
}

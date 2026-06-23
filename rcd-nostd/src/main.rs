//! RCD Reset Controller — no-std (esp-hal) rewrite, ESP32-H2.
//!
//! Phase 2: bring up Thread via OpenThread on the H2 802.15.4 radio. Joins the
//! network from `THREAD_DATASET` (env) and logs role/addresses/free heap. Later
//! phases add BLE (trouble) + Matter (rs-matter); the Thread credentials will
//! then come from Matter commissioning instead of a hardcoded dataset.
//! See docs/no-std-plan.md.

#![no_std]
#![no_main]

use core::net::Ipv6Addr;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use log::info;

use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ieee802154::Ieee802154;

use openthread::esp::EspRadio;
use openthread::{
    OpenThread, OtResources, OtRngCore, OtSrpResources, OtUdpResources, SimpleRamSettings,
};

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

const UDP_MAX_SOCKETS: usize = 2;
const UDP_SOCKETS_BUF: usize = 1280;
const SRP_MAX_SERVICES: usize = 2;
const SRP_SERVICE_BUF: usize = 300;

/// Thread operational dataset (hex TLV). For Phase 2 standalone testing, set
/// THREAD_DATASET to your border router's dataset. Under Matter, the commissioner
/// supplies this over BLE, so it will eventually be removed.
const THREAD_DATASET: &str = if let Some(d) = option_env!("THREAD_DATASET") {
    d
} else {
    // OpenThread default test network (replace with your own).
    "0e080000000000010000000300000b35060004001fffe002083a90e3a319a904940708fd1fa298dbd1e3290510fe0458f7db96354eaa6041b880ea9c0f030f4f70656e5468726561642d35386431010258d10410888f813c61972446ab616ee3c556a5910c0402a0f7f8"
};

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_alloc::heap_allocator!(size: 64 * 1024);
    esp_println::logger::init_logger_from_env();
    info!("RCD no-std (Thread bring-up) starting…");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(
        timg0.timer0,
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT)
            .software_interrupt0,
    );

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

    info!("Thread enabled; waiting to attach…");

    loop {
        Timer::after(Duration::from_secs(5)).await;
        let mut n = 0;
        ot.ipv6_addrs(|addr| {
            if let Some((ip, _prefix)) = addr {
                if ip != Ipv6Addr::UNSPECIFIED {
                    info!("  addr: {ip}");
                    n += 1;
                }
            }
            Ok(())
        })
        .unwrap();
        info!(
            "Thread: {} addr(s), free heap = {} bytes",
            n,
            esp_alloc::HEAP.free()
        );
    }
}

#[embassy_executor::task]
async fn run_ot(ot: OpenThread<'static>, radio: EspRadio<'static>) -> ! {
    ot.run(radio).await
}

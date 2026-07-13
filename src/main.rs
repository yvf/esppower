//! RCD Reset Controller - no-std (esp-hal) rewrite, ESP32-H2.
//!
//! Phase 3: Thread (OpenThread) AND BLE (trouble) running concurrently on the
//! shared H2 radio (the coexistence Matter needs - commission over BLE, operate
//! over Thread). BLE currently advertises a placeholder GATT service; Phase 4
//! replaces it with the Matter BTP service and wires the rs-matter-stack glue.
//! See docs/no-std-plan.md.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use log::info;
// `error` is only used by the controller task (skipped in the thread-only test).
#[cfg(not(feature = "thread-only-test"))]
use log::error;
// `warn` is only used on the Matter (BLE) path.
#[cfg(feature = "matter-ble")]
use log::warn;

use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ieee802154::Ieee802154;

use openthread::esp::EspRadio;
use openthread::{OpenThread, OtResources, OtSrpResources, OtUdpResources};
// RAM-only settings for the thread-only diagnostic; the normal build persists settings to
// flash via `matter::FlashSettings` (so the SRP key survives reboots).
#[cfg(not(feature = "matter-ble"))]
use openthread::SimpleRamSettings;

use esp_backtrace as _;
use esp_println as _;
use tinyrlibc as _;

esp_bootloader_esp_idf::esp_app_desc!();

// Controller + its sensor/actuator/config are skipped in the thread-only radio test.
#[cfg(not(feature = "thread-only-test"))]
mod actuator; // L12 linear actuator over LEDC (GPIO10)
#[cfg(not(feature = "thread-only-test"))]
mod config; // power-monitor / actuator timing constants
#[cfg(not(feature = "thread-only-test"))]
mod controller; // EMF power monitor + auto-reset state machine
#[cfg(not(feature = "thread-only-test"))]
mod link; // shared state between the controller and the Matter device-model handlers
#[cfg(feature = "matter-ble")]
mod matter; // Phase 4b transport glue (BLE/Thread Matter stack; needs the `ble` feature)
#[cfg(not(feature = "thread-only-test"))]
mod sensor; // contactless EMF power-presence sensor over ADC1 (GPIO4)

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

// --- Thread (OpenThread) sizing ----------------------------------------------
const UDP_MAX_SOCKETS: usize = 2;
const UDP_SOCKETS_BUF: usize = 1280;
const SRP_MAX_SERVICES: usize = 2;
const SRP_SERVICE_BUF: usize = 300;

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_alloc::heap_allocator!(size: 64 * 1024);
    esp_println::logger::init_logger_from_env();
    info!("RCD no-std (Thread + BLE coex) starting...");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(
        timg0.timer0,
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT)
            .software_interrupt0,
    );

    // -- Thread ----------------------------------------------------------------
    let rng = mk_static!(Rng, Rng::new());

    // Stable device identity from the chip's factory MAC. This MUST be constant across
    // reboots - it derives the Thread extended (MAC) address, the SRP hostname, and the
    // BLE address. A random EUI-64 each boot makes the device look brand-new to the mesh
    // and the SRP server every time (stale AAAA records, re-registration conflicts).
    // Expand the 6-byte EUI-48 to an EUI-64 the standard way (insert 0xFF 0xFE).
    let mac = esp_hal::efuse::base_mac_address();
    let mac = mac.as_bytes();
    let ieee_eui64 = [mac[0], mac[1], mac[2], 0xFF, 0xFE, mac[3], mac[4], mac[5]];
    info!("Device EUI-64 (stable): {ieee_eui64:02x?}");

    let ot_resources = mk_static!(OtResources, OtResources::new());
    let ot_udp_resources =
        mk_static!(OtUdpResources<UDP_MAX_SOCKETS, UDP_SOCKETS_BUF>, OtUdpResources::new());
    let ot_srp_resources =
        mk_static!(OtSrpResources<SRP_MAX_SERVICES, SRP_SERVICE_BUF>, OtSrpResources::new());
    // OpenThread settings storage. The normal (matter-ble) build persists to flash so the
    // SRP key survives reboots (else re-registration collides with the border router's
    // stale lease -> OT_ERROR_DUPLICATED -> HomeKit can't reach the device). The thread-only
    // diagnostic keeps them in RAM.
    #[cfg(feature = "matter-ble")]
    let ot_settings: &'static mut dyn openthread::Settings =
        mk_static!(matter::FlashSettings, matter::FlashSettings::new().unwrap());
    #[cfg(not(feature = "matter-ble"))]
    let ot_settings: &'static mut dyn openthread::Settings = {
        let ot_settings_buf = mk_static!([u8; 1024], [0; 1024]);
        mk_static!(SimpleRamSettings, SimpleRamSettings::new(ot_settings_buf))
    };

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

    // -- Power monitor + auto-reset ----------------------------------------------
    // Runs independently of Matter/Thread: the safety function must work whether or
    // not the device is commissioned. Monitors the EMF sensor (GPIO4/ADC1) and drives
    // the L12 actuator (GPIO10/LEDC) on power loss.
    // Skipped in the thread-only radio test so the build matches the openthread esp
    // example exactly (isolates whether ADC/LEDC init disturbs 802.15.4 RX).
    #[cfg(not(feature = "thread-only-test"))]
    spawner.spawn(
        run_controller(
            peripherals.ADC1,
            peripherals.GPIO4,
            peripherals.LEDC,
            peripherals.GPIO10,
        )
        .unwrap(),
    );

    // Diagnostic: attach Thread directly, with BLE never initialized, to isolate the
    // 802.15.4 radio from BLE<->154 contention. Enabled via `--features thread-only-test`
    // + THREAD_TEST_DATASET (hex TLV). Never returns.
    #[cfg(feature = "thread-only-test")]
    thread_only_test(ot).await;

    // Bring the IPv6 interface up (link-local) so the Matter operational stack has
    // a netif to initialize against during BLE commissioning. Thread itself is NOT
    // attached here - the commissioner supplies the operational dataset over BLE and
    // `matter::OtNetCtl` applies it (set_active_dataset_tlv + enable_thread) then.
    #[cfg(feature = "matter-ble")]
    {
        ot.enable_ipv6(true).unwrap();

        // Factory-reset button on GPIO5 (momentary push-button to GND). Held for 3 s it
        // wipes the saved Matter/Thread pairing and reboots into BLE commissioning.
        spawner.spawn(run_reset_button(peripherals.GPIO5).unwrap());

        // Hand the BLE peripheral to `run_matter`: it (re)builds the BLE controller
        // internally so it can supervise and restart the Matter stack on an unforeseen
        // error (see its supervised run loop). This call returns only if the one-time
        // pre-loop init fails; the run loop itself never returns.
        info!("Starting Matter (commission over BLE, operate over Thread)...");
        if let Err(e) = matter::run_matter(ot, peripherals.BT, ieee_eui64, Rng::new()).await {
            warn!("Matter failed to initialize (before the supervised run loop): {e:?}");
        }
    }
}

/// Diagnostic: apply a hardcoded Thread dataset and attach, with BLE never brought up.
/// The dataset TLV comes from the `THREAD_TEST_DATASET` env var (hex) at build time -
/// it carries the network key, so keep it out of the repo / shell history. Logs the
/// device role every 2s so we can see whether attach succeeds when the 802.15.4 radio
/// has no BLE contention.
#[cfg(feature = "thread-only-test")]
async fn thread_only_test(ot: OpenThread<'static>) -> ! {
    use embassy_time::{Duration, Timer};

    const DATASET_HEX: &str = env!(
        "THREAD_TEST_DATASET",
        "thread-only-test needs THREAD_TEST_DATASET=<hex dataset TLV> at build time"
    );

    // Decode the hex TLV into a fixed buffer (datasets are ~100-110 bytes).
    let mut ds = [0u8; 256];
    let bytes = DATASET_HEX.as_bytes();
    let mut n = 0;
    let hexval = |c: u8| -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    };
    let mut i = 0;
    while i + 1 < bytes.len() && n < ds.len() {
        ds[n] = (hexval(bytes[i]) << 4) | hexval(bytes[i + 1]);
        n += 1;
        i += 2;
    }
    info!("THREAD-ONLY TEST: applying {n}-byte dataset (BLE disabled)");

    ot.set_active_dataset_tlv(&ds[..n]).unwrap();
    ot.enable_ipv6(true).unwrap();
    ot.enable_thread(true).unwrap();

    loop {
        let st = ot.net_status();
        info!(
            "THREAD-ONLY: role={:?} ip6={} ext_pan_id={:?}",
            st.role, st.ip6_enabled, st.ext_pan_id
        );
        Timer::after(Duration::from_secs(2)).await;
    }
}

// --- Thread tasks ------------------------------------------------------------

#[embassy_executor::task]
async fn run_ot(ot: OpenThread<'static>, radio: EspRadio<'static>) -> ! {
    ot.run(radio).await
}

// --- Power-monitor task --------------------------------------------------------

// --- Factory-reset button ------------------------------------------------------

/// Poll a momentary push-button on GPIO5 (active-low, internal pull-up). When held
/// continuously for 3 s, erase the persisted Matter/Thread pairing and reboot - the
/// device then comes up un-commissioned and re-enters BLE commissioning. Polling (rather
/// than edge interrupts) keeps it simple and debounces naturally.
#[cfg(feature = "matter-ble")]
#[embassy_executor::task]
async fn run_reset_button(pin: esp_hal::peripherals::GPIO5<'static>) {
    use embassy_time::{Duration, Timer};
    use esp_hal::gpio::{Input, InputConfig, Pull};

    let button = Input::new(pin, InputConfig::default().with_pull(Pull::Up));

    const POLL_MS: u64 = 50;
    const HOLD_MS: u64 = 3000;
    let mut held_ms: u64 = 0;

    loop {
        Timer::after(Duration::from_millis(POLL_MS)).await;
        if button.is_low() {
            held_ms += POLL_MS;
            if held_ms >= HOLD_MS {
                warn!("Factory reset: button held {}s - wiping pairing and rebooting", HOLD_MS / 1000);
                if let Err(e) = matter::wipe_pairing_data() {
                    warn!("Factory reset: wipe failed: {e:?} - rebooting anyway");
                }
                esp_hal::system::software_reset();
            }
        } else {
            held_ms = 0;
        }
    }
}

#[cfg(not(feature = "thread-only-test"))]
#[embassy_executor::task]
async fn run_controller(
    adc1: esp_hal::peripherals::ADC1<'static>,
    emf_pin: esp_hal::peripherals::GPIO4<'static>,
    ledc: esp_hal::peripherals::LEDC<'static>,
    actuator_pin: esp_hal::peripherals::GPIO10<'static>,
) -> ! {
    let sensor = sensor::EmfSensor::new(adc1, emf_pin);
    let actuator = match actuator::Actuator::new(ledc, actuator_pin) {
        Ok(a) => a,
        Err(e) => {
            error!("Controller: actuator init failed: {e:?} - power monitor disabled");
            core::future::pending().await
        }
    };
    let mut controller = controller::Controller::new(actuator, sensor);
    controller.run().await
}

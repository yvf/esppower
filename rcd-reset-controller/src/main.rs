//! RCD Reset Controller — entry point.
//!
//! Hardware:
//!   - ESP32-H2 (RISC-V, IEEE 802.15.4 / Thread radio + BLE)
//!   - Actuonix L12-50-210-12-I on GPIO10 (RC servo PWM, via 3.3→5 V level shifter)
//!   - EMF detection antenna + gain circuit on GPIO4 (read as ADC1_CH3)
//!   - SCT-013-000 + RBDimmer adapter on GPIO1 (ADC1_CH0) — optional CT backend
//!
//! Two independent threads, each driven by `esp_idf_svc::hal::task::block_on`:
//!   1. Matter thread — `esp-idf-matter` Thread+BLE stack; commissions into Apple
//!      Home and serves the data model. Runs in its own higher-priority thread
//!      with a large stack (the `async-io` reactor misbehaves on the low-priority
//!      ESP-IDF main task, and rs-matter futures are large).
//!   2. Controller (main task) — the RCD reset state machine; owns the actuator
//!      and power sensor. Runs independently of Thread/HomeKit connectivity so the
//!      device keeps resetting the breaker even with no network.
//!
//! The two communicate over static `embassy-sync` channels (thread-safe via
//! `CriticalSectionRawMutex`). NOTE (Stage 1): the Matter side does not yet read
//! or write these channels — that wiring is Stage 2.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use log::{error, info};

use rcd_reset_controller::actuator::Actuator;
use rcd_reset_controller::config;
use rcd_reset_controller::controller::Controller;
use rcd_reset_controller::matter::{self, ToController, ToMatter};
use rcd_reset_controller::sensor::ActiveSensor;

// ─── Static inter-task channels ───────────────────────────────────────────────

static CTRL_CHANNEL: Channel<CriticalSectionRawMutex, ToController, 4> = Channel::new();
static MATTER_CHANNEL: Channel<CriticalSectionRawMutex, ToMatter, 4> = Channel::new();

/// Stack for the Matter thread. rs-matter futures are large; the reference uses
/// 20 KB (can drop to ~15 KB on esp32c6).
const MATTER_THREAD_STACK: usize = 20 * 1024;

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> Result<(), anyhow::Error> {
    // Link the ESP-IDF runtime patches and route logging through ESP-IDF.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!(
        "RCD Reset Controller v{} starting",
        config::MATTER_SW_VERSION_STR
    );

    // Split the peripherals: the radio modem goes to the Matter thread; the LEDC
    // PWM, ADC, and GPIOs go to the controller on the main task.
    let peripherals = Peripherals::take()?;
    let modem = peripherals.modem;
    let ledc = peripherals.ledc;
    let gpio10 = peripherals.pins.gpio10;
    #[cfg(feature = "sensor-emf")]
    let (adc1, sensor_pin) = (peripherals.adc1, peripherals.pins.gpio4);
    #[cfg(feature = "sensor-ct")]
    let (adc1, sensor_pin) = (peripherals.adc1, peripherals.pins.gpio1);

    let ctrl_rx = CTRL_CHANNEL.receiver();
    let matter_tx = MATTER_CHANNEL.sender();

    // ── Matter thread (Thread + BLE commissioning) ────────────────────────────
    // Run in a higher-priority, large-stacked thread (see module docs).
    ThreadSpawnConfiguration {
        name: Some(c"matter"),
        ..Default::default()
    }
    .set()?;

    std::thread::Builder::new()
        .stack_size(MATTER_THREAD_STACK)
        .spawn(move || {
            if let Err(e) = block_on(matter::node::run(modem)) {
                error!("Matter stack exited with error: {e:?}");
            }
        })?;

    // ── Controller (RCD reset state machine) on the main task ─────────────────
    let actuator = Actuator::new(ledc.timer0, ledc.channel0, gpio10)
        .map_err(|e| anyhow::anyhow!("Actuator init failed: {e}"))?;

    let power_sensor: ActiveSensor<'static> = ActiveSensor::new(adc1, sensor_pin)
        .map_err(|e| anyhow::anyhow!("Power sensor init failed: {e}"))?;

    info!("All subsystems initialized; entering control loop");

    let mut controller = Controller::new(actuator, power_sensor, matter_tx);
    block_on(controller.run(ctrl_rx)); // never returns
}

//! RCD Reset Controller — entry point.
//!
//! Hardware:
//!   - ESP32-H2 (RISC-V, IEEE 802.15.4 / Thread radio)
//!   - Actuonix L12-50-210-12-I on GPIO10 (RC servo PWM, via 3.3→5 V level shifter)
//!   - SCT-013-000 + RBDimmer adapter on GPIO1 (ADC1_CH1, 11 dB attenuation) (Optional)
//!   - EMF detection antenna + gain circuit on GPIO4 (as an ADC)
//!
//! Two Embassy async tasks run concurrently on ESP-IDF FreeRTOS:
//!   1. `controller_task` — state machine; owns actuator + sensor drivers
//!   2. `matter_task`     — rs-matter node; handles HomeKit commands
//!
//! Channel pairs:
//!   ctrl_tx / ctrl_rx   — Matter → Controller (manual trigger commands)
//!   matter_tx / matter_rx — Controller → Matter (attribute state updates)

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::{
    nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault},
};
use log::info;

use rcd_reset_controller::actuator::Actuator;
use rcd_reset_controller::config;
use rcd_reset_controller::controller::Controller;
use rcd_reset_controller::matter::{self, ToController, ToMatter};
use rcd_reset_controller::sensor::ActiveSensor;

// ─── Static inter-task channels ───────────────────────────────────────────────

static CTRL_CHANNEL: Channel<CriticalSectionRawMutex, ToController, 4> = Channel::new();
static MATTER_CHANNEL: Channel<CriticalSectionRawMutex, ToMatter, 4> = Channel::new();

// ─── Entry point ─────────────────────────────────────────────────────────────

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("RCD Reset Controller v{} starting", config::MATTER_SW_VERSION_STR);

    let peripherals = Peripherals::take().unwrap();
    let nvs_partition = EspDefaultNvsPartition::take().unwrap();
    let nvs = EspNvs::new(nvs_partition, "rcd", true).unwrap();

    // ── Actuator (LEDC PWM on GPIO10) ─────────────────────────────────────────
    let actuator = Actuator::new(
        peripherals.ledc.timer0,
        peripherals.ledc.channel0,
        peripherals.pins.gpio10,
    )
    .expect("Actuator init failed");

    // ── Power sensor ──────────────────────────────────────────────────────────
    // Backend selected at compile time (see `sensor` module + Cargo features).
    // EMF (default): contactless detector on ADC1_CH3 (GPIO4), 12 dB attenuation.
    // CT: SCT-013 current transformer on ADC1_CH0 (GPIO1), 12 dB attenuation.
    #[cfg(feature = "sensor-emf")]
    let power_sensor: ActiveSensor<'static> =
        ActiveSensor::new(peripherals.adc1, peripherals.pins.gpio4)
            .expect("Power sensor init failed");

    #[cfg(feature = "sensor-ct")]
    let power_sensor: ActiveSensor<'static> =
        ActiveSensor::new(peripherals.adc1, peripherals.pins.gpio1)
            .expect("Power sensor init failed");

    // ── Channel endpoints ─────────────────────────────────────────────────────
    let ctrl_tx   = CTRL_CHANNEL.sender();
    let ctrl_rx   = CTRL_CHANNEL.receiver();
    let matter_tx = MATTER_CHANNEL.sender();
    let matter_rx = MATTER_CHANNEL.receiver();

    // ── Spawn async tasks ─────────────────────────────────────────────────────
    spawner.spawn(controller_task(actuator, power_sensor, matter_tx, ctrl_rx).expect("controller spawn"));

    spawner.spawn(matter_task_entry(matter_rx, ctrl_tx, nvs).expect("matter spawn"));

    info!("All tasks running");
}

// ─── Controller task ──────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn controller_task(
    actuator: Actuator<'static>,
    sensor: ActiveSensor<'static>,
    matter_tx: embassy_sync::channel::Sender<'static, CriticalSectionRawMutex, ToMatter, 4>,
    ctrl_rx: embassy_sync::channel::Receiver<'static, CriticalSectionRawMutex, ToController, 4>,
) {
    let mut controller = Controller::new(actuator, sensor, matter_tx);
    controller.run(ctrl_rx).await;
}

// ─── Matter task ─────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn matter_task_entry(
    matter_rx: embassy_sync::channel::Receiver<'static, CriticalSectionRawMutex, ToMatter, 4>,
    ctrl_tx: embassy_sync::channel::Sender<'static, CriticalSectionRawMutex, ToController, 4>,
    nvs: EspNvs<NvsDefault>,
) {
    matter::node::matter_task(matter_rx, ctrl_tx, nvs).await;
}

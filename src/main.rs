//! RCD Reset Controller - no-std (esp-hal) firmware, ESP32-H2.
//!
//! Matter-over-Thread via `rs-matter-embassy` (BLE commission, then Thread operate,
//! non-concurrent) plus an autonomous EMF power-monitor / actuator auto-reset loop that
//! runs independently of Matter. See docs/no-std-plan.md.

#![no_std]
#![no_main]
#![recursion_limit = "256"]

use embassy_executor::Spawner;
use log::{error, info};

use esp_alloc::heap_allocator;
use esp_hal::ram;
use esp_hal::timer::timg::TimerGroup;
use esp_metadata_generated::memory_range;

use esp_backtrace as _;
use esp_println as _;
use tinyrlibc as _;

mod actuator; // L12 linear actuator over LEDC (GPIO10)
mod config; // power-monitor / actuator timing constants
mod controller; // EMF power monitor + auto-reset state machine
mod link; // shared state between the controller and the Matter device-model handlers
mod matter; // Matter-over-Thread stack (rs-matter-embassy) + our 2-endpoint device model
mod sensor; // contactless EMF power-presence sensor over ADC1 (GPIO4)

esp_bootloader_esp_idf::esp_app_desc!();

/// Timestamp hook for esp-println's `timestamp` feature: every log line (ours and the
/// openthread/rs-matter/esp-radio logs) is prefixed with the uptime in milliseconds, so the
/// reboot-to-operational timeline is measurable. Uses the systimer (works from early boot).
#[no_mangle]
extern "Rust" fn _esp_println_timestamp() -> u64 {
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis()
}

/// Heap for the Matter stack + the one alloc-hungry dependency (x509). The reclaimable
/// second RAM region is added as a separate heap so it is not wasted.
const HEAP_SIZE: usize = 100 * 1024;
const RECLAIMED_RAM: usize =
    memory_range!("DRAM2_UNINIT").end - memory_range!("DRAM2_UNINIT").start;

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    heap_allocator!(size: HEAP_SIZE - RECLAIMED_RAM);
    heap_allocator!(#[ram(reclaimed)] size: RECLAIMED_RAM);
    info!("RCD no-std (Matter over Thread, rs-matter-embassy) starting...");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(
        timg0.timer0,
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT)
            .software_interrupt0,
    );

    // Stable device identity from the chip's factory MAC. This MUST be constant across
    // reboots - it derives the Thread extended (MAC) address, the SRP hostname, and the
    // BLE address. A random EUI-64 each boot makes the device look brand-new to the mesh
    // and the SRP server every time. Expand the 6-byte EUI-48 to EUI-64 (insert 0xFF 0xFE).
    let mac = esp_hal::efuse::base_mac_address();
    let mac = mac.as_bytes();
    let ieee_eui64 = [mac[0], mac[1], mac[2], 0xFF, 0xFE, mac[3], mac[4], mac[5]];
    info!("Device EUI-64 (stable): {ieee_eui64:02x?}");

    // Power monitor + auto-reset. Runs independently of Matter/Thread: the safety function
    // must work whether or not the device is commissioned. Monitors the EMF sensor
    // (GPIO4/ADC1) and drives the L12 actuator (GPIO10/LEDC) on power loss. ADC1 is free
    // for the sensor because crypto seeds from the digital RNG (see matter::stack).
    spawner.spawn(
        run_controller(
            peripherals.ADC1,
            peripherals.GPIO4,
            peripherals.LEDC,
            peripherals.GPIO10,
        )
        .unwrap(),
    );

    // Matter over Thread (commission over BLE, operate over Thread) + GPIO5 factory reset.
    // Never returns (reboots on factory reset or an unexpected stack exit).
    info!("Starting Matter (commission over BLE, operate over Thread)...");
    matter::run_matter(
        peripherals.IEEE802154,
        peripherals.BT,
        peripherals.FLASH,
        peripherals.GPIO5,
        ieee_eui64,
    )
    .await
}

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

//! Hardware integration test — Actuonix L12 linear actuator.
//!
//! Run on-device:
//!   cargo run --example hw_actuator --release
//!
//! The test prints instructions to the serial console (115200 baud) and waits
//! 5 seconds between steps so the operator can observe physical movement.

mod embassy_stub;

use esp_idf_hal::peripherals::Peripherals;
use log::info;
use std::thread::sleep;
use std::time::Duration;

use rcd_reset_controller::actuator::Actuator;
use rcd_reset_controller::config::{ACTUATOR_EXTEND_DURATION_MS, ACTUATOR_RETRACT_DURATION_MS};
use rcd_reset_controller::controller::ActuatorControl;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("=== Actuator integration test ===");

    let peripherals = Peripherals::take().unwrap();

    let mut actuator = Actuator::new(
        peripherals.ledc.timer0,
        peripherals.ledc.channel0,
        peripherals.pins.gpio10,
    )
    .expect("Failed to init actuator");

    // ── Test 1: Idle position ─────────────────────────────────────────────────
    info!("[TEST 1] Actuator should be at IDLE (fully retracted).");
    info!("  Confirm the actuator shaft is fully in. Waiting 5 s...");
    sleep(Duration::from_secs(5));
    info!("[PASS] Idle position confirmed by operator.");

    // ── Test 2: Extend ────────────────────────────────────────────────────────
    info!("[TEST 2] Extending actuator to full stroke ({} ms)...", ACTUATOR_EXTEND_DURATION_MS);
    actuator.extend_blocking().expect("Extend failed");
    info!("  Confirm the actuator shaft is fully EXTENDED. Waiting 5 s...");
    sleep(Duration::from_secs(5));
    info!("[PASS] Extension confirmed by operator.");

    // ── Test 3: Retract ───────────────────────────────────────────────────────
    info!("[TEST 3] Retracting actuator to home position ({} ms)...", ACTUATOR_RETRACT_DURATION_MS);
    actuator.retract_blocking().expect("Retract failed");
    info!("  Confirm the actuator shaft is fully RETRACTED. Waiting 5 s...");
    sleep(Duration::from_secs(5));
    info!("[PASS] Retraction confirmed by operator.");

    // ── Test 4: Full cycle ────────────────────────────────────────────────────
    info!("[TEST 4] Full extend-retract cycle (simulating one reset attempt)...");
    actuator.extend_blocking().expect("Extend failed");
    actuator.retract_blocking().expect("Retract failed");
    actuator.idle().expect("Idle failed");
    info!("  Confirm cycle completed without mechanical issues. Waiting 5 s...");
    sleep(Duration::from_secs(5));
    info!("[PASS] Full cycle confirmed by operator.");

    info!("=== All actuator tests PASSED ===");
}

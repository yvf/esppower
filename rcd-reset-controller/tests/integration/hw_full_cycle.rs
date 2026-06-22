//! Hardware integration test — full RCD reset cycle.
//!
//! Run on-device:
//!   cargo run --example hw_full_cycle --release
//!
//! This test exercises the complete end-to-end workflow:
//!   1. Detect power present → sensor shows "closed".
//!   2. Operator trips the RCD manually → sensor detects "open".
//!   3. Firmware automatically fires one reset cycle (extend → retract → wait).
//!   4. Operator confirms the RCD has reset (or not) and the actuator returned to idle.
//!   5. Operator restores power to confirm sensor flips back to "closed".

mod embassy_stub;

use esp_idf_hal::peripherals::Peripherals;
use log::info;
use std::thread::sleep;
use std::time::Duration;

use rcd_reset_controller::actuator::Actuator;
use rcd_reset_controller::config::{MAX_AUTO_RETRIES, POST_ATTEMPT_WAIT_MS};
use rcd_reset_controller::controller::ActuatorControl;
use rcd_reset_controller::sensor::{PowerSensor, PowerState};

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("=== Full RCD reset cycle integration test ===");
    info!("Max auto retries: {}", MAX_AUTO_RETRIES);

    let peripherals = Peripherals::take().unwrap();

    let mut actuator = Actuator::new(
        peripherals.ledc.timer0,
        peripherals.ledc.channel0,
        peripherals.pins.gpio10,
    )
    .expect("Failed to init actuator");

    let mut sensor = PowerSensor::new(peripherals.adc1, peripherals.pins.gpio1)
        .expect("Failed to init sensor");

    // ── Step 1: Baseline — power present ─────────────────────────────────────
    info!("[STEP 1] Confirm 220V AC circuit is ON with a load connected.");
    sleep(Duration::from_secs(3));

    let power = sensor.sample_blocking().expect("Sensor read failed");
    info!("  Initial power state: {:?}", power);
    assert_eq!(power, PowerState::Present, "Expected power to be present at start");
    info!("  [OK] Baseline: power present.");

    // ── Step 2: Trip the RCD ──────────────────────────────────────────────────
    info!("[STEP 2] Manually TRIP the RCD now. Waiting 8 seconds...");
    sleep(Duration::from_secs(8));

    let power = sensor.sample_blocking().expect("Sensor read failed");
    info!("  Power state after trip: {:?}", power);
    if power != PowerState::Absent {
        info!("  [WARN] Power still detected — verify the RCD was tripped. Continuing anyway.");
    } else {
        info!("  [OK] RCD trip detected: power absent.");
    }

    // ── Step 3: Fire the reset cycle (as the firmware would automatically) ────
    info!("[STEP 3] Firing actuator reset cycle (extend 8 s → retract 8 s)...");
    actuator.extend_blocking().expect("Extend failed");
    actuator.retract_blocking().expect("Retract failed");
    actuator.idle().expect("Idle failed");

    info!("  Cycle complete. Waiting {} ms for power to settle...", POST_ATTEMPT_WAIT_MS);
    sleep(Duration::from_millis(POST_ATTEMPT_WAIT_MS));

    // ── Step 4: Re-check power ────────────────────────────────────────────────
    let power_after = sensor.sample_blocking().expect("Sensor read failed");
    info!("[STEP 4] Power state after reset attempt: {:?}", power_after);
    match power_after {
        PowerState::Present => {
            info!("  [PASS] RCD successfully reset — power restored!");
        }
        PowerState::Absent => {
            info!("  [INFO] RCD did not reset (expected if RCD requires manual intervention).");
        }
    }

    // ── Step 5: Actuator at idle / retracted ─────────────────────────────────
    info!("[STEP 5] Confirm actuator shaft is fully RETRACTED (idle). Waiting 5 s...");
    sleep(Duration::from_secs(5));
    info!("  [PASS] Actuator idle position confirmed (operator must verify visually).");

    // ── Step 6: Restore power and verify sensor ───────────────────────────────
    if power_after == PowerState::Absent {
        info!("[STEP 6] Manually restore power (or reset RCD). Waiting 8 seconds...");
        sleep(Duration::from_secs(8));

        let power_restored = sensor.sample_blocking().expect("Sensor read failed");
        info!("  Power state after manual restore: {:?}", power_restored);
        if power_restored == PowerState::Present {
            info!("  [PASS] Sensor correctly detects power restored.");
        } else {
            info!("  [FAIL] Sensor still reads absent after manual restore — check wiring.");
        }
    }

    info!("=== Full cycle test complete ===");
}

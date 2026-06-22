//! Hardware integration test — SCT-013 CT power sensor.
//!
//! Run on-device:
//!   cargo run --example hw_sensor --release
//!
//! Prerequisites:
//!   - RBDimmer SCT-013 adapter wired to GPIO1 (ADC1_CH1).
//!   - CT clamp around one live wire of a 220 V AC circuit under load.

mod embassy_stub;

use esp_idf_hal::peripherals::Peripherals;
use log::info;
use std::thread::sleep;
use std::time::Duration;

use rcd_reset_controller::config::{CT_DETECTION_THRESHOLD, CT_SAMPLE_COUNT};
use rcd_reset_controller::sensor::{PowerSensor, PowerState};

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("=== CT Sensor integration test ===");
    info!("Threshold: {} ADC counts", CT_DETECTION_THRESHOLD);
    info!("Samples per reading: {}", CT_SAMPLE_COUNT);

    let peripherals = Peripherals::take().unwrap();
    let mut sensor = PowerSensor::new(peripherals.adc1, peripherals.pins.gpio1)
        .expect("Failed to init sensor");

    // ── Test 1: AC circuit PRESENT ────────────────────────────────────────────
    info!("[TEST 1] Ensure the 220V AC circuit is ON and under load.");
    info!("  Waiting 5 seconds then sampling...");
    sleep(Duration::from_secs(5));

    let state = sensor.sample_blocking().expect("Sensor read failed");
    match state {
        PowerState::Present => info!("[PASS] Power correctly detected as PRESENT."),
        PowerState::Absent => {
            info!("[FAIL] Power NOT detected. Check CT clamp and wiring.");
            info!("  Tip: Increase load on the circuit, or lower CT_DETECTION_THRESHOLD.");
        }
    }

    // ── Test 2: AC circuit ABSENT ─────────────────────────────────────────────
    info!("[TEST 2] Switch the 220V AC circuit OFF (trip the RCD or switch off).");
    info!("  Waiting 5 seconds then sampling...");
    sleep(Duration::from_secs(5));

    let state = sensor.sample_blocking().expect("Sensor read failed");
    match state {
        PowerState::Absent => info!("[PASS] Power correctly detected as ABSENT."),
        PowerState::Present => {
            info!("[FAIL] Power still detected as PRESENT after switch-off.");
            info!("  Tip: Increase CT_DETECTION_THRESHOLD in config.rs.");
        }
    }

    // ── Test 3: 10 consecutive samples when circuit is back on ───────────────
    info!("[TEST 3] Switch the 220V circuit back ON. Sampling 10 times over 10 seconds.");
    sleep(Duration::from_secs(3));

    let mut pass_count = 0u32;
    for i in 0..10 {
        let state = sensor.sample_blocking().expect("Sensor read failed");
        let ok = state == PowerState::Present;
        info!("  Sample {}/10: {:?} — {}", i + 1, state, if ok { "OK" } else { "UNEXPECTED" });
        if ok {
            pass_count += 1;
        }
        sleep(Duration::from_millis(500));
    }

    if pass_count == 10 {
        info!("[PASS] All 10 samples correctly detected power as PRESENT.");
    } else {
        info!("[WARN] Only {}/10 samples detected power — consider reducing threshold.", pass_count);
    }

    info!("=== CT Sensor integration test complete ===");
}

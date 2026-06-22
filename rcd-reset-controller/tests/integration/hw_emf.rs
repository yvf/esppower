//! Hardware integration test — contactless BC547 EMF power sensor.
//!
//! Run on-device (this is the default sensor backend):
//!   cargo run --example hw_emf --release
//!
//! Prerequisites:
//!   - 3-stage BC547 EMF-detector cascade wired per `220AC_EMF_detector.md`,
//!     output through a 10 kΩ series resistor into GPIO4 (read as ADC1 channel 3).
//!   - Spiral copper antenna on the base of Q1; 1–10 MΩ bleeder to GND.
//!   - To exercise "present", bring the antenna near (or coil it around the outer
//!     insulation of) a live 220 V AC conductor under power.

mod embassy_stub;

use esp_idf_hal::peripherals::Peripherals;
use log::info;
use std::thread::sleep;
use std::time::Duration;

use rcd_reset_controller::config::{EMF_DETECTION_THRESHOLD, EMF_SAMPLE_COUNT};
use rcd_reset_controller::sensor::{EmfSensor, PowerState};

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("=== EMF Sensor integration test ===");
    info!(
        "Peak-to-peak detection threshold: {} ADC counts",
        EMF_DETECTION_THRESHOLD
    );
    info!("Samples per window: {} (~40 ms)", EMF_SAMPLE_COUNT);

    let peripherals = Peripherals::take().unwrap();
    let mut sensor = EmfSensor::new(peripherals.adc1, peripherals.pins.gpio4)
        .expect("Failed to init EMF sensor");

    // ── Test 1: Field PRESENT ─────────────────────────────────────────────────
    info!("[TEST 1] Hold the antenna against a LIVE 220V AC cable.");
    info!("  Waiting 5 seconds then sampling...");
    sleep(Duration::from_secs(5));

    let state = sensor.sample_blocking().expect("Sensor read failed");
    match state {
        PowerState::Present => info!("[PASS] EMF correctly detected as PRESENT."),
        PowerState::Absent => {
            info!("[FAIL] EMF NOT detected. Check antenna placement and cascade wiring.");
            info!("  Tip: lengthen/coil the antenna, or lower EMF_DETECTION_THRESHOLD.");
        }
    }

    // ── Test 2: Field ABSENT ──────────────────────────────────────────────────
    info!("[TEST 2] Move the antenna well away from any mains cable (or kill power).");
    info!("  Waiting 5 seconds then sampling...");
    sleep(Duration::from_secs(5));

    let state = sensor.sample_blocking().expect("Sensor read failed");
    match state {
        PowerState::Absent => info!("[PASS] EMF correctly detected as ABSENT."),
        PowerState::Present => {
            info!("[FAIL] EMF still detected after moving away.");
            info!("  Tip: shorten the antenna or lower the bleeder resistor (faster discharge),");
            info!("       or raise EMF_DETECTION_THRESHOLD in config.rs.");
        }
    }

    // ── Test 3: 10 consecutive samples while field is present ─────────────────
    info!("[TEST 3] Hold the antenna on the live cable again. Sampling 10 times.");
    sleep(Duration::from_secs(3));

    let mut pass_count = 0u32;
    for i in 0..10 {
        let state = sensor.sample_blocking().expect("Sensor read failed");
        let ok = state == PowerState::Present;
        info!(
            "  Sample {}/10: {:?} — {}",
            i + 1,
            state,
            if ok { "OK" } else { "UNEXPECTED" }
        );
        if ok {
            pass_count += 1;
        }
        sleep(Duration::from_millis(500));
    }

    if pass_count == 10 {
        info!("[PASS] All 10 samples correctly detected EMF as PRESENT.");
    } else {
        info!(
            "[WARN] Only {}/10 samples detected EMF — adjust antenna or threshold.",
            pass_count
        );
    }

    info!("=== EMF Sensor integration test complete ===");
}

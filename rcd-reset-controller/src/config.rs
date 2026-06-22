//! Compile-time configuration constants for the RCD Reset Controller.

// ─── Pin assignments (reference only — consumed by HAL peripheral types, not by name) ──

/// GPIO1 = ADC1 channel 1.  Connected to RBDimmer SCT-013 adapter SIG pin.
#[cfg(feature = "sensor-ct")]
#[allow(dead_code)]
pub const CT_SENSOR_ADC_PIN: u8 = 1;

/// GPIO4 = ADC1 channel 3. Analog input from the BC547 EMF-detector cascade
/// (via 10 kΩ series resistor). Read as an ADC, not a digital pin — see `emf`.
#[cfg(feature = "sensor-emf")]
#[allow(dead_code)]
pub const EMF_SENSOR_PIN: u8 = 4;

/// GPIO10: LEDC PWM output → 3.3 V→5 V level shifter → L12 White wire (RC signal).
#[allow(dead_code)]
pub const ACTUATOR_PWM_PIN: u8 = 10;

// ─── Actuator RC-servo timing ────────────────────────────────────────────────

/// LEDC PWM frequency for standard RC servo protocol.
pub const SERVO_FREQ_HZ: u32 = 50;

/// Pulse width for fully retracted position (1 ms at 50 Hz = 5 % duty).
/// With 14-bit timer resolution (max duty = 16383): 16383 * 5% ≈ 819.
pub const SERVO_DUTY_RETRACTED: u32 = 819;

/// Pulse width for fully extended position (2 ms at 50 Hz = 10 % duty).
pub const SERVO_DUTY_EXTENDED: u32 = 1638;

/// LEDC timer resolution bits (14-bit → max duty = 2^14 - 1 = 16383).
pub const SERVO_TIMER_RESOLUTION_BITS: u8 = 14;

// ─── Actuator cycle timing ────────────────────────────────────────────────────

/// Time to hold the actuator fully extended (210:1 gear, 50 mm stroke ≈ 7.7 s).
pub const ACTUATOR_EXTEND_DURATION_MS: u64 = 8_000;

/// Time to hold the actuator fully retracted (same speed, reverse).
pub const ACTUATOR_RETRACT_DURATION_MS: u64 = 8_000;

/// Settling time after a reset attempt before re-reading the power sensor.
pub const POST_ATTEMPT_WAIT_MS: u64 = 2_000;

/// Maximum number of automatic retry attempts after initial power-loss trigger.
/// 1 retry = 2 total attempts (initial + 1 retry).
pub const MAX_AUTO_RETRIES: u8 = 1;

// ─── Power-sensor polling (shared by all backends) ───────────────────────────

/// How often the controller's polling task re-samples the power sensor (ms).
pub const POWER_POLL_INTERVAL_MS: u64 = 500;

// ─── CT power sensor (feature = "sensor-ct") ─────────────────────────────────

/// Number of ADC samples per detection window.
/// At 100 µs between samples → 200 samples = 20 ms (one full 50 Hz cycle).
#[cfg(feature = "sensor-ct")]
pub const CT_SAMPLE_COUNT: usize = 200;

/// Delay between individual ADC samples (microseconds).
#[cfg(feature = "sensor-ct")]
pub const CT_SAMPLE_INTERVAL_US: u64 = 100;

/// Peak-to-peak ADC count threshold above which AC current is considered present.
/// 12-bit ADC, 11 dB attenuation (≈ 0–3.9 V full scale → 4095 counts).
/// The RBDimmer adapter centres the AC signal at ~1.65 V (≈ 2048 counts).
/// A 10 A load on the 100 A/1 V SCT-013 produces ~0.1 V swing → ~105 counts.
/// Set threshold conservatively at 80 counts to handle light loads.
#[cfg(feature = "sensor-ct")]
pub const CT_DETECTION_THRESHOLD: u16 = 80;

// ─── EMF power sensor (feature = "sensor-emf") ───────────────────────────────

/// Number of ADC samples per detection window.
/// At 100 µs between samples → 400 samples = 40 ms (≈ 2 full 50 Hz cycles).
#[cfg(feature = "sensor-emf")]
pub const EMF_SAMPLE_COUNT: usize = 400;

/// Delay between individual ADC reads inside the sampling window (microseconds).
#[cfg(feature = "sensor-emf")]
pub const EMF_SAMPLE_INTERVAL_US: u64 = 100;

/// Peak-to-peak ADC count threshold above which a live 50 Hz field is considered
/// present. 12-bit ADC, 12 dB attenuation (≈ 0–3.9 V full scale → 4095 counts,
/// ≈ 1.05 counts/mV).
///
/// Calibrated from scope measurements at the cascade output node (`220AC_EMF_detector.md`):
///   - field present : ~750 mV pp ≈ 790 counts
///   - field absent   : ~300 mV pp ≈ 315 counts (ambient noise floor)
/// The threshold sits at the midpoint (~525 mV ≈ 550 counts), leaving ~235 counts
/// of margin to the noise floor. Raise it if ambient hum causes false positives;
/// lower it if a real field is missed.
#[cfg(feature = "sensor-emf")]
pub const EMF_DETECTION_THRESHOLD: u16 = 550;

// ─── Matter commissioning ─────────────────────────────────────────────────────

/// Matter Vendor ID (0xFFF1 = test/development vendor).
/// Replace with your certified Vendor ID before production.
pub const MATTER_VENDOR_ID: u16 = 0xFFF1;

/// Matter Product ID.
pub const MATTER_PRODUCT_ID: u16 = 0x8001;

/// Matter product name (shown in HomeKit).
pub const MATTER_PRODUCT_NAME: &str = "RCD Reset Controller";

/// Matter vendor name.
pub const MATTER_VENDOR_NAME: &str = "DIY";

/// Matter software version (BCD, displayed in HomeKit accessory detail).
pub const MATTER_SW_VERSION: u32 = 1;
pub const MATTER_SW_VERSION_STR: &str = "0.1.0";

/// Discriminator: 12-bit value used during commissioning discovery.
/// Choose a unique value per device in production (can be stored in NVS).
pub const MATTER_DISCRIMINATOR: u16 = 3840;

/// Passcode: 27-bit setup code.  Use a random value in production.
pub const MATTER_PASSCODE: u32 = 20_202_021;

// ─── Matter data model identifiers ───────────────────────────────────────────

/// Matter device type: On/Off Plug-In Unit.
pub const DEV_TYPE_ON_OFF_PLUGIN_UNIT: u32 = 0x010A;

/// Matter device type: Contact Sensor.
pub const DEV_TYPE_CONTACT_SENSOR: u32 = 0x0015;

/// Matter endpoint IDs (reference constants; endpoint structs embed the values directly).
#[allow(dead_code)] pub const ENDPOINT_ROOT:   u16 = 0;
#[allow(dead_code)] pub const ENDPOINT_PLUG:   u16 = 1;
#[allow(dead_code)] pub const ENDPOINT_SENSOR: u16 = 2;

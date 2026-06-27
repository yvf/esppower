//! Compile-time configuration for the RCD controller's power-monitor / auto-reset
//! loop. Ported from the esp-idf `rcd-reset-controller` crate, adapted for esp-hal
//! (LEDC duty is expressed as an integer percentage rather than a raw 14-bit count).

// ─── Pin assignments (reference only — the HAL consumes the peripheral singletons) ──

/// GPIO4 = ADC1 channel 3. Analog input from the BC547 EMF-detector cascade
/// (via 10 kΩ series resistor). Read as an ADC, not a digital pin — see [`crate::sensor`].
/// Reference only: the HAL binds the pin by its peripheral singleton, not this number.
#[allow(dead_code)]
pub const EMF_SENSOR_PIN: u8 = 4;

/// GPIO10: LEDC PWM output → 3.3 V→5 V level shifter → L12 White wire (RC signal).
/// Reference only: the HAL binds the pin by its peripheral singleton, not this number.
#[allow(dead_code)]
pub const ACTUATOR_PWM_PIN: u8 = 10;

// ─── Actuator RC-servo timing ────────────────────────────────────────────────

/// LEDC PWM frequency for the standard RC-servo protocol (20 ms period).
pub const SERVO_FREQ_HZ: u32 = 50;

/// LEDC timer duty resolution. 14-bit (max count 16383) gives the servo pulse
/// fine-enough granularity while keeping 50 Hz well within range. Reference only:
/// the actuator selects the resolution via the `Duty14Bit` enum, not this constant.
#[allow(dead_code)]
pub const SERVO_TIMER_RESOLUTION_BITS: u8 = 14;

/// Duty for the fully-retracted position: 1 ms pulse at 50 Hz = 5 % duty.
/// (At 14-bit this is 16384 × 5 % ≈ 819, matching the esp-idf build's raw value.)
pub const SERVO_DUTY_RETRACTED_PCT: u8 = 5;

/// Duty for the fully-extended position: 2 ms pulse at 50 Hz = 10 % duty.
pub const SERVO_DUTY_EXTENDED_PCT: u8 = 10;

// ─── Actuator cycle timing ────────────────────────────────────────────────────

/// Time to hold the actuator fully extended (210:1 gear, 50 mm stroke ≈ 7.7 s).
pub const ACTUATOR_EXTEND_DURATION_MS: u64 = 8_000;

/// Time to hold the actuator fully retracted (same speed, reverse).
pub const ACTUATOR_RETRACT_DURATION_MS: u64 = 8_000;

/// Settling time after a reset attempt before re-reading the power sensor.
pub const POST_ATTEMPT_WAIT_MS: u64 = 2_000;

/// Maximum number of automatic retry attempts after the initial power-loss trigger.
/// 1 retry = 2 total attempts (initial + 1 retry).
pub const MAX_AUTO_RETRIES: u8 = 1;

// ─── Power-sensor polling ────────────────────────────────────────────────────

/// How often the controller re-samples the power sensor while idle (ms).
pub const POWER_POLL_INTERVAL_MS: u64 = 500;

// ─── EMF power sensor ────────────────────────────────────────────────────────

/// Number of ADC samples per detection window.
/// At 100 µs between samples → 400 samples = 40 ms (≈ 2 full 50 Hz cycles).
pub const EMF_SAMPLE_COUNT: usize = 400;

/// Delay between individual ADC reads inside the sampling window (microseconds).
pub const EMF_SAMPLE_INTERVAL_US: u64 = 100;

/// Peak-to-peak ADC count threshold above which a live 50 Hz field is "present".
/// 12-bit ADC, 11 dB attenuation (≈ 0–3.9 V full scale → 4095 counts, ≈ 1 count/mV).
///
/// Calibrated from scope measurements at the cascade output node (`220AC_CT_detector.md`):
///   - field present : ~750 mV pp ≈ 790 counts
///   - field absent  : ~300 mV pp ≈ 315 counts (ambient noise floor)
/// The threshold sits near the midpoint (~525 mV ≈ 550 counts), leaving ~235 counts
/// of margin to the noise floor. Raise it if ambient hum causes false positives;
/// lower it if a real field is missed.
pub const EMF_DETECTION_THRESHOLD: u16 = 550;

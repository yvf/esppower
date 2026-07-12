//! Compile-time configuration for the RCD controller's power-monitor / auto-reset loop:
//! pin references, actuator RC-servo timing, reset-trigger timing, and EMF-sensor
//! parameters. (LEDC duty is expressed as an integer percentage, not a raw 14-bit count.)

// ─── Pin assignments (reference only — the HAL consumes the peripheral singletons) ──

/// GPIO4 = ADC1 channel 3. Analog input from the LM358 EMF-amplifier output (over a
/// shielded cable). Read as a calibrated ADC, not a digital pin — see [`crate::sensor`]
/// and `220AC_EMF_Remote_detector.md`.
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
/// (At 14-bit resolution this is 16384 × 5 % ≈ 819 raw counts.)
pub const SERVO_DUTY_RETRACTED_PCT: u8 = 5;

/// Duty for the fully-extended position: 2 ms pulse at 50 Hz = 10 % duty.
pub const SERVO_DUTY_EXTENDED_PCT: u8 = 10;

// ─── Actuator cycle timing ────────────────────────────────────────────────────

/// Time to hold the actuator fully extended (210:1 gear, 50 mm stroke ≈ 7.7 s).
pub const ACTUATOR_EXTEND_DURATION_MS: u64 = 8_000;

/// Time to hold the actuator fully retracted (same speed, reverse).
pub const ACTUATOR_RETRACT_DURATION_MS: u64 = 8_000;

// ─── Reset-trigger timing ─────────────────────────────────────────────────────

/// After power loss is first detected, require this much *continuous* absence
/// before the first actuator activation. Debounces brief dropouts/sensor glitches;
/// power returning within this window cancels the reset entirely.
pub const POWER_LOSS_DEBOUNCE_MS: u64 = 60_000; // 60 s

/// After the first reset cycle, watch this long for power to return. If it stays
/// *continuously* absent for the whole window, run the second (final) reset cycle;
/// power returning within it means the first cycle worked → back to monitoring.
pub const POST_RESET_RECHECK_MS: u64 = 60_000; // 60 s

/// Minimum *continuous* presence before any "power restored" transition is accepted
/// (during the loss debounce and the post-cycle recheck). Stops a brief glitch on the
/// EMF sensor from prematurely flipping the state back to present.
pub const POWER_PRESENT_CONFIRM_MS: u64 = 2_000; // 2 s

/// After the second cycle the controller latches and stops actuating. It re-arms
/// (returns to normal monitoring) only once power has been *continuously present*
/// for this long, so a flickering supply can't immediately re-trigger.
pub const POWER_RESTORE_CONFIRM_MS: u64 = 10_000; // 10 s

// ─── Power-sensor polling ────────────────────────────────────────────────────

/// How often the controller re-samples the power sensor (ms). Also the resolution
/// at which power restoration is detected during the debounce and backoff waits.
pub const POWER_POLL_INTERVAL_MS: u64 = 500;

// ─── EMF power sensor ────────────────────────────────────────────────────────

/// Number of ADC samples per detection window.
/// At 100 µs between samples → 400 samples = 40 ms (≈ 2 full 50 Hz cycles).
pub const EMF_SAMPLE_COUNT: usize = 400;

/// Delay between individual ADC reads inside the sampling window (microseconds).
pub const EMF_SAMPLE_INTERVAL_US: u64 = 100;

/// Peak-to-peak threshold, in **millivolts**, above which a live 50 Hz field is
/// "present". The sensor uses a calibrated ADC read (`AdcCalLine`), so samples — and
/// hence this peak-to-peak value — are already in mV at the ADC pin (no count↔mV
/// conversion). See [`crate::sensor`].
///
/// Field-calibrated for the LM358 front end (with the 2.2 nF feedback low-pass):
///   - field absent  : pp mostly < 100 mV, occasional spikes to ~190 mV
///   - field present : pp ~200 mV (150–300 mV)
/// 95 mV sits just above the typical absent floor.
pub const EMF_DETECTION_THRESHOLD: u16 = 95;

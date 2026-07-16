//! Driver for the Actuonix L12-50-210-12-I linear actuator (esp-hal LEDC backend).
//!
//! Control interface: RC servo (standard hobby protocol).
//! - 50 Hz PWM carrier (20 ms period)
//! - 1 ms pulse (5 % duty)  -> fully retracted
//! - 2 ms pulse (10 % duty) -> fully extended
//! - Signal: 5 V CMOS via an external 3.3->5 V level shifter on GPIO10
//!
//! The `-I` integrated controller auto-detects RC-servo mode on power-up when it
//! sees a valid pulse on lead 4 (White wire).
//!
//! Duty updates go through `Channel::configure` rather than `Channel::set_duty`. On
//! the ESP32-H2 LEDC a duty change is applied via the "gamma" range RAM, and
//! esp-hal 1.1's bare `set_duty` does not reset the gamma write-pointer, so only the
//! *first* duty after channel setup reaches the output (the actuator buzzes once at
//! boot, then never moves). `configure` runs the full channel setup - including the
//! write-pointer reset - every time, so re-running it per duty change reliably
//! re-applies the new pulse width.

use embassy_time::{Duration, Timer};
use esp_hal::gpio::DriveMode;
use esp_hal::ledc::channel::{self, ChannelIFace};
use esp_hal::ledc::timer::{self, LSClockSource, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::peripherals::{GPIO10, LEDC};
use esp_hal::time::Rate;
use log::{info, warn};
use static_cell::StaticCell;

use crate::config::{
    ACTUATOR_EXTEND_DURATION_MS, ACTUATOR_RETRACT_DURATION_MS, SERVO_DUTY_EXTENDED_PCT,
    SERVO_DUTY_RETRACTED_PCT, SERVO_FREQ_HZ,
};

// The `Ledc` and `Timer` must outlive the `Channel` (which borrows the timer) and
// must keep running for as long as PWM is needed. Leaking them to `'static` via
// these cells avoids a self-referential struct and guarantees the timer counter is
// never dropped - dropping it would halt the counter and stop all PWM output, so
// the actuator would stop seeing valid RC pulses and drift to an undefined position.
static LEDC_CELL: StaticCell<Ledc<'static>> = StaticCell::new();
static TIMER_CELL: StaticCell<timer::Timer<'static, LowSpeed>> = StaticCell::new();

// NOTE: esp-hal 1.1 mis-computed the H2 LEDC timer divisor from `Clocks::get().apb_clock`
// while the real LowSpeed source is the 32 MHz XTAL, so we used to pre-scale the requested
// frequency by `apb_clock / 32 MHz` to achieve the true 50 Hz. The esp-hal snapshot pinned
// here (rev 10e48dd) restructured the clock API (`Clocks` is gone), so that workaround was
// removed and we now request `SERVO_FREQ_HZ` directly. TODO(hw): re-verify on hardware that
// the achieved LEDC frequency is really 50 Hz (scope the GPIO10 pulse ~1-2 ms); if it is
// ~1/3, the H2 LEDC clock bug persists and the pre-scale must be reinstated with the new
// clock accessor.

pub struct Actuator {
    channel: channel::Channel<'static, LowSpeed>,
    timer: &'static timer::Timer<'static, LowSpeed>,
}

impl Actuator {
    /// Initialise the LEDC peripheral for RC-servo output on GPIO10 and park the
    /// actuator in the retracted position.
    ///
    /// Returns an error only if the LEDC timer/channel configuration is rejected
    /// (e.g. an unattainable frequency/resolution combination - not expected here).
    pub fn new(ledc: LEDC<'static>, pin: GPIO10<'static>) -> Result<Self, channel::Error> {
        let ledc = LEDC_CELL.init(Ledc::new(ledc));
        ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

        let timer = TIMER_CELL.init(ledc.timer::<LowSpeed>(timer::Number::Timer0));
        timer
            .configure(timer::config::Config {
                duty: timer::config::Duty::Duty14Bit,
                clock_source: LSClockSource::APBClk,
                frequency: Rate::from_hz(SERVO_FREQ_HZ),
            })
            .map_err(|_| channel::Error::Timer)?;
        let timer: &'static timer::Timer<'static, LowSpeed> = timer;

        let channel = ledc.channel::<LowSpeed>(channel::Number::Channel0, pin);
        let mut actuator = Self { channel, timer };
        // Apply the initial (retracted) pulse via the same path used for every later
        // duty change.
        actuator.apply_duty(SERVO_DUTY_RETRACTED_PCT)?;
        info!(
            "Actuator: LEDC ready on GPIO10 @ {} Hz (retracted={}% extended={}%)",
            SERVO_FREQ_HZ, SERVO_DUTY_RETRACTED_PCT, SERVO_DUTY_EXTENDED_PCT
        );

        Ok(actuator)
    }

    /// (Re)configure the channel at the given duty %, which on the H2 is what reliably
    /// pushes a new pulse width to the output. Returns the LEDC error if rejected.
    fn apply_duty(&mut self, duty_pct: u8) -> Result<(), channel::Error> {
        self.channel.configure(channel::config::Config {
            timer: self.timer,
            duty_pct,
            drive_mode: DriveMode::PushPull,
        })
    }

    /// Set the duty %, logging (but not propagating) any LEDC error so a transient
    /// failure can't abort an in-progress reset cycle.
    fn set_duty(&mut self, duty_pct: u8) {
        if let Err(e) = self.apply_duty(duty_pct) {
            warn!("Actuator: set_duty({duty_pct}%) failed: {e:?}");
        }
    }

    /// Drive to the fully-extended position and hold until the stroke completes.
    pub async fn extend(&mut self) {
        info!("Actuator: extending");
        self.set_duty(SERVO_DUTY_EXTENDED_PCT);
        Timer::after(Duration::from_millis(ACTUATOR_EXTEND_DURATION_MS)).await;
    }

    /// Drive to the fully-retracted position and hold until the stroke completes.
    pub async fn retract(&mut self) {
        info!("Actuator: retracting");
        self.set_duty(SERVO_DUTY_RETRACTED_PCT);
        Timer::after(Duration::from_millis(ACTUATOR_RETRACT_DURATION_MS)).await;
    }

    /// Park retracted (resting position between cycles).
    pub fn idle(&mut self) {
        info!("Actuator: idle");
        self.set_duty(SERVO_DUTY_RETRACTED_PCT);
    }
}

//! Driver for the Actuonix L12-50-210-12-I linear actuator.
//!
//! Control interface: RC servo (standard hobby protocol).
//! - 50 Hz PWM carrier (20 ms period)
//! - 1 ms pulse → fully retracted
//! - 2 ms pulse → fully extended
//! - Signal voltage: 5 V CMOS (via external 3.3→5 V level shifter on GPIO10)
//!
//! The `-I` integrated controller auto-detects RC servo mode on power-up when
//! it sees a valid pulse on lead 4 (White wire).

use crate::config::{
    ACTUATOR_EXTEND_DURATION_MS, ACTUATOR_RETRACT_DURATION_MS, SERVO_DUTY_EXTENDED,
    SERVO_DUTY_RETRACTED, SERVO_FREQ_HZ,
};
use crate::controller::ActuatorControl;
use embassy_time::{Duration, Timer};
use esp_idf_hal::ledc::{
    config::TimerConfig, LedcDriver, LedcTimerDriver, LowSpeed, Resolution,
};
use esp_idf_hal::units::*;
use log::info;

pub struct Actuator<'d> {
    // The LEDC timer driver MUST be kept alive for as long as the channel is in
    // use. `LedcDriver` does not retain the timer it is built from, and dropping
    // a `LedcTimerDriver` calls `ledc_timer_rst()`, which halts the timer counter
    // and stops all PWM output (the channel would then hold a static level and the
    // actuator would never see a valid RC pulse). Storing it here prevents that.
    _timer: LedcTimerDriver<'d, LowSpeed>,
    driver: LedcDriver<'d>,
}

impl<'d> Actuator<'d> {
    /// Initialise the LEDC peripheral for RC servo output.
    ///
    /// `timer`   — any available LEDC timer (e.g. `peripherals.ledc.timer0`)
    /// `channel` — any available LEDC channel (e.g. `peripherals.ledc.channel0`)
    /// `pin`     — GPIO10 (connected to level shifter input)
    pub fn new(
        timer: impl esp_idf_hal::ledc::LedcTimer<SpeedMode = LowSpeed> + 'd,
        channel: impl esp_idf_hal::ledc::LedcChannel<SpeedMode = LowSpeed> + 'd,
        pin: impl esp_idf_hal::gpio::OutputPin + 'd,
    ) -> anyhow::Result<Self> {
        let timer_config = TimerConfig::default()
            .frequency(SERVO_FREQ_HZ.Hz().into())
            .resolution(Resolution::Bits14);

        let timer_driver = LedcTimerDriver::new(timer, &timer_config)
            .map_err(|e| anyhow::anyhow!("LEDC timer init: {e}"))?;

        // Pass the timer driver by reference so ownership stays with us: the
        // returned `LedcDriver` only borrows it during construction and does not
        // keep it, so we move it into the struct below to keep the timer running.
        let driver = LedcDriver::new(channel, &timer_driver, pin)
            .map_err(|e| anyhow::anyhow!("LEDC channel init: {e}"))?;

        let mut actuator = Self {
            _timer: timer_driver,
            driver,
        };
        actuator.set_duty(SERVO_DUTY_RETRACTED)?;
        Ok(actuator)
    }

    fn set_duty(&mut self, duty: u32) -> anyhow::Result<()> {
        self.driver
            .set_duty(duty)
            .map_err(|e| anyhow::anyhow!("LEDC set_duty: {e}"))
    }
}

// ─── Blocking methods (used by integration test examples) ────────────────────

impl<'d> Actuator<'d> {
    pub fn extend_blocking(&mut self) -> anyhow::Result<()> {
        info!("Actuator: extending");
        self.set_duty(SERVO_DUTY_EXTENDED)?;
        std::thread::sleep(std::time::Duration::from_millis(ACTUATOR_EXTEND_DURATION_MS));
        Ok(())
    }

    pub fn retract_blocking(&mut self) -> anyhow::Result<()> {
        info!("Actuator: retracting");
        self.set_duty(SERVO_DUTY_RETRACTED)?;
        std::thread::sleep(std::time::Duration::from_millis(ACTUATOR_RETRACT_DURATION_MS));
        Ok(())
    }
}

// ─── ActuatorControl trait implementation ─────────────────────────────────────

impl<'d> ActuatorControl for Actuator<'d> {
    async fn extend(&mut self) -> anyhow::Result<()> {
        info!("Actuator: extending");
        self.set_duty(SERVO_DUTY_EXTENDED)?;
        Timer::after(Duration::from_millis(ACTUATOR_EXTEND_DURATION_MS)).await;
        Ok(())
    }

    async fn retract(&mut self) -> anyhow::Result<()> {
        info!("Actuator: retracting");
        self.set_duty(SERVO_DUTY_RETRACTED)?;
        Timer::after(Duration::from_millis(ACTUATOR_RETRACT_DURATION_MS)).await;
        Ok(())
    }

    fn idle(&mut self) -> anyhow::Result<()> {
        info!("Actuator: idle");
        self.set_duty(SERVO_DUTY_RETRACTED)
    }
}

// ─── Unit tests (host-runnable with `cargo test`) ────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SERVO_TIMER_RESOLUTION_BITS;

    #[test]
    fn servo_duty_ratio_is_two_to_one() {
        assert_eq!(SERVO_DUTY_EXTENDED, SERVO_DUTY_RETRACTED * 2);
    }

    #[test]
    fn servo_duties_within_14bit_range() {
        let max_duty = (1u32 << SERVO_TIMER_RESOLUTION_BITS) - 1;
        assert!(SERVO_DUTY_RETRACTED <= max_duty);
        assert!(SERVO_DUTY_EXTENDED <= max_duty);
    }

    #[test]
    fn retracted_duty_is_five_percent() {
        let max_duty = (1u32 << SERVO_TIMER_RESOLUTION_BITS) - 1;
        let expected = max_duty / 20; // 5 %
        assert!((SERVO_DUTY_RETRACTED as i32 - expected as i32).abs() <= 1);
    }

    #[test]
    fn extended_duty_is_ten_percent() {
        let max_duty = (1u32 << SERVO_TIMER_RESOLUTION_BITS) - 1;
        let expected = max_duty / 10; // 10 %
        assert!((SERVO_DUTY_EXTENDED as i32 - expected as i32).abs() <= 1);
    }

    #[test]
    fn timing_constants_are_valid() {
        assert!(ACTUATOR_EXTEND_DURATION_MS > 0);
        assert!(ACTUATOR_RETRACT_DURATION_MS > 0);
        assert_eq!(ACTUATOR_EXTEND_DURATION_MS, ACTUATOR_RETRACT_DURATION_MS);
    }
}

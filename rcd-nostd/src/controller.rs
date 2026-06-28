//! Power-monitor / auto-reset controller.
//!
//! Owns the EMF sensor and the actuator and runs autonomously, independent of
//! Matter/Thread connectivity (the safety function must work whether or not the
//! device is commissioned).
//!
//! Flow:
//!  1. Monitor mains presence. When power is lost, require `POWER_LOSS_DEBOUNCE_MS`
//!     of *continuous* absence before acting (ignores brief dropouts/glitches).
//!  2. On confirmed loss, run one actuator cycle (extend → retract) to re-arm the RCD.
//!  3. Watch a further `POST_RESET_RECHECK_MS`. If power returns, the cycle worked →
//!     back to monitoring. If it stays absent the whole window, run a second (final)
//!     actuator cycle.
//!  4. After the second cycle, *latch*: stop actuating. Re-arm (return to monitoring)
//!     only once power is *continuously present* for `POWER_RESTORE_CONFIRM_MS`.
//!  5. Power returning during either wait aborts the sequence and returns to idle
//!     monitoring; the next outage starts fresh.

use embassy_time::{Duration, Instant, Timer};
use log::info;

use crate::actuator::Actuator;
use crate::config::{
    POST_RESET_RECHECK_MS, POWER_LOSS_DEBOUNCE_MS, POWER_POLL_INTERVAL_MS,
    POWER_RESTORE_CONFIRM_MS,
};
use crate::sensor::{EmfSensor, PowerState};

pub struct Controller {
    actuator: Actuator,
    sensor: EmfSensor,
    last_power_state: PowerState,
}

impl Controller {
    pub fn new(actuator: Actuator, sensor: EmfSensor) -> Self {
        Self {
            actuator,
            sensor,
            last_power_state: PowerState::Present,
        }
    }

    /// Main run loop. Never returns.
    pub async fn run(&mut self) -> ! {
        info!("Controller: starting (EMF power monitor + auto-reset)");

        // Home the actuator to a known retracted position on startup, no matter what
        // position it powered up in.
        info!("Controller: homing actuator to retracted position");
        self.actuator.retract().await;
        self.actuator.idle();

        // Establish a baseline so the first real transition is logged correctly.
        let baseline = self.sensor.sample().await;
        self.note_power(baseline);

        loop {
            // 1. Idle until power is lost.
            self.wait_until_absent().await;

            // 2. Debounce: require continuous absence before acting.
            info!(
                "Controller: power loss detected — confirming {} s of continuous loss",
                POWER_LOSS_DEBOUNCE_MS / 1_000
            );
            if self
                .wait_for_power_or_timeout(Duration::from_millis(POWER_LOSS_DEBOUNCE_MS))
                .await
            {
                info!("Controller: power restored during debounce — no action taken");
                continue;
            }

            // 3. Confirmed sustained loss → first reset cycle.
            info!("Controller: sustained power loss confirmed — running reset cycle 1");
            self.run_actuator_cycle(1).await;

            // 4. Re-check: watch a further window for continuous absence.
            info!(
                "Controller: cycle 1 done — watching {} s for power",
                POST_RESET_RECHECK_MS / 1_000
            );
            if self
                .wait_for_power_or_timeout(Duration::from_millis(POST_RESET_RECHECK_MS))
                .await
            {
                info!("Controller: power restored after cycle 1 — back to monitoring");
                continue;
            }

            // 5. Still absent → second (final) reset cycle.
            info!("Controller: power still absent — running reset cycle 2 (final)");
            self.run_actuator_cycle(2).await;

            // 6. Latch: no further cycles. Re-arm only after power returns and holds.
            info!(
                "Controller: latched after 2 cycles — re-arming only after {} s of continuous power",
                POWER_RESTORE_CONFIRM_MS / 1_000
            );
            self.wait_for_sustained_power(Duration::from_millis(POWER_RESTORE_CONFIRM_MS))
                .await;
            info!("Controller: power restored and held — re-arming, back to monitoring");
        }
    }

    /// Run one reset cycle: extend to trip the RCD lever, retract, park.
    async fn run_actuator_cycle(&mut self, cycle: u32) {
        info!("Controller: reset cycle {} — actuating", cycle);
        // TODO(matter): report the plug as "On" while a reset cycle is active.
        self.actuator.extend().await;
        self.actuator.retract().await;
        self.actuator.idle();
        // TODO(matter): report the plug as "Off" now that the cycle is complete.
    }

    /// Poll the sensor until a reading is ABSENT, then return.
    async fn wait_until_absent(&mut self) {
        loop {
            if !self.sample().await.is_present() {
                return;
            }
            Timer::after(Duration::from_millis(POWER_POLL_INTERVAL_MS)).await;
        }
    }

    /// Wait up to `timeout`, polling power every `POWER_POLL_INTERVAL_MS`.
    /// Returns `true` if power becomes PRESENT during the wait (restored — abort),
    /// `false` if the timeout elapses with power still absent.
    async fn wait_for_power_or_timeout(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            Timer::after(Duration::from_millis(POWER_POLL_INTERVAL_MS)).await;
            if self.sample().await.is_present() {
                return true;
            }
        }
        false
    }

    /// Block until power has been *continuously present* for `required`. Any dropout
    /// restarts the confirmation, so this only returns once the supply has held for
    /// the full window. Used to release the post-second-cycle latch.
    async fn wait_for_sustained_power(&mut self, required: Duration) {
        loop {
            // Wait for power to appear.
            while !self.sample().await.is_present() {
                Timer::after(Duration::from_millis(POWER_POLL_INTERVAL_MS)).await;
            }
            // Confirm it stays present for the whole window.
            let deadline = Instant::now() + required;
            let mut held = true;
            while Instant::now() < deadline {
                Timer::after(Duration::from_millis(POWER_POLL_INTERVAL_MS)).await;
                if !self.sample().await.is_present() {
                    held = false;
                    break;
                }
            }
            if held {
                return;
            }
        }
    }

    /// Sample the sensor and log any power-state transition.
    async fn sample(&mut self) -> PowerState {
        let power = self.sensor.sample().await;
        self.note_power(power);
        power
    }

    fn note_power(&mut self, power: PowerState) {
        if power != self.last_power_state {
            info!("Controller: power state changed to {:?}", power);
            self.last_power_state = power;
            // TODO(matter): report contact-sensor state (closed = power present).
        }
    }
}

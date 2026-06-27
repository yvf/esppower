//! Power-monitor / auto-reset controller.
//!
//! Owns the EMF sensor and the actuator and runs autonomously, independent of
//! Matter/Thread connectivity (the safety function must work whether or not the
//! device is commissioned).
//!
//! Flow:
//!  1. Monitor mains presence. When power is lost, require `POWER_LOSS_DEBOUNCE_MS`
//!     of *continuous* absence before acting (ignores brief dropouts/glitches).
//!  2. On confirmed loss, run an actuator cycle (extend → retract) to re-arm the RCD.
//!  3. If power is still absent afterwards, retry with exponential backoff:
//!     `INITIAL_RETRY_DELAY_MS`, then ×`RETRY_BACKOFF_FACTOR` each time, capped at
//!     `MAX_RETRY_DELAY_MS`.
//!  4. Power returning at any point (during the debounce or any backoff wait) aborts
//!     the sequence and returns to idle monitoring; the next outage starts fresh.

use embassy_time::{Duration, Instant, Timer};
use log::info;

use crate::actuator::Actuator;
use crate::config::{
    INITIAL_RETRY_DELAY_MS, MAX_RETRY_DELAY_MS, POWER_LOSS_DEBOUNCE_MS, POWER_POLL_INTERVAL_MS,
    RETRY_BACKOFF_FACTOR,
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

            // 3. Confirmed sustained loss → reset attempts with exponential backoff.
            info!("Controller: sustained power loss confirmed — beginning reset attempts");
            let mut attempt: u32 = 0;
            let mut backoff_ms = INITIAL_RETRY_DELAY_MS;
            loop {
                attempt += 1;
                self.run_actuator_cycle(attempt).await;

                // Wait out the backoff; if power returns we're done with this outage.
                info!(
                    "Controller: attempt {} done — waiting up to {} s for power",
                    attempt,
                    backoff_ms / 1_000
                );
                if self
                    .wait_for_power_or_timeout(Duration::from_millis(backoff_ms))
                    .await
                {
                    info!(
                        "Controller: power restored after attempt {} — back to monitoring",
                        attempt
                    );
                    break;
                }

                info!("Controller: power still absent after attempt {} — retrying", attempt);
                backoff_ms = (backoff_ms * RETRY_BACKOFF_FACTOR).min(MAX_RETRY_DELAY_MS);
            }
        }
    }

    /// Run one reset cycle: extend to trip the RCD lever, retract, park.
    async fn run_actuator_cycle(&mut self, attempt: u32) {
        info!("Controller: reset attempt {} — actuating", attempt);
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

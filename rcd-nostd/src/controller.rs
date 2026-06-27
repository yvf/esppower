//! Power-monitor / auto-reset state machine.
//!
//! Owns the EMF sensor and the actuator and runs autonomously: it polls the mains
//! field and, on power loss, drives the actuator through an extend → retract → settle
//! cycle to physically re-arm the tripped RCD, re-checking power and retrying up to
//! `MAX_AUTO_RETRIES` times.
//!
//! This loop is deliberately independent of Matter/Thread connectivity — the safety
//! function must work whether or not the device is commissioned. (A later step wires
//! the cycle state back to the Matter On/Off plug for status reporting; the hook is
//! marked TODO below.)
//!
//! State transitions:
//!
//! ```text
//! Idle ──(PowerLost)──► Extending { retries: 0 }
//! Extending { n } ──(extend done)──►   Retracting { n }
//! Retracting { n } ──(retract done)──► Waiting { n }
//! Waiting { n } ──(settle, re-sample)──►
//!     power restored        ──► Idle
//!     absent, n < MAX       ──► Extending { n+1 }
//!     absent, n >= MAX      ──► Idle  (give up)
//! ```

use embassy_time::{Duration, Timer};
use log::{info, warn};

use crate::actuator::Actuator;
use crate::config::{MAX_AUTO_RETRIES, POST_ATTEMPT_WAIT_MS, POWER_POLL_INTERVAL_MS};
use crate::sensor::{EmfSensor, PowerState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerState {
    Idle,
    Extending { retries_done: u8 },
    Retracting { retries_done: u8 },
    Waiting { retries_done: u8 },
}

pub struct Controller {
    actuator: Actuator,
    sensor: EmfSensor,
    state: ControllerState,
    last_power_state: PowerState,
}

impl Controller {
    pub fn new(actuator: Actuator, sensor: EmfSensor) -> Self {
        Self {
            actuator,
            sensor,
            state: ControllerState::Idle,
            last_power_state: PowerState::Present,
        }
    }

    /// Main run loop. Never returns.
    pub async fn run(&mut self) -> ! {
        info!("Controller: starting (EMF power monitor + auto-reset)");

        // Home the actuator to a known retracted position on startup, no matter what
        // position it powered up in. The actuator can come up extended due to GPIO /
        // level-shifter transients during boot, and merely *configuring* the PWM to
        // the retracted duty (done in `Actuator::new`) does not move it back — only an
        // actively-held retract stroke does. This guarantees a safe, known baseline
        // before we start monitoring.
        info!("Controller: homing actuator to retracted position");
        self.actuator.retract().await;
        self.actuator.idle();

        // Establish a baseline so the first real transition is logged correctly.
        let baseline = self.sensor.sample().await;
        self.update_power_state(baseline);

        loop {
            match self.state {
                ControllerState::Idle => {
                    Timer::after(Duration::from_millis(POWER_POLL_INTERVAL_MS)).await;
                    let power = self.sensor.sample().await;
                    self.update_power_state(power);
                }

                ControllerState::Extending { retries_done } => {
                    self.actuator.extend().await;
                    info!(
                        "Controller: extension complete, retracting (attempt {})",
                        retries_done + 1
                    );
                    self.transition(ControllerState::Retracting { retries_done });
                }

                ControllerState::Retracting { retries_done } => {
                    self.actuator.retract().await;
                    info!("Controller: retraction complete, waiting to settle");
                    self.transition(ControllerState::Waiting { retries_done });
                }

                ControllerState::Waiting { retries_done } => {
                    Timer::after(Duration::from_millis(POST_ATTEMPT_WAIT_MS)).await;
                    let power = self.sensor.sample().await;
                    self.update_power_state(power);

                    if power.is_present() {
                        info!("Controller: power restored after reset attempt");
                        self.end_cycle();
                    } else if retries_done < MAX_AUTO_RETRIES {
                        info!(
                            "Controller: power still absent, retrying ({}/{})",
                            retries_done + 1,
                            MAX_AUTO_RETRIES
                        );
                        self.start_cycle(retries_done + 1);
                    } else {
                        warn!("Controller: power still absent after all retries — giving up");
                        self.end_cycle();
                    }
                }
            }
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn start_cycle(&mut self, retries_done: u8) {
        self.transition(ControllerState::Extending { retries_done });
        // TODO(matter): report the plug as "On" while a reset cycle is active.
    }

    fn end_cycle(&mut self) {
        self.transition(ControllerState::Idle);
        self.actuator.idle();
        // TODO(matter): report the plug as "Off" now that the cycle is complete.
    }

    fn transition(&mut self, next: ControllerState) {
        info!("Controller: {:?} → {:?}", self.state, next);
        self.state = next;
    }

    fn update_power_state(&mut self, power: PowerState) {
        if power != self.last_power_state {
            info!("Controller: power state changed to {:?}", power);
            self.last_power_state = power;
            // TODO(matter): report contact-sensor state (closed = power present).

            if !power.is_present() && self.state == ControllerState::Idle {
                info!("Controller: power lost — starting automatic reset cycle");
                self.start_cycle(0);
            }
        }
    }
}

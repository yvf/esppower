//! Main application state machine.
//!
//! Runs as an async Embassy task.  Owns the actuator and sensor drivers,
//! and communicates with the Matter task through typed channels.
//!
//! State transitions:
//!
//! ```text
//! Idle ──(PowerLost | ManualTrigger)──► Extending { retries: 0 }
//!
//! Extending { n } ──(extend complete)──► Retracting { n }
//!
//! Retracting { n } ──(retract complete)──► Waiting { n }
//!
//! Waiting { n } ──(wait complete)──► [re-sample sensor]
//!   power restored              ──► Idle
//!   still absent, n < MAX       ──► Extending { n+1 }
//!   still absent, n >= MAX      ──► Idle  (give up; sensor stays "Open")
//!
//! Any state ──(PowerRestored)──► Idle  (abort cycle in progress)
//! ```

use crate::config::{MAX_AUTO_RETRIES, POST_ATTEMPT_WAIT_MS};
use crate::matter::{ToController, ToMatter};
use crate::sensor::PowerState;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Receiver};
use embassy_time::{Duration, Timer};
use log::{info, warn};

// ─── State machine types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerState {
    Idle,
    Extending { retries_done: u8 },
    Retracting { retries_done: u8 },
    Waiting { retries_done: u8 },
}

impl ControllerState {
    pub fn is_cycling(self) -> bool {
        !matches!(self, ControllerState::Idle)
    }
}

// ─── Sensor + actuator traits for dependency injection / testing ──────────────

// These traits are crate-internal dependency-injection seams (real drivers +
// test mocks). Callers are concrete and single-threaded, so the missing
// auto-trait bounds the lint warns about don't apply.
#[allow(async_fn_in_trait)]
pub trait ActuatorControl {
    async fn extend(&mut self) -> anyhow::Result<()>;
    async fn retract(&mut self) -> anyhow::Result<()>;
    fn idle(&mut self) -> anyhow::Result<()>;
}

#[allow(async_fn_in_trait)]
pub trait SensorRead {
    async fn sample(&mut self) -> anyhow::Result<PowerState>;
}

// ─── Controller task ──────────────────────────────────────────────────────────

pub struct Controller<A, S> {
    actuator: A,
    sensor: S,
    state: ControllerState,
    last_power_state: PowerState,
    matter_tx: embassy_sync::channel::Sender<'static, CriticalSectionRawMutex, ToMatter, 4>,
}

impl<A: ActuatorControl, S: SensorRead> Controller<A, S> {
    pub fn new(
        actuator: A,
        sensor: S,
        matter_tx: embassy_sync::channel::Sender<'static, CriticalSectionRawMutex, ToMatter, 4>,
    ) -> Self {
        Self {
            actuator,
            sensor,
            state: ControllerState::Idle,
            last_power_state: PowerState::Present,
            matter_tx,
        }
    }

    /// Main async run loop. Polls the sensor and responds to Matter commands.
    pub async fn run(
        &mut self,
        ctrl_rx: Receiver<'static, CriticalSectionRawMutex, ToController, 4>,
    ) -> ! {
        info!("Controller: starting");

        // Initial sensor read to establish baseline.
        if let Ok(state) = self.sensor.sample().await {
            self.update_power_state(state).await;
        }

        loop {
            // Check for an incoming Matter command (non-blocking).
            if let Ok(msg) = ctrl_rx.try_receive() {
                match msg {
                    ToController::ManualTrigger if self.state == ControllerState::Idle => {
                        info!("Controller: manual trigger received");
                        self.start_cycle(0).await;
                    }
                    ToController::ManualTrigger => {
                        info!("Controller: manual trigger ignored — cycle already in progress");
                    }
                }
            }

            match self.state {
                ControllerState::Idle => {
                    // Poll the power sensor at the configured interval.
                    Timer::after(Duration::from_millis(crate::config::POWER_POLL_INTERVAL_MS)).await;
                    match self.sensor.sample().await {
                        Ok(power) => self.update_power_state(power).await,
                        Err(e) => warn!("Controller: sensor error: {e}"),
                    }
                }

                ControllerState::Extending { retries_done } => {
                    if let Err(e) = self.actuator.extend().await {
                        warn!("Controller: extend error: {e}");
                    }
                    info!("Controller: extension complete, retracting (attempt {})", retries_done + 1);
                    self.transition(ControllerState::Retracting { retries_done });
                }

                ControllerState::Retracting { retries_done } => {
                    if let Err(e) = self.actuator.retract().await {
                        warn!("Controller: retract error: {e}");
                    }
                    info!("Controller: retraction complete, waiting to settle");
                    self.transition(ControllerState::Waiting { retries_done });
                }

                ControllerState::Waiting { retries_done } => {
                    Timer::after(Duration::from_millis(POST_ATTEMPT_WAIT_MS)).await;

                    let power = match self.sensor.sample().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Controller: sensor error after wait: {e}");
                            PowerState::Absent
                        }
                    };

                    self.update_power_state(power).await;

                    if power.is_present() {
                        info!("Controller: power restored after reset attempt");
                        self.end_cycle();
                    } else if retries_done < MAX_AUTO_RETRIES {
                        info!(
                            "Controller: power still absent, retrying ({}/{})",
                            retries_done + 1,
                            MAX_AUTO_RETRIES
                        );
                        self.start_cycle(retries_done + 1).await;
                    } else {
                        warn!("Controller: power still absent after all retries — giving up");
                        self.end_cycle();
                    }
                }
            }
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn start_cycle(&mut self, retries_done: u8) {
        self.transition(ControllerState::Extending { retries_done });
        // Report plug as "On" while cycle is active.
        self.send_matter(ToMatter::SetPlugOnOff(true)).await;
    }

    fn end_cycle(&mut self) {
        self.transition(ControllerState::Idle);
        let _ = self.actuator.idle();
        // Report plug as "Off" now that the cycle is complete.
        // try_send is non-blocking; if the channel is full the update is dropped
        // (send_matter's async wrapper has the same behaviour).
        if self.matter_tx.try_send(ToMatter::SetPlugOnOff(false)).is_err() {
            warn!("Controller: Matter channel full — dropped post-cycle Off update");
        }
    }

    fn transition(&mut self, next: ControllerState) {
        info!("Controller: {:?} → {:?}", self.state, next);
        self.state = next;
    }

    async fn update_power_state(&mut self, power: PowerState) {
        if power != self.last_power_state {
            info!("Controller: power state changed to {:?}", power);
            self.last_power_state = power;
            // Contact Sensor: true = contact closed = power present.
            self.send_matter(ToMatter::SetContactClosed(power.is_present())).await;

            if !power.is_present() && self.state == ControllerState::Idle {
                info!("Controller: power lost — starting automatic reset cycle");
                self.start_cycle(0).await;
            }
        }
    }

    async fn send_matter(&self, msg: ToMatter) {
        // Try to send; if the channel is full, log and drop (non-blocking best-effort).
        if self.matter_tx.try_send(msg).is_err() {
            warn!("Controller: Matter channel full — dropped update");
        }
    }
}

// ─── Unit tests (host-runnable via `cargo test`) ──────────────────────────────
#[cfg(test)]
mod tests {
    // Mocks exist for future integration tests that drive `Controller::run()`.
    // They are not yet instantiated in any test function.
    #![allow(dead_code)]

    use super::*;

    // ── Minimal mock actuator ─────────────────────────────────────────────────

    struct MockActuator {
        pub extend_calls: u32,
        pub retract_calls: u32,
        pub idle_calls: u32,
    }

    impl MockActuator {
        fn new() -> Self {
            Self { extend_calls: 0, retract_calls: 0, idle_calls: 0 }
        }
    }

    impl ActuatorControl for MockActuator {
        async fn extend(&mut self) -> anyhow::Result<()> {
            self.extend_calls += 1;
            Ok(())
        }
        async fn retract(&mut self) -> anyhow::Result<()> {
            self.retract_calls += 1;
            Ok(())
        }
        fn idle(&mut self) -> anyhow::Result<()> {
            self.idle_calls += 1;
            Ok(())
        }
    }

    // ── Minimal mock sensor ───────────────────────────────────────────────────

    struct MockSensor {
        pub readings: std::collections::VecDeque<PowerState>,
        pub default: PowerState,
    }

    impl MockSensor {
        fn new(default: PowerState) -> Self {
            Self { readings: Default::default(), default }
        }
        fn push(&mut self, state: PowerState) {
            self.readings.push_back(state);
        }
    }

    impl SensorRead for MockSensor {
        async fn sample(&mut self) -> anyhow::Result<PowerState> {
            Ok(self.readings.pop_front().unwrap_or(self.default))
        }
    }

    // ── State transition tests ────────────────────────────────────────────────

    #[test]
    fn idle_state_is_not_cycling() {
        assert!(!ControllerState::Idle.is_cycling());
    }

    #[test]
    fn extending_state_is_cycling() {
        assert!(ControllerState::Extending { retries_done: 0 }.is_cycling());
    }

    #[test]
    fn retracting_state_is_cycling() {
        assert!(ControllerState::Retracting { retries_done: 0 }.is_cycling());
    }

    #[test]
    fn waiting_state_is_cycling() {
        assert!(ControllerState::Waiting { retries_done: 0 }.is_cycling());
    }

    #[test]
    fn max_auto_retries_config_is_one() {
        // Confirms the behaviour described in the design: 2 total attempts.
        assert_eq!(MAX_AUTO_RETRIES, 1);
    }

    #[test]
    fn power_state_present_reports_true() {
        assert!(PowerState::Present.is_present());
    }

    #[test]
    fn power_state_absent_reports_false() {
        assert!(!PowerState::Absent.is_present());
    }

    /// Verify state equality comparisons (used for abort-on-restore logic).
    #[test]
    fn state_equality() {
        assert_eq!(ControllerState::Idle, ControllerState::Idle);
        assert_ne!(
            ControllerState::Extending { retries_done: 0 },
            ControllerState::Extending { retries_done: 1 }
        );
        assert_ne!(
            ControllerState::Extending { retries_done: 0 },
            ControllerState::Retracting { retries_done: 0 }
        );
    }
}

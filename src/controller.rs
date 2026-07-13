//! Power-monitor / auto-reset controller.
//!
//! Owns the EMF sensor and the actuator and runs autonomously, independent of
//! Matter/Thread connectivity (the safety function must work whether or not the
//! device is commissioned).
//!
//! Flow:
//!  1. Monitor mains presence. When power is lost, require `POWER_LOSS_DEBOUNCE_MS`
//!     of *continuous* absence before acting (ignores brief dropouts/glitches).
//!  2. On confirmed loss, run one actuator cycle (extend -> retract) to re-arm the RCD.
//!  3. Watch a further `POST_RESET_RECHECK_MS`. If power returns, the cycle worked ->
//!     back to monitoring. If it stays absent the whole window, run a second (final)
//!     actuator cycle.
//!  4. After the second cycle, *latch*: stop actuating. Re-arm (return to monitoring)
//!     only once power is *continuously present* for `POWER_RESTORE_CONFIRM_MS`.
//!  5. Power returning during either wait aborts the sequence and returns to idle
//!     monitoring; the next outage starts fresh. Every "power restored" transition
//!     requires a brief continuous-presence confirmation (`POWER_PRESENT_CONFIRM_MS`,
//!     or `POWER_RESTORE_CONFIRM_MS` for the latch release) so a sensor glitch can't
//!     flip the state.

use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Instant, Timer};
use log::info;

use crate::actuator::Actuator;
use crate::config::{
    POST_RESET_RECHECK_MS, POWER_LOSS_DEBOUNCE_MS, POWER_POLL_INTERVAL_MS,
    POWER_PRESENT_CONFIRM_MS, POWER_RESTORE_CONFIRM_MS,
};
use crate::link;
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
            // 1. Idle until power is lost. A HomeKit manual-reset request is honored
            //    *inside* every poll wait (see `poll_power`), so it works in any state -
            //    idle, mid-debounce, or latched - not only while idle.
            self.wait_for_loss().await;

            // 2. Debounce: require continuous absence before acting.
            info!(
                "Controller: power loss detected - confirming {} s of continuous loss",
                POWER_LOSS_DEBOUNCE_MS / 1_000
            );
            if self
                .wait_for_power_or_timeout(Duration::from_millis(POWER_LOSS_DEBOUNCE_MS))
                .await
            {
                info!("Controller: power restored during debounce - no action taken");
                continue;
            }

            // 3. Confirmed sustained loss -> first reset cycle.
            info!("Controller: sustained power loss confirmed - running reset cycle 1");
            self.run_actuator_cycle(1).await;

            // 4. Re-check: watch a further window for continuous absence.
            info!(
                "Controller: cycle 1 done - watching {} s for power",
                POST_RESET_RECHECK_MS / 1_000
            );
            if self
                .wait_for_power_or_timeout(Duration::from_millis(POST_RESET_RECHECK_MS))
                .await
            {
                info!("Controller: power restored after cycle 1 - back to monitoring");
                continue;
            }

            // 5. Still absent -> second (final) reset cycle.
            info!("Controller: power still absent - running reset cycle 2 (final)");
            self.run_actuator_cycle(2).await;

            // 6. Latch: no further cycles. Re-arm only after power returns and holds.
            info!(
                "Controller: latched after 2 cycles - re-arming only after {} s of continuous power",
                POWER_RESTORE_CONFIRM_MS / 1_000
            );
            self.wait_for_sustained_power(Duration::from_millis(POWER_RESTORE_CONFIRM_MS))
                .await;
            info!("Controller: power restored and held - re-arming, back to monitoring");
        }
    }

    /// Run one reset cycle: extend to trip the RCD lever, retract, park. Reports the
    /// Matter On/Off plug as "On" for the duration of the cycle and "Off" once complete
    /// (covers both manual and automatic cycles), keeping HomeKit's tile consistent.
    async fn run_actuator_cycle(&mut self, cycle: u32) {
        info!("Controller: reset cycle {} - actuating", cycle);
        link::set_plug_active(true);
        self.actuator.extend().await;
        self.actuator.retract().await;
        self.actuator.idle();
        link::set_plug_active(false);
    }

    /// One poll tick. Waits up to `POWER_POLL_INTERVAL_MS`, but wakes immediately if
    /// HomeKit requests a manual reset - in which case it runs one full actuator cycle
    /// *now* (safe: we are between polls, never mid-actuation) before sampling. This is
    /// the single choke-point through which every wait in the controller passes, so a
    /// manual reset is honored in ANY state (idle, debounce, re-check, or latched) without
    /// ever interrupting an in-progress cycle. Returns the sampled power state.
    async fn poll_power(&mut self) -> PowerState {
        if let Either::First(()) = select(
            link::wait_manual_trigger(),
            Timer::after(Duration::from_millis(POWER_POLL_INTERVAL_MS)),
        )
        .await
        {
            info!("Controller: manual reset requested via Matter - running one cycle");
            self.run_actuator_cycle(0).await;
        }
        self.sample().await
    }

    /// Idle until mains power is lost. A manual reset requested while idle is handled
    /// transparently by `poll_power` (runs a cycle, keeps monitoring).
    async fn wait_for_loss(&mut self) {
        while self.poll_power().await.is_present() {}
    }

    /// Wait up to `timeout` for power to be *restored*, polling every
    /// `POWER_POLL_INTERVAL_MS`. A restoration is only accepted once power has stayed
    /// present continuously for `POWER_PRESENT_CONFIRM_MS` - a single glitch is ignored
    /// and the wait continues. Returns `true` if restoration is confirmed (abort the
    /// reset), `false` if the timeout elapses with power still absent.
    async fn wait_for_power_or_timeout(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.poll_power().await.is_present()
                && self
                    .confirm_present(Duration::from_millis(POWER_PRESENT_CONFIRM_MS))
                    .await
            {
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
            // Wait for power to appear (manual reset honored via `poll_power`).
            while !self.poll_power().await.is_present() {}
            // Confirm it stays present for the whole window.
            if self.confirm_present(required).await {
                return;
            }
        }
    }

    /// Starting now, verify power stays PRESENT continuously for `required`, polling
    /// every `POWER_POLL_INTERVAL_MS`. Returns `false` the instant a sample reads
    /// absent, `true` if it holds for the whole window.
    async fn confirm_present(&mut self, required: Duration) -> bool {
        let deadline = Instant::now() + required;
        while Instant::now() < deadline {
            if !self.poll_power().await.is_present() {
                return false;
            }
        }
        true
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
        }
        // Mirror to the Matter contact sensor (closed = power present). Idempotent - only
        // pushes a subscription report when the value actually changes.
        link::set_power_present(power.is_present());
    }
}

//! Power-presence sensing.
//!
//! Two interchangeable backends are provided, selected at compile time via Cargo
//! features (exactly one must be active):
//!
//! - `sensor-emf` (default) — contactless electromagnetic-field detector: a BC547
//!   transistor cascade feeds an amplified 50 Hz signal into GPIO4, read as an
//!   analog ADC1 input. Power presence is inferred from the peak-to-peak swing
//!   within a sampling window. See [`emf`].
//! - `sensor-ct` — SCT-013 current transformer via the RBDimmer adapter on an ADC
//!   pin. Power presence is inferred from the analog peak-to-peak swing. See
//!   [`ct`].
//!
//! Both backends expose a concrete sensor type that implements
//! [`crate::controller::SensorRead`] plus a blocking `sample_blocking` method for
//! the hardware integration tests. The active backend is re-exported as
//! [`ActiveSensor`] so the rest of the firmware stays backend-agnostic.

#[cfg(all(feature = "sensor-ct", feature = "sensor-emf"))]
compile_error!(
    "features `sensor-ct` and `sensor-emf` are mutually exclusive — enable only one"
);

#[cfg(not(any(feature = "sensor-ct", feature = "sensor-emf")))]
compile_error!(
    "a power-sensor backend must be selected: enable `sensor-emf` (default) or `sensor-ct`"
);

/// Detected state of the monitored mains circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    Present,
    Absent,
}

impl PowerState {
    pub fn is_present(self) -> bool {
        matches!(self, PowerState::Present)
    }
}

// ─── Backend modules + active-backend re-export ──────────────────────────────

#[cfg(feature = "sensor-ct")]
pub mod ct;
#[cfg(feature = "sensor-ct")]
pub use ct::{Gpio1Sensor, PowerSensor};

#[cfg(feature = "sensor-emf")]
pub mod emf;
#[cfg(feature = "sensor-emf")]
pub use emf::EmfSensor;

/// The power sensor selected for this build. Constructed in `main.rs`; the exact
/// constructor arguments depend on the active backend.
#[cfg(feature = "sensor-ct")]
pub type ActiveSensor<'d> = Gpio1Sensor<'d>;

/// The power sensor selected for this build. Constructed in `main.rs`; the exact
/// constructor arguments depend on the active backend.
//
// `not(feature = "sensor-ct")` keeps this from colliding with the CT alias above
// when both features are mistakenly enabled, so the only error surfaced is the
// friendly mutual-exclusion `compile_error!`.
#[cfg(all(feature = "sensor-emf", not(feature = "sensor-ct")))]
pub type ActiveSensor<'d> = EmfSensor<'d>;

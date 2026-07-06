//! Shared state between the autonomous power-monitor [`Controller`](crate::controller)
//! and the Matter device-model handlers (EP1 On/Off plug, EP2 contact sensor).
//!
//! The controller runs independently of Matter — the safety function must work whether
//! or not the device is commissioned — but the Matter endpoints have to (a) reflect the
//! controller's state to HomeKit and (b) let HomeKit trigger a manual reset. Both live on
//! the same executor; this module is a small set of statics with a typed API so neither
//! side reaches into the other.
//!
//! Data flow:
//! ```text
//! Controller  --set_plug_active-->   PLUG_ACTIVE  --plug_changed-->  EP1 On/Off push
//! Controller  --set_power_present--> POWER_PRESENT --power_changed--> EP2 contact push
//! EP1 "On"    --request_manual_cycle--> MANUAL_TRIGGER --wait_manual_trigger--> Controller
//! ```

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

/// A reset cycle (manual or automatic) is currently running. Drives the EP1 plug tile.
static PLUG_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Sensed mains presence. Drives the EP2 contact sensor. Defaults to present.
static POWER_PRESENT: AtomicBool = AtomicBool::new(true);

/// Wake the Matter background push tasks the instant a state changes, so HomeKit's tile /
/// trip notification updates immediately rather than on its next poll.
static PLUG_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static POWER_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// HomeKit "On" → controller: run one manual reset cycle.
static MANUAL_TRIGGER: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// ── Controller side ─────────────────────────────────────────────────────────

/// Report whether a reset cycle is currently running. `true` = On (actuating),
/// `false` = Off. The controller is the sole authority for this state.
pub fn set_plug_active(active: bool) {
    if PLUG_ACTIVE.swap(active, Ordering::Relaxed) != active {
        PLUG_CHANGED.signal(());
    }
}

/// Report the sensed mains presence: `true` = present ("Closed"), `false` = absent
/// (RCD tripped, "Open").
pub fn set_power_present(present: bool) {
    if POWER_PRESENT.swap(present, Ordering::Relaxed) != present {
        POWER_CHANGED.signal(());
    }
}

/// Await a HomeKit-initiated manual reset request.
pub async fn wait_manual_trigger() {
    MANUAL_TRIGGER.wait().await
}

// ── Matter side ─────────────────────────────────────────────────────────────

/// Current plug on/off state (a reset cycle is running).
pub fn plug_active() -> bool {
    PLUG_ACTIVE.load(Ordering::Relaxed)
}

/// Current mains presence (contact "Closed").
pub fn power_present() -> bool {
    POWER_PRESENT.load(Ordering::Relaxed)
}

/// Resolve once the plug on/off state changes.
pub async fn plug_changed() {
    PLUG_CHANGED.wait().await
}

/// Resolve once the mains-presence state changes.
pub async fn power_changed() {
    POWER_CHANGED.wait().await
}

/// HomeKit "On" command → request one manual reset cycle from the controller.
pub fn request_manual_cycle() {
    MANUAL_TRIGGER.signal(());
}

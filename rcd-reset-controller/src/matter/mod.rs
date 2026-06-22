//! Matter protocol integration (Matter over Thread, commissioned over BLE).
//!
//! Built on `esp-idf-matter`, which wraps `rs-matter` with Espressif's
//! Thread/BLE transports and NVS persistence. The stack is booted and run by
//! [`node::run`] on a dedicated thread (see `main.rs`).
//!
//! Device model (Matter node):
//!   Endpoint 0 — Root Node (hidden system clusters, provided by the stack)
//!   Endpoint 1 — On/Off Plug-In Unit (0x010A) — actuator trigger
//!   Endpoint 2 — Contact Sensor (0x0015)       — downstream power state
//!
//! Staging:
//!   - STAGE 1 (current): Endpoint 1 uses rs-matter's stock On/Off test logic to
//!     prove build + boot + QR + commissioning end-to-end. Not yet wired to the
//!     actuator; no Contact Sensor endpoint yet.
//!   - STAGE 2: replace with a custom On/Off handler that emits
//!     [`ToController::ManualTrigger`], add the Contact Sensor endpoint fed by
//!     [`ToMatter::SetContactClosed`], and persist fabric data via
//!     `EspKvBlobStore`.

pub mod node;

// ─── Inter-task channel messages ─────────────────────────────────────────────

/// Messages sent from the Matter task → Controller task.
#[derive(Debug, Clone, Copy)]
pub enum ToController {
    /// HomeKit user tapped the plug tile: fire one actuator cycle.
    ManualTrigger,
}

/// Messages sent from the Controller task → Matter task.
#[derive(Debug, Clone, Copy)]
pub enum ToMatter {
    /// Update endpoint 1 On/Off attribute (true while cycle is running).
    SetPlugOnOff(bool),
    /// Update endpoint 2 Boolean State attribute (true = power present = "closed").
    SetContactClosed(bool),
}

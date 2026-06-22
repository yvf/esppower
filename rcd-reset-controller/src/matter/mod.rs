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
//!   - STAGE 1 (done): boot the Thread+BLE stack, print the QR, commission.
//!   - STAGE 2 (done): Endpoint 1 On/Off backed by `node::PlugHooks` — HomeKit
//!     toggle emits [`ToController::ManualTrigger`]; controller cycle state is
//!     reflected back via [`ToMatter::SetPlugOnOff`]. (Endpoint 1 still
//!     advertises the On/Off "light" device type; switching to a plug/outlet
//!     device type is a cosmetic follow-up.)
//!   - STAGE 3 (pending): add the Contact Sensor endpoint (Boolean State, fed by
//!     [`ToMatter::SetContactClosed`]) and persist fabric data via
//!     `EspKvBlobStore`. rs-matter has no stock Boolean State handler, so this
//!     needs a from-scratch cluster + handler.

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

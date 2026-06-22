//! Matter protocol integration.
//!
//! This module exposes:
//! - Channel message types for controller ↔ Matter communication.
//! - [`MatterNode`]: the rs-matter based node that implements the data model.
//!
//! Data model (Matter node, single device):
//!   Endpoint 0 — Root Node
//!   Endpoint 1 — On/Off Plug-In Unit (0x010A) — actuator control
//!   Endpoint 2 — Contact Sensor (0x0015)       — downstream power state

pub mod commissioning;
pub mod endpoints;
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

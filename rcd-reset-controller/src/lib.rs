//! RCD Reset Controller — library crate.
//!
//! Exposes all application modules so both the binary (`src/main.rs`) and the
//! integration-test examples can reference types (Actuator, PowerSensor,
//! EmfSensor, ActuatorControl …) without duplicating source files.

pub mod actuator;
pub mod config;
pub mod controller;
pub mod error;
pub mod matter;
pub mod sensor;

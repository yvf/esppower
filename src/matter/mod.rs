//! Matter-over-Thread integration, built on `rs-matter-embassy`.
//!
//! `rs-matter-embassy`'s `EmbassyThreadMatterStack` + `EspThreadDriver` provide the whole
//! transport (openthread + esp-radio + trouble BLE + edge-nal-openthread + KV-backed
//! persistence, non-concurrent BLE-commission then Thread). This module supplies only the
//! device model - the 2-endpoint node (contact sensor + reset plug) and its handlers - and
//! the run entry point. See docs/no-std-plan.md.

mod contact;
mod plug;
mod stack;

pub use stack::run_matter;

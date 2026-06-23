//! Matter integration glue (Phase 4b): adapters feeding rs-matter-stack's
//! `PreexistingWireless` from our openthread (Thread) + trouble (BLE) transports.
//! See docs/phase4b-glue-design.md. Built incrementally; not yet wired into main.
#![allow(dead_code)]

mod gatt;
mod mdns;
mod net;
mod netctl;
mod netif;

pub use gatt::OtGattPeripheral;
pub use mdns::OtMdns;
pub use net::OtNetStack;
pub use netctl::OtNetCtl;
pub use netif::OtNetif;

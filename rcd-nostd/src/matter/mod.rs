//! Matter integration glue (Phase 4b): adapters feeding rs-matter-stack's
//! `PreexistingWireless` from our openthread (Thread) + trouble (BLE) transports.
//! See docs/phase4b-glue-design.md. Built incrementally; not yet wired into main.
#![allow(dead_code)]

mod contact;
mod gatt;
mod kv;
mod mdns;
mod net;
mod netctl;
mod netif;
mod ot_settings;
mod plug;
mod stack;

pub use gatt::OtGattPeripheral;
pub use kv::wipe_pairing_data;
pub use ot_settings::FlashSettings;
pub use mdns::OtMdns;
pub use net::OtNetStack;
pub use netctl::OtNetCtl;
pub use netif::OtNetif;
pub use stack::run_matter;

//! Matter QR-code commissioning payload.
//!
//! rs-matter 0.1.x changed the QR-code API from the planned 0.2 design.
//! The QR payload is now best generated via `Matter::print_standard_qr_text()` /
//! `Matter::print_standard_qr_code()` after the Matter object is constructed.
//! The stubs below keep the module compiling; replace them when wiring up the
//! full Matter run loop.

use crate::config::{MATTER_DISCRIMINATOR, MATTER_PASSCODE, MATTER_PRODUCT_ID, MATTER_VENDOR_ID};
use log::info;

/// Log the commissioning QR-code payload.
///
/// TODO: replace with `matter.print_standard_qr_code(DiscoveryCapabilities::IP)` once the
/// Matter object is available here, or call it from the Matter task after init.
pub fn log_qr_code() {
    info!(
        "Matter commissioning: VID={:#06x} PID={:#06x} discriminator={} passcode={}",
        MATTER_VENDOR_ID, MATTER_PRODUCT_ID, MATTER_DISCRIMINATOR, MATTER_PASSCODE,
    );
    info!("Scan the QR code printed by Matter::print_standard_qr_code() during commissioning.");
}

/// Return the `MT:…` commissioning payload string.
///
/// TODO: rewrite against rs-matter 0.1.x `QrPayload` API.
#[allow(dead_code)]
pub fn build_qr_payload() -> anyhow::Result<heapless::String<128>> {
    anyhow::bail!("QR payload generation not yet implemented for rs-matter 0.1.x")
}

/// Return the numeric manual pairing code.
///
/// TODO: rewrite against rs-matter 0.1.x manual pairing code API.
#[allow(dead_code)]
pub fn manual_pairing_code() -> heapless::String<32> {
    let mut s = heapless::String::<32>::new();
    // placeholder — real encoding per Matter spec §5.1.4.1
    let _ = core::fmt::write(
        &mut s,
        format_args!("{:05}-{:03}-{:05}", MATTER_DISCRIMINATOR >> 2, 0, MATTER_PASSCODE % 100000),
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passcode_is_within_matter_spec_range() {
        assert!(MATTER_PASSCODE >= 1);
        assert!(MATTER_PASSCODE <= 99_999_999);
    }

    #[test]
    fn discriminator_fits_in_12_bits() {
        assert!(MATTER_DISCRIMINATOR <= 0xFFF);
    }

    #[test]
    fn vendor_and_product_ids_are_nonzero() {
        assert_ne!(MATTER_VENDOR_ID, 0);
        assert_ne!(MATTER_PRODUCT_ID, 0);
    }
}

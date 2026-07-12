# Upstream PR: fix ESP32-H2/C6 IEEE 802.15.4 ext-address filter byte order

**Repository:** `esp-rs/esp-hal` (crate: `esp-radio`)
**File:** `esp-radio/src/ieee802154/mod.rs`
**Diff base:** `esp-radio` 0.18.0 — verified byte-identical to `main` for the affected
function as of 2026-07-04. Rebase onto `main` before submitting.

---

## PR title

    fix(esp-radio): program the 802.15.4 extended-address filter in on-air octet order

## Summary

The hardware extended-address acceptance filter is programmed with the address
octets **reversed** relative to how IEEE 802.15.4 transmits them on air. As a result
the filter never matches a unicast frame addressed to the node: only broadcast frames
(which bypass the extended-address filter) are received. Any protocol that relies on
receiving unicast frames addressed to the node's extended address is broken — most
visibly, an OpenThread node can never attach (it receives the router's broadcast MLE
Advertisements but never the unicast MLE Parent Response addressed to its EUI-64, so it
stays `Detached` indefinitely).

## Root cause

IEEE 802.15.4 transmits addresses least-significant-octet first, and the ESP 802.15.4
MAC extended-address filter registers (`extend_addr0`/`extend_addr1`) expect the octets
in that same on-air order (first on-air octet in the low byte of `extend_addr0`).

`Config::ext_addr` is a `u64`. The convention used by the crate's 802.15.4 consumer,
`openthread` (`esp-rs/openthread`), is to build that `u64` from the on-air octets with
`u64::from_be_bytes` — so the **first on-air octet is the `u64`'s most-significant
byte**. This is consistent across that ecosystem: the `otPlatRadioSetExtendedAddress`
shim uses `u64::from_be_bytes`, and the crate's own software MAC-header parser reads the
destination extended address the same way (`MacHeader::dst_ext_addr = u64::from_be_bytes(...)`).

`update_driver_config` then converts that `u64` back to filter octets with
`to_le_bytes()`, which puts the most-significant byte (the first on-air octet) **last**.
The filter is therefore byte-reversed and never matches an incoming unicast frame.

Using `to_be_bytes()` restores on-air octet order and the filter matches.

## Fix / diff

```diff
--- a/esp-radio/src/ieee802154/mod.rs
+++ b/esp-radio/src/ieee802154/mod.rs
@@ -156,7 +156,13 @@
 
         if let Some(ext_addr) = cfg.ext_addr {
             let mut address = [0u8; IEEE802154_FRAME_EXT_ADDR_SIZE];
-            address.copy_from_slice(&ext_addr.to_le_bytes());
+            // IEEE 802.15.4 transmits addresses least-significant-octet first, and the
+            // hardware ext-address filter expects the octets in that same on-air order.
+            // `Config::ext_addr` follows the convention used by the `openthread`
+            // consumer, where the u64 is built from the on-air octets via
+            // `u64::from_be_bytes` (so the first on-air octet is the u64's MSB).
+            // `to_be_bytes` therefore yields on-air order; `to_le_bytes` reversed it and
+            // the filter never matched a unicast frame addressed to this node.
+            address.copy_from_slice(&ext_addr.to_be_bytes());
 
             set_extended_address(0, address);
         }
```

## How to reproduce

1. ESP32-H2 (or C6) running OpenThread via `esp-radio` + `esp-rs/openthread`.
2. Apply a valid Active Operational Dataset for a Thread network that has a reachable
   border router / router in radio range, then `enable_thread(true)`.
3. Observe the node cycle `Attach attempt N ... unsuccessful` forever, role stuck at
   `Detached`.

With the receive path otherwise functional, instrumenting `isr_handle_rx_done` shows
that **only broadcast frames** (dest addressing mode = short, dest = `0xFFFF`, FCF
ack-request bit clear) are ever delivered; no unicast frame addressed to the node's
extended address is received. After this fix the node receives the unicast MLE
responses and reaches role `Child`.

## Notes for maintainers

- The fix is a one-liner and self-contained. The surrounding `short_addr`/`pan_id`
  filters use `u16` and are unaffected.
- The choice of `to_be_bytes` follows the de-facto `u64` convention already established
  by the `openthread` integration (`from_be_bytes` in both the platform shim and the
  software MAC parser). If maintainers would rather define `Config::ext_addr` as the
  "canonical" numeric address (`0x0011_2233_4455_6677` for `00:11:...:77`), the
  equivalent fix is to make the `openthread` shim and `MacHeader` parser use
  `from_le_bytes` and keep `to_le_bytes` here — but that is a larger, cross-crate change
  and reverses the existing convention. Fixing it here keeps the change local and
  consistent with current callers.
- This affects every ESP chip with the 802.15.4 MAC (H2, C6, C5, C61); it is not
  H2-specific. It likely went unnoticed because broadcast-only reception is enough for
  some quick tests but not for a real unicast exchange like Thread attach.

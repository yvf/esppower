# Upstream PR: fix ESP32-H2/C6 IEEE 802.15.4 ext-address filter byte order (revert #5314)

**Repository:** `esp-rs/esp-hal` (crate: `esp-radio`)
**File:** `esp-radio/src/ieee802154/mod.rs`
**Diff base:** current `main` (`esp-radio` 1.0.0-beta.0). One functional line.
**Fork branch:** `fix/ieee802154-ext-addr-filter-byte-order` (in `~/github/yanf-esp-hal`).

---

## PR title

    fix(esp-radio): program the 802.15.4 extended-address filter in on-air octet order

## Summary

The hardware extended-address acceptance filter is programmed with the address
octets **reversed** relative to how IEEE 802.15.4 transmits them on air, so the
filter never matches a unicast frame addressed to the node - only broadcast
frames (which bypass the extended-address filter) are received. Most visibly, an
OpenThread node can never attach: it receives the router's broadcast MLE
Advertisements but never the unicast MLE Parent Response addressed to its
EUI-64, so it stays `Detached` indefinitely.

**This reverts [#5314].** That PR flipped the octet order from `to_be_bytes` to
`to_le_bytes` ("use little-endian byte order..."), but the original `to_be_bytes`
was correct - it even carried a literal `// LE or BE?` comment, i.e. the order
was a known open question and #5314 guessed wrong.

## Root cause

IEEE 802.15.4 transmits addresses least-significant-octet first, and the ESP
802.15.4 MAC extended-address filter registers (`extend_addr0`/`extend_addr1`)
expect the octets in that same on-air order (first on-air octet in the low byte
of `extend_addr0`).

`Config::ext_addr` is a `u64`. The convention used by the crate's 802.15.4
consumer, `openthread` (`esp-rs/openthread`), is to build that `u64` from the
on-air octets with `u64::from_be_bytes` - so the **first on-air octet is the
`u64`'s most-significant byte**. This is consistent across that ecosystem:

- the platform shim `otPlatRadioSetExtendedAddress` builds the address with
  `u64::from_be_bytes(...)` (`openthread/src/platform.rs`);
- the crate's own software MAC-header parser reads the destination extended
  address the same way: `dst_ext_addr = u64::from_be_bytes(psdu[5..13])`
  (`openthread/src/radio.rs`), and compares it against `Config::ext_addr`.

`update_driver_config` must therefore convert that `u64` back to filter octets
with `to_be_bytes()` to restore on-air order. `to_le_bytes()` (as #5314 set it)
puts the most-significant byte (the first on-air octet) **last**, so the filter
is byte-reversed and never matches an incoming unicast frame.

## Fix / diff

```diff
--- a/esp-radio/src/ieee802154/mod.rs
+++ b/esp-radio/src/ieee802154/mod.rs
@@ -156,7 +156,16 @@
 
         if let Some(ext_addr) = cfg.ext_addr {
             let mut address = [0u8; IEEE802154_FRAME_EXT_ADDR_SIZE];
-            address.copy_from_slice(&ext_addr.to_le_bytes());
+            // The hardware extended-address acceptance filter expects the octets in
+            // on-air order (IEEE 802.15.4 transmits addresses least-significant-octet
+            // first, and the first on-air octet goes in the low byte of `extend_addr0`).
+            // `Config::ext_addr` follows the convention established by the `openthread`
+            // consumer, which builds the `u64` from the on-air octets via
+            // `u64::from_be_bytes` (so the first on-air octet is the `u64`'s MSB).
+            // `to_be_bytes` therefore reproduces on-air order; `to_le_bytes` reversed it,
+            // so the filter never matched a unicast frame addressed to this node and only
+            // broadcast frames (which bypass the ext-address filter) were received.
+            address.copy_from_slice(&ext_addr.to_be_bytes());
 
             set_extended_address(0, address);
         }
```

## How to reproduce

1. ESP32-H2 (or C6) running OpenThread via `esp-radio` + `esp-rs/openthread`.
2. Apply a valid Active Operational Dataset for a Thread network that has a
   reachable router in radio range, then `enable_thread(true)`.
3. Observe the node cycle `Attach attempt N ... unsuccessful` forever, role stuck
   at `Detached`.

Instrumenting `isr_handle_rx_done` shows that **only broadcast frames** (dest
addressing mode = short, dest = `0xFFFF`) are ever delivered; no unicast frame
addressed to the node's extended address is received. After this fix the node
receives the unicast MLE responses and reaches role `Child`. **Verified on
ESP32-H2 hardware.**

## Notes for maintainers

- This is a one-line revert of #5314; the rest of that PR area is unchanged. The
  surrounding `short_addr`/`pan_id` filters use `u16` and are unaffected.
- The direction is fixed by the `openthread` integration, which is the only
  in-tree consumer of `Config::ext_addr` and already uses `from_be_bytes` on
  both the set and parse paths (see the two references above). If maintainers
  would instead prefer `Config::ext_addr` to be the "canonical" numeric address
  (`0x0011_2233_4455_6677` for `00:11:...:77`), the equivalent change is to make
  the `openthread` shim and MAC parser use `from_le_bytes` and keep `to_le_bytes`
  here - but that is a larger, cross-crate change that reverses the existing
  convention. Fixing it here keeps the change local and consistent with the
  current caller.
- This affects every ESP chip with the 802.15.4 MAC (H2, C6, C5, C61); it is not
  H2-specific. It likely went unnoticed because broadcast-only reception is
  enough for some quick tests but not for a real unicast exchange like a Thread
  attach - the same reason #5314's LE guess passed review.

[#5314]: https://github.com/esp-rs/esp-hal/pull/5314

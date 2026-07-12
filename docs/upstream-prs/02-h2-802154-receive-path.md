# Upstream PR: fix ESP32-H2 IEEE 802.15.4 receive path (never re-arms / never delivers)

**Repository:** `esp-rs/esp-hal` (crate: `esp-radio`)
**File:** `esp-radio/src/ieee802154/raw.rs`
**Diff base:** `esp-radio` 0.18.0 — verified byte-identical to `main` for the affected
functions (`ieee802154_set_txrx_pti`, `enable_rx`, `ensure_receive_enabled`,
`isr_handle_rx_done`) as of 2026-07-04. Rebase onto `main` before submitting.

> Companion to the ext-address filter fix
> (`01-ieee802154-ext-addr-filter-byte-order.md`). Both are required for OpenThread to
> attach on ESP32-H2; they are independent and can be reviewed separately. This PR makes
> the receiver reliably deliver frames; the companion PR makes it accept unicast frames
> addressed to the node.

---

## PR title

    fix(esp-radio): reliable 802.15.4 receive on ESP32-H2 (re-arm on abort, deliver on RxDone)

## Summary

On ESP32-H2 the 802.15.4 receiver transmits fine but is extremely unreliable at
receiving: it detects the start of most frames (`RxSfdDone`) but the reception is
aborted (`rx_abort = SfdTimeout`) and `RxDone` almost never fires; when a frame *is*
completed it may not be delivered to the upper layer. In practice an OpenThread node
cannot attach.

Four independent issues in the receive state machine combine to cause this. They are
grouped here because the fixes are all in `raw.rs` and interact (each is necessary but
not sufficient on its own); maintainers may prefer to split (1)/(4) out — see notes.

## Root causes and fixes

### (1) `ensure_receive_enabled()` aborts in-flight frames

`ensure_receive_enabled()` is polled from `raw_received()` on **every** receive poll and
unconditionally issues `set_cmd(Command::RxStart)` while in `Receive` state. If a frame
is mid-reception (after `RxSfdDone`, before `RxDone`), this restarts the receiver and
aborts the in-flight frame → `SfdTimeout`, and `RxDone` never fires. The function is
already flagged with a `FIXME` and a comment noting it "shouldn't be necessary." With
the proper re-arm mechanism in (2) it is not: make it a no-op.

### (2) RX-abort events are masked, so RX never re-arms after an abort

`ieee802154_mac_init` enables only `TxAckTimeout | TxAckCoexBreak` RX-abort events.
So when RX aborts while waiting for / receiving a frame (`SfdTimeout`, `CrcError`,
`InvalidLen`, `FilterFail`, `NoRss`), **no MAC interrupt is raised**, the ISR never runs,
and RX is never re-armed — the receiver goes dead until the next TX happens to call
`enable_rx()` again. The recovery code already exists (`isr_handle_rx_phase_rx_abort`
handles exactly these reasons and requests `next_operation → enable_rx`); it just never
runs because the events are masked. Enable all RX-abort events in `enable_rx()`.

### (3) Delivery deferred on auto-ACK never happens (`AckTxDone` doesn't fire on H2)

`isr_handle_rx_done` copies the frame into `rx_queue`, then for ACK-required frames sets
state `TxAck` and **defers** the `rx_available()` notification until `isr_handle_ack_tx_done`
(i.e. until `AckTxDone`). On ESP32-H2 `AckTxDone` is not observed to fire, so those
frames — including a unicast MLE Parent Response — are copied but **never delivered** to
the upper layer, and the radio stays in `TxAck`. Since the frame is already safely in
`rx_queue`, notify immediately in all branches; the hardware still transmits the auto-ACK
independently.

### (4) Active RX runs at low coex priority and gets preempted mid-frame

`ieee802154_set_txrx_pti` sets active TX/RX to `IEEE802154_LOW` (priority 3 of 4; lower
number = higher priority). On ESP32-H2 this lets the coexistence arbiter preempt
continuous RX listening mid-frame: short TX bursts still complete (explaining the
"TX works, RX doesn't" asymmetry), but the receiver rarely holds the radio long enough
to demodulate a full incoming frame — an `SfdTimeout` flood with almost no `RxDone`.
Raise active TX/RX to `IEEE802154_HIGH`.

## Fix / diff

```diff
--- a/esp-radio/src/ieee802154/raw.rs
+++ b/esp-radio/src/ieee802154/raw.rs
@@ -19,8 +19,8 @@
     radio_clocks::{clocks_ll::enable_ieee802154, init_radio_clocks},
     sys::include::{
         ieee802154_coex_event_t,
+        ieee802154_coex_event_t_IEEE802154_HIGH,
         ieee802154_coex_event_t_IEEE802154_IDLE,
-        ieee802154_coex_event_t_IEEE802154_LOW,
         ieee802154_coex_event_t_IEEE802154_MIDDLE,
     },
 };
@@ -172,7 +172,12 @@ fn ieee802154_set_txrx_pti(txrx_scene: Ieee802154TxRxScene) {
             unsafe { esp_coex_ieee802154_txrx_pti_set(ieee802154_coex_event_t_IEEE802154_IDLE) };
         }
         Ieee802154TxRxScene::Tx | Ieee802154TxRxScene::Rx => {
-            unsafe { esp_coex_ieee802154_txrx_pti_set(ieee802154_coex_event_t_IEEE802154_LOW) };
+            // IEEE802154_LOW let the coexistence arbiter preempt continuous RX listening
+            // mid-frame on ESP32-H2: short TX bursts still completed, but the receiver
+            // rarely held the radio long enough to demodulate a full incoming frame
+            // (observed as an SfdTimeout flood with almost no RxDone). Raise active
+            // TX/RX to IEEE802154_HIGH so reception can complete.
+            unsafe { esp_coex_ieee802154_txrx_pti_set(ieee802154_coex_event_t_IEEE802154_HIGH) };
         }
         Ieee802154TxRxScene::TxAt | Ieee802154TxRxScene::RxAt => {
             unsafe { esp_coex_ieee802154_txrx_pti_set(ieee802154_coex_event_t_IEEE802154_MIDDLE) };
@@ -279,6 +284,14 @@ fn enable_rx() {
     set_next_rx_buffer();
     ieee802154_set_txrx_pti(Ieee802154TxRxScene::Rx);
 
+    // Unmask all RX-abort events while receiving. At init only
+    // TxAckTimeout|TxAckCoexBreak are enabled, so an RX error while waiting for a frame
+    // (SfdTimeout/CrcError/InvalidLen/FilterFail/NoRss) aborts without raising the MAC
+    // interrupt: the ISR never runs and RX is never re-armed, leaving the receiver dead
+    // until the next TX happens to call enable_rx() again. With these enabled each abort
+    // fires the interrupt and isr_handle_rx_phase_rx_abort re-arms RX via next_operation().
+    enable_rx_abort_events(RxAbortReason::all());
+
     set_cmd(Command::RxStart);
 }
 
@@ -486,13 +499,13 @@
-// FIXME: we shouldn't need this - we need to re-align the original driver with our port
 pub(crate) fn ensure_receive_enabled() {
-    // shouldn't be necessary but avoids a problem with rx stopping
-    // unexpectedly when used together with BLE
-    STATE.with(|state| {
-        if state.state == Ieee802154State::Receive {
-            set_cmd(Command::RxStart);
-        }
-    });
+    // Intentionally a no-op. This is polled from `raw_received()` on every receive poll;
+    // re-issuing `RxStart` while a frame is mid-reception (after RxSfdDone, before
+    // RxDone) restarts the receiver and aborts the in-flight frame. On ESP32-H2 this
+    // manifested as continuous RxSfdDone with rx_abort=SfdTimeout and RxDone that never
+    // fired. The receiver is already armed by `enable_rx()` and re-armed on abort, so no
+    // poll-driven restart is needed.
 }
 
@@ -611,20 +624,26 @@ fn isr_handle_rx_done(needs_next_op: &mut bool) {
             // advances the index in isr_handle_rx_done via next_rx_buffer().
             receive_done(state);
 
-            if will_auto_send_ack(frm) {
-                // auto tx ack for frame version 0b00 and 0b01
-                // Frame data already copied above. Defer rx_available()
-                // notification until ACK completes (isr_handle_ack_tx_done).
+            // Deliver the received frame to the upper layer immediately. It is already
+            // copied into rx_queue by receive_done() above, so there is no buffer-reuse
+            // hazard. Delivery was previously deferred for ACK-required frames until
+            // AckTxDone fired, but AckTxDone is not observed to fire on ESP32-H2, so such
+            // frames (e.g. a unicast MLE Parent Response) were copied but never delivered
+            // and the radio stayed in TxAck. The hardware still transmits the auto-ACK
+            // independently of this notification.
+            super::rx_available();
+
+            if will_auto_send_ack(frm) {
+                // hardware-driven auto-ACK for frame version 0b00 and 0b01
                 state.state = Ieee802154State::TxAck;
                 *needs_next_op = false;
             } else if should_send_enhanced_ack(frm) {
                 // Enhanced ACK for frame version 0b10 - TODO: full enh-ack support
-                // Frame data already copied above.
                 state.state = Ieee802154State::TxEnhAck;
                 *needs_next_op = false;
             } else {
-                // No ACK needed, notify immediately (data already copied above)
-                super::rx_available();
+                // No ACK needed
                 *needs_next_op = true;
             }
```

## How to reproduce / verify

Bring up OpenThread on an ESP32-H2 via `esp-radio` + `esp-rs/openthread`, apply a valid
dataset with a router in range, `enable_thread(true)`.

- **Before:** instrumenting the MAC ISR shows `RxSfdDone` firing continuously with
  `rx_abort = SfdTimeout`, `RxDone` essentially never; role stuck `Detached`.
- **After (this PR + the ext-addr fix):** `RxDone` fires steadily, frames are delivered,
  role reaches `Child`.

Each fix's individual contribution is observable: (1)+(2) take `RxDone` from ~0 to
non-zero, (4) sharply reduces the `SfdTimeout` rate, (3) is required for ACK-bearing
unicast frames to be delivered at all.

## Notes for maintainers

- **Coexistence scope of (1) and (4).** These two touch BLE↔802.15.4 coexistence. The
  original `ensure_receive_enabled()` comment attributes the poll-restart hack to "rx
  stopping unexpectedly when used together with BLE," and (4) raises 802.15.4 priority.
  On a standalone Thread device these are unambiguous improvements. If you are concerned
  about regressing a concurrent BLE + 802.15.4 workload, (1) and (4) can be split into a
  separate PR from (2) and (3) (which are pure correctness fixes with no coexistence
  implications). We can provide separate branches if preferred.
- **Chip scope.** Observed and fixed on ESP32-H2. The same code paths are shared with
  C6/C5/C61; the fixes are not H2-specific in principle, but we have only validated on
  H2 hardware. Worth a check on C6.
- **`AckTxDone` on H2 (issue 3).** We did not root-cause *why* `AckTxDone` does not fire
  on H2 — it may be a separate MAC/event-enable issue. Delivering on `RxDone` is correct
  regardless (the frame is already queued and the hardware ACKs autonomously), but if a
  maintainer knows why `AckTxDone` is absent that would be worth a follow-up.

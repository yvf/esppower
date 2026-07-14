# Upstream PR: fix ESP32-H2 IEEE 802.15.4 receive path (re-arm, deliver, enhanced-ACK)

**Repository:** `esp-rs/esp-hal` (crate: `esp-radio`)
**Files:** `esp-radio/src/ieee802154/raw.rs`, `esp-radio/src/ieee802154/hal.rs`
**Diff base:** current `main` (`esp-radio` 1.0.0-beta.0).
**Fork branch:** `fix/h2-ieee802154-receive-path` (in `~/github/yanf-esp-hal`), three
commits - one per fix group below.

> Standalone esp-radio PR. (An earlier draft paired this with an ext-address
> filter "fix"; that turned out to be an `esp-rs/openthread` bug, fixed upstream in
> openthread PR #84 - stock esp-radio byte order is correct - so this receive-path
> PR is the only esp-radio change. See the README.)
>
> **Builds on [#5650]** (already in `main`), which fixed the FCF octet offset in
> `frame_is_ack_required`/`frame_get_version`. Because those helpers now read the
> correct octet, this PR can keep using them and does not need to hand-parse the
> FCF.

---

## PR title

    fix(esp-radio): reliable 802.15.4 receive + enhanced-ACK on ESP32-H2

## Summary

On ESP32-H2 the 802.15.4 receiver transmits fine but is unreliable at receiving,
and never completes the 802.15.4-2015 (v2) ACK handshake that Thread 1.3 links
depend on. Concretely: it detects the start of most frames (`RxSfdDone`) but the
reception aborts (`rx_abort = SfdTimeout`) and `RxDone` almost never fires; when
a frame *is* completed, ACK-required frames are copied but never delivered; and
version-2 ack-required frames are never acknowledged, so the parent evicts the
child. In practice an OpenThread node cannot attach, and even if it does,
operational CASE/SRP traffic never completes.

Three independent fixes, one commit each:

1. **Re-arm RX after an aborted reception** (`raw.rs`).
2. **Deliver received frames on `RxDone`**, not on the never-firing `AckTxDone`
   (`raw.rs`).
3. **Generate and transmit an enhanced ACK** for 802.15.4-2015 v2 ack-required
   frames (`raw.rs` + `hal.rs`).

They interact (each is necessary but not sufficient on its own) but are cleanly
separable; (1) and (2) are pure correctness fixes, (3) adds a small amount of new
functionality gated on the existing `Config::enhance_ack_tx`.

## Root causes and fixes

### (1) The receiver goes dead after an abort, and a poll-hack aborts live frames

Two related issues in the same area:

- **RX-abort events are masked, so RX never re-arms after an abort.**
  `ieee802154_mac_init` enables only `TxAckTimeout | TxAckCoexBreak` RX-abort
  events. So when RX aborts while waiting for / receiving a frame (`SfdTimeout`,
  `CrcError`, `InvalidLen`, `FilterFail`, `NoRss`), **no MAC interrupt is
  raised**, the ISR never runs, and RX is never re-armed - the receiver goes
  dead until the next TX happens to call `enable_rx()` again. The recovery code
  already exists (`isr_handle_rx_phase_rx_abort` requests
  `next_operation -> enable_rx`); it just never runs because the events are
  masked. **Fix:** unmask all RX-abort events in `enable_rx()`.

- **`ensure_receive_enabled()` aborts in-flight frames.** It is polled from the
  receive poll on **every** call and unconditionally issues
  `set_cmd(Command::RxStart)` while in `Receive`. If a frame is mid-reception
  (after `RxSfdDone`, before `RxDone`), this restarts the receiver and aborts the
  in-flight frame -> `SfdTimeout`, and `RxDone` never fires. The function is
  already flagged with a `FIXME` noting it "shouldn't be necessary." With the
  re-arm mechanism above it is not. **Fix:** make it a no-op.

Together these take `RxDone` on ESP32-H2 from ~never to steady.

### (2) Delivery deferred to auto-ACK never happens (`AckTxDone` doesn't fire on H2)

`isr_handle_rx_done` copies the frame into `rx_queue`, then for ACK-required
frames sets state `TxAck` and **defers** the `rx_available()` notification until
`isr_handle_ack_tx_done` (i.e. until `AckTxDone`). On ESP32-H2 `AckTxDone` is not
observed to fire, so those frames - including a unicast MLE Parent Response -
are copied but **never delivered** to the upper layer, and the radio stays in
`TxAck`. Since the frame is already safely in `rx_queue`, **notify immediately in
all branches**; the hardware still transmits the auto-ACK independently.
`isr_handle_ack_tx_done` correspondingly stops re-notifying (that would signal
the same queued frame twice - observed as OpenThread's "Failed to process UDP:
Duplicated" flood).

### (3) 802.15.4-2015 (v2) frames are never acknowledged (`should_send_enhanced_ack` is a stub)

For a frame-version-`0b10` ack-required frame, `should_send_enhanced_ack`
returned true but the ISR only parked the radio in `TxEnhAck` and never generated
or sent an ACK, so the radio wedged and the frame went unacknowledged. Thread 1.3
links (e.g. Apple border routers) use version-2 data frames that **require** an
enhanced ACK - without one the parent marks the link failed, retransmits, and
evicts the child, so operational traffic over Thread never completes.

**Fix:** build a minimal enhanced ACK (frame type Acknowledgement, version
`0b10`, sequence number matched to the acknowledged frame, no addressing, no IEs,
unsecured) into a dedicated buffer, point the TX DMA at it, and strobe a new
`enhack_generate_done_notify()` (writing
`ENHANCE_ACK_CFG.tx_enh_ack_generate_done_notify`) so the MAC transmits it in the
ACK window - mirroring the ESP-IDF C driver. Completion raises `AckTxDone`, which
re-arms RX via the same path the immediate ACK uses. `should_send_enhanced_ack`
is also tightened from `<= FRAME_VERSION_2` to `== FRAME_VERSION_2`, so a v0/v1
frame gets the immediate-ACK path and never a v2-format enhanced ACK.

A minimal (no-IE, unsecured) enhanced ACK is sufficient for an rx-on (non-sleepy)
child with CSL and link-metrics disabled, which is the common Thread node
configuration; CSL/link-metrics would need IEs and are out of scope here.

## How to reproduce / verify

Bring up OpenThread on an ESP32-H2 via `esp-radio` + `esp-rs/openthread`
(>= PR #84, so the ext-address filter matches), apply a valid dataset with a
Thread 1.3 border router in range, `enable_thread(true)`.

- **Before:** `RxSfdDone` fires continuously with `rx_abort = SfdTimeout`,
  `RxDone` essentially never; role stuck `Detached`. If patched only up to (2),
  the node attaches but a Thread 1.3 parent drops it during operational traffic
  ("Duplicated" / link-failure churn) because v2 frames go unacknowledged.
- **After (this PR, with openthread >= #84):** `RxDone` fires steadily, frames are
  delivered, v2 frames are acknowledged, and the node reaches `Child` and
  completes CASE/SRP. **Verified on ESP32-H2 hardware.**

Each fix's individual contribution is observable: (1) takes `RxDone` from ~0 to
steady; (2) is required for ACK-bearing unicast frames to be delivered at all;
(3) is required for a Thread 1.3 parent to keep the child.

## Notes for maintainers

- **Splitting.** The branch is already three commits: (1) re-arm, (2) deliver on
  `RxDone`, (3) enhanced ACK. (1) and (2) are pure receive-path correctness with
  no new API surface; (3) adds enhanced-ACK generation behind the existing
  `Config::enhance_ack_tx` flag. Any subset can be taken independently.
- **We did NOT touch coexistence PTI.** An earlier iteration raised the active
  TX/RX coex priority from `IEEE802154_LOW` to `IEEE802154_HIGH` to fight the
  `SfdTimeout` flood. That turned out to be **unnecessary** once the real RX bugs
  (the openthread ext-address decode, fixed upstream in #84, and the abort-event
  re-arm here) were fixed, and raising it
  **starved concurrent BLE** advertising/commissioning on the H2's shared radio.
  The PTI values are left at their upstream defaults.
- **`AckTxDone` on H2.** We did not root-cause *why* `AckTxDone` does not fire for
  the immediate-ACK path on H2 - it may be a separate MAC/event-enable issue.
  Delivering on `RxDone` (fix 2) is correct regardless: the frame is already
  queued and the hardware ACKs autonomously. A maintainer who knows why
  `AckTxDone` is absent may want a follow-up.
- **Chip scope.** Observed and fixed on ESP32-H2. The same code paths are shared
  with C6/C5/C61; the fixes are not H2-specific in principle, but only H2 has
  been validated. Worth a check on C6.

[#5650]: https://github.com/esp-rs/esp-hal/pull/5650

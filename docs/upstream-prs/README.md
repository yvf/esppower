# Upstream PRs — esp-radio ESP32-H2 802.15.4 fixes

> **STATUS (2026-07-05) — final vendored patch set (post de-vendoring review). The
> detailed PR files below (01, 02) are partly stale and need a rewrite pass before actual
> submission.** Checked against upstream `esp-radio` **1.0.0-beta.0** (latest): it STILL
> has the root-cause bugs — `ext_addr.to_le_bytes()`, `ensure_receive_enabled` restarting
> RX, and `isr_handle_rx_done` deferring delivery to a `AckTxDone` that never fires on H2.
> So **the vendor cannot be dropped**; the goal is a minimal, upstreamable diff.
>
> The vendored patches, all in `src/ieee802154/`:
> 1. **ext-address filter byte order** (`mod.rs`, 1 line) — PR 01, confirmed root cause.
> 2. **re-arm RX on all abort events** in `enable_rx` (`raw.rs`).
> 3. **`ensure_receive_enabled` → no-op** (`raw.rs`) — stop restarting RX mid-frame.
> 4. **deliver on `RxDone`** for all frames (`raw.rs`) — AckTxDone is unreliable on H2.
> 5. **`isr_handle_ack_tx_done` de-dup** (`raw.rs`) — matches (4); avoids double-delivery.
> 6. **enhanced-ACK TX** for 802.15.4-2015 v2 ack-required frames (`raw.rs` + `hal.rs`
>    `enhack_generate_done_notify`) — completes upstream's stubbed `should_send_enhanced_ack`
>    (which today parks the radio in `TxEnhAck` forever and never sends the ACK). Kept as
>    **protective**: without it, the first v2 ack-required frame would wedge RX.


Five bugs in `esp-rs/esp-hal`'s `esp-radio` crate that together made IEEE 802.15.4
**receive** non-functional on ESP32-H2, so an OpenThread node could never attach
(transmit worked; the node stayed `Detached`). All five were found and fixed while
bringing up a Matter-over-Thread device on the H2; the device now attaches as a `Child`.

Grouped into two independent PRs:

| File                                                                                         | PR                            | Scope                                                                                                                                                                                |
|----------------------------------------------------------------------------------------------|-------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| [`01-ieee802154-ext-addr-filter-byte-order.md`](01-ieee802154-ext-addr-filter-byte-order.md) | ext-address filter byte order | `mod.rs`, 1 line. **Root cause** of "attach impossible": the HW ext-address filter was byte-reversed, so unicast frames addressed to the node were never accepted (only broadcasts). |
| [`02-h2-802154-receive-path.md`](02-h2-802154-receive-path.md)                               | receive path reliability      | `raw.rs`, 4 related fixes: re-arm RX on abort, don't restart RX mid-frame, deliver on `RxDone` instead of the never-firing `AckTxDone`, and raise RX coex priority.                  |

The two PRs are independent and can be reviewed/merged separately, but **both** are
required for a working Thread attach: PR 02 makes the receiver reliably complete and
deliver frames; PR 01 makes it accept the unicast frames the attach handshake depends on.

Diffs are against `esp-radio` 0.18.0, verified identical to `main` for the affected
functions as of 2026-07-04. Rebase onto `main` before submitting.

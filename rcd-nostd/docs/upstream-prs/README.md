# Upstream PRs — esp-radio ESP32-H2 802.15.4 fixes

> **STATUS (2026-07-05) — these PR write-ups are being reconciled with the final
> vendored patch set and are partly stale.** After further hardware bring-up the actual
> patches in `vendor/esp-radio` are: **(1)** ext-address filter byte order [PR 01,
> confirmed root cause, KEEP]; **(2)** re-arm RX on all abort events; **(3)**
> `ensure_receive_enabled` → no-op; **(4)** deliver on `RxDone` + **(5)** the matching
> `isr_handle_ack_tx_done` de-dup [(2)–(5) = the receive-path PR, KEEP]; **(6)** ACK
> **coex** PTI MIDDLE→HIGH (this REPLACED the *RX-scene* coex-priority change described
> in PR 02 §4, which was **reverted** — it starved BLE discovery); **(7)** enhanced-ACK
> TX for 802.15.4-2015 version-2 frames (a separate concern; dormant unless the peer
> sends v2 ack-required frames). The docs below still describe PR 02 §4 as an RX-scene
> priority bump — that is out of date. This README + the PR files will be rewritten to
> the minimal upstreamable set once the de-vendoring review settles which of (6)/(7) are
> load-bearing.



Five bugs in `esp-rs/esp-hal`'s `esp-radio` crate that together made IEEE 802.15.4
**receive** non-functional on ESP32-H2, so an OpenThread node could never attach
(transmit worked; the node stayed `Detached`). All five were found and fixed while
bringing up a Matter-over-Thread device on the H2; the device now attaches as a `Child`.

Grouped into two independent PRs:

| File | PR | Scope |
|------|----|-------|
| [`01-ieee802154-ext-addr-filter-byte-order.md`](01-ieee802154-ext-addr-filter-byte-order.md) | ext-address filter byte order | `mod.rs`, 1 line. **Root cause** of "attach impossible": the HW ext-address filter was byte-reversed, so unicast frames addressed to the node were never accepted (only broadcasts). |
| [`02-h2-802154-receive-path.md`](02-h2-802154-receive-path.md) | receive path reliability | `raw.rs`, 4 related fixes: re-arm RX on abort, don't restart RX mid-frame, deliver on `RxDone` instead of the never-firing `AckTxDone`, and raise RX coex priority. |

The two PRs are independent and can be reviewed/merged separately, but **both** are
required for a working Thread attach: PR 02 makes the receiver reliably complete and
deliver frames; PR 01 makes it accept the unicast frames the attach handshake depends on.

Diffs are against `esp-radio` 0.18.0, verified identical to `main` for the affected
functions as of 2026-07-04. Rebase onto `main` before submitting.

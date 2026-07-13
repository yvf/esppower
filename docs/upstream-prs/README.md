# Upstream PRs

Fixes made to dependencies while bringing up this Matter-over-Thread device on
the ESP32-H2, prepared for submission upstream. Two are real PRs against
`esp-rs/esp-hal` (`esp-radio`); a third dependency (`ccm`) turned out to need
**no** PR - see below.

Diffs are against current upstream `main` (`esp-radio` 1.0.0-beta.0), verified to
still have the bugs. Fork branches are prepared and compile-checked for
`riscv32imac-unknown-none-elf` (esp32h2, ieee802154).

This repo no longer vendors any dependency source: both `esp-radio` and `ccm` are
now consumed from branches on the public forks via `[patch.crates-io]` (`vendor/`
is gone). **Before a clean checkout can build, push the two consumed branches** -
see "Pushing" at the end.

## esp-radio - ESP32-H2 802.15.4 fixes

Together these made IEEE 802.15.4 **receive** functional on ESP32-H2 so an
OpenThread node can attach and operate. Grouped into two independent PRs.

| Doc | Fork branch (`~/github/yanf-esp-hal`) | Scope |
|-----|----------------------------------------|-------|
| [`01-ieee802154-ext-addr-filter-byte-order.md`](01-ieee802154-ext-addr-filter-byte-order.md) | `fix/ieee802154-ext-addr-filter-byte-order` | `mod.rs`, 1 line. **Reverts #5314.** The HW ext-address filter was byte-reversed (LE), so unicast frames addressed to the node were never accepted - only broadcasts. Root cause of "attach impossible". |
| [`02-h2-802154-receive-path.md`](02-h2-802154-receive-path.md) | `fix/h2-ieee802154-receive-path` | `raw.rs` + `hal.rs`, 3 commits: re-arm RX on abort, deliver on `RxDone` instead of the never-firing `AckTxDone`, and generate an enhanced ACK for 802.15.4-2015 (v2) frames. |

The two PRs are independent and can be reviewed/merged separately, but **both**
are required for a working Thread node: PR 02 makes the receiver reliably
complete, deliver, and acknowledge frames; PR 01 makes it accept the unicast
frames the attach handshake depends on.

**What changed from earlier drafts of these docs** (kept here so the history
makes sense):

- PR 01 is now framed as a **revert of #5314**. That PR ("use little-endian byte
  order...") changed the correct `to_be_bytes` to `to_le_bytes`; the original had a
  literal `// LE or BE?` comment and #5314 guessed wrong. See the doc for the
  `openthread` `from_be_bytes` evidence.
- PR 02 **dropped the coex-priority raise** (`IEEE802154_LOW -> HIGH`). It was
  unnecessary once the real RX bugs were fixed and it starved concurrent BLE on
  the shared H2 radio.
- PR 02 **added enhanced-ACK generation** for v2 frames (needed for Thread 1.3 /
  Apple links), and no longer hand-parses the FCF: upstream **#5650** (already in
  `main`) fixed the FCF octet offset, so the existing
  `frame_is_ack_required`/`frame_get_version` helpers are now correct.

### How this repo consumes the fixes (no more vendor tree)

The two PR branches above are against `main` (esp-radio **1.0.0-beta.0**) for
upstreaming. This project, however, is pinned to esp-radio **^0.18** (so are
`openthread` and `esp-rtos`), so it can't `[patch]` to a beta.0 branch. So there
is a **third, non-upstream branch** carrying the same fixes on the 0.18.0 base:

- **`rcd/esp-radio-0.18-h2-154-fixes`** (`yvf/esp-hal`, rev `4b0e8323`): the
  crates.io-published esp-radio **0.18.0** crate (normalized manifest -> registry
  deps, so it builds against esp-hal 1.1) plus the ext-address + receive-path
  fixes. Consumed via `[patch.crates-io] esp-radio = { git = "...yvf/esp-hal",
  rev = "4b0e8323..." }`.

This **replaces the former `vendor/esp-radio` source tree**, which has been
deleted. The whole project was built against this fork branch over git
(`./build.sh`, exit 0) to confirm the diff works for us.

## ccm (`RustCrypto/AEADs`) - no upstream PR; consumed from a fork branch

The only change `ccm` needs is a one-line `Cargo.toml` pin: released `ccm` 0.4.4
pins `subtle = "=2.4"` exactly, which collides with `rs-matter`'s `subtle ^2.6`
(no source changes are involved).

There is **nothing to PR against AEADs upstream**: current `master` already ships
`ccm` 0.6.0-rc.3 with `subtle = "2"` (allows 2.6). But that does not help us -
`esp-radio` pulls `ccm` via `ieee802154 0.6.1`, which requires `ccm ^0.4.0`, and
**every public `ccm` 0.4.x pins `subtle` too tightly** (`=2.4`, or at loosest
`>=2, <2.5` - still excludes 2.6). The versions that relax it (0.5.0+) are
semver-incompatible with `ieee802154`'s `^0.4.0`. Upstream will not cut a new
0.4.x patch release, so there is no pristine public version to point at, and the
proper long-term fix is on the `esp-radio`/`ieee802154` side (bump `ccm` to >=0.5),
not a change to AEADs.

Instead of vendoring the `ccm` 0.4.4 source into this repo, we now consume it from
a **one-line branch on the public `yvf/AEADs` fork** and delete `vendor/ccm/`:

- Fork branch: **`ccm-0.4.4-relax-subtle`** (in `~/github/yanf-AEADs`), one commit
  off the `ccm-v0.4.4` tag, changing `subtle = "=2.4"` -> `subtle = "2"` (the same
  value ccm 0.5.0 uses upstream). Rev `790a0a5`.
- Wired in via `[patch.crates-io] ccm = { git = "https://github.com/yvf/AEADs",
  rev = "790a0a5..." }` (consistent with how this project already patches
  `rs-matter`/`rs-matter-stack` to git forks).

See "Pushing" below for the required push. (The relaxed 0.4.4 was verified to
resolve and build locally: `subtle` unifies to a single 2.6.1.)

## Pushing (required for a clean checkout to build)

`[patch.crates-io]` in the root `Cargo.toml` pins both consumed branches by
immutable git **rev**, so they must exist on the public fork remotes:

```sh
git -C ~/github/yanf-AEADs   push -u origin ccm-0.4.4-relax-subtle          # rev 790a0a5
git -C ~/github/yanf-esp-hal push -u origin rcd/esp-radio-0.18-h2-154-fixes  # rev 4b0e8323
```

Both were built together via a local `file://` git patch (`./build.sh`, exit 0),
so the GitHub form resolves identically once pushed.

To also open the upstream esp-radio PRs, push the two `fix/*` branches:

```sh
git -C ~/github/yanf-esp-hal push -u origin fix/ieee802154-ext-addr-filter-byte-order
git -C ~/github/yanf-esp-hal push -u origin fix/h2-ieee802154-receive-path
```

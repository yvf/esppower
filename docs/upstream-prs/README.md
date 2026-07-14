# Upstream PRs

Fixes made to dependencies while bringing up this Matter-over-Thread device on
the ESP32-H2, prepared for submission upstream. One is a real PR against
`esp-rs/esp-hal` (`esp-radio`); a second dependency (`ccm`) needs **no** PR - see
below.

This repo does not vendor any dependency source: `esp-radio` and `ccm` are
consumed from branches on the public forks via `[patch.crates-io]`.

## esp-radio - ESP32-H2 802.15.4 receive-path fix

| Doc | Fork branch (`~/github/yanf-esp-hal`) | Scope |
|-----|----------------------------------------|-------|
| [`02-h2-802154-receive-path.md`](02-h2-802154-receive-path.md) | `fix/h2-ieee802154-receive-path` | `raw.rs` + `hal.rs`, 3 commits: re-arm RX on abort, deliver on `RxDone` instead of the never-firing `AckTxDone`, and generate an enhanced ACK for 802.15.4-2015 (v2) frames. |

The diff is against current upstream `main` (`esp-radio` 1.0.0-beta.0), verified to
still have the bugs, and compile-checked for `riscv32imac-unknown-none-elf`
(esp32h2, ieee802154).

### The ext-address byte-order issue was NOT an esp-radio bug (former PR 01, withdrawn)

An earlier draft carried a second PR ("`01-ieee802154-ext-addr-filter-byte-order`")
that reverted esp-hal **#5314**, changing the HW ext-address filter from
`ext_addr.to_le_bytes()` back to `to_be_bytes()`. **That was wrong and has been
withdrawn.** `otPlatRadioSetExtendedAddress` passes the extended address
little-endian (per `openthread/platform/radio.h`), so #5314's `to_le_bytes()` is
the spec-correct behaviour.

The real bug was in the **`esp-rs/openthread`** platform shim, which decoded that
little-endian address with `from_be_bytes` (introduced by openthread `d48d3d7`,
Mar 2025). That is what made the filter never match unicast frames. It was fixed
**upstream in openthread PR #84** ("Fix extended-address byte order: decode
little-endian", Jun 2026), which decodes little-endian at both the platform
callback and the `MacRadio` frame parser and explicitly cites #5314 as correct.

Consequence for this project: we run **openthread >= #84** (pinned esp-rs/openthread
`main`) and **stock esp-radio byte order** (no ext-addr patch). Carrying the old
`to_be_bytes` revert on top of a fixed openthread would re-reverse the filter.

### How this repo consumes the receive-path fix (no vendor tree)

The PR branch above is against `main` (esp-radio **1.0.0-beta.0**) for upstreaming.
This project is pinned to esp-radio **^0.18** (so are `openthread` and `esp-rtos`),
so it can't `[patch]` to a beta.0 branch. A **non-upstream branch** carries the same
receive-path fixes on the 0.18.0 base:

- **`rcd/esp-radio-0.18-h2-154-rx-fixes`** (`yvf/esp-hal`, rev `81b88cfe`): the
  crates.io-published esp-radio **0.18.0** crate (normalized manifest -> registry
  deps, so it builds against esp-hal 1.1) plus the **receive-path** fixes only.
  Consumed via `[patch.crates-io] esp-radio = { git = "...yvf/esp-hal",
  rev = "81b88cfe..." }`.

This supersedes the earlier `rcd/esp-radio-0.18-h2-154-fixes` branch (rev
`4b0e8323`), which also carried the now-withdrawn ext-address `to_be_bytes` patch;
that branch should be retired.

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

Instead of vendoring the `ccm` 0.4.4 source into this repo, we consume it from
a **one-line branch on the public `yvf/AEADs` fork**:

- Fork branch: **`ccm-0.4.4-relax-subtle`** (in `~/github/yanf-AEADs`), one commit
  off the `ccm-v0.4.4` tag, changing `subtle = "=2.4"` -> `subtle = "2"` (the same
  value ccm 0.5.0 uses upstream). Rev `790a0a5`.
- Wired in via `[patch.crates-io] ccm = { git = "https://github.com/yvf/AEADs",
  rev = "790a0a5..." }`.

(The relaxed 0.4.4 was verified to resolve and build locally: `subtle` unifies to a
single 2.6.1.)

## Pushing (required for a clean checkout to build)

`[patch.crates-io]` in the root `Cargo.toml` pins both consumed branches by
immutable git **rev**, so they must exist on the public fork remotes:

```sh
git -C ~/github/yanf-AEADs   push -u origin ccm-0.4.4-relax-subtle             # rev 790a0a5
git -C ~/github/yanf-esp-hal push -u origin rcd/esp-radio-0.18-h2-154-rx-fixes  # rev 81b88cfe
```

The esp-radio patch currently points at a local `file://` path; switch it to
`https://github.com/yvf/esp-hal` once the `rx-fixes` branch is pushed.

To also open the upstream esp-radio receive-path PR, push the `fix/*` branch:

```sh
git -C ~/github/yanf-esp-hal push -u origin fix/h2-ieee802154-receive-path
```

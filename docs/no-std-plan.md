# no-std Matter-over-Thread firmware (ESP32-H2) - architecture & build

## Why no-std (rather than ESP-IDF)

The conventional path for Matter-over-Thread on Espressif is ESP-IDF: the Bluedroid BLE
stack + OpenThread + rs-matter, glued by esp-idf-matter. On the ESP32-H2 that ran into a
hard memory wall - with only **320 KB SRAM**, bringing up Bluedroid + OpenThread +
rs-matter together drove the heap to nearly zero, and the Matter UDP network task failed
to spawn (out of memory). Bluedroid could not be trimmed enough to recover the headroom.

This firmware is bare-metal `no_std` instead. Removing the FreeRTOS/Bluedroid overhead
frees most of the 320 KB for the heap, which is enough to run commissioning and
operation comfortably. (esp-alloc places the heap in the SRAM left over after statics.)

## The stack (all no-std, target `riscv32imac-unknown-none-elf`, nightly)

| Layer      | Crate(s)                                                                            | Notes                                                                                                                  |
|------------|-------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------|
| HAL / chip | **esp-hal 1.1** (`esp32h2`, `unstable`)                                             | peripherals, ADC, LEDC, GPIO                                                                                           |
| RTOS/async | **esp-rtos 0.3** (`esp-radio`,`embassy`) + embassy 0.10/0.8/0.5                     | scheduler + radio glue                                                                                                 |
| Heap       | **esp-alloc 0.10**                                                                  | most of the 320 KB SRAM becomes heap                                                                                  |
| Radio      | **esp-radio 0.18** (`ieee802154` + BLE) - **patched** (fork branch)                 | one crate, both radios + coex. `[patch.crates-io]` to a fork branch that fixes the ESP32-H2 802.15.4 receive path (see `docs/upstream-prs/`).  |
| Thread     | **openthread 0.2** (pinned esp-rs/openthread `main`): `esp-radio`,`udp`,`srp`,`dns-client` | OpenThread C via `openthread-sys`; H2 802.15.4 out of the box; native `UdpSocket` + SRP (Matter's mDNS-over-Thread). edge-nal glue via `edge-nal-openthread`. |
| BLE host   | **trouble-host 0.6** on esp-radio's BLE controller                                  | GATT server; version-pinned, see below                                                                                 |
| Matter     | **rs-matter 0.2** (`no_std`) + **rs-matter-stack** (edge-nal 0.7)                   | generic stack; edge-nal UDP + a `Gatt`/`ThreadCoex` transport                                                          |
| Boot       | esp-bootloader-esp-idf 0.5, esp-backtrace/println                                   | (`esp-bootloader-esp-idf` = the IDF partition-table/app-descriptor format, not the IDF runtime)                       |

## The transport layer (custom glue)

rs-matter-stack is generic over its network and BLE transports but ships no no-std ESP
implementation - the openthread/trouble examples are Thread-only / BLE-only, and
rs-matter-stack's own examples are std/Linux. So the core custom code (`src/matter/`) is
a transport layer implementing rs-matter-stack's `NetStack` / `Netif` / `NetCtl` / `Mdns`
/ `GattPeripheral` traits over **openthread** (Thread netif/UDP/SRP) and **trouble** (BLE
GATT peripheral). The `NetStack`'s UDP factory is the upstream **`edge-nal-openthread`**
crate (`OtUdpStack`) - openthread 0.2 dropped its own edge-nal integration, and that glue
now lives in a dedicated crate, so `src/matter/net.rs` is just a thin wrapper around it.
The device joins Thread *during* commissioning: the commissioner sends
the operational dataset via the NetworkCommissioning cluster, and `OtNetCtl` applies it
to openthread. See `docs/phase4b-glue-design.md` for the adapter/BTP design detail.

## Build prerequisites (host)

`openthread-sys` links a prebuilt OpenThread lib for `riscv32imac-unknown-none-elf`
(no C build), but `mbedtls-rs-sys` does an **on-the-fly mbedtls C build** because our
crypto feature subset differs from its prebuilt config. That needs, on PATH:
- a **RISC-V-capable clang** - Apple's `/usr/bin/clang` has NO riscv32 target; use
  brew LLVM: `/opt/homebrew/opt/llvm/bin/clang` (+ `LIBCLANG_PATH=/opt/homebrew/opt/llvm/lib`).
- **cmake** - the system one is fine (3.x or **>= 4.4**). The mbedtls toolchain file sets
  `CMAKE_SYSTEM_NAME=Generic` so cmake must not add the macOS host `-arch` flag (clang
  rejects `-arch` for riscv: `unsupported option '-arch' for target 'riscv32'`). NOTE: early
  cmake **4.x (4.0-4.3)** had a regression that leaked `-arch` despite `Generic`; that was
  fixed by **4.4**. `build.sh` warns if it sees a 4.0-4.3.

**Just use the wrapper** - `./build.sh` sets all of the above:
```sh
./build.sh                 # cargo build
./build.sh run --release   # flash + monitor
```

> Also: a fresh `cargo build` after a failed one can leave a corrupt mbedtls cmake
> cache (compiler-test failures). `cargo clean -p mbedtls-rs-sys -p openthread-sys`
> then rebuild.

## Key implementation decisions & constraints

**Commissioning is non-concurrent, not coex.** The H2 has a single 2.4 GHz radio shared
between BLE and 802.15.4; running both simultaneously is unreliable. The stack therefore
uses rs-matter-stack's non-concurrent `run` (which advertises
`SupportsConcurrentConnection = false`): BLE only while un-commissioned, then Thread-only
once a fabric exists. `PreexistingWireless` implements both `Thread`+`Gatt` and
`ThreadCoex`, so it is the same adapters either way - just different orchestration. See
`src/matter/stack.rs`.

**Patched esp-radio for H2 802.15.4 RX.** esp-radio 0.18's 802.15.4 receive path is
broken on the ESP32-H2 (the RX ISR never re-arms after an abort, never delivers on
`RxDone`, and never generates an enhanced ACK for v2 frames, so OpenThread can never
attach or stay attached). `[patch.crates-io]` points esp-radio at a fork branch
(`yvf/esp-hal` @ `rcd/esp-radio-0.18-h2-154-rx-fixes`) - the published 0.18.0 crate plus
those RX-correctness fixes - instead of a vendored source tree. The clean, upstreamable
form (against current `main`/beta.0) is on the fork's `fix/h2-ieee802154-receive-path`
branch; the diff and PR write-up are in `docs/upstream-prs/`.

Note: there is **no** ext-address byte-order patch. That symptom (unicast frames never
matching the HW filter) was an `esp-rs/openthread` bug - its platform shim decoded the
little-endian ext address with `from_be_bytes` - fixed upstream in openthread PR #84.
Stock esp-radio uses `to_le_bytes` (esp-hal #5314), which is spec-correct, so with
openthread >= #84 (we pin a recent `main`) the filter matches without any esp-radio
change. See `docs/upstream-prs/README.md`.

**Persistence kept off the radio hot path.** The Matter fabric and the OpenThread SRP key
are flash-backed (`src/matter/kv.rs`, `src/matter/ot_settings.rs`) so pairing survives
reboots and a commissioned device comes straight back up over Thread (no BLE). esp-storage
flash writes run with interrupts off (~15 ms/sector), which would starve 802.15.4 during
SRP registration, so writes are deferred / whitelisted to land in radio lulls.

**BLE version matrix (don't drift).** openthread pins **esp-radio 0.18 -> bt-hci 0.8.1**,
so BLE must use **trouble-host 0.6** (also bt-hci 0.8.1); trouble `main`/0.7 needs bt-hci
0.9 and won't typecheck against `BleConnector`. trouble 0.6 uses **embassy-sync 0.7**, so
this crate's *direct* `embassy-sync` dep is pinned to **0.7** (trouble's `#[gatt_server]`
macro resolves a bare `embassy_sync` path against our deps). openthread/rs-matter pull
**embassy-sync 0.8** transitively; the two coexist as long as one version's mutex is never
passed where the other is expected. esp-radio needs both `ieee802154` and `ble` features.

**Crypto version conflict (resolved).** `ccm 0.4.4` (pulled by `ieee802154 0.6.1` ->
esp-radio's 802.15.4 AES-CCM) pins `subtle "=2.4"` exactly, vs rs-matter's `subtle ^2.6`.
No public `ccm 0.4.x` relaxes that pin (0.5.0+ do - with `subtle = "2"` - but are
semver-incompatible with `ieee802154`'s `^0.4.0`), so `[patch.crates-io] ccm` points at a
one-line branch of the `yvf/AEADs` fork that relaxes it to `subtle = "2"` (the same value
ccm 0.5.0 uses; API-compatible for ccm's constant-time tag compare). A direct
`subtle = "2.6"` dep forces unification to 2.6.1, so `ccm 0.4.4` (esp-radio) and `ccm 0.5`
(rs-matter) coexist on one `subtle`. (Previously this was a `vendor/ccm` path copy; the
fork branch removes the in-repo source tree - the branch is the only change.)

## Device credentials (bring-up)

The firmware uses rs-matter's **TEST** device credentials (VID 0xFFF1). chip-tool and
Home Assistant accept these; **Apple Home requires a real DAC** (it rejects the CHIP test
PAA root), which needs CSA membership + a certified VID. The Active Operational Dataset
(Thread network key) is supplied at build time via env, never committed.

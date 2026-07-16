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

The firmware is built on **`rs-matter-embassy`** (ivmarkov's off-the-shelf esp
Matter-over-Thread integration), which bundles the whole transport. All the `esp-*` crates
are pinned - via `[patch.crates-io]` in `Cargo.toml` - to a single esp-hal git revision
(`10e48dd...`), the coordinated stack the `rs-matter-embassy` esp example uses;
`openthread` / `openthread-sys` / `mbedtls-rs-sys` track `esp-rs` `main`. See "Why these
exact versions" below.

| Layer      | Crate(s)                                                                            | Notes                                                                                                                  |
|------------|-------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------|
| Integration| **rs-matter-embassy** (`esp`,`openthread`,`mbedtls`)                                | `EmbassyThreadMatterStack` + `EspThreadDriver` = the whole Thread+BLE transport + persistence; we supply only the device model |
| HAL / chip | **esp-hal ~1.1** (`esp32h2`, `unstable`) @ `10e48dd`                                | peripherals, ADC, LEDC, GPIO                                                                                           |
| RTOS/async | **esp-rtos 0.3** (`esp-radio`,`embassy`) + embassy                                  | scheduler + radio glue                                                                                                 |
| Heap       | **esp-alloc 0.10**                                                                  | most of the 320 KB SRAM becomes heap                                                                                  |
| Radio      | **esp-radio 0.18** (`ieee802154` + BLE), **stock** @ `10e48dd`                      | one crate, both radios. The `10e48dd` base has esp-radio 0.18 **with #5650** (the FCF-offset fix); no local patch (see below). |
| Thread     | **openthread 0.2** (pinned esp-rs/openthread `main`)                                | OpenThread C via `openthread-sys`; H2 802.15.4 out of the box; native `UdpSocket` + SRP; edge-nal glue via `edge-nal-openthread`. |
| BLE host   | **trouble-host 0.6** on esp-radio's BLE controller                                  | GATT server; version-pinned via rs-matter-embassy (bt-hci 0.8), see below                                            |
| Matter     | **rs-matter 0.2** + **rs-matter-stack 0.1** (edge-nal 0.7), both crates.io          | generic stack under rs-matter-embassy                                                                                  |
| Boot       | esp-bootloader-esp-idf 0.5, esp-backtrace/println                                   | (`esp-bootloader-esp-idf` = the IDF partition-table/app-descriptor format, not the IDF runtime)                       |

## What we write vs. what rs-matter-embassy provides

`rs-matter-embassy`'s `EmbassyThreadMatterStack` + `EspThreadDriver::new(IEEE802154, BT)`
subsume the entire transport: openthread (Thread netif/UDP/SRP) + esp-radio + trouble (BLE
GATT/BTP) + `edge-nal-openthread` + KV-backed persistence, orchestrated as non-concurrent
BLE-commission-then-Thread. So `src/matter/` is now just the **device model**:

- `stack.rs` - assembles `EmbassyThreadMatterStack`, seeds crypto, builds the node +
  handler chain, wires flash persistence and the factory-reset button, runs the stack.
- `contact.rs` / `plug.rs` - the two endpoint handlers (Contact Sensor + On/Off plug).
- plus `src/{link,controller,sensor,actuator,config}.rs` - the autonomous EMF power-monitor
  / actuator loop, which runs independently of Matter.

The device joins Thread *during* commissioning: rs-matter-embassy applies the operational
dataset the commissioner sends over the NetworkCommissioning cluster. (An earlier version
hand-rolled all of the above transport glue - `NetStack`/`Netif`/`NetCtl`/`Mdns`/
`GattPeripheral` adapters over openthread+trouble - now deleted; `docs/phase4b-glue-design.md`
documents that superseded design.)

## Building (the Makefile)

Build via the **`Makefile`** (it sets up the C-build toolchain env, picks the compile-time
log level, runs preflight checks, and finds the serial port). `build.sh` is the older
wrapper it replaces.

```sh
make                 # cargo build, ESP_LOG=debug   (default target)
make release         # cargo build --release, ESP_LOG=info
make flash           # build debug + flash
make flash-release   # build release + flash
make run             # build debug + flash + monitor (symbolized panics)
make monitor         # attach espmonitor to the device
make clean           # cargo clean + remove esp*.out / chip-tool.out captures
make clean-cache     # drop only the mbedtls/openthread C-build caches (see below)
make help            # list targets + the detected serial port
```

- **Log level** is a *compile-time* constant: the firmware calls
  `esp_println::logger::init_logger_from_env()`, which bakes `ESP_LOG` in at build time, so
  changing it forces a rebuild. Per-target defaults are `debug` for debug builds and `info`
  for release; override on any target with `LOG=trace|debug|info` (e.g.
  `make flash LOG=trace`).
- **Serial port** is auto-detected (`/dev/cu.usbserial-*` / `cu.usbmodem*`) and exported as
  `ESPFLASH_PORT`; override with `PORT=/dev/cu.XXX`.

## Build requirements (host)

**macOS only.** The C-build setup below assumes Homebrew LLVM paths; the Makefile hard-fails
on non-Darwin. Requirements:

- **Rust nightly >= 1.95** (the `10e48dd` esp crates require it). The toolchain is pinned to
  `nightly` with the `riscv32imac-unknown-none-elf` target + `rust-src` via
  `rust-toolchain.toml`; `rustup update nightly` if it's too old.
- **A RISC-V-capable clang** - Apple's `/usr/bin/clang` has **no** riscv32 target, so the
  `mbedtls-rs-sys` / `openthread-sys` C build needs **Homebrew LLVM**: `brew install llvm`.
  The Makefile discovers it with `brew --prefix llvm` (Apple-Silicon and Intel) and puts its
  `bin` on `PATH` + sets `LIBCLANG_PATH` (for bindgen). Override with
  `make <target> LLVM_PREFIX=/path/to/llvm`.
- **cmake** - `brew install cmake`. Any 3.x or **>= 4.4** works. The mbedtls toolchain file
  sets `CMAKE_SYSTEM_NAME=Generic` so cmake must not add the macOS host `-arch` flag (clang
  rejects `-arch` for riscv: `unsupported option '-arch' for target 'riscv32'`). Early cmake
  **4.x (4.0-4.3)** had a regression that leaked `-arch` despite `Generic` (fixed in 4.4);
  the Makefile warns if it sees that range.
- **espflash** / **espmonitor** - `cargo install espflash espmonitor` (needed for `make
  flash` / `run` / `monitor`).

The Makefile's `preflight` target checks the OS, cmake, and LLVM before building and emits an
actionable message (with the `brew`/`cargo install` command) if any is missing.

> `mbedtls-rs-sys` does an **on-the-fly mbedtls C build** (our crypto feature subset differs
> from its committed prebuilt config), and `openthread-sys` links OpenThread; both are why
> clang + cmake are required. A fresh build after a failed one can leave a corrupt mbedtls
> cmake cache (compiler-test / configure failures) - run **`make clean-cache`** (=
> `cargo clean -p mbedtls-rs-sys -p openthread-sys`) then rebuild, rather than a full
> `make clean`.

## Key implementation decisions & constraints

**Commissioning is non-concurrent, not coex.** The H2 has a single 2.4 GHz radio shared
between BLE and 802.15.4; running both simultaneously is unreliable. `rs-matter-embassy`'s
`EmbassyThreadMatterStack::run` is non-concurrent (advertises
`SupportsConcurrentConnection = false`): BLE only while un-commissioned, then Thread-only
once a fabric exists. `EspThreadDriver` implements the driver traits for both modes, so this
is just an orchestration choice, not different code paths.

**Why these exact versions (stock esp-radio, the `10e48dd` base).** We pin all esp crates to
esp-hal git rev `10e48dd` - the last commit where **esp-radio is still 0.18 and already has
#5650** (the FCF AR-bit / frame-version offset fix). This is the base the `rs-matter-embassy`
esp example uses, and on it **stock esp-radio works end-to-end on H2 + Apple** (BLE commission
-> attach as child -> SRP register -> operational CASE over Thread), with no local esp-radio
patch.

**Persistence is handled by rs-matter-embassy.** The Matter fabric, the Thread network, and
the OpenThread SRP key are flash-backed via rs-matter-embassy's `SeqMapKvBlobStore` over the
`nvs` partition (located at runtime from the esp-idf partition table; `src/matter/stack.rs`
`get_persistent_store`, wrapping esp-storage's blocking `FlashStorage` in
`embassy_embedded_hal::adapter::BlockingAsync`). So pairing survives reboots and a
commissioned device comes straight back up over Thread (no BLE). GPIO5 held 3 s triggers a
factory reset (`stack.matter().reset_persist(kv)` then software reset).

**BLE version matrix (don't drift).** openthread pins **esp-radio 0.18 -> bt-hci 0.8.1**, so
BLE uses **trouble-host 0.6** (also bt-hci 0.8.1); trouble `main`/0.7 needs bt-hci 0.9 and
won't typecheck against `BleConnector`. rs-matter-embassy owns the trouble/bt-hci dependency
and the trouble `#[gatt_server]` / `embassy-sync` version juggling internally, so our direct
`embassy-sync` dep is plain **0.8**. esp-radio needs both `ieee802154` and `ble` features.

**Crypto version conflict (resolved).** `ccm 0.4.4` (pulled by `ieee802154 0.6.1` ->
esp-radio's 802.15.4 AES-CCM) pins `subtle "=2.4"` exactly, vs rs-matter's `subtle ^2.6`.
No public `ccm 0.4.x` relaxes that pin (0.5.0+ do - with `subtle = "2"` - but are
semver-incompatible with `ieee802154`'s `^0.4.0`), so `[patch.crates-io] ccm` points at a
one-line branch of the `yvf/AEADs` fork that relaxes it to `subtle = "2"` (the same value
ccm 0.5.0 uses; API-compatible for ccm's constant-time tag compare). A direct
`subtle = "2.6"` dep forces unification to 2.6.1, so `ccm 0.4.4` (esp-radio) and `ccm 0.5`
(rs-matter) coexist on one `subtle`. (Previously this was a `vendor/ccm` path copy; the
fork branch removes the in-repo source tree - the branch is the only change.)

**Crypto RNG seeds from the digital RNG, not the TRNG (ADC1 stays free).** The
rs-matter-embassy esp example seeds its CSPRNG from `TrngSource::new(RNG, ADC1)` - but this
firmware needs **ADC1** for the contactless EMF sensor (GPIO4). So `src/matter/stack.rs`
seeds `default_crypto` from the plain digital `Rng::new()` (a ZST, no ADC1) via a small
rand_core-0.6 shim (`EspRng`), leaving ADC1 for the sensor.

## Device credentials (bring-up)

The firmware uses rs-matter's **TEST** device credentials (VID 0xFFF1). chip-tool and
Home Assistant accept these; **Apple Home requires a real DAC** (it rejects the CHIP test
PAA root), which needs CSA membership + a certified VID. The Active Operational Dataset
(Thread network key) is supplied at build time via env, never committed.

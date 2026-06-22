# Development Environment Setup

## 1. Install Espressif Rust toolchain

```bash
cargo install espup
espup install          # installs the `esp` Rust fork + RISC-V/Xtensa targets
source ~/export-esp.sh # add to your shell profile
```

## 2. ESP-IDF v5.5.3 (auto-fetched)

`embuild` fetches ESP-IDF **v5.5.3** automatically on first build (pinned in
`.cargo/config.toml` via `ESP_IDF_VERSION`). v5.5.x is required by the Matter
Thread support (see the vendored-patch note below). The first build is long
(downloads + compiles ESP-IDF with OpenThread + BLE).

> If you previously built against a different ESP-IDF version, a stale
> `target/.../build/esp-idf-sys-*/` dir will cause confusing cmake/toolchain
> errors that `cargo clean -p esp-idf-sys` does NOT fix. Delete it manually:
> `rm -rf target/riscv32imac-esp-espidf/*/build/esp-idf-sys-*` and rebuild.

## 3. Install flash + monitor tools

```bash
cargo install espflash
cargo install espmonitor
```

## 4. Build and flash

```bash
# Check compilation (no hardware needed)
cargo check

# Build release binary
cargo build --release

# Flash to ESP32-H2 and open serial monitor
cargo run --release
# (uses `espflash flash --monitor` as the runner — see .cargo/config.toml)
```

## 5. Run host-side unit tests

The pure-logic unit tests (sensor algorithm, state machine, Matter IDs) run on
the host without any hardware:

```bash
# Override the target to run tests on the host
cargo test --target $(rustup show active-toolchain | cut -d- -f1)-unknown-linux-gnu
# or on macOS:
cargo test --target aarch64-apple-darwin  # Apple Silicon
cargo test --target x86_64-apple-darwin   # Intel Mac
```

## 6. Run hardware integration tests

Requires the full hardware assembly connected.
See `tests/integration/README.md` for wiring and procedure.

```bash
cargo test --test hw_actuator   --target riscv32imac-esp-espidf --release
cargo test --test hw_sensor     --target riscv32imac-esp-espidf --release
cargo test --test hw_full_cycle --target riscv32imac-esp-espidf --release
```

## ⚠️ Vendored `esp-idf-matter` patch (ESP32-H2 Thread support)

The Matter-over-Thread stack uses [`esp-idf-matter`](https://github.com/sysgrok/esp-idf-matter),
**vendored locally under `vendor/esp-idf-matter/` and patched**, rather than
pulled directly as a git dependency. Why:

- Upstream `esp-idf-matter` (rev `a8a1b98`, current `master` HEAD as of writing)
  gates its high-level Thread stack (`EspThreadMatterStack`, `EspMatterThread`,
  in `src/wireless/`) behind `#[cfg(all(not(esp32h2), …, esp_idf_comp_esp_wifi_enabled))]`.
  So on the **pure-Thread ESP32-H2 (no Wi-Fi)** those types don't exist — the
  upstream `light_thread` example actually only compiles for the Wi-Fi-capable
  ESP32-C6. H2 Matter-over-Thread is effectively unfinished upstream.
- The underlying pieces it needs (the `thread` and `ble` modules, the generic
  `rs-matter-stack` Thread stack) *are* available on H2. The exclusion was an
  overly-broad module gate, not a fundamental limitation.

**The patch** (see `LOCAL PATCH (RCD)` comments in `vendor/esp-idf-matter/src/`):

1. `src/lib.rs` — relax the `pub mod wireless` gate from "not H2 + Wi-Fi" to
   "Wi-Fi **or** (OpenThread + 802.15.4)", so the module compiles on H2.
2. `src/wireless.rs` — additionally exclude the **`wifi` submodule** on H2 with
   `not(esp32h2)`, because the `esp_wifi` *component* cfg is set on H2 even
   though the chip has no radio and `esp_idf_svc::wifi` is absent.

`Cargo.toml` points at `{ path = "vendor/esp-idf-matter" }`. **Revisit / drop the
vendored copy once upstream supports H2 Thread directly.** To re-sync with a
newer upstream, re-vendor and re-apply the two patches above.

## Thread Border Router requirement

Matter-over-Thread needs a **Thread Border Router that is also an Apple Home
hub** on the same network to commission and reach the device:
- HomePod mini / HomePod (2nd gen), or
- Apple TV 4K (Wi-Fi + Ethernet / Thread-capable model).

There is no firmware difference between them — commissioning is over BLE, then
the device joins Thread via whichever border router is present.

## Flash image size

A **debug** Matter+Thread+BLE image is ~3.7 MB; the `partitions.csv` factory
partition is sized to ~3.9 MB to fit it on a **4 MB-flash** H2 module. A
**release** image (`cargo build --release`) is far smaller and is recommended
for actual use. If your module has <4 MB flash, shrink `factory` and build
release.

## Matter commissioning (first boot)

1. Flash the firmware (`cargo run --release`, which runs `./flash.sh`).
2. Open the serial monitor. On first boot (no fabric provisioned yet) the Matter
   stack prints, automatically, a **scannable ASCII QR code** plus the text
   payload and manual pairing code:
   ```
   SetupQRCode: [MT:XXXXXXXX]
   PairingCode: [XXXX-XXX-XXXX]
   ```
3. On iPhone: Home app → + → Add Accessory → scan the QR directly from the
   terminal (or enter the manual pairing code). Commissioning runs over BLE,
   then the device joins Thread via your border router (see above).

> **Stages 1–2 (done):** the device commissions as a single **On/Off** endpoint
> wired to the actuator — toggling it **ON in HomeKit fires one reset cycle**
> (`ToController::ManualTrigger`). The controller reports cycle progress back, so
> the tile shows ON during the ~16 s cycle and returns to OFF afterwards.
> (It still advertises the On/Off "light" device type; switching to a plug/outlet
> icon is a one-line cosmetic change in `node.rs` once commissioning is confirmed.)
>
> **Stage 3 (planned):** a Contact Sensor endpoint reflecting EMF power-presence
> (`ToMatter::SetContactClosed`), plus persistent fabric storage. The RCD reset
> state machine already runs independently of Matter on its own thread.

## Adjusting the detection threshold

If the CT sensor false-triggers (power seen when absent) or misses detection:

Edit `src/config.rs`:
```rust
pub const CT_DETECTION_THRESHOLD: u16 = 80; // increase to reduce sensitivity
```

The serial log always prints the live peak-to-peak ADC count when
`CONFIG_LOG_DEFAULT_LEVEL_DEBUG` is set in `sdkconfig.defaults`.

## Pin summary

| Signal | GPIO | Notes |
|--------|------|-------|
| CT sensor ADC | GPIO1 | ADC1_CH1; RBDimmer adapter SIG pin |
| Actuator PWM | GPIO10 | LEDC 50 Hz → level shifter → L12 White wire |
| Actuator 12 V | — | External 12 V supply → L12 Red wire |
| Actuator GND | GND | L12 Black wire |
| Level shifter LV | 3.3 V | From ESP32-H2 3.3 V pin |
| Level shifter HV | 5 V | From external 5 V supply |

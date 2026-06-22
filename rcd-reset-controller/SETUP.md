# Development Environment Setup

## 1. Install Espressif Rust toolchain

```bash
cargo install espup
espup install          # installs the `esp` Rust fork + RISC-V/Xtensa targets
source ~/export-esp.sh # add to your shell profile
```

## 2. Install ESP-IDF v5.3

```bash
cargo install esp-idf-sys --example install_esp_idf  # or use idf.py
# Alternatively let embuild fetch it automatically on first build (see build.rs).
```

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

## Matter commissioning (first boot)

1. Flash the firmware.
2. Open the serial monitor — the commissioning QR payload is logged:
   ```
   Matter commissioning QR payload:  MT:XXXXXXXX
   ```
3. Go to a QR-code generator, paste the `MT:…` string, print the code.
4. On iPhone: Home app → + → Add Accessory → scan the QR code.
5. The device appears as two tiles:
   - **RCD Resetter** — tap to fire a reset cycle
   - **Power Sensor** — shows "Closed" (power on) or "Open" (RCD tripped)

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

# Integration Tests — Hardware Required

These tests run **on the actual ESP32-H2 hardware**.  They are interactive:
the operator must confirm physical behaviour through the serial monitor.

---

## Prerequisites

1. ESP32-H2 development board flashed with the `rcd-reset-controller` firmware.
2. Actuonix L12-50-210-12-I connected to GPIO10 via a 3.3 V→5 V level shifter.
   - White wire (RC signal) → level shifter output
   - Red wire  (12 V)      → 12 V supply
   - Black wire (GND)      → common GND
3. SCT-013-000 + RBDimmer adapter wired to GPIO1 (ADC1_CH1).
   - Adapter SIG → GPIO1
   - Adapter VCC → 3.3 V
   - Adapter GND → GND
   - CT sensor clamp around one live wire of a 220 V AC circuit under load.
4. `espflash` and `espmonitor` installed:
   ```
   cargo install espflash
   cargo install espmonitor
   ```

---

## Running

These tests are declared as `[[example]]` targets so they build and flash
via `cargo run --example`, which goes through the configured runner
(`espflash flash --monitor`). `cargo test` is not used because the
`build-std` sysroot conflicts with the embedded test harness.

```bash
# EMF backend (default) — contactless detector on GPIO4
cargo run --example hw_actuator --release
cargo run --example hw_emf      --release

# CT backend — SCT-013 current transformer on GPIO1.
# Select with --no-default-features --features sensor-ct.
cargo run --example hw_sensor     --release --no-default-features --features sensor-ct
cargo run --example hw_full_cycle --release --no-default-features --features sensor-ct
```

Each command builds the binary, flashes it to the connected device, and
opens the serial monitor automatically. Each test prints `[PASS]` or
`[FAIL]` to the UART (115200 baud).

### Power-sensor backends (Cargo features)

The firmware can sense mains presence two ways; exactly one is compiled in:

| Feature       | Default | Sensor                              | Pin    | Test          |
|---------------|---------|-------------------------------------|--------|---------------|
| `sensor-emf`  | ✅      | Contactless BC547 EMF cascade       | GPIO4  | `hw_emf`      |
| `sensor-ct`   |         | SCT-013 current transformer + ADC   | GPIO1  | `hw_sensor`   |

The `hw_sensor` and `hw_full_cycle` examples require `sensor-ct`; `hw_emf`
requires `sensor-emf`. `cargo` skips examples whose `required-features` are not
met, so the commands above select the right backend explicitly.

---

## Test descriptions

| Test file | Feature | What it checks |
|-----------|---------|---------------|
| `hw_actuator.rs` | any | Actuator extends fully, retracts fully, holds idle position |
| `hw_emf.rs` | `sensor-emf` | Contactless EMF detection of AC present / absent; transition-threshold calibration |
| `hw_sensor.rs` | `sensor-ct` | CT detection of AC present / absent; peak-to-peak threshold calibration |
| `hw_full_cycle.rs` | `sensor-ct` | Full RCD reset cycle: power loss → auto-trigger → actuate → re-check; also tests manual HomeKit trigger |

---

## Safety notes

- Never connect to 220 V AC mains without appropriate protection and isolation.
- The CT sensor is non-invasive (no direct mains contact), but keep the ESP32
  circuit physically separated from the mains cable.
- The actuator operates at 12 V — ensure the power supply can handle stall
  current (246 mA) for the 210:1 gearing model.

# Non-Contact 220V AC EMF Presence Detector

This document outlines the design, wiring, and embedded Rust code required to build a contactless 50/60Hz electromagnetic field (EMF) detector using an **ESP32-H2** and **BC547** transistors.

---

## ⚠️ Important Safety Notice
* **Contactless Isolation:** This project does **not** physically connect to dangerous 220V mains voltage wires. The antenna reads the radiating electromagnetic wave through the wire's outer plastic insulation.
* **Safe Handling:** Never strip the insulation off the 220V AC lines. Keep the copper antenna wire completely isolated from exposed copper conductors.

---

## 🛠️ Hardware Requirements
* **Microcontroller:** ESP32-H2 (RISC-V architecture)
* **Transistors:** 3x BC547 NPN Transistors (Labeled **Q1**, **Q2**, and **Q3**)
* **Current-Limiting Resistor:** 1x 10kΩ Resistor
* **Bleeder/Stabilizing Resistor:** 1x 1MΩ to 10MΩ Resistor
* **Antenna:** A 5cm piece of insulated solid-core copper wire wound into a tight spiral.

### BC547 Pin Configuration
Looking directly at the **flat side** of the transistor with the pins pointing downwards:
1. **Pin 1 (Left):** Collector (C)
2. **Pin 2 (Middle):** Base (B)
3. **Pin 3 (Right):** Emitter (E)

---

## 🔌 Circuit Wiring Diagram

The circuit uses three BC547 transistors stacked into a high-gain Darlington cascade to amplify weak alternating electric fields.

```text
               ESP32-H2 3.3V
                 │
                 ├───┐├───┐
                 │   │   │
               ┌─┴─┐┌─┴─┐┌─┴─┐
               │ C ││ C ││ C │
  Antenna ───> │ B ││   ││   │
    │          │ Q1││ Q2││ Q3│
   [ ] 1M-10M  │ E ││ E ││ E │
    │          └─┬─┘└─┬─┘└─┬─┘
   GND ──────────┘    │    │
                      ├────┤    ┌───────┐
                      │    └────┤ 10kΩ  ├────> ESP32 GPIO 4
                      ▼         └───────┘
                    Base Q3
```

### Step-by-Step Connections:
1. **The Darlington Cascade:**
   * Connect **Emitter (E) of Q1** to **Base (B) of Q2**.
   * Connect **Emitter (E) of Q2** to **Base (B) of Q3**.
2. **Power & Ground Rails:**
   * Connect the **Collectors (C) of Q1, Q2, and Q3** together, then connect them to the **3.3V** pin on the ESP32-H2.
   * Connect **Emitter (E) of Q3** directly to a **GND** pin on the ESP32-H2.
3. **The Sensor Input (Antenna):**
   * Connect your spiral copper wire antenna directly to the **Base (B) of Q1**.
   * Place your high-value bleeder resistor (**1MΩ to 10MΩ**) between the **Base (B) of Q1 and GND**. *Note: This prevents static charge buildup from keeping the sensor permanently locked on.*
4. **The Output to ESP32-H2:**
   * Connect a wire to the junction where the **Emitter of Q2 meets the Base of Q3**.
   * Run this wire through a **10kΩ resistor** straight into **GPIO 4** on your ESP32-H2.

---

## 🦀 Embedded Rust Solution (`no_std`)

This program utilizes `esp-hal` to track state transitions over a brief 40ms sampling window (safely capturing 2 complete cycles of a 50Hz/60Hz wave).

### 📄 `Cargo.toml`
Ensure your configuration points to the appropriate version targeting the ESP32-H2.

```toml
[package]
name = "esp32h2-emf-detector"
version = "0.1.0"
edition = "2021"

[dependencies]
esp-backtrace = { version = "0.12.0", features = ["esp32h2", "panic-handler", "print-uart"] }
esp-hal = { version = "0.18.0", features = ["esp32h2"] }
esp-println = { version = "0.9.0", features = ["esp32h2", "log"] }
log = "0.4.20"
```

### 📄 `src/main.rs`
```rust
#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    clock::ClockControl,
    delay::Delay,
    gpio::{Input, Pull, Io},
    peripherals::Peripherals,
    prelude::*,
    system::SystemControl,
};
use esp_println::println;

#[entry]
fn main() -> ! {
    // 1. Initialize core system peripherals and clocks
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();
    let delay = Delay::new(&clocks);

    // 2. Set up IO Mux and configure GPIO 4 as a Digital Input
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let mut emf_pin = Input::new(io.pins.gpio4, Pull::None);

    println!("Contactless 220V AC EMF Detector Initialized!");

    let mut edge_transitions;
    let mut last_state;

    loop {
        // Sample the pin state continuously inside a brief 40ms tracking window
        // 40ms handles roughly 2 complete cycles of a 50Hz or 60Hz wave
        last_state = emf_pin.is_high();
        edge_transitions = 0;

        for _ in 0..400 {
            let current_state = emf_pin.is_high();
            if current_state != last_state {
                edge_transitions += 1;
                last_state = current_state;
            }
            // Tiny 100-microsecond delay between samples inside the window
            delay.delay_us(100);
        }

        // Evaluate if alternating frequency was present within the window
        if edge_transitions >= 2 {
            println!("⚡ 220V AC Presence Detected! (Transitions: {})", edge_transitions);
        } else {
            println!("❌ AC Power ABSENT / Off");
        }

        // Wait a quarter second before running the next 40ms window scan
        delay.delay_ms(250);
    }
}
```

---

## 🛠️ Calibration and Tuning

* **Too Sensitive (Stuck on "Detected"):** If the circuit displays AC presence when the antenna is far away from the wire, shorten the antenna length or lower the bleeder resistor value (e.g., from 10MΩ down to 1MΩ) to discharge static faster.
* **Not Sensitive Enough:** Increase the length of your spiral copper antenna, or physically coil the antenna wire around the outside jacket of the 220V electrical cable.

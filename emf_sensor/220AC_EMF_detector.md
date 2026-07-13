# Non-Contact 220V AC EMF Presence Detector

> **⚠️ SUPERSEDED - historical reference only.** This BC547-cascade design only worked
> with the antenna and ESP within a few inches of the live cable; its high-impedance
> output could not be run the several feet the installation needs. It was abandoned in
> favour of the buffered LM358 design in **`220AC_EMF_Remote_detector.md`** (the current,
> working sensor).

This document outlines the design, wiring, and embedded Rust integration required to build a contactless 50/60Hz electromagnetic field (EMF) detector using an **ESP32-H2** and **BC547** transistors.

> **Design note (revised):** the cascade output is read as an **analog ADC input**, not a digital pin. The original digital design did not work; see [Why the output is read as analog](#-why-the-output-is-read-as-analog-not-digital) for the bench investigation that led here.

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

The circuit uses three BC547 transistors stacked into a high-gain Darlington-style cascade to amplify weak alternating electric fields.

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
                      │    └────┤ 10kΩ  ├────> ESP32 GPIO4 (ADC1_CH3)
                      ▼         └───────┘
                    Base Q3
```

### Step-by-Step Connections:
1. **The Cascade:**
   * Connect **Emitter (E) of Q1** to **Base (B) of Q2**.
   * Connect **Emitter (E) of Q2** to **Base (B) of Q3**.
2. **Power & Ground Rails:**
   * Connect the **Collectors (C) of Q1, Q2, and Q3** together, then connect them to the **3.3V** pin on the ESP32-H2.
   * Connect **Emitter (E) of Q3** directly to a **GND** pin on the ESP32-H2.
3. **The Sensor Input (Antenna):**
   * Connect your spiral copper wire antenna directly to the **Base (B) of Q1**.
   * Place your high-value bleeder resistor (**1MΩ to 10MΩ**) between the **Base (B) of Q1 and GND**. *Note: This prevents static charge buildup from keeping the sensor permanently locked on.*
4. **The Output to ESP32-H2:**
   * Connect a wire to the junction where the **Emitter of Q2 meets the Base of Q3** (the output node).
   * Run this wire through a **10kΩ** series resistor into **GPIO4** on your ESP32-H2.
   * GPIO4 is configured as **ADC1 channel 3** (analog input) - *not* a digital input.

---

## 🔬 Why the output is read as analog (not digital)

The first version of this firmware configured GPIO4 as a **digital input** and counted logic-level transitions. On the bench it detected **zero** transitions even with the antenna held on a live cable. A scope investigation (Rigol DS1104Z) explained why, and the firmware was changed to read the node with the **ADC** instead.

### Node names used below

| Node | Physical point |
|------|----------------|
| `N_ant` | Base of Q1 - antenna + bleeder |
| `N_A` | Q1 emitter = Q2 base |
| `N_out` | Q2 emitter = Q3 base = the 10 kΩ tap -> GPIO4 |

### Findings

* **The output node is clamped to ~ 0.8 V.** `N_out` is also Q3's *base*, and Q3's emitter is at GND, so the base-emitter junction clamps the node to roughly one diode drop (~0.7-0.8 V). It physically cannot reach the ESP32-H2 input-HIGH threshold (V_IH ~ 0.75 x 3.3 V ~ **2.5 V**). A digital read therefore returns a constant LOW -> no transitions, regardless of how good the antenna coupling is.
* **There is, however, a healthy AC signal there.** Measured peak-to-peak 50 Hz swing at `N_out` (AC-coupled, 20 MHz BW limit, AC-Line trigger):

  | Condition | `N_ant` (Vpp) | `N_out` (Vpp) |
  |-----------|---------------|---------------|
  | Field present (antenna on live cable) | ~350 mV | **~750 mV** |
  | Field absent (away from mains)        | -             | **~300 mV** (ambient noise floor) |

  Both swings sit entirely **below** the 0.8 V clamp.

### Conclusion

The analog front end works fine; only the *interface* was wrong. Reading `N_out` with the ADC and detecting the **peak-to-peak swing** captures the 750 mV-vs-300 mV difference cleanly and sidesteps the logic-threshold problem entirely. No hardware change is required.

> The cascade is a chain of emitter-followers (current gain, ~unity voltage gain) and Q3 effectively acts as a clamp on the output node. If you ever want a true logic-level digital output, the last stage would need to be reworked into a **common-emitter** stage (collector resistor to 3.3 V, output taken at the collector) with base biasing - but for this project the ADC approach is simpler and robust.

---

## 🦀 Firmware integration (this project)

GPIO4 is read as **ADC1 channel 3** using the ESP32-H2's one-shot ADC, sampled fast enough to capture the 50 Hz waveform, then reduced to a peak-to-peak swing and thresholded. The detection logic is identical in shape to the CT backend. (This was the early local-sensor concept; the shipped design is the remote front end in [`220AC_EMF_Remote_detector.md`](220AC_EMF_Remote_detector.md).)

* Implementation: [`src/sensor.rs`](../src/sensor.rs)
* Tunable constants: [`src/config.rs`](../src/config.rs)

### Sampling & detection parameters (`config.rs`)

| Constant | Value | Meaning |
|----------|-------|---------|
| `EMF_SAMPLE_COUNT` | `400` | ADC samples per detection window |
| `EMF_SAMPLE_INTERVAL_US` | `100` | spacing -> 400 x 100 us = **40 ms ~ 2 full 50 Hz cycles** |
| `EMF_DETECTION_THRESHOLD` | `550` | peak-to-peak ADC counts above which AC is "present" |

**Threshold derivation.** 12-bit ADC at 12 dB attenuation ~ 0-3.9 V full scale -> ~ 1.05 counts/mV.

* field present : ~750 mV pp ~ **790 counts**
* field absent   : ~300 mV pp ~ **315 counts**
* threshold sits at the midpoint, ~525 mV ~ **550 counts** (~ 235 counts of margin above the noise floor).

### Core logic (abridged)

```rust
// GPIO4 = ADC1 channel 3, 12 dB attenuation.
let adc = AdcDriver::new(peripherals.adc1)?;
let cfg = AdcChannelConfig { attenuation: attenuation::DB_12, ..Default::default() };
let mut channel = AdcChannelDriver::new(adc, peripherals.pins.gpio4, &cfg)?;

// Sample one 40 ms window (~ two 50 Hz cycles).
let mut buf = [0u16; EMF_SAMPLE_COUNT];
for slot in buf.iter_mut() {
    *slot = channel.read_raw()?;
    // 100 us between samples (Timer in async, sleep in the blocking test path)
}

// Peak-to-peak swing -> presence.
let pp = buf.iter().max().unwrap() - buf.iter().min().unwrap();
let present = pp > EMF_DETECTION_THRESHOLD; // 550 counts
```

---

## 🛠️ Calibration and Tuning

Tune `EMF_DETECTION_THRESHOLD` (ADC counts) in `config.rs`:

* **False positives (reads "present" with no field):** ambient mains hum is exceeding the threshold. Raise `EMF_DETECTION_THRESHOLD`, shorten the antenna, or lower the bleeder resistor (e.g. 10 MΩ -> 1 MΩ) to discharge static faster.
* **Misses a real field:** lower `EMF_DETECTION_THRESHOLD`, lengthen the spiral antenna, or coil it around the outside jacket of the 220 V cable.
* **Re-calibrating from scratch:** scope `N_out` (AC-coupled) with the field present and absent, note the two peak-to-peak voltages, convert to ADC counts (~ 1.05 counts/mV at 12 dB), and set the threshold to the midpoint. The `hw_emf` example also logs the measured peak-to-peak each sample, so you can read the live counts over the serial monitor and pick a threshold without the scope.

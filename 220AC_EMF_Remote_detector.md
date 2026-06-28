# 220 V AC Contactless EMF Presence Detector — Remote (LM358) Design

This is the **current, working** mains-presence sensor for the RCD reset controller. It
uses an **LM358** op-amp as a high-gain, band-limited AC amplifier placed **at the
antenna** (the far/measurement end), buffering the signal so it survives a multi-foot
cable run back to the ESP32-H2. The ESP reads it on **GPIO4 / ADC1_CH3** with a
**calibrated** ADC and infers power presence from the peak-to-peak swing of the 50 Hz
field.

------------------------------
## Background — earlier approaches (abandoned)

This is the third iteration of mains sensing for this project:

1. **CT clamp** (`220AC_CT_detector.md`) — an SCT-013 current transformer with the
   RBDimmer adapter. Explored first, then set aside in favour of a fully contactless,
   lower-cost EMF approach.
2. **BC547 transistor cascade** (`220AC_EMF_detector.md`) — a 3-stage emitter-follower
   amplifier with a ~5 cm spiral antenna, read via the ADC. It detected the field fine,
   **but only with the antenna (and the ESP) within a few inches of the live cable.**
   Its output was high-impedance and low-level, so it could not be run more than a few
   inches to the microcontroller without the signal collapsing into noise.

The installation needs the sensor mounted on the mains cable with the ESP32 **several
feet away**. That distance is exactly what killed the cascade design. This **remote**
design fixes it: the LM358 amplifies *and* low-impedance-buffers the signal right at the
antenna, so the amplified output survives a multi-foot shielded-cable run.

> Both earlier documents are retained for history but are **superseded by this one.**

------------------------------
## System Overview
An LM358 op-amp is configured as a high-gain **inverting AC amplifier** with a low-pass
feedback capacitor, placed directly at the measurement point (end of the wire) to
amplify the weak 50/60 Hz electrostatic field before sending a clean, low-impedance
signal over a long shielded cable. The signal is biased to a ~1.65 V "virtual ground" so
the AC wave swings within the ESP32's 3.3 V tolerance.

------------------------------
## Component Checklist

* IC: LM358 Op-Amp
* Resistors:
  * Two 10 kΩ (virtual-ground voltage divider)
  * One 100 kΩ (input resistor — sets gain with the feedback resistor)
  * One 1 MΩ (feedback / gain — gain ≈ Rf/100 kΩ = 10; see *Tuning* for the 2.2 MΩ option)
  * One 100 Ω (cable stabilization)
* Capacitors:
  * Two 0.1 µF (104) ceramic (input DC-block + Pin-3 power-noise filter)
  * **One 2.2 nF (222) ceramic (feedback low-pass — band-limits the amp to ~50 Hz)**
* Antenna: 2-to-3 inch solid copper wire loop or coil
* Cable: 3-to-6 foot shielded cable (3 internal strands + outer shield, 22–24 AWG
  stranded recommended for durability)

------------------------------
## LM358 Circuit Schematic (Sensor End)

```
               +3.3V (From ESP32)
                 │
                [10kΩ]
                 │
  Pin 3 (+) ─────┴─────[10kΩ]───── GND
                 │
               [0.1µF]
                 │
                GND

                 ┌──────────[1MΩ feedback]──────────┐
                 │        ┌────[2.2nF]────┐          │
                 │        │               │          │
  Antenna ─[0.1µF]── Pin 2 (-) ───────────┴──────────┤─ Pin 1 (Out) ─[100Ω]─> Signal wire
                 │                                    │
                [100kΩ]                               │
                 │                                    │
                 └────────────────────────────────────┘
```

The **2.2 nF feedback capacitor sits in parallel with the 1 MΩ feedback resistor**
(Pin 1 ↔ Pin 2). With the input high-pass (0.1 µF + 100 kΩ ≈ 16 Hz) it makes the stage a
band-pass of roughly **16–72 Hz**, centred on 50 Hz, which rejects the HF hash and
interference spikes that otherwise blur the present/absent detection.

------------------------------
## Pin-by-Pin Wiring Guide

### 1. Power & Virtual Ground
* **Pin 8 (VCC):** incoming 3.3 V wire from the ESP32.
* **Pin 4 (GND):** incoming GND wire from the ESP32.
* **Pin 3 (Non-Inverting Input):** middle of the two-10 kΩ divider (one to 3.3 V, one to
  GND → ~1.65 V). 0.1 µF from Pin 3 to GND filters power noise.

### 2. Input & Feedback Loop
* **Pin 2 (Inverting Input):** the copper antenna through a 0.1 µF DC-blocking cap; a
  100 kΩ resistor between Pin 2 and Pin 3.
* **Feedback loop:** a 1 MΩ resistor **and** a 2.2 nF capacitor, in parallel, between
  Pin 2 and Pin 1.

### 3. Long-Distance Output
* **Pin 1 (Output):** through the 100 Ω stabilization resistor, then onto the long signal
  wire to the ESP32.

------------------------------
## Cable & Microcontroller Connections

| Long Cable Component | Connection at Sensor End (LM358) | Connection at Host End (ESP32)  |
|----------------------|----------------------------------|---------------------------------|
| VCC Wire             | Pin 8                            | 3V3 Pin                         |
| GND Wire             | Pin 4                            | GND Pin                         |
| Signal Wire          | Pin 1 (after 100 Ω resistor)     | GPIO4 (ADC1_CH3)                |
| Outer Cable Shield   | Leave Disconnected               | GND Pin (drains EMI noise)      |

------------------------------
## Firmware Integration (this project — `rcd-nostd/`)

GPIO4 is read as **ADC1 channel 3** using esp-hal's one-shot ADC with **line-fit efuse
calibration** (`AdcCalLine`). Calibration matters: the ESP32-H2's *uncalibrated* ADC has
a large built-in offset/gain error (it read ~3730 raw for a 1.65 V input, jamming the
operating point against the 4095 ceiling and **clipping** the AC swing). The calibrated
read corrects the offset/gain and returns values **directly in millivolts**, so the
1.65 V bias reads ~1.3 V mid-range with full headroom and the swing is faithful.

* Implementation: [`rcd-nostd/src/sensor.rs`](rcd-nostd/src/sensor.rs)
* Tunable constants: [`rcd-nostd/src/config.rs`](rcd-nostd/src/config.rs)

### Sampling & detection parameters (`config.rs`)

| Constant | Value | Meaning |
|----------|-------|---------|
| `EMF_SAMPLE_COUNT` | `400` | ADC samples per detection window |
| `EMF_SAMPLE_INTERVAL_US` | `100` | spacing → 400 × 100 µs = **40 ms ≈ 2 full 50 Hz cycles** |
| `EMF_DETECTION_THRESHOLD` | `95` | **peak-to-peak millivolts** above which AC is "present" |

Because the read is calibrated, samples are already in mV — there is **no count↔mV
conversion**; the threshold is a true peak-to-peak voltage.

### Field-calibrated levels (LM358 + 2.2 nF feedback)

| Condition | Peak-to-peak at GPIO4 |
|-----------|-----------------------|
| Field present | ~200 mV (ranges 150–300 mV) |
| Field absent  | mostly < 100 mV (occasional spikes to ~190 mV) |
| DC operating point (mean) | ~1285 mV, stable |

`EMF_DETECTION_THRESHOLD = 95` sits just above the typical absent floor.

------------------------------
## Tuning

* **See the live readings:** build with debug logging — `cd rcd-nostd && ./build.sh -d run
  --release` — and watch the `EMF sensor: pp = N mV … mean = N mV` line. `./build.sh -d`
  sets `ESP_LOG=debug` (a compile-time level), so a rebuild is required to change it.
* **Misses a real field / want more amplitude:** increase the feedback resistor 1 MΩ → 2.2
  MΩ (gain 10 → 22), keeping a feedback cap (1 nF with 2.2 MΩ ≈ 72 Hz). A bigger/closer
  antenna improves signal-to-noise better than raw gain alone.
* **LM358 headroom caveat:** on a 3.3 V supply the LM358 output can only swing up to
  ~1.8 V. With the bias at ~1.3 V there is ~0.5 V of upward room. **Watch the `max` field
  in the debug log — if it approaches ~1800 mV the op-amp is clipping**; lower the Pin-3
  bias (e.g. divider 27 kΩ/10 kΩ → ~0.9 V) for symmetric headroom, or use a rail-to-rail
  op-amp (MCP6002/TLV9062).
* **False positives from noise:** the absent-state spikes can momentarily cross a low
  peak-to-peak threshold. Raise the threshold, tighten the feedback low-pass (larger
  feedback cap), or switch the firmware metric from peak-to-peak to an RMS / multi-window
  average (more robust to single-sample spikes).

------------------------------
## Key Construction Rules

* **Noise Mitigation:** use a shielded cable (old USB or audio cable) for runs longer than
  3 feet to stop the wire acting as a giant interference antenna. Ground the shield at the
  **ESP end only**.
* **Wire Selection:** 22–24 AWG stranded. The circuit draws under 2 mA, so thickness
  doesn't affect power loss, but stranded wire survives flexing over long runs.
* **Signal Protection:** the 100 Ω resistor at Pin 1 keeps the long cable's capacitance
  from making the op-amp oscillate. The 2.2 nF feedback cap also helps here by rolling off
  HF gain.

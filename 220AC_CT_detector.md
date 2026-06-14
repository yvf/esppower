# AC Mains Power Detection with ESP32

## Overview

This document summarises the approach for detecting AC mains power presence/absence using a non-invasive inductive sensor connected to an ESP32 microcontroller.

---

## Sensing Method: Current Transformer (CT) Sensor

A **non-invasive current transformer (CT) sensor** is the recommended approach. It clamps around a single wire of the mains cable and requires no direct electrical contact, providing full galvanic isolation between mains voltage and the ESP32.

**Key safety points:**
- The CT clamps around **one wire only** (live or neutral, not both together)
- No mains voltage ever reaches the ESP32
- Keep mains wiring and low-voltage circuitry physically separated

---

## Selected Solution: RBDimmer SCT-013 Sensor Adapter

**Supplier:** [rbdimmer.com](https://www.rbdimmer.com/shop/sct-013-sensor-adapter-31)

This is a purpose-built, commercially available signal conditioning board designed specifically to interface SCT-013 current transformers with microcontrollers like the ESP32. It includes a complete signal conditioning circuit with precision voltage divider and filtering, converting the CT's AC output to a microcontroller-friendly **0–3.3V signal** — no additional components or soldering required.

### Wiring

| Adapter Pin | ESP32 Pin |
|---|---|
| GND | GND |
| VCC | 3.3V |
| SIG | GPIO 32, 33, 34, 35, 36, or 39 (ADC1 pins) |

### CT Sensor

Pair the adapter with an **SCT-013-030** (30A, 1V output variant) or **SCT-013-000** (100A, current output variant). The SCT-013-030 is preferred for this use case as its built-in burden resistor produces a clean 0–1V output directly compatible with the adapter.

Connect the CT sensor's 3.5mm jack directly into the adapter's socket.

Selected the SCT013 (100A, 1V) from [yhdc.com](https://www.poweruc.pl/collections/split-core-current-transformers2/products/split-core-current-transformer-sct013-rated-input-5a-100a?variant=6876754968620)


---

## ESP32 Code — Presence Detection

```cpp
#define CT_PIN 34
#define SAMPLES 100
#define THRESHOLD 10  // ADC counts above noise floor

bool detectACPresence() {
    int minVal = 4095, maxVal = 0;

    for (int i = 0; i < SAMPLES; i++) {
        int val = analogRead(CT_PIN);
        if (val < minVal) minVal = val;
        if (val > maxVal) maxVal = val;
        delayMicroseconds(100);
    }

    int peakToPeak = maxVal - minVal;
    return peakToPeak > THRESHOLD;
}

void setup() {
    Serial.begin(115200);
    analogReadResolution(12);
}

void loop() {
    bool powerPresent = detectACPresence();
    Serial.println(powerPresent ? "AC PRESENT" : "NO AC");
    delay(500);
}
```

The code samples the ADC over 100 readings and measures the peak-to-peak swing. A swing above the threshold indicates AC current is flowing (mains present); a flat signal indicates no power.

---

## Bill of Materials

| Item | Description | Source |
|---|---|---|
| SCT-013 | 100A/1v non-invasive CT clamp sensor | Amazon / AliExpress |
| RBDimmer SCT-013 Adapter | Signal conditioning board | rbdimmer.com |
| ESP32 DevKit | Microcontroller | Any supplier |

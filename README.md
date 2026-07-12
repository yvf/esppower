# AC Power Detector & Linear Actuator Driver for HomeKit / Matter

A small battery-powered device that watches for mains AC power and, when the power
drops, extends a  with a linear actuator (meant to physically switch power back on)
and interfaces with Apple Home over Matter/Thread.

It's aimed at unattended installations where a *nuisance* trip would otherwise leave
something important without power until someone visits: an off-grid pump, a remote
freezer, a holiday home. The device notices the outage, attempts a reset, and if the
supply doesn't come back it stops and raises an alert.

## What it does

- **Senses mains power** contactlessly — a custom EMF sensor board picks up the field
  around a live conductor, so there's no direct mains connection to the electronics.
- **Auto-resets on power loss** — after a debounce, it drives the actuator through one or
  two reset cycles. If power returns, it stands down; if not, it latches and waits.
- **Manual reset from your phone** — tap the tile in Apple Home to fire a reset cycle.
- **Live status + trip alerts** — Home shows whether power is present and can notify you
  when it's lost.
- **Battery friendly** — contactless sensing (no mains tap), efficient bare-metal
  firmware, and a low-power Thread mesh radio make long battery operation practical.
- **12v supply** - requires a 12v supply (required for the actuator) and drives 5v from
  there.

## In Apple Home

The device appears as two tiles under one accessory:

- a **contact sensor** (the main tile) — power present / lost, with trip notifications;
- a **switch** — tap to trigger a manual reset (it returns to off on its own).

See [`docs/matter_device_choices.md`](docs/matter_device_choices.md) for why those Matter
device types were chosen.

## Hardware

- **Espressif ESP32-H2** — a RISC-V microcontroller with built-in 802.15.4 (Thread) and
  Bluetooth radios. Development uses the ESP32-H2 devkit.
- **Custom EMF sensor board** — a contactless mains-presence detector with an op-amp front
  end. Design files (schematic, PCB, BOM, gerbers) and write-up are in
  [`emf_sensor/`](emf_sensor/).
- **Actuonix L12 linear actuator** — the small motor that mechanically flips the breaker
  lever, driven as a standard RC servo.

## Software & stack

Bare-metal **`no_std` Rust** (no operating system), talking **Matter over Thread** so the
device joins an existing Apple Home / Thread network. The main building blocks:

- **esp-hal / esp-rtos** — chip support and async runtime
- **OpenThread** — the Thread (802.15.4) mesh networking
- **trouble** — Bluetooth, used only for initial pairing
- **rs-matter** — the Matter application layer

Running bare-metal (rather than on a full RTOS framework) is what makes the whole stack
fit in the ESP32-H2's limited memory — and keeps power draw low. The reasoning, the full
component list, and how to build/flash are in
[**`docs/no-std-plan.md`**](docs/no-std-plan.md).

## Build

```sh
./build.sh                 # build the firmware
./build.sh run --release   # flash to a connected ESP32-H2 and monitor
```

Host prerequisites are listed in [`docs/no-std-plan.md`](docs/no-std-plan.md).

## More documentation

- [`docs/no-std-plan.md`](docs/no-std-plan.md) — architecture, dependency stack, build setup
- [`docs/matter_device_choices.md`](docs/matter_device_choices.md) — the HomeKit / Matter device model
- [`docs/phase4b-glue-design.md`](docs/phase4b-glue-design.md) — the Matter transport layer design
- [`emf_sensor/`](emf_sensor/) — the custom power-sensor board and detection approach
- [`docs/upstream-prs/`](docs/upstream-prs/) — fixes to the ESP32-H2 802.15.4 radio driver

## Status

Working end-to-end on hardware: it commissions over Bluetooth, operates over Thread,
drives the actuator, and reports power status to Home. It currently uses Matter's standard
**test** device credentials — fine with tools like chip-tool and Home Assistant; adding it
to **Apple Home** requires production Matter credentials (a certified vendor ID + device
certificate).

## Disclaimer

More seriously: the code is almost entirely written by Claude, with only high-level human review.
It works for me, compiled on my M2 MacBook, on my esp32-h2 version and other bits. No guarantee it
will for you.

Somewhat less seriously, but perhaps needs saying: this implements a device that affects the real
world. Exercise good judgement, and use at your own risk. Fiddling around with mains AC power and
RCDs is inherently risky. It works for my usecase, but YMMV. If you don't know what you're doing, and
this ends up launching your house all the way to the moon and you with it, don't blame me.

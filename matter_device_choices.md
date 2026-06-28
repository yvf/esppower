# Matter Device Choices for RCD Reset Controller

## Hardware Overview

- **Linear actuator**: extends to reset an RCD to "on", then automatically retracts
- **Power sensor**: detects power state (on/off) downstream of the RCD
- **Controller**: ESP with Matter/Thread radio
- **Ecosystem**: Apple HomeKit via Matter over Thread

---

## Matter Node Structure

Single Matter node, two functional endpoints:

| Endpoint | Device Type         | Type ID  | Purpose                               |
|----------|---------------------|----------|---------------------------------------|
| 0        | Root Node           | —        | Required, standard                    |
| 1        | On/Off Plug-In Unit | `0x010A` | Actuator control (manual + automatic) |
| 2        | Contact Sensor      | `0x0015` | Downstream power state                |

---

## Endpoint 1 — On/Off Plug-In Unit (`0x010A`)

**Required clusters**: On/Off (server-side), Identify

**Firmware behavior**:
- Receiving `On` command (manual or automatic) → extend actuator → wait → retract → report `Off`
- `Off` commands are ignored (device is always effectively off at rest)
- During any actuator cycle (manual or automatic), set cluster state to `On`; set back to `Off` when cycle completes — keeps HomeKit's view consistent

**HomeKit UI**: tappable switch/outlet tile; always returns to off after a cycle, naturally communicating the one-shot/momentary nature of the action

**Why not `Generic Switch (0x000F)` with `MomentarySwitch` feature?**
HomeKit treats Generic Switch as an automation trigger only — no direct control tile in the Home app. The plug-in unit gives a tappable tile while the auto-off behavior communicates the momentary semantics.

**Why not `On/Off Light (0x0100)`?**
Lightbulb icon misleads users.

**Why not `On/Off Light Switch (0x0103)`?**
That is a client-side switch controller, not a controllable output.

---

## Endpoint 2 — Contact Sensor (`0x0015`)

**Required clusters**: Boolean State (server-side), Identify

**Firmware behavior**:
- `StateValue = true` → power present → HomeKit shows "Closed"
- `StateValue = false` → power absent (RCD tripped) → HomeKit shows "Open" with alert notification

**HomeKit UI**: read-only contact sensor tile; HomeKit raises a notification automatically when state goes to "Open" (power lost), providing RCD-trip alerting for free

---

## Automatic Retry Logic

When the power sensor detects loss of power, the firmware automatically triggers an actuator extend/retract cycle (which may or may not restore power). This logic is entirely internal to the firmware — it does not change the Matter device model.

The automatic cycle uses the same code path as a manual `On` command:

```
power sensor → false (StateValue = false)
  └─→ firmware triggers actuator cycle
        └─→ set On/Off cluster to On
              └─→ wait
                    └─→ retract
                          └─→ set On/Off cluster to Off
                                └─→ re-read power sensor → update Boolean State cluster
```

**Future extension**: if auto-retry needs to be user-configurable from HomeKit, add a third endpoint (another On/Off Plug-In Unit as an enable/disable toggle). This is additive and does not affect the existing two endpoints.

---

## HomeKit Home App Appearance

Both endpoints appear under a single device card:

```
[RCD Resetter]            [Power Sensor]
  Plug tile                 Contact tile
  Off (always at rest)      Closed = power on
  Tap → fires actuator      Open (!) = RCD tripped
  Auto-fires on trip        Notification on trip
```

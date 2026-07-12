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
| 1        | Contact Sensor      | `0x0015` | Downstream power state (**primary**)  |
| 2        | On/Off Plug-In Unit | `0x010A` | Actuator control (manual + automatic) |

### Which endpoint is the primary Home tile

The **contact sensor is the primary accessory** — the tile that represents this device
on the main Home view — and the plug is the secondary tile. Power state is the
at-a-glance information the user cares about (is the RCD tripped?); the plug is an
occasional-use control.

Matter has no explicit "primary endpoint" flag for a composed (non-bridge) device like
this. In practice **Apple Home treats the first application endpoint — the lowest
endpoint ID, listed first in the node's `PartsList` — as the primary tile.** So the lever
is simply endpoint ordering: whichever functional endpoint is numbered `1` becomes the
main tile. We therefore assign the contact sensor to endpoint 1 and the plug to
endpoint 2 (they were originally the other way round).

Notes:
- This is a change in device *composition*, so it takes effect on a fresh commissioning
  (re-pair after flashing). Apple's exact primary-selection heuristic is not publicly
  specified, so verify on hardware after re-pairing.
- Endpoint IDs live in `rcd-nostd/src/matter/`: `CONTACT_ENDPOINT_ID` (`contact.rs`) and
  `PLUG_ENDPOINT_ID` (`stack.rs`), and the `NODE` endpoint order in `stack.rs`.

---

## Endpoint 1 — Contact Sensor (`0x0015`) — primary tile

**Required clusters**: Boolean State (server-side), Identify

**Firmware behavior**:
- `StateValue = true` → power present → HomeKit shows "Closed"
- `StateValue = false` → power absent (RCD tripped) → HomeKit shows "Open" with alert notification

**HomeKit UI**: read-only contact sensor tile — the **primary** tile for this device (see "Which endpoint is the primary Home tile" above); HomeKit raises a notification automatically when state goes to "Open" (power lost), providing RCD-trip alerting for free

---

## Endpoint 2 — On/Off Plug-In Unit (`0x010A`) — secondary tile

**Required clusters**: On/Off (server-side), Identify

**Firmware behavior**:
- Receiving `On` command (manual or automatic) → extend actuator → wait → retract → report `Off`
- `Off` commands are ignored (device is always effectively off at rest)
- During any actuator cycle (manual or automatic), set cluster state to `On`; set back to `Off` when cycle completes — keeps HomeKit's view consistent

**HomeKit UI**: tappable switch/outlet tile (secondary); always returns to off after a cycle, naturally communicating the one-shot/momentary nature of the action

**Why not `Generic Switch (0x000F)` with `MomentarySwitch` feature?**
HomeKit treats Generic Switch as an automation trigger only — no direct control tile in the Home app. The plug-in unit gives a tappable tile while the auto-off behavior communicates the momentary semantics.

**Why not `On/Off Light (0x0100)`?**
Lightbulb icon misleads users.

**Why not `On/Off Light Switch (0x0103)`?**
That is a client-side switch controller, not a controllable output.

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

Both endpoints appear under a single device card; the contact sensor is the primary
(main) tile and the plug is the secondary tile:

```
[Power Sensor]  (primary)    [RCD Resetter]  (secondary)
  Contact tile                 Plug tile
  Closed = power on            Off (always at rest)
  Open (!) = RCD tripped       Tap → fires actuator
  Notification on trip         Auto-fires on trip
```

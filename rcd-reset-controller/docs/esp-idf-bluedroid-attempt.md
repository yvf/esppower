# ESP-IDF + Bluedroid Matter attempt — RAM wall on ESP32-H2 (archived)

This documents the **esp-idf-based** Matter-over-Thread integration attempt and
why it was abandoned for a no-std rewrite. It is preserved so we can return to
this exact point if the no-std path doesn't pan out.

## Resume point

- **Branch / commit:** the full esp-idf attempt is on `vibe-code-1` up to commit
  **`b8f64d5`** ("Free ~7 KB more (matter thread stack + bump arena)…").
- Everything below (deps, sdkconfig, vendored patch) is the state at that commit.
- To resume: check out `b8f64d5`, `rm -rf target/.../build/esp-idf-sys-*` if the
  ESP-IDF version/toolchain looks stale (see `rcd_build_env_gotchas`), build.

## The stack that was built (and works up to a point)

- `esp-idf-matter` (vendored under `vendor/esp-idf-matter`, patched to enable the
  Thread stack on esp32h2 — upstream gates it off; see SETUP.md).
- `rs-matter` + `rs-matter-stack` (sysgrok `next` forks via `[patch.crates-io]`).
- ESP-IDF **v5.5.3**, OpenThread (FTD) + **Bluedroid** BLE host + SW coexistence.
- Two `block_on` threads: Matter stack (own thread) + RCD controller (main task).
- Endpoint 1 On/Off wired to the actuator (Stage 2, `PlugHooks`).

## How far it got (on real hardware)

Booting and bring-up **succeed**:
- Thread driver starts, OpenThread attaches.
- BLE controller inits + enables (MAC printed).
- **Bluedroid host fully initializes**, BLE GAP + GATTS init, **BTP GATT app
  registered**, async-io reactor starts.

Then it dies — **out of system heap**:
```
E pthread: Failed to create task!
W rs_matter_stack: Matter UDP network error: OutOfMemory: "Not enough space"
```
The Matter UDP/IP network task can't be spawned, so the device never listens or
advertises for commissioning → Apple Home reports **"Accessory not found"**.

## Why it can't be fixed by tuning

ESP32-H2 has **320 KB SRAM**. The combination doesn't fit for *operation*:
- Bluedroid alone consumes ~66 KB of heap during init (71 KB free → ~0.2 KB by
  the end of BLE bring-up; verified with `heap_caps_*` probes).
- OpenThread holds ~40 KB; rs-matter static is ~86 KB `.bss`.

Tuning applied (all in the archived commit), which got us from "fails early in
gatt_init" to "fails at the Matter network task" but no further:
- rs-matter sizing: `max-fabrics-2`, `max-subscriptions-1`, `max-im-buffers-4`,
  `max-sessions-4`; `BUMP_SIZE` 17000 → 14000.
- Stacks (heap-allocated): main task 16384 → 6144, BTC task 15000 → 6144, Matter
  thread 20 KB → 16 KB.
- `CONFIG_BT_ACL_CONNECTIONS=1`, `CONFIG_OPENTHREAD_NUM_MESSAGE_BUFFERS` 65 → 14.

**No big lever remains.** The Bluedroid host is the hog, but its features can't be
trimmed: disabling `CONFIG_BT_GATTC_ENABLE` or BLE SMP **breaks the esp-idf-svc
build** — esp-idf-svc references `esp_ble_gattc_register_callback`,
`esp_ble_gap_set_security_param`, and `esp_ble_set_encryption` unconditionally.
And `esp-idf-svc`'s GATT server is **Bluedroid-only**, so the lighter **NimBLE**
(what Espressif's own H2 Matter uses) is not reachable without reworking
esp-idf-svc.

## Conclusion → no-std

To free the tens of KB needed you must drop either Bluedroid or esp-idf itself.
On the same H2 hardware that means a **no-std** stack (esp-hal + a lightweight
Rust BLE host + Rust OpenThread + rs-matter `no_std`), which removes the
FreeRTOS/Bluedroid overhead. That rewrite is tracked on a separate branch. The
alternative — an **ESP32-C6** (512 KB SRAM, esp-idf-matter's native target) —
would let the esp-idf stack here work largely as-is, if hardware can change.

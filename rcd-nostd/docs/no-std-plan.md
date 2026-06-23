# no-std Matter-over-Thread rewrite — stack & plan (ESP32-H2)

Replaces the esp-idf+Bluedroid attempt, which hit a hard RAM wall on the H2's
320 KB SRAM (see `rcd-reset-controller/docs/esp-idf-bluedroid-attempt.md`). Going
bare-metal removes the FreeRTOS/Bluedroid overhead so most of the SRAM is heap.

## The stack (all no-std, target `riscv32imac-unknown-none-elf`, nightly)

| Layer | Crate(s) | Notes |
|-------|----------|-------|
| HAL / chip | **esp-hal 1.1** (`esp32h2`, `unstable`) | peripherals, ADC, LEDC, GPIO |
| RTOS/async | **esp-rtos 0.3** (`esp-radio`,`embassy`) + embassy 0.10/0.8/0.5 | scheduler + radio glue |
| Heap | **esp-alloc 0.10** | the win: allocate most of 320 KB as heap |
| Radio | **esp-radio 0.18** (`ieee802154` + BLE) | one crate, both radios + coex |
| Thread | **openthread** (esp-rs/openthread): `esp-radio`,`udp`,`srp`,`dns-client`,`edge-nal` | OpenThread C via `openthread-sys`; H2 802.15.4 out of the box; provides edge-nal UDP + SRP (Matter's mDNS-over-Thread) |
| BLE host | **trouble-host** on esp-radio's BLE controller (`bt-hci`) | GATT server for commissioning |
| Matter | **rs-matter** (`no_std`) + **rs-matter-stack** | generic stack; edge-nal UDP + a `Gatt`/`ThreadCoex` transport |
| Boot | esp-bootloader-esp-idf 0.5, esp-backtrace/println | |

## The integration we must write (no off-the-shelf glue exists)

esp-idf-matter implemented rs-matter-stack's `ThreadCoex` + `Gatt` traits using
ESP-IDF. For no-std there is **no equivalent** — the openthread/trouble examples
are Thread-only / BLE-only, and rs-matter-stack's examples are std/Linux. So the
core new work is a ~few-hundred-line transport layer implementing those traits
with `openthread` (Thread netif/UDP/SRP) and `trouble` (BLE GATT peripheral).

## Build prerequisites (host)

`openthread-sys` links a prebuilt OpenThread lib for `riscv32imac-unknown-none-elf`
(no C build), but `mbedtls-rs-sys` does an **on-the-fly mbedtls C build** because our
crypto feature subset differs from its prebuilt config. That needs, on PATH:
- **cmake** (`/opt/homebrew/bin`)
- a **RISC-V-capable clang** — Apple's `/usr/bin/clang` has NO riscv32 target; use
  brew LLVM: `/opt/homebrew/opt/llvm/bin/clang` (+ `LIBCLANG_PATH=/opt/homebrew/opt/llvm/lib`).

So build with:
```sh
export PATH="/opt/homebrew/opt/llvm/bin:/opt/homebrew/bin:$PATH"
export LIBCLANG_PATH="/opt/homebrew/opt/llvm/lib"
cargo build
```

## Phases

1. **[DONE] Skeleton** — esp-hal+esp-rtos+esp-radio+embassy+heap boot. Builds in ~27 s.
2. **[DONE — builds] Thread up** — openthread `srp`-style: radio (`EspRadio`/
   `Ieee802154`), `OpenThread::new_with_udp_srp`, join via `THREAD_DATASET`, log
   role/addrs/heap. Compiles+links (prebuilt OT lib + built mbedtls). Next: flash
   and confirm it attaches to a real border router.
3. **BLE up** — trouble-host GATT server on esp-radio BLE; advertise a service.
4. **Matter glue** — implement rs-matter-stack `ThreadCoex` + `Gatt` over (2)+(3);
   run the rs-matter-stack run loop; print the commissioning QR.
5. **Device model** — port the On/Off plug (`PlugHooks`) + controller state machine
   from the esp-idf version (logic is reusable; it's no_std-friendly already).
6. **Peripherals** — re-implement the EMF ADC sensor (esp-hal `adc`) and the
   actuator LEDC servo (esp-hal `ledc`) — the detection/timing constants port as-is.
7. **Commission + iterate** — flash, pair in Home, watch free heap (should be far
   healthier than the ~0 we hit under esp-idf).

## Reusable from the esp-idf version (`rcd-reset-controller/`)

Pure-logic, no_std-friendly, copy nearly verbatim: the EMF/CT detection functions
(`peak_to_peak`, thresholds), the controller state machine, config constants, and
the `PlugHooks` On/Off→trigger intent. Only the HAL-touching init changes.

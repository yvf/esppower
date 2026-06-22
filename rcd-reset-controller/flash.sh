#!/usr/bin/env bash
# Cargo `runner` wrapper for flashing + monitoring on the ESP32-H2.
#
# Why this exists instead of a bare `espflash flash --monitor`:
#   `espflash flash <app.elf>` flashes espflash's OWN bundled bootloader and
#   partition table, which are built from whatever ESP-IDF version espflash
#   ships with (v5.5.1 in espflash 4.4). This project builds against ESP-IDF
#   v5.2.5, and a v5.5.1 bootloader handing off to a v5.2.5 app crash-loops the
#   chip before `cpu_start` (observed: rst:0x3 LP_SW_HPSYS, no app output).
#
# This wrapper locates the bootloader and partition table that esp-idf-sys
# generated for THIS build (matching IDF version + chip) and flashes those, so
# `cargo run`/`cargo run --example` work out of the box.
set -euo pipefail

APP="${1:?usage: flash.sh <path-to-app-elf>}"

# Derive the profile build root (.../debug or .../release) from the binary path,
# handling both bin targets (.../release/<name>) and example targets
# (.../release/examples/<name>).
build_root="${APP%/*}"             # strip the binary filename
build_root="${build_root%/examples}"  # collapse the examples/ subdir if present

BL=$(ls "$build_root"/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin 2>/dev/null | head -1)
PT=$(ls "$build_root"/build/esp-idf-sys-*/out/build/partition_table/partition-table.bin 2>/dev/null | head -1)

if [[ -z "$BL" || -z "$PT" ]]; then
    echo "flash.sh: could not find esp-idf-sys bootloader/partition table under $build_root/build/" >&2
    echo "          falling back to espflash's bundled images (may crash-loop on version mismatch)" >&2
    exec espflash flash --monitor "$APP"
fi

echo "flash.sh: using project bootloader:      $BL" >&2
echo "flash.sh: using project partition table: $PT" >&2
exec espflash flash --bootloader "$BL" --partition-table "$PT" --monitor "$APP"

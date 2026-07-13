#!/usr/bin/env bash
# Build/flash wrapper. The mbedtls-rs-sys riscv C build needs, on PATH:
#  - a RISC-V-capable clang - brew LLVM; Apple's /usr/bin/clang has NO riscv32 target.
#  - a working cmake - the system one is fine. mbedtls-rs-sys cross-builds mbedtls with
#    a toolchain file that sets CMAKE_SYSTEM_NAME=Generic, so cmake must NOT add the macOS
#    host `-arch` flag (clang rejects `-arch` for riscv). cmake 3.x and 4.4+ honor Generic
#    correctly; a regression in early cmake 4.x (4.0-4.3) leaked `-arch` anyway - the guard
#    below warns for that range. (We used to pin the esp-idf-bundled cmake 3.30.2; no longer
#    needed once the system cmake is 4.4+.)
#  - LIBCLANG_PATH for bindgen.
# See docs/no-std-plan.md.
#
# Log level: the firmware calls `esp_println::logger::init_logger_from_env()`, which
# bakes the `ESP_LOG` env var in at COMPILE time. To see `debug!`/`trace!` output
# (e.g. the EMF "peak-to-peak = N" line) you must rebuild with ESP_LOG=debug.
# Pass `-d`/`--debug` (or set ESP_LOG yourself) to do that.
#
# Usage:
#   ./build.sh                    # cargo build       (ESP_LOG=info, default)
#   ./build.sh -d                 # cargo build       (ESP_LOG=debug)
#   ./build.sh run --release      # cargo run --release (flash + monitor), info
#   ./build.sh -d run --release   # same, with debug logging
#   ESP_LOG=trace ./build.sh run  # any level via the env var directly
set -euo pipefail

# Optional leading debug flag. Strip it before handing the rest to cargo.
if [[ "${1:-}" == "-d" || "${1:-}" == "--debug" || "${1:-}" == "debug" ]]; then
  export ESP_LOG="debug"
  shift
fi
# Default level when the caller didn't set one.
export ESP_LOG="${ESP_LOG:-info}"
echo "build.sh: ESP_LOG=$ESP_LOG (compile-time log level)" >&2

# Warn if the system cmake is in the known-bad early-4.x range (leaks a macOS `-arch`
# flag into the riscv cross-build despite CMAKE_SYSTEM_NAME=Generic -> mbedtls fails).
# cmake 3.x and >= 4.4 are fine.
if cmake_ver="$(command cmake --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')"; then
  cmake_maj="${cmake_ver%%.*}"; cmake_min="$(printf '%s' "$cmake_ver" | cut -d. -f2)"
  if [ "$cmake_maj" = "4" ] && [ "$cmake_min" -lt 4 ] 2>/dev/null; then
    echo "build.sh: WARNING: cmake $cmake_ver - early 4.x (4.0-4.3) leaks a macOS -arch flag into the riscv mbedtls build. Use cmake >= 4.4 (or 3.x)." >&2
  fi
else
  echo "build.sh: WARNING: cmake not found on PATH." >&2
fi

export PATH="/opt/homebrew/opt/llvm/bin:$PATH"
export LIBCLANG_PATH="/opt/homebrew/opt/llvm/lib"

cargo "${@:-build}"

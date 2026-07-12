#!/usr/bin/env bash
# Build/flash wrapper. The mbedtls-rs-sys riscv C build needs, on PATH:
#  - a RISC-V-capable clang — brew LLVM; Apple's /usr/bin/clang has NO riscv32 target.
#  - cmake 3.x — cmake 4.x injects `-arch` (a macOS host flag) into the cross-compile
#    even with CMAKE_SYSTEM_NAME=Generic, and clang rejects it for riscv. We ship the
#    cmake 3.30.2 that was bundled with the (now-removed) esp-idf toolchain under
#    ./tools/cmake/ (gitignored, machine-local); it takes precedence over system cmake.
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

# Resolve relative to this script so it works regardless of the caller's CWD.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CMAKE_3X="$SCRIPT_DIR/tools/cmake/3.30.2/CMake.app/Contents/bin"
if [ -d "$CMAKE_3X" ]; then
  export PATH="$CMAKE_3X:$PATH"
else
  echo "build.sh: WARNING: bundled cmake 3.x not found at $CMAKE_3X — falling back to system cmake (4.x breaks the riscv mbedtls build)." >&2
fi
export PATH="/opt/homebrew/opt/llvm/bin:$PATH"
export LIBCLANG_PATH="/opt/homebrew/opt/llvm/lib"

cargo "${@:-build}"

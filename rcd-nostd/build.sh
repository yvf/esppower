#!/usr/bin/env bash
# Build/flash wrapper. The mbedtls-rs-sys riscv C build needs, on PATH:
#  - a RISC-V-capable clang — brew LLVM; Apple's /usr/bin/clang has NO riscv32 target.
#  - cmake 3.x — cmake 4.x injects `-arch` (a macOS host flag) into the cross-compile
#    even with CMAKE_SYSTEM_NAME=Generic, and clang rejects it for riscv. We prefer
#    the esp-idf-bundled cmake 3.30.2 when present.
#  - LIBCLANG_PATH for bindgen.
# See docs/no-std-plan.md.
#
# Usage:
#   ./build.sh                 # cargo build
#   ./build.sh run --release   # cargo run --release (flash + monitor)
set -euo pipefail

CMAKE_3X="/Users/yan/dev/rust/esppower1/rcd-reset-controller/.embuild/espressif/tools/cmake/3.30.2/CMake.app/Contents/bin"
[ -d "$CMAKE_3X" ] && export PATH="$CMAKE_3X:$PATH"
export PATH="/opt/homebrew/opt/llvm/bin:$PATH"
export LIBCLANG_PATH="/opt/homebrew/opt/llvm/lib"

cargo "${@:-build}"

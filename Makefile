# Makefile for the RCD Reset Controller (ESP32-H2, no-std Matter-over-Thread).
#
# Supersedes build.sh (kept for now). Handles the mbedtls-rs-sys / openthread-sys riscv C
# build environment (brew LLVM clang + LIBCLANG_PATH) and the COMPILE-TIME log level.
#
# Log level: the firmware calls esp_println::logger::init_logger_from_env(), which bakes
# ESP_LOG in at COMPILE time - so changing it requires a rebuild (cargo re-runs the affected
# build scripts on an ESP_LOG change). Per-target defaults: debug/flash/run -> `debug`,
# release/flash-release/run-release -> `info`. Override in ANY target with LOG=trace|debug|info.
#
# Usage:
#   make                       # debug build            (ESP_LOG=debug)
#   make release               # release build          (ESP_LOG=info)
#   make flash                 # build debug + flash
#   make flash-release         # build release + flash
#   make run                   # build debug + flash + monitor (symbolized, via cargo runner)
#   make monitor               # attach espmonitor to the device
#   make clean                 # cargo clean + remove capture artifacts
#   make clean-cache           # drop only the flaky mbedtls/openthread C-build caches
#   make LOG=trace flash       # override the log level in any target
#   make PORT=/dev/cu.usbserial-10 monitor   # override the auto-detected serial port

SHELL := /bin/bash

TARGET      := riscv32imac-unknown-none-elf
BIN         := rcd-nostd
DEBUG_ELF   := target/$(TARGET)/debug/$(BIN)
RELEASE_ELF := target/$(TARGET)/release/$(BIN)

# --- riscv C-build toolchain (mbedtls-rs-sys / openthread-sys) ---------------------------
# brew LLVM's clang has a riscv32 target; Apple's /usr/bin/clang does not. LIBCLANG_PATH is
# for bindgen. LLVM_PREFIX is discovered via `brew --prefix llvm` (works on Apple-Silicon and
# Intel), falling back to the Apple-Silicon default; override it if your LLVM lives elsewhere.
LLVM_PREFIX          ?= $(shell brew --prefix llvm 2>/dev/null || echo /opt/homebrew/opt/llvm)
export PATH          := $(LLVM_PREFIX)/bin:$(PATH)
export LIBCLANG_PATH := $(LLVM_PREFIX)/lib

# --- serial port (auto-detected; override with PORT=/dev/cu.XXX) -------------------------
# Prefer the UART-bridge (usbserial-*) device over the native USB-JTAG (usbmodem*): a single
# `ls` of both globs sorts alphabetically and 'usbmodem' < 'usbserial', so it would wrongly
# pick the usbmodem port. Fall back to usbmodem only when no usbserial device is present.
PORT ?= $(shell p=$$(ls /dev/cu.usbserial-* 2>/dev/null | head -n1); \
                [ -n "$$p" ] || p=$$(ls /dev/cu.usbmodem* 2>/dev/null | head -n1); \
                echo "$$p")
# Exported so both `espflash flash` and the cargo runner (espflash flash --monitor) use it.
# Only exported when non-empty, so an empty value doesn't defeat espflash's auto-detect.
ifneq ($(strip $(PORT)),)
export ESPFLASH_PORT := $(strip $(PORT))
endif

# --- log level -------------------------------------------------------------------------
# LOG (if set on the command line) overrides the per-target ESP_LOG default below.
LOG ?=

.DEFAULT_GOAL := debug
.PHONY: debug build release flash flash-release run run-release monitor clean clean-cache help \
        preflight check-os check-cmake check-llvm check-espflash check-espmonitor

## debug / build: cargo build, ESP_LOG=debug by default
debug build: export ESP_LOG := $(or $(LOG),debug)
debug build: preflight
	cargo build

## release: cargo build --release, ESP_LOG=info by default
release: export ESP_LOG := $(or $(LOG),info)
release: preflight
	cargo build --release

## flash: build debug + flash (no monitor)
flash: export ESP_LOG := $(or $(LOG),debug)
flash: preflight check-espflash
	cargo build
	espflash flash $(DEBUG_ELF)

## flash-release: build release + flash (no monitor)
flash-release: export ESP_LOG := $(or $(LOG),info)
flash-release: preflight check-espflash
	cargo build --release
	espflash flash $(RELEASE_ELF)

## run / run-release: build + flash + monitor in one step (symbolized via the cargo runner)
run: export ESP_LOG := $(or $(LOG),debug)
run: preflight check-espflash
	cargo run

run-release: export ESP_LOG := $(or $(LOG),info)
run-release: preflight check-espflash
	cargo run --release

## monitor: attach a serial monitor. Symbolizes panics against the debug ELF if it exists.
## Override the symbol source with MON_ELF=... and the baud with BAUD=...
MON_ELF ?= $(DEBUG_ELF)
BAUD    ?= 115200
monitor: check-espmonitor
	@test -n "$(strip $(PORT))" || { echo "make: no serial device found - set PORT=/dev/cu.XXX"; exit 1; }
	espmonitor --speed $(BAUD) $(if $(wildcard $(MON_ELF)),--bin $(MON_ELF),) $(PORT)

## clean: cargo clean (removes target/, incl. the mbedtls/openthread C builds) + captures
clean:
	cargo clean
	rm -f esp*.out chip-tool.out

## clean-cache: drop only the C-build caches (fixes a stale mbedtls cmake cache after a
## failed build, without a full rebuild of everything else)
clean-cache:
	cargo clean -p mbedtls-rs-sys -p openthread-sys

## preflight: hard build requirements. macOS + Homebrew LLVM (riscv clang) + cmake are all
## needed to cross-build mbedtls/openthread; fail early with an actionable message if missing.
preflight: check-os check-cmake check-llvm

BREW_HINT = command -v brew >/dev/null 2>&1 || echo "       (Homebrew not found - install it first: https://brew.sh)"

## check-os: this build's C-toolchain setup (Homebrew LLVM paths) is macOS-only.
check-os:
	@if [ "$$(uname -s)" != "Darwin" ]; then \
	  echo "error: this firmware build is supported on macOS only (detected: $$(uname -s))."; \
	  echo "       It relies on a Homebrew LLVM riscv toolchain for the mbedtls/openthread C build."; \
	  exit 1; \
	fi

## check-cmake: cmake is required (cross-builds mbedtls); also warn about the bad 4.0-4.3 range
## that leaks a macOS -arch flag into the riscv build (CMAKE_SYSTEM_NAME=Generic).
check-cmake:
	@v=$$(cmake --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+'); \
	if [ -z "$$v" ]; then \
	  echo "error: cmake not found (needed to cross-build mbedtls)."; \
	  echo "       Install it with:  brew install cmake"; \
	  $(BREW_HINT); \
	  exit 1; \
	elif [ "$${v%%.*}" = "4" ] && [ "$$(echo $$v | cut -d. -f2)" -lt 4 ]; then \
	  echo "warning: cmake $$v - early 4.x (4.0-4.3) leaks -arch into the riscv build; use cmake >= 4.4 (or 3.x)."; \
	fi

## check-llvm: Apple's /usr/bin/clang has no riscv32 target, so we need Homebrew LLVM's clang
## (+ libclang for bindgen) at LLVM_PREFIX.
check-llvm:
	@if [ ! -x "$(LLVM_PREFIX)/bin/clang" ]; then \
	  echo "error: RISC-V-capable clang not found at $(LLVM_PREFIX)/bin/clang."; \
	  echo "       Apple's /usr/bin/clang has no riscv32 target; this build needs Homebrew LLVM."; \
	  echo "       Install it with:  brew install llvm"; \
	  echo "       (or point at an existing install:  make <target> LLVM_PREFIX=/path/to/llvm)"; \
	  $(BREW_HINT); \
	  exit 1; \
	fi; \
	if [ ! -e "$(LLVM_PREFIX)/lib/libclang.dylib" ]; then \
	  echo "error: libclang not found at $(LLVM_PREFIX)/lib (bindgen needs it)."; \
	  echo "       Install/repair Homebrew LLVM:  brew install llvm"; \
	  $(BREW_HINT); \
	  exit 1; \
	fi

## check-espflash / check-espmonitor: the flash/monitor tools (cargo-installed).
check-espflash:
	@command -v espflash >/dev/null 2>&1 || { \
	  echo "error: espflash not found (needed to flash the device)."; \
	  echo "       Install it with:  cargo install espflash"; \
	  exit 1; }

check-espmonitor:
	@command -v espmonitor >/dev/null 2>&1 || { \
	  echo "error: espmonitor not found (needed for 'make monitor')."; \
	  echo "       Install it with:  cargo install espmonitor"; \
	  exit 1; }

help:
	@echo "RCD firmware (ESP32-H2). Targets:"
	@echo "  debug (default)   cargo build           ESP_LOG=debug"
	@echo "  release           cargo build --release ESP_LOG=info"
	@echo "  flash | flash-release   build + flash"
	@echo "  run   | run-release     build + flash + monitor (symbolized)"
	@echo "  monitor           espmonitor on the device"
	@echo "  clean | clean-cache"
	@echo "Vars: LOG=trace|debug|info (compile-time, forces rebuild)  PORT=/dev/cu.XXX  BAUD=$(BAUD)"
	@echo "Detected PORT: $(if $(strip $(PORT)),$(PORT),<none>)"

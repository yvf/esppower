//! RCD Reset Controller — no-std (esp-hal) rewrite, ESP32-H2.
//!
//! Skeleton: validates the bare-metal esp-hal 1.1 + esp-rtos + esp-radio + embassy
//! toolchain/version matrix before layering on Thread (openthread), BLE (trouble),
//! and Matter (rs-matter). See docs/no-std-plan.md for the full stack + phases.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use log::info;

use esp_backtrace as _;
use esp_println as _;

// ESP-IDF-style image header so the second-stage bootloader accepts the image.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    // Heap for rs-matter / openthread later. The whole point of no-std is that we
    // get most of the 320 KB SRAM as heap; start modest, grow as the stack lands.
    esp_alloc::heap_allocator!(size: 64 * 1024);

    esp_println::logger::init_logger_from_env();
    info!("RCD no-std skeleton starting…");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(
        timg0.timer0,
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT)
            .software_interrupt0,
    );

    info!("esp-hal + esp-rtos + embassy + heap initialized");

    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("alive (free heap = {} bytes)", esp_alloc::HEAP.free());
    }
}

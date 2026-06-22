//! Matter node: boots the `esp-idf-matter` Thread+BLE stack and runs it.
//!
//! Ported from the `esp-idf-matter` `light_thread.rs` example (the known-good
//! reference for ESP32-H2). The stack:
//!   - commissions over BLE (the QR code + manual pairing code are printed to the
//!     serial console automatically by `run_coex` when no fabric is provisioned),
//!   - then operates over Thread (joining the network advertised by an Apple TV /
//!     HomePod Thread Border Router),
//!   - persists fabric/ACL data in NVS via `EspKvBlobStore`.
//!
//! STAGE 1 (current): a single On/Off endpoint using rs-matter's stock
//! `TestOnOffDeviceLogic`, purely to validate that the device builds, boots,
//! prints a scannable QR, and commissions into Apple Home. The On/Off state is
//! NOT yet wired to the actuator, and there is no Contact Sensor endpoint yet —
//! that is Stage 2 (see `matter` module docs).
//!
//! The RCD reset state machine runs independently on its own thread (see
//! `main.rs`); it deliberately does NOT depend on Matter/Thread connectivity.

use core::pin::pin;

use esp_idf_matter::init_async_io;
use esp_idf_matter::matter::clusters;
use esp_idf_matter::matter::crypto::{default_crypto, Crypto};
use esp_idf_matter::matter::dm::clusters::app::on_off::{self, OnOffHandler, OnOffHooks};
use esp_idf_matter::matter::dm::clusters::app::on_off::test::TestOnOffDeviceLogic;
use esp_idf_matter::matter::dm::clusters::desc::{self, ClusterHandler as _, DescHandler};
use esp_idf_matter::matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET,
};
use esp_idf_matter::matter::dm::devices::DEV_TYPE_ON_OFF_LIGHT;
use esp_idf_matter::matter::dm::{
    Async, Dataver, EmptyHandler, Endpoint, EpClMatcher, Node,
};
use esp_idf_matter::matter::devices;
use esp_idf_matter::matter::persist::DummyKvBlobStore;
use esp_idf_matter::matter::utils::init::InitMaybeUninit;
use esp_idf_matter::wireless::{EspMatterThread, EspThreadMatterStack};

use esp_idf_svc::bt::reduce_bt_memory;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::io::vfs::MountedEventfs;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

use alloc::sync::Arc;
use log::info;
use static_cell::StaticCell;

extern crate alloc;

/// Endpoint 0 (the hidden Matter system endpoint) is always present; functional
/// endpoints start at 1.
const LIGHT_ENDPOINT_ID: u16 = 1;

/// Bump-allocator size for the Matter stack. 17000 is the value the reference
/// `light_thread.rs` uses for esp32c6/h2.
const BUMP_SIZE: usize = 17000;

/// The Matter stack is large and MUST be allocated statically (mandatory for the
/// Thread+BLE coex stack, and avoids blowing the thread stack).
static MATTER_STACK: StaticCell<EspThreadMatterStack<BUMP_SIZE, ()>> = StaticCell::new();

/// The Matter data model: root system endpoint + our single On/Off endpoint.
const NODE: Node = Node {
    endpoints: &[
        EspThreadMatterStack::<0, ()>::root_endpoint(),
        Endpoint::new(
            LIGHT_ENDPOINT_ID,
            devices!(DEV_TYPE_ON_OFF_LIGHT),
            clusters!(DescHandler::CLUSTER, TestOnOffDeviceLogic::CLUSTER),
        ),
    ],
};

/// Run the Matter stack to completion (i.e. forever). Intended to be driven by
/// `esp_idf_svc::hal::task::block_on` on a dedicated, large-stacked thread.
///
/// `modem` is the radio peripheral (Thread + BLE); it is moved in from `main`
/// after the controller peripherals have been split off.
pub async fn run(mut modem: Modem<'static>) -> Result<(), anyhow::Error> {
    // Initialize the Matter stack (can be done only once per boot).
    let stack = MATTER_STACK
        .uninit()
        .init_with(EspThreadMatterStack::init(
            &TEST_DEV_DET,
            TEST_DEV_COMM,
            &TEST_DEV_ATT,
        ));

    info!("Matter: stack initialized");

    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mounted_event_fs = Arc::new(MountedEventfs::mount(6)?);
    init_async_io(mounted_event_fs.clone())?;

    // Free BT memory we will not use (BLE-only commissioning). Takes a reborrow
    // so the modem itself can still be moved into the Thread stack below.
    reduce_bt_memory(unsafe { modem.reborrow() })?;

    info!("Matter: basics initialized");

    // Default crypto provider over the std CSPRNG.
    let crypto = default_crypto(rand::thread_rng(), DAC_PRIVKEY);
    let mut weak_rand = crypto.weak_rand()?;

    // STAGE 1: stock test On/Off device logic on Endpoint 1.
    let on_off = OnOffHandler::new_standalone(
        Dataver::new_rand(&mut weak_rand),
        LIGHT_ENDPOINT_ID,
        TestOnOffDeviceLogic::new(true),
    );

    // Chain our endpoint-1 clusters onto the (root) system clusters.
    let handler = EmptyHandler
        .chain(
            EpClMatcher::new(
                Some(LIGHT_ENDPOINT_ID),
                Some(TestOnOffDeviceLogic::CLUSTER.id),
            ),
            on_off::HandlerAsyncAdaptor(&on_off),
        )
        .chain(
            EpClMatcher::new(Some(LIGHT_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
            Async(desc::DescHandler::new(Dataver::new_rand(&mut weak_rand)).adapt()),
        );

    info!("Matter: handler initialized");

    // STAGE 1 uses a no-op KV store (fabric data is not persisted across reboots,
    // so the device must be re-commissioned after each flash). Stage 2 swaps in
    // `EspKvBlobStore` for real persistence.
    let mut kv = DummyKvBlobStore;
    stack.startup(&crypto, &mut kv).await?;
    let kv = stack.create_shared_kv(kv)?;

    // Run the stack with concurrent Thread+BLE commissioning. `run_coex` prints
    // the commissioning QR code + manual pairing code to the console when no
    // fabric is provisioned. `()` = no network-dependent user task.
    let matter = pin!(stack.run_coex(
        EspMatterThread::new(modem, sysloop, nvs, mounted_event_fs, stack),
        &crypto,
        (NODE, handler),
        &kv,
        (),
    ));

    info!("Matter: running (commissioning QR will be printed below if unprovisioned)");

    matter.await?;

    Ok(())
}

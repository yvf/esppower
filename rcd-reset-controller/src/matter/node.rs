//! Matter node: boots the `esp-idf-matter` Thread+BLE stack and runs it.
//!
//! Ported from the `esp-idf-matter` `light_thread.rs` example (the known-good
//! reference for ESP32-H2). The stack:
//!   - commissions over BLE (the QR code + manual pairing code are printed to the
//!     serial console automatically by `run_coex` when no fabric is provisioned),
//!   - then operates over Thread (joining the network advertised by an Apple TV /
//!     HomePod Thread Border Router),
//!   - persists fabric/ACL data in NVS via `EspKvBlobStore` (currently a no-op
//!     `DummyKvBlobStore` — see the KV note in `run`).
//!
//! Endpoint 1 is an On/Off endpoint backed by [`PlugHooks`]: toggling it ON in
//! HomeKit fires one actuator reset cycle, and the controller's cycle state is
//! reflected back into the tile. (Cluster metadata is borrowed from rs-matter's
//! stock On/Off logic; the device still advertises as an On/Off "light" — a
//! plug/outlet device type and a Contact Sensor endpoint are the remaining
//! refinements, see `matter` module docs.)
//!
//! The RCD reset state machine runs independently on its own thread (see
//! `main.rs`); it deliberately does NOT depend on Matter/Thread connectivity.

use core::pin::pin;

use core::sync::atomic::{AtomicBool, Ordering};

use esp_idf_matter::init_async_io;
use esp_idf_matter::matter::clusters;
use esp_idf_matter::matter::crypto::{default_crypto, Crypto};
use esp_idf_matter::matter::dm::clusters::app::on_off::test::TestOnOffDeviceLogic;
use esp_idf_matter::matter::dm::clusters::app::on_off::{
    self, EffectVariantEnum, OnOffHandler, OnOffHooks, OutOfBandMessage, StartUpOnOffEnum,
};
use esp_idf_matter::matter::dm::clusters::desc::{self, ClusterHandler as _, DescHandler};
use esp_idf_matter::matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET,
};
use esp_idf_matter::matter::dm::devices::DEV_TYPE_ON_OFF_LIGHT;
use esp_idf_matter::matter::dm::{
    Async, Cluster, Dataver, EmptyHandler, Endpoint, EpClMatcher, Node,
};
use esp_idf_matter::matter::devices;
use esp_idf_matter::matter::error::Error;
use esp_idf_matter::matter::persist::DummyKvBlobStore;
use esp_idf_matter::matter::tlv::Nullable;
use esp_idf_matter::matter::utils::init::InitMaybeUninit;
use esp_idf_matter::wireless::{EspMatterThread, EspThreadMatterStack};

use esp_idf_svc::bt::reduce_bt_memory;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::io::vfs::MountedEventfs;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Receiver, Sender};

use alloc::sync::Arc;
use log::{info, warn};
use static_cell::StaticCell;

use crate::matter::{ToController, ToMatter};

extern crate alloc;

/// Channel endpoint types shared with the controller task (capacity 4).
type CtrlSender = Sender<'static, CriticalSectionRawMutex, ToController, 4>;
type MatterReceiver = Receiver<'static, CriticalSectionRawMutex, ToMatter, 4>;

/// On/Off device logic for the RCD reset trigger (Endpoint 1).
///
/// HomeKit toggling the endpoint ON fires one actuator reset cycle
/// (`ToController::ManualTrigger`). The controller reports cycle progress back
/// via `ToMatter::SetPlugOnOff`, which `run()` reflects into the reported
/// attribute (ON while a cycle runs, OFF when it completes) so the tile tracks
/// reality. Re-triggers while a cycle is already running are ignored by the
/// controller, so firing on every ON is safe.
pub struct PlugHooks {
    on: AtomicBool,
    ctrl_tx: CtrlSender,
    matter_rx: MatterReceiver,
}

impl PlugHooks {
    pub fn new(ctrl_tx: CtrlSender, matter_rx: MatterReceiver) -> Self {
        Self {
            on: AtomicBool::new(false),
            ctrl_tx,
            matter_rx,
        }
    }
}

impl OnOffHooks for PlugHooks {
    /// Reuse rs-matter's stock On/Off cluster metadata (validated in Stage 1).
    const CLUSTER: Cluster<'static> = TestOnOffDeviceLogic::CLUSTER;

    fn on_off(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    fn set_on_off(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        // ON from HomeKit = "reset now". The controller ignores triggers while a
        // cycle is already running, so firing on every ON is safe.
        if on && self.ctrl_tx.try_send(ToController::ManualTrigger).is_err() {
            warn!("Matter: controller channel full — manual trigger dropped");
        }
    }

    fn start_up_on_off(&self) -> Nullable<StartUpOnOffEnum> {
        // No persisted power-on behaviour: the trigger always rests OFF.
        Nullable::none()
    }

    fn set_start_up_on_off(&self, _value: Nullable<StartUpOnOffEnum>) -> Result<(), Error> {
        Ok(())
    }

    async fn handle_off_with_effect(&self, _effect: EffectVariantEnum) {}

    async fn run<F: Fn(OutOfBandMessage)>(&self, notify: F) {
        // Reflect controller-driven plug state into the cluster attribute.
        // `OutOfBandMessage::Update` re-reads `on_off()` and reports to
        // subscribers WITHOUT calling `set_on_off`, so there is no trigger loop.
        loop {
            match self.matter_rx.receive().await {
                ToMatter::SetPlugOnOff(on) => {
                    self.on.store(on, Ordering::Relaxed);
                    notify(OutOfBandMessage::Update);
                }
                // Contact Sensor endpoint not implemented yet (Stage 3).
                ToMatter::SetContactClosed(_) => {}
            }
        }
    }
}

/// Endpoint 0 (the hidden Matter system endpoint) is always present; functional
/// endpoints start at 1.
const LIGHT_ENDPOINT_ID: u16 = 1;

/// Bump-allocator size for the Matter stack (rs-matter future arena, lives in
/// static BSS). The reference uses 17000; on-device logging showed only ~6.3 KB
/// used through BLE bring-up, so 14000 still leaves headroom for the
/// commissioning futures while freeing ~3 KB of RAM toward the heap. If
/// commissioning ever panics with a bump-exhausted message, raise this.
const BUMP_SIZE: usize = 14000;

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
            clusters!(DescHandler::CLUSTER, PlugHooks::CLUSTER),
        ),
    ],
};

/// Run the Matter stack to completion (i.e. forever). Intended to be driven by
/// `esp_idf_svc::hal::task::block_on` on a dedicated, large-stacked thread.
///
/// `modem` is the radio peripheral (Thread + BLE); it is moved in from `main`
/// after the controller peripherals have been split off. `ctrl_tx`/`matter_rx`
/// are the channel endpoints shared with the controller task: HomeKit toggles
/// fire `ToController::ManualTrigger`, and controller cycle state arrives as
/// `ToMatter::SetPlugOnOff`.
pub async fn run(
    mut modem: Modem<'static>,
    ctrl_tx: CtrlSender,
    matter_rx: MatterReceiver,
) -> Result<(), anyhow::Error> {
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

    // On/Off device logic on Endpoint 1: HomeKit toggle → actuator reset cycle.
    let on_off = OnOffHandler::new_standalone(
        Dataver::new_rand(&mut weak_rand),
        LIGHT_ENDPOINT_ID,
        PlugHooks::new(ctrl_tx, matter_rx),
    );

    // Chain our endpoint-1 clusters onto the (root) system clusters.
    let handler = EmptyHandler
        .chain(
            EpClMatcher::new(Some(LIGHT_ENDPOINT_ID), Some(PlugHooks::CLUSTER.id)),
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

    // Diagnostic: Bluedroid mallocs a large environment at init; on the
    // RAM-constrained H2 this is the most likely thing to fail. Log the free
    // heap going into Thread+BLE bring-up so we can see how tight it is.
    info!(
        "Matter: free heap before Thread+BLE init = {} bytes",
        unsafe { esp_idf_svc::sys::esp_get_free_heap_size() }
    );

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

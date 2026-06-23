//! Phase 4b (6/6): assemble the Matter stack and run it.
//!
//! Builds a `ThreadMatterStack` and runs it via `run_preex`, feeding our five
//! openthread/trouble adapters. Auto-advertises BLE for commissioning and prints
//! the pairing QR/code; once commissioned, operates over Thread.
//!
//! Uses the rs-matter TEST device credentials + a no-persistence KV — enough to
//! commission with chip-tool / for bring-up. Real DAC + a flash-backed KV come
//! later. Port of rs-matter-stack `examples/light.rs`.

use openthread::OpenThread;
use trouble_host::prelude::Controller;

use rs_matter_stack::matter::crypto::{default_crypto, Crypto};
use rs_matter_stack::matter::dm::clusters::app::on_off;
use rs_matter_stack::matter::dm::clusters::app::on_off::test::TestOnOffDeviceLogic;
use rs_matter_stack::matter::dm::clusters::app::on_off::OnOffHooks as _; // for ::CLUSTER
use rs_matter_stack::matter::dm::clusters::desc::{ClusterHandler as _, DescHandler};
use rs_matter_stack::matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET,
};
use rs_matter_stack::matter::dm::devices::DEV_TYPE_ON_OFF_LIGHT;
use rs_matter_stack::matter::dm::{Async, Dataver, EmptyHandler, Endpoint, EpClMatcher, Node};
use rs_matter_stack::matter::error::Error;
use rs_matter_stack::matter::persist::DummyKvBlobStore;
use rs_matter_stack::matter::utils::init::InitMaybeUninit;
use rs_matter_stack::matter::{clusters, devices};
use rs_matter_stack::wireless::{PreexistingWireless, ThreadMatterStack};

use static_cell::StaticCell;

use super::{OtGattPeripheral, OtMdns, OtNetCtl, OtNetStack, OtNetif};

/// Bump-allocator arena for the rs-matter-stack run futures (static RAM, not heap).
/// Tune up if init panics for lack of space. (light.rs uses 23500.)
const BUMP_SIZE: usize = 23500;

/// Endpoint 1 — endpoint 0 is the root (system clusters).
const LIGHT_ENDPOINT_ID: u16 = 1;

static MATTER_STACK: StaticCell<ThreadMatterStack<BUMP_SIZE>> = StaticCell::new();

/// The Matter node: root endpoint + a placeholder On/Off light (Phase 5 swaps in
/// the real RCD plug/controller).
const NODE: Node = Node {
    endpoints: &[
        ThreadMatterStack::<0, ()>::root_endpoint(),
        Endpoint::new(
            LIGHT_ENDPOINT_ID,
            devices!(DEV_TYPE_ON_OFF_LIGHT),
            clusters!(DescHandler::CLUSTER, TestOnOffDeviceLogic::CLUSTER),
        ),
    ],
};

/// A rand_core 0.6 `CryptoRngCore` over esp-hal's (0.9) Rng, for rs-matter's crypto.
#[derive(Clone, Copy)]
struct EspRng(esp_hal::rng::Rng);

impl rand_core_06::RngCore for EspRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        rand_core::RngCore::fill_bytes(&mut self.0, &mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        rand_core::RngCore::fill_bytes(&mut self.0, &mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        rand_core::RngCore::fill_bytes(&mut self.0, dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core_06::Error> {
        rand_core::RngCore::fill_bytes(&mut self.0, dest);
        Ok(())
    }
}
impl rand_core_06::CryptoRng for EspRng {}

/// Build and run the Matter-over-Thread stack until commissioning + operation are
/// torn down. `ot` must be the initialized OpenThread instance; `controller` the
/// BLE controller; `eui64` the IEEE EUI-64 (hostname + BLE address material).
pub async fn run_matter<C: Controller>(
    ot: OpenThread<'static>,
    controller: C,
    eui64: [u8; 8],
    rng: esp_hal::rng::Rng,
) -> Result<(), Error> {
    let stack = MATTER_STACK
        .uninit()
        .init_with(ThreadMatterStack::init(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT));

    let crypto = default_crypto(EspRng(rng), DAC_PRIVKEY);
    let mut rand = crypto.weak_rand()?;

    // Placeholder On/Off light on EP1 (Phase 5 replaces with the RCD plug).
    let device = on_off::OnOffHandler::new_standalone(
        Dataver::new_rand(&mut rand),
        LIGHT_ENDPOINT_ID,
        TestOnOffDeviceLogic::new(true),
    );

    let handler = EmptyHandler
        .chain(
            EpClMatcher::new(Some(LIGHT_ENDPOINT_ID), Some(TestOnOffDeviceLogic::CLUSTER.id)),
            on_off::HandlerAsyncAdaptor(&device),
        )
        .chain(
            EpClMatcher::new(Some(LIGHT_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
            Async(DescHandler::new(Dataver::new_rand(&mut rand)).adapt()),
        );

    // No-persistence KV (commissioning won't survive reboot yet). Load (no-op for
    // the dummy store) then pair it with Matter's buffer as a KvBlobStoreAccess.
    let mut store = DummyKvBlobStore;
    stack.startup(&crypto, &mut store).await?;
    let kv = stack.matter().kv(store);

    // BLE address: locally-administered random static, derived from the EUI-64.
    let mut addr = eui64[2..8].try_into().unwrap_or([0xff; 6]);
    addr[0] |= 0xc0; // mark as random-static

    stack
        .run_coex(
            PreexistingWireless::new(
                OtNetStack::new(ot.clone()),
                OtNetif::new(ot.clone(), eui64),
                OtNetCtl::new(ot.clone()),
                OtMdns::new(ot.clone(), eui64),
                OtGattPeripheral::new(controller, addr),
            ),
            &crypto,
            (NODE, handler),
            kv,
            (),
        )
        .await
}

//! Assemble and run the Matter-over-Thread stack on `rs-matter-embassy`.
//!
//! `EmbassyThreadMatterStack` + `EspThreadDriver` provide the whole transport (openthread +
//! esp-radio + trouble BLE + edge-nal-openthread + KV-backed persistence, non-concurrent:
//! BLE while un-commissioned, then Thread-only). This module supplies only the device model
//! - the 2-endpoint node (contact sensor + reset plug) - plus persistence over the `nvs`
//! flash partition, a GPIO5 factory-reset button, and the commissioning-QR print.
//!
//! Ports the openthread/trouble hand-rolled glue (deleted) to ivmarkov's blessed stack;
//! modelled on rs-matter-embassy `examples/esp/{light_thread,light_wifi_persistent}.rs`.

use core::borrow::BorrowMut;
use core::pin::pin;

use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Timer};

use esp_bootloader_esp_idf::partitions::{
    read_partition_table, DataPartitionSubType, PartitionType, PARTITION_TABLE_MAX_LEN,
};
use esp_hal::peripherals::{BT, FLASH, GPIO5, IEEE802154};
use esp_hal::rng::Rng;
use esp_storage::FlashStorage;

use rs_matter_embassy::matter::crypto::{default_crypto, Crypto};
use rs_matter_embassy::matter::dm::clusters::app::on_off::{self, OnOffHooks as _};
use rs_matter_embassy::matter::dm::clusters::basic_info::BasicInfoConfig;
use rs_matter_embassy::matter::dm::clusters::desc::{ClusterHandler as _, DescHandler};
use rs_matter_embassy::matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET,
};
use rs_matter_embassy::matter::dm::{
    Async, Dataver, DeviceType, EmptyHandler, Endpoint, EpClMatcher, Node,
};
use rs_matter_embassy::matter::pairing::qr::{
    no_optional_data, CommFlowType, NoOptionalData, Qr, QrPayload, QrTextType,
};
use rs_matter_embassy::matter::pairing::DiscoveryCapabilities;
use rs_matter_embassy::matter::persist::KvBlobStore;
use rs_matter_embassy::matter::utils::init::InitMaybeUninit;
use rs_matter_embassy::matter::{clusters, devices, BasicCommData};
use rs_matter_embassy::persist::SeqMapKvBlobStore;
use rs_matter_embassy::wireless::esp::EspThreadDriver;
use rs_matter_embassy::wireless::{EmbassyThread, EmbassyThreadMatterStack};

use static_cell::StaticCell;

use super::contact::{ContactHandler, CONTACT_CLUSTER, CONTACT_ENDPOINT_ID};
use super::plug::RcdPlugHooks;

/// Bump-allocator arena for the rs-matter-stack run futures (static RAM, not heap).
/// Tune up if init panics for lack of space. (rs-matter-embassy examples use 25000.)
const BUMP_SIZE: usize = 25000;

/// Fixed commissioning discriminator - we use the rs-matter TEST credentials, so it is
/// constant and the printed QR is stable across boots.
const DISCRIMINATOR: u16 = 3840;

/// Endpoint 2 - On/Off Plug-In Unit (the RCD resetter). Endpoint 1 is the contact sensor
/// (the primary tile - see `docs/matter_device_choices.md`); endpoint 0 is the root.
const PLUG_ENDPOINT_ID: u16 = 2;

/// On/Off Plug-In Unit device type (Matter Device Library `0x010A`) - a tappable outlet
/// tile (vs the misleading lightbulb of On/Off Light). See docs/matter_device_choices.md.
const DEV_TYPE_ON_OFF_PLUG_IN_UNIT: DeviceType = DeviceType {
    dtype: 0x010A,
    drev: 3,
};

/// Contact Sensor device type (Matter Device Library `0x0015`). Read-only tile fed by the
/// Boolean State cluster; HomeKit alerts when it opens (power lost / RCD tripped).
const DEV_TYPE_CONTACT_SENSOR: DeviceType = DeviceType {
    dtype: 0x0015,
    drev: 2,
};

/// Basic device info: the rs-matter TEST details plus a session-active interval hint.
const TEST_BASIC_INFO: BasicInfoConfig = BasicInfoConfig {
    sai: Some(500),
    ..TEST_DEV_DET
};

/// The Matter node (docs/matter_device_choices.md):
///   EP0 - Root Node (system clusters)
///   EP1 - Contact Sensor: downstream mains presence (Boolean State) - the PRIMARY tile
///   EP2 - On/Off Plug-In Unit: the RCD resetter (tap -> actuator reset cycle)
const NODE: Node = Node {
    endpoints: &[
        EmbassyThreadMatterStack::<0, ()>::root_endpoint(),
        Endpoint::new(
            CONTACT_ENDPOINT_ID,
            devices!(DEV_TYPE_CONTACT_SENSOR),
            clusters!(DescHandler::CLUSTER, CONTACT_CLUSTER),
        ),
        Endpoint::new(
            PLUG_ENDPOINT_ID,
            devices!(DEV_TYPE_ON_OFF_PLUG_IN_UNIT),
            clusters!(DescHandler::CLUSTER, RcdPlugHooks::CLUSTER),
        ),
    ],
};

static MATTER_STACK: StaticCell<EmbassyThreadMatterStack<BUMP_SIZE, ()>> = StaticCell::new();
static QR_BUF: StaticCell<[u8; 1024]> = StaticCell::new();

/// A rand_core 0.6 `CryptoRngCore` over esp-hal's (0.9) digital `Rng`, for rs-matter's
/// crypto. Uses the digital RNG (no ADC1) so ADC1 stays free for the EMF sensor.
#[derive(Clone, Copy)]
struct EspRng(Rng);

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

/// Build and run the Matter-over-Thread stack. Never returns during normal operation:
/// on a 3 s hold of the GPIO5 factory-reset button it wipes the persisted pairing and
/// reboots; on an unexpected stack exit it also reboots (the controller safety task runs
/// independently and is unaffected).
pub async fn run_matter(
    ieee802154: IEEE802154<'static>,
    bt: BT<'static>,
    flash: FLASH<'static>,
    reset_pin: GPIO5<'static>,
    eui64: [u8; 8],
) -> ! {
    let crypto = default_crypto(EspRng(Rng::new()), DAC_PRIVKEY);
    let mut weak_rand = crypto.weak_rand().unwrap();

    let stack = MATTER_STACK.uninit().init_with(EmbassyThreadMatterStack::init(
        &TEST_BASIC_INFO,
        BasicCommData {
            password: TEST_DEV_COMM.password,
            discriminator: DISCRIMINATOR,
        },
        &TEST_DEV_ATT,
    ));

    // EP2 - On/Off Plug-In Unit (RCD resetter) and EP1 - Contact Sensor (mains presence).
    // Both bridge to the autonomous controller via `crate::link`.
    let plug = on_off::OnOffHandler::new_standalone(
        Dataver::new_rand(&mut weak_rand),
        PLUG_ENDPOINT_ID,
        RcdPlugHooks,
    );
    let contact = ContactHandler::new(Dataver::new_rand(&mut weak_rand));

    let handler = EmptyHandler
        .chain(
            EpClMatcher::new(Some(PLUG_ENDPOINT_ID), Some(RcdPlugHooks::CLUSTER.id)),
            on_off::HandlerAsyncAdaptor(&plug),
        )
        .chain(
            EpClMatcher::new(Some(PLUG_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
            Async(DescHandler::new(Dataver::new_rand(&mut weak_rand)).adapt()),
        )
        .chain(
            EpClMatcher::new(Some(CONTACT_ENDPOINT_ID), Some(CONTACT_CLUSTER.id)),
            Async(&contact),
        )
        .chain(
            EpClMatcher::new(Some(CONTACT_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
            Async(DescHandler::new(Dataver::new_rand(&mut weak_rand)).adapt()),
        );

    // Flash-backed KV over the `nvs` partition (fabrics + Thread networks + the OpenThread
    // SRP key, so pairing and the SRP identity survive reboot). `SeqMapKvBlobStore` is
    // rs-matter-embassy's sequential-storage NOR-flash store.
    let mut pt_buf = [0u8; PARTITION_TABLE_MAX_LEN];
    let mut store = get_persistent_store(flash, &mut pt_buf[..]);
    log::info!("[matter] Loading persisted state from flash (fabrics + Thread network)...");
    stack.startup(&crypto, &mut store).await.unwrap();
    log::info!("[matter] Persisted state loaded (commissioned={})", stack.is_commissioned());
    let kv = stack.matter().kv(store);

    if stack.is_commissioned() {
        log::info!("[matter] ===== MODE: OPERATIONAL (saved pairing loaded) =====");
        log::info!("[matter] Operating over Thread; BLE is OFF. Hold the GPIO5 button 3s to re-pair.");
    } else {
        log::info!("[matter] ===== MODE: COMMISSIONING (no saved pairing) =====");
        log::info!("[matter] Commissioning open over BLE - TEST credentials:");
        log::info!("[matter]   discriminator : {DISCRIMINATOR}");
        log::info!("[matter]   passcode      : 20202021");
        log::info!("[matter]   manual code   : 3497-011-2332");
        print_commissioning_qr();
    }

    // Run Matter, racing a factory-reset hold on GPIO5. `stack.run` drives the whole
    // non-concurrent flow (BLE commission -> Thread operate) and normally never returns.
    // The inner scope ends the `&kv` borrow (held by the run future) before the possible
    // `reset_persist(kv)`, which moves `kv`.
    if stack.is_commissioned() {
        log::info!(
            "[matter] Entering operational run: re-attaching to Thread + re-registering SRP. \
             Watch for 'Role -> child' then 'Registered SRP host'. Apple Home may report \
             'No Response' until it re-resolves us via DNS-SD and re-establishes CASE + subscription."
        );
    } else {
        log::info!("[matter] Entering run: advertising BLE for commissioning.");
    }
    let do_reset = {
        let matter = pin!(stack.run(
            EmbassyThread::new(
                EspThreadDriver::new(ieee802154, bt),
                crypto.rand().unwrap(),
                eui64,
                &kv,
                stack,
                true, // static-random BLE address
            ),
            &crypto,
            (NODE, handler),
            &kv,
            (),
        ));
        match select(matter, pin!(wait_reset_hold(reset_pin))).await {
            Either::First(res) => {
                log::warn!("[matter] stack.run returned ({res:?}) - rebooting");
                false
            }
            Either::Second(()) => true,
        }
    };

    if do_reset {
        log::warn!("[matter] Factory reset: wiping persisted pairing and rebooting");
        let _ = stack.matter().reset_persist(kv).await;
    }
    esp_hal::system::software_reset()
}

/// Complete after the GPIO5 momentary button (active-low, internal pull-up) is held low
/// continuously for 3 s. Polling debounces naturally.
async fn wait_reset_hold(pin: GPIO5<'static>) {
    use esp_hal::gpio::{Input, InputConfig, Pull};

    let button = Input::new(pin, InputConfig::default().with_pull(Pull::Up));
    const POLL_MS: u64 = 50;
    const HOLD_MS: u64 = 3000;
    let mut held_ms: u64 = 0;
    loop {
        Timer::after(Duration::from_millis(POLL_MS)).await;
        if button.is_low() {
            held_ms += POLL_MS;
            if held_ms >= HOLD_MS {
                return;
            }
        } else {
            held_ms = 0;
        }
    }
}

/// KV BLOB store persisting in the first `nvs` data partition of the on-board NOR flash
/// (located at runtime via the esp-idf partition table). Mirrors the rs-matter-embassy
/// persistent example.
fn get_persistent_store<'d>(
    flash: FLASH<'d>,
    mut buf: impl BorrowMut<[u8]>,
) -> impl KvBlobStore + 'd {
    let mut flash = FlashStorage::new(flash);
    let pt_buf = &mut buf.borrow_mut()[..PARTITION_TABLE_MAX_LEN];
    let pt = read_partition_table(&mut flash, pt_buf).unwrap();
    let nvs = pt
        .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
        .unwrap()
        .unwrap();
    let start = nvs.offset();
    let end = nvs.offset() + nvs.len();
    log::info!("[matter] Using NVS partition at {start:#x}..{end:#x}");
    // `SeqMapKvBlobStore` needs an async MultiwriteNorFlash; esp-storage's FlashStorage is
    // blocking, so wrap it in `BlockingAsync`.
    SeqMapKvBlobStore::new(BlockingAsync::new(flash), start..end)
}

/// Render the commissioning QR code (ASCII) plus its text payload. We print it ourselves
/// (rather than via rs-matter's `log`) from the fixed TEST credentials.
fn print_commissioning_qr() {
    let buf = QR_BUF.init([0u8; 1024]);

    let payload = QrPayload::new_from_basic_info(
        DiscoveryCapabilities::BLE,
        CommFlowType::Standard,
        TEST_DEV_COMM,
        &TEST_DEV_DET,
        no_optional_data as NoOptionalData,
    );

    let (text, rest) = match payload.as_str(buf) {
        Ok(v) => v,
        Err(_) => {
            log::warn!("[matter] could not compute QR payload");
            return;
        }
    };

    log::info!("[matter]   QR payload (paste into a QR-code generator app): {text}");

    let (tmp_buf, out_buf) = rest.split_at_mut(rest.len() / 2);
    match Qr::compute(text, tmp_buf, out_buf) {
        Ok(qr) => {
            const BORDER: u8 = 4;
            log::info!("[matter] Scan this QR code in Apple Home / your Matter commissioner:");
            for y in qr.lines_range(QrTextType::Ascii, BORDER) {
                match qr.line_as_str(QrTextType::Ascii, BORDER, false, false, y, tmp_buf) {
                    Ok((line, _)) => log::info!("{line}"),
                    Err(_) => break,
                }
            }
        }
        Err(_) => log::warn!("[matter] could not render QR art (payload text above still works)"),
    }
}

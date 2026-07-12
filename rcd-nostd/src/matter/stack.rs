//! Phase 4b (6/6): assemble the Matter stack and run it.
//!
//! Builds a `ThreadMatterStack` and runs it via `run_preex`, feeding our five
//! openthread/trouble adapters. Auto-advertises BLE for commissioning and prints
//! the pairing QR/code; once commissioned, operates over Thread.
//!
//! Uses the rs-matter TEST device credentials + a no-persistence KV — enough to
//! commission with chip-tool / for bring-up. Real DAC + a flash-backed KV come
//! later. Port of rs-matter-stack `examples/light.rs`.

use embassy_time::{Duration, Timer};

use esp_radio::ble::controller::BleConnector;

use openthread::OpenThread;
use trouble_host::prelude::ExternalController;

use rs_matter_stack::matter::crypto::{default_crypto, Crypto};
use rs_matter_stack::matter::dm::clusters::app::on_off;
use rs_matter_stack::matter::dm::clusters::app::on_off::OnOffHooks as _; // for ::CLUSTER
use rs_matter_stack::matter::dm::clusters::desc::{ClusterHandler as _, DescHandler};
use rs_matter_stack::matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET,
};
use rs_matter_stack::matter::dm::{
    Async, Dataver, DeviceType, EmptyHandler, Endpoint, EpClMatcher, Node,
};
use rs_matter_stack::matter::error::{Error, ErrorCode};
use rs_matter_stack::matter::pairing::qr::{
    no_optional_data, CommFlowType, NoOptionalData, Qr, QrPayload, QrTextType,
};
use rs_matter_stack::matter::pairing::DiscoveryCapabilities;
use super::kv::FlashKv;
use rs_matter_stack::matter::utils::init::InitMaybeUninit;
use rs_matter_stack::matter::{clusters, devices};
use rs_matter_stack::wireless::{PreexistingWireless, ThreadMatterStack};

use static_cell::StaticCell;

use super::contact::{ContactHandler, CONTACT_CLUSTER, CONTACT_ENDPOINT_ID};
use super::plug::RcdPlugHooks;
use super::{OtGattPeripheral, OtMdns, OtNetCtl, OtNetStack, OtNetif};

/// Bump-allocator arena for the rs-matter-stack run futures (static RAM, not heap).
/// Tune up if init panics for lack of space. (light.rs uses 23500.)
const BUMP_SIZE: usize = 23500;

/// Endpoint 1 — On/Off Plug-In Unit (the RCD resetter). Endpoint 0 is the root.
const PLUG_ENDPOINT_ID: u16 = 1;

/// On/Off Plug-In Unit device type (Matter Device Library `0x010A`). Gives HomeKit a
/// tappable outlet tile (vs the misleading lightbulb of On/Off Light). See
/// matter_device_choices.md.
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

static MATTER_STACK: StaticCell<ThreadMatterStack<BUMP_SIZE>> = StaticCell::new();

/// Scratch buffer for rendering the commissioning QR code at startup. Held in a
/// static (not on the tight main-task stack) and used once before the stack runs.
static QR_BUF: StaticCell<[u8; 1024]> = StaticCell::new();

/// The Matter node (matter_device_choices.md):
///   EP0 — Root Node (system clusters)
///   EP1 — On/Off Plug-In Unit: the RCD resetter (tap → actuator reset cycle)
///   EP2 — Contact Sensor: downstream mains presence (Boolean State)
const NODE: Node = Node {
    endpoints: &[
        ThreadMatterStack::<0, ()>::root_endpoint(),
        Endpoint::new(
            PLUG_ENDPOINT_ID,
            devices!(DEV_TYPE_ON_OFF_PLUG_IN_UNIT),
            clusters!(DescHandler::CLUSTER, RcdPlugHooks::CLUSTER),
        ),
        Endpoint::new(
            CONTACT_ENDPOINT_ID,
            devices!(DEV_TYPE_CONTACT_SENSOR),
            clusters!(DescHandler::CLUSTER, CONTACT_CLUSTER),
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
pub async fn run_matter(
    ot: OpenThread<'static>,
    bt: esp_hal::peripherals::BT<'static>,
    eui64: [u8; 8],
    rng: esp_hal::rng::Rng,
) -> Result<(), Error> {
    // ── One-time init — MUST stay outside the restart loop below ─────────────────
    // A `StaticCell` can be initialized only ONCE (a second `init_with` panics), and the
    // Matter stack is a large fixed static that the supervised loop deliberately REUSES:
    // `stack.run()` re-initializes its own transport and resets its bump allocator on
    // every call, so a single instance is driven across restarts. Hence this init, the
    // device-model handlers, and the one-shot persistence load all live outside the loop.
    let stack = MATTER_STACK
        .uninit()
        .init_with(ThreadMatterStack::init(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT));

    let crypto = default_crypto(EspRng(rng), DAC_PRIVKEY);
    let mut rand = crypto.weak_rand()?;

    // EP1 — On/Off Plug-In Unit (RCD resetter) and EP2 — Contact Sensor (mains presence).
    // Built once so their cluster Dataver stays continuous across restarts; both bridge to
    // the autonomous controller via `crate::link` (On → run a reset cycle; state pushed back).
    let plug = on_off::OnOffHandler::new_standalone(
        Dataver::new_rand(&mut rand),
        PLUG_ENDPOINT_ID,
        RcdPlugHooks,
    );
    let contact = ContactHandler::new(Dataver::new_rand(&mut rand));

    // Flash-backed KV over the `nvs` partition (fabrics + Thread networks store), built
    // ONCE: `FlashKv::new()` claims a module `StaticCell` buffer, so calling it twice
    // panics. `startup` loads the persisted fabric into the (reused) stack — deciding the
    // mode — and the same store is then wrapped as the KV and shared with every run below
    // by reference (`&kv`; `impl KvBlobStoreAccess for &T`). On a restart the fabric is
    // already loaded, so this is not repeated — the device stays operational over Thread.
    let mut store = FlashKv::new()?;
    let commissioned_before = {
        stack.startup(&crypto, &mut store).await?;
        stack.matter().is_commissioned()
    };
    let kv = stack.matter().kv(store);

    if commissioned_before {
        // Already paired: the non-concurrent run() skips BLE and operates over Thread.
        // Do NOT print the QR / commissioning credentials — nothing is listening on BLE.
        log::info!("[matter] ===== MODE: OPERATIONAL (saved pairing loaded) =====");
        log::info!("[matter] Operating over Thread; BLE is OFF. Hold the reset button 3s to re-pair.");
    } else {
        // Not paired: BLE commissioning window is open. Print the pairing info + QR.
        // (We build with rs-matter's `log` feature OFF — its QR/PairingCode print pairs
        // with a stack-overflowing attribute-error Debug format — so we print it ourselves.
        // These are constant because we use the rs-matter TEST device credentials.)
        log::info!("[matter] ===== MODE: COMMISSIONING (no saved pairing) =====");
        log::info!("[matter] Commissioning open over BLE — TEST credentials:");
        log::info!("[matter]   discriminator : 3840");
        log::info!("[matter]   passcode      : 20202021");
        log::info!("[matter]   manual code   : 3497-011-2332");
        log::info!("[matter]   chip-tool: pairing ble-thread <node-id> hex:<dataset> 20202021 3840");
        print_commissioning_qr();
    }

    // BLE address: static random, derived from the EUI-64. A static random address
    // (Core spec Vol 6, Part B §1.3.2.1) MUST have its two most-significant bits = 0b11.
    // BdAddr is little-endian on the HCI wire, so the MSB is byte 5 — set 0xC0 there,
    // NOT byte 0 (the LSB), or the controller rejects advertising with HCI error 0x12.
    let mut addr: [u8; 6] = eui64[2..8].try_into().unwrap_or([0xff; 6]);
    addr[5] |= 0xc0; // two MSBs = 0b11 → static random

    // ── Supervised run loop ──────────────────────────────────────────────────────
    // `stack.run()` should not return during normal operation, but if it ever exits — Ok,
    // or Err from an unforeseen internal/transport failure — restart it rather than leave
    // Matter dead until the next reboot. (A border-router outage alone does NOT trigger
    // this: its report/reconnect failures are handled internally.) The controller's safety
    // loop runs in a separate task and is unaffected. Only the values `run()` consumes by
    // value are rebuilt per iteration.
    let mut bt = Some(bt);
    loop {
        // Fresh BLE controller each run: `stack.run()` owns and drops the wireless bundle
        // (and thus the controller) when its future ends — `BleConnector`'s Drop de-inits
        // BLE. Use the real peripheral the first time; on a restart the previous connector
        // has already been dropped, so re-acquiring `BT` via `steal()` cannot alias a live
        // one. (In operational mode BLE is never brought up, but the type still needs a
        // `GattPeripheral`.)
        let device = bt
            .take()
            .unwrap_or_else(|| unsafe { esp_hal::peripherals::BT::steal() });

        // Rebuild what `run()` takes by value and can't be shared: the BLE controller and
        // the handler chain. (The KV is build-once and shared by reference; `stack`,
        // `crypto`, `plug`/`contact` live outside the loop.) Wrapped in an async block so a
        // setup failure is caught by the loop below rather than escaping `run_matter`.
        let outcome: Result<(), Error> = async {
            let controller: ExternalController<_, 1> = ExternalController::new(
                BleConnector::new(device, Default::default())
                    .map_err(|_| Error::from(ErrorCode::NoNetworkInterface))?,
            );

            let mut rand = crypto.weak_rand()?;
            let handler = EmptyHandler
                .chain(
                    EpClMatcher::new(Some(PLUG_ENDPOINT_ID), Some(RcdPlugHooks::CLUSTER.id)),
                    on_off::HandlerAsyncAdaptor(&plug),
                )
                .chain(
                    EpClMatcher::new(Some(PLUG_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
                    Async(DescHandler::new(Dataver::new_rand(&mut rand)).adapt()),
                )
                .chain(
                    EpClMatcher::new(Some(CONTACT_ENDPOINT_ID), Some(CONTACT_CLUSTER.id)),
                    Async(&contact),
                )
                .chain(
                    EpClMatcher::new(Some(CONTACT_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
                    Async(DescHandler::new(Dataver::new_rand(&mut rand)).adapt()),
                );

            // Non-concurrent (BLE-only until paired, then Thread-only) — NOT run_coex.
            // The H2's single 2.4 GHz radio can't run BLE + 802.15.4 reliably at once, so
            // `run` advertises SupportsConcurrentConnection=false and drops BLE once a
            // fabric exists; the deferred ConnectNetwork is replayed via OtNetCtl::connect.
            // `&kv` (not `kv`): the KV is owned outside the loop (FlashKv is build-once) and
            // shared by reference each run.
            stack
                .run(
                    PreexistingWireless::new(
                        OtNetStack::new(ot.clone()),
                        OtNetif::new(ot.clone(), eui64),
                        OtNetCtl::new(ot.clone()),
                        OtMdns::new(ot.clone(), eui64),
                        OtGattPeripheral::new(controller, addr),
                    ),
                    &crypto,
                    (NODE, handler),
                    &kv,
                    (),
                )
                .await
        }
        .await;

        match outcome {
            Ok(()) => log::warn!("[matter] stack.run returned unexpectedly — restarting in 2 s"),
            Err(e) => log::warn!("[matter] Matter stack error ({e:?}) — restarting in 2 s"),
        }
        // Brief pause so a hard-failing restart can't spin the CPU; the OpenThread task
        // keeps the radio serviced in the meantime.
        Timer::after(Duration::from_secs(2)).await;
    }
}

/// Render the commissioning QR code (ASCII) plus its text payload to the console.
///
/// rs-matter's own `print_standard_qr_code` renders with its internal `info!`, which is
/// a no-op unless rs-matter's `log` feature is enabled — and we keep that OFF (it pairs
/// with a deep attribute-error Debug format that overflows the main task stack). So we
/// build the same standard QR payload from the TEST device credentials and render it with
/// our own logger. Uses `QrTextType::Ascii` (space / `#` blocks) for the widest console
/// font compatibility, and also prints the `MT:…` payload string to paste into a QR-code
/// generator app if the ASCII art doesn't scan in a given terminal.
fn print_commissioning_qr() {
    let buf = QR_BUF.init([0u8; 1024]);

    // Standard commissioning flow, BLE discovery — mirrors `Matter::standard_qr_payload`.
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

    // Text payload — paste into any QR-code generator app if the ASCII QR below doesn't
    // render / scan in your console font.
    log::info!("[matter]   QR payload (paste into a QR-code generator app): {text}");

    // `as_str` returned the remaining buffer; split it for the QR encoder's scratch
    // (`tmp_buf`) and output matrix (`out_buf`). `tmp_buf` is reused per-line for rendering.
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

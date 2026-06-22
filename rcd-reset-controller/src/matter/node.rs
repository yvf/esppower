//! Matter node initialisation and async run task.
//!
//! rs-matter 0.1.x changed the runtime API significantly from the planned 0.2 design:
//!  - Matter::new takes (dev_det, dev_comm, dev_att, port) — no passcode/discriminator/caps
//!  - BasicCommData holds a Spake2pVerifierPassword (not a raw u32)
//!  - matter.run takes (crypto, send, recv, multicast) network-layer objects
//!  - EspThread::init_default() → EspThread::new(modem, sysloop, nvs, event_fs)
//!  - KvBlobStore keys are u16, not &str
//!
//! These parts are stubbed with TODO comments until the full rs-matter 0.1.x
//! integration is written.

use crate::config::{
    MATTER_DISCRIMINATOR, MATTER_PASSCODE, MATTER_PRODUCT_ID, MATTER_PRODUCT_NAME,
    MATTER_SW_VERSION, MATTER_SW_VERSION_STR, MATTER_VENDOR_ID, MATTER_VENDOR_NAME,
};
use crate::matter::{
    endpoints::{make_node, EndpointState, RcdHandler},
    ToController, ToMatter,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::{Receiver, Sender}};
use log::info;
use rs_matter::dm::clusters::basic_info::BasicInfoConfig;
use std::sync::Mutex;

/// Runs the Matter stack and bridges the `ToMatter` channel into attribute updates.
pub async fn matter_task(
    matter_rx: Receiver<'static, CriticalSectionRawMutex, ToMatter, 4>,
    ctrl_tx: Sender<'static, CriticalSectionRawMutex, ToController, 4>,
    _nvs: esp_idf_svc::nvs::EspNvs<esp_idf_svc::nvs::NvsDefault>,
) -> ! {
    info!("Matter: initialising node (stub — full integration pending rs-matter 0.1.x rewrite)");

    let _basic_info = BasicInfoConfig {
        vid: MATTER_VENDOR_ID,
        pid: MATTER_PRODUCT_ID,
        hw_ver: 1,
        hw_ver_str: "1",
        sw_ver: MATTER_SW_VERSION,
        sw_ver_str: MATTER_SW_VERSION_STR,
        serial_no: "RCD-001",
        product_name: MATTER_PRODUCT_NAME,
        vendor_name: MATTER_VENDOR_NAME,
        device_name: MATTER_PRODUCT_NAME,
        ..BasicInfoConfig::new()
    };

    // Log commissioning info.
    crate::matter::commissioning::log_qr_code();
    info!(
        "Matter: discriminator={} passcode={} discovery=IP(Thread via border router)",
        MATTER_DISCRIMINATOR, MATTER_PASSCODE,
    );

    // Shared endpoint attribute state.
    static STATE: Mutex<EndpointState> = Mutex::new(EndpointState::new());

    let _node = make_node();
    let _handler = RcdHandler::new(&STATE, ctrl_tx);

    // TODO: construct Matter, EspThread, and call matter.run() with the rs-matter 0.1.x API:
    //
    //   let dev_comm = rs_matter::BasicCommData {
    //       password: <derive Spake2pVerifierPassword from MATTER_PASSCODE>,
    //       discriminator: MATTER_DISCRIMINATOR,
    //   };
    //   let dev_att = rs_matter::dm::devices::test::TestDeviceAttestation;
    //   let matter = rs_matter::Matter::new(&basic_info, dev_comm, &dev_att, 5540);
    //
    //   // Thread transport via ESP-IDF OpenThread:
    //   let thread = esp_idf_svc::thread::EspThread::new(modem, sysloop, nvs_partition, event_fs)?;
    //
    //   // Run loop:
    //   embassy_futures::select::select(
    //       matter.run(crypto, send, recv, multicast),
    //       apply_updates_loop(&matter_rx, &STATE),
    //   ).await;

    info!("Matter: task running (no-op stub — attribute updates only)");

    // Bridge attribute updates from the controller even while Matter isn't fully wired.
    apply_updates_loop(&matter_rx, &STATE).await
}

async fn apply_updates_loop(
    rx: &Receiver<'static, CriticalSectionRawMutex, ToMatter, 4>,
    state: &'static Mutex<EndpointState>,
) -> ! {
    loop {
        let msg = rx.receive().await;
        match msg {
            ToMatter::SetPlugOnOff(on) => {
                info!("Matter: set endpoint 1 OnOff = {on}");
                let mut s = state.lock().unwrap();
                s.plug_on_off = on;
                s.plug_dataver.changed();
            }
            ToMatter::SetContactClosed(closed) => {
                info!("Matter: set endpoint 2 StateValue = {closed}");
                let mut s = state.lock().unwrap();
                s.contact_closed = closed;
                s.sensor_dataver.changed();
            }
        }
    }
}

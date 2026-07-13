//! EP1 - On/Off Plug-In Unit (`0x010A`) device logic.
//!
//! Implements rs-matter's `OnOffHooks` for the RCD resetter plug. Per the device design
//! (docs/matter_device_choices.md) the plug is *momentary*: an `On` command fires one actuator
//! reset cycle and the plug returns to `Off` automatically when the cycle completes; `Off`
//! commands are ignored (the device is always effectively off at rest). The actual
//! actuation is owned by the autonomous [`Controller`](crate::controller); this hook only
//! requests a cycle and mirrors the controller's state to HomeKit via [`crate::link`].

use rs_matter_stack::matter::dm::clusters::app::on_off::{
    EffectVariantEnum, OnOffHooks, OutOfBandMessage, StartUpOnOffEnum,
};
use rs_matter_stack::matter::dm::clusters::decl::on_off as on_off_cluster;
use rs_matter_stack::matter::dm::Cluster;
use rs_matter_stack::matter::error::Error;
use rs_matter_stack::matter::tlv::Nullable;
use rs_matter_stack::matter::with;

use crate::link;

/// On/Off device logic for the RCD resetter plug.
pub struct RcdPlugHooks;

impl OnOffHooks for RcdPlugHooks {
    // On/Off Plug-In Unit only needs the base On/Off cluster: the `OnOff` attribute plus
    // the On / Off / Toggle commands. No Lighting (`LT`) feature - that gates the
    // StartUpOnOff / OnTime / OffWaitTime lighting attributes and the effect commands,
    // none of which a momentary reset plug uses.
    const CLUSTER: Cluster<'static> = on_off_cluster::FULL_CLUSTER
        .with_revision(6)
        .with_features(0)
        .with_attrs(with!(required; on_off_cluster::AttributeId::OnOff))
        .with_cmds(with!(
            on_off_cluster::CommandId::Off
                | on_off_cluster::CommandId::On
                | on_off_cluster::CommandId::Toggle
        ));

    fn on_off(&self) -> bool {
        // Reflects the controller: On while a reset cycle is actuating, Off at rest.
        link::plug_active()
    }

    fn set_on_off(&self, on: bool) {
        // `On` -> request one actuator reset cycle. The controller owns the actuator and
        // sets the plug state On for the duration of the cycle, then back to Off, which is
        // pushed to HomeKit by `run` below. `Off` is ignored - the plug is momentary and
        // always returns to Off on its own.
        // Logged so the console shows, unambiguously, that an operational (Thread/CASE)
        // command from HomeKit reached the device - distinguishing "Home can't reach us"
        // from "the controller didn't act on the request".
        log::info!("[matter] EP1 On/Off command from HomeKit: {}", if on { "On" } else { "Off" });
        if on {
            link::request_manual_cycle();
        }
    }

    fn start_up_on_off(&self) -> Nullable<StartUpOnOffEnum> {
        // No StartUpOnOff attribute is advertised (no `LT` feature); always null.
        Nullable::none()
    }

    fn set_start_up_on_off(&self, _value: Nullable<StartUpOnOffEnum>) -> Result<(), Error> {
        Ok(())
    }

    async fn handle_off_with_effect(&self, _effect: EffectVariantEnum) {
        // No OffWithEffect command advertised.
    }

    async fn run<F: Fn(OutOfBandMessage)>(&self, notify: F) {
        // Push On/Off changes (a reset cycle starting/finishing - manual OR automatic) to
        // HomeKit so the tile tracks the actuator. `OutOfBandMessage::Update` makes the
        // OnOff handler re-read `on_off()` and report it to subscribers.
        loop {
            link::plug_changed().await;
            notify(OutOfBandMessage::Update);
        }
    }
}

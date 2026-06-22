//! Matter data-model cluster handlers for the two functional endpoints.
//!
//! rs-matter 0.1.x restructured the Handler trait and Node/Endpoint types.
//! The Handler read/invoke bodies are stubbed pending a full rewrite against
//! the new ReadContext/ReadReply/InvokeContext/InvokeReply API.

use crate::config::{DEV_TYPE_CONTACT_SENSOR, DEV_TYPE_ON_OFF_PLUGIN_UNIT};
use crate::matter::ToController;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Sender};
use log::warn;
use rs_matter::dm::{
    Attribute, Cluster, Command, Dataver, DeviceType, Endpoint, Event, Handler, InvokeContext,
    InvokeReply, MatchContext, Node, ReadContext, ReadReply, WithAttrs, WithCmds, WithEvents,
};
use rs_matter::error::{Error, ErrorCode};

// ─── Cluster metadata stubs ───────────────────────────────────────────────────

fn no_attr(_: &Attribute, _: u16, _: u32) -> bool { true }
fn no_cmd(_: &Command, _: u16, _: u32) -> bool { true }
fn no_evt(_: &Event, _: u16, _: u32) -> bool { true }

const NO_ATTRS: WithAttrs = no_attr;
const NO_CMDS: WithCmds = no_cmd;
const NO_EVTS: WithEvents = no_evt;

const fn stub_cluster(id: u32, revision: u16) -> Cluster<'static> {
    Cluster {
        id,
        revision,
        feature_map: 0,
        attributes: &[],
        commands: &[],
        events: &[],
        with_attrs: NO_ATTRS,
        with_cmds: NO_CMDS,
        with_events: NO_EVTS,
    }
}

const CLUSTER_ON_OFF: Cluster<'static>       = stub_cluster(0x0006, 4);
const CLUSTER_BOOL_STATE: Cluster<'static>   = stub_cluster(0x0045, 1);
const CLUSTER_DESCRIPTOR: Cluster<'static>   = stub_cluster(0x001D, 2);
const CLUSTER_IDENTIFY: Cluster<'static>     = stub_cluster(0x0003, 4);

const DEV_TYPE_PLUG: DeviceType    = DeviceType { dtype: DEV_TYPE_ON_OFF_PLUGIN_UNIT as u16, drev: 2 };
const DEV_TYPE_CONTACT: DeviceType = DeviceType { dtype: DEV_TYPE_CONTACT_SENSOR as u16, drev: 1 };
const DEV_TYPE_ROOT: DeviceType    = DeviceType { dtype: 0x0016u16, drev: 1 };

// ─── Shared attribute state ───────────────────────────────────────────────────

pub struct EndpointState {
    pub plug_on_off: bool,
    pub contact_closed: bool,
    pub plug_dataver: Dataver,
    pub sensor_dataver: Dataver,
}

impl EndpointState {
    pub const fn new() -> Self {
        Self {
            plug_on_off: false,
            contact_closed: true,
            plug_dataver: Dataver::new(0),
            sensor_dataver: Dataver::new(1),
        }
    }
}

// ─── Combined cluster handler ─────────────────────────────────────────────────

pub struct RcdHandler<'a> {
    state: &'a std::sync::Mutex<EndpointState>,
    ctrl_tx: Sender<'a, CriticalSectionRawMutex, ToController, 4>,
}

impl<'a> RcdHandler<'a> {
    pub fn new(
        state: &'a std::sync::Mutex<EndpointState>,
        ctrl_tx: Sender<'a, CriticalSectionRawMutex, ToController, 4>,
    ) -> Self {
        Self { state, ctrl_tx }
    }
}

// TODO: rewrite read/invoke against rs-matter 0.1.x ReadContext / ReadReply API.
impl<'a> Handler for RcdHandler<'a> {
    fn read(&self, _ctx: impl ReadContext, _reply: impl ReadReply) -> Result<(), Error> {
        Err(ErrorCode::AttributeNotFound.into())
    }

    fn invoke(&self, _ctx: impl InvokeContext, _reply: impl InvokeReply) -> Result<(), Error> {
        let _ = &self.ctrl_tx;
        let _ = self.state.lock().unwrap().plug_on_off;
        warn!("Matter: invoke not yet implemented");
        Err(ErrorCode::CommandNotFound.into())
    }

    fn bump_dataver(&self, _ctx: impl MatchContext) {}
}

// ─── Node descriptor (static data model) ─────────────────────────────────────

static ENDPOINTS: [Endpoint<'static>; 3] = [
    Endpoint::new(0, &[DEV_TYPE_ROOT],    &[CLUSTER_DESCRIPTOR, CLUSTER_IDENTIFY]),
    Endpoint::new(1, &[DEV_TYPE_PLUG],    &[CLUSTER_DESCRIPTOR, CLUSTER_IDENTIFY, CLUSTER_ON_OFF]),
    Endpoint::new(2, &[DEV_TYPE_CONTACT], &[CLUSTER_DESCRIPTOR, CLUSTER_IDENTIFY, CLUSTER_BOOL_STATE]),
];

pub fn make_node() -> Node<'static> {
    Node::new(&ENDPOINTS)
}

// ─── Unit tests ───────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_off_cluster_id_matches_matter_spec() {
        assert_eq!(CLUSTER_ON_OFF.id, 0x0006);
    }

    #[test]
    fn boolean_state_cluster_id_matches_matter_spec() {
        assert_eq!(CLUSTER_BOOL_STATE.id, 0x0045);
    }

    #[test]
    fn endpoint_state_initial_values() {
        let state = EndpointState::new();
        assert!(!state.plug_on_off, "plug starts Off");
        assert!(state.contact_closed, "sensor starts Closed");
    }

    #[test]
    fn device_type_ids_match_matter_spec() {
        assert_eq!(DEV_TYPE_ON_OFF_PLUGIN_UNIT, 0x010A);
        assert_eq!(DEV_TYPE_CONTACT_SENSOR, 0x0015);
    }
}

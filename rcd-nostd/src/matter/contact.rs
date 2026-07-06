//! EP2 — Contact Sensor (`0x0015`) with the Boolean State cluster (`0x0045`).
//!
//! rs-matter generates the Boolean State cluster *metadata* but ships no handler, so this
//! module provides one. `StateValue = true` → power present → HomeKit "Closed";
//! `StateValue = false` → power absent (RCD tripped) → HomeKit "Open" (+ notification).
//! The value comes from the autonomous [`Controller`](crate::controller) via [`crate::link`].
//!
//! Read is delegated to the generated `boolean_state::HandlerAdaptor`; we override the
//! background `run` to push a subscription report the instant the sensed state changes, so
//! HomeKit raises its trip alert without waiting for a poll.

use rs_matter_stack::matter::dm::clusters::decl::boolean_state;
use rs_matter_stack::matter::dm::{
    Cluster, Dataver, Handler, HandlerContext, MatchContext, NonBlockingHandler, ReadContext,
    ReadReply,
};
use rs_matter_stack::matter::error::Error;
use rs_matter_stack::matter::with;

use crate::link;

/// EP2 endpoint id — must match the endpoint number used in the Node (`stack.rs`).
pub const CONTACT_ENDPOINT_ID: u16 = 2;

/// The Boolean State cluster metadata for this handler (used in the Node's cluster list).
pub const CONTACT_CLUSTER: Cluster<'static> =
    <ContactInner as boolean_state::ClusterHandler>::CLUSTER;

/// Inner cluster handler: answers `StateValue` reads and owns the cluster `Dataver`.
struct ContactInner {
    dataver: Dataver,
}

impl boolean_state::ClusterHandler for ContactInner {
    const CLUSTER: Cluster<'static> = boolean_state::FULL_CLUSTER
        .with_attrs(with!(required))
        .with_cmds(with!());

    fn dataver(&self) -> u32 {
        self.dataver.get()
    }

    fn dataver_changed(&self) {
        self.dataver.changed();
    }

    fn state_value(&self, _ctx: impl ReadContext) -> Result<bool, Error> {
        // StateValue: true = contact closed = mains present.
        Ok(link::power_present())
    }
}

/// Contact-sensor handler: delegates attribute reads to the generated Boolean State
/// adaptor and pushes subscription reports when the sensed state changes.
pub struct ContactHandler {
    inner: ContactInner,
}

impl ContactHandler {
    pub fn new(dataver: Dataver) -> Self {
        Self {
            inner: ContactInner { dataver },
        }
    }
}

impl Handler for ContactHandler {
    fn read(&self, ctx: impl ReadContext, reply: impl ReadReply) -> Result<(), Error> {
        boolean_state::HandlerAdaptor(&self.inner).read(ctx, reply)
    }

    fn bump_dataver(&self, ctx: impl MatchContext) {
        boolean_state::HandlerAdaptor(&self.inner).bump_dataver(ctx)
    }

    async fn run(&self, ctx: impl HandlerContext) -> Result<(), Error> {
        // Push StateValue changes so HomeKit updates the contact tile immediately and
        // raises its "Open" (RCD tripped) notification without waiting for a poll.
        // `notify_cluster_changed` bumps the cluster Dataver and notifies subscribers.
        loop {
            link::power_changed().await;
            ctx.notify_cluster_changed(CONTACT_ENDPOINT_ID, CONTACT_CLUSTER.id);
        }
    }
}

// Attribute reads are synchronous (no awaiting), so the handler can be adapted into the
// async handler chain via `Async`.
impl NonBlockingHandler for ContactHandler {}

//! `NetStack` adapter: rs-matter-stack's UDP/TCP/DNS network stack over openthread.
//!
//! Matter-over-Thread is UDP-only. rs-matter-stack's `NetStack` requires edge-nal
//! `UdpBind`/`UdpConnect` factories; the openthread<->edge-nal glue lives in the
//! upstream `edge-nal-openthread` crate (`OtUdpStack`), so this is just the thin
//! `NetStack` wrapper that hands out an `OtUdpStack` for UDP and `NoopNet` for the
//! unsupported TCP/DNS slots.

use edge_nal_openthread::OtUdpStack;
use openthread::OpenThread;

use rs_matter_stack::nal::{NetStack, NoopNet};

/// `NetStack` over an OpenThread instance.
pub struct OtNetStack<'a> {
    ot: OpenThread<'a>,
}

impl<'a> OtNetStack<'a> {
    pub const fn new(ot: OpenThread<'a>) -> Self {
        Self { ot }
    }
}

impl<'a> NetStack for OtNetStack<'a> {
    type UdpBind<'t>
        = OtUdpStack<'a>
    where
        Self: 't;
    type UdpConnect<'t>
        = OtUdpStack<'a>
    where
        Self: 't;
    type TcpBind<'t>
        = NoopNet
    where
        Self: 't;
    type TcpConnect<'t>
        = NoopNet
    where
        Self: 't;
    type Dns<'t>
        = NoopNet
    where
        Self: 't;

    fn udp_bind(&self) -> Option<Self::UdpBind<'_>> {
        Some(OtUdpStack::new(self.ot.clone()))
    }

    fn udp_connect(&self) -> Option<Self::UdpConnect<'_>> {
        Some(OtUdpStack::new(self.ot.clone()))
    }

    // Matter-over-Thread uses neither TCP nor a DNS client on the device side.
    fn tcp_bind(&self) -> Option<Self::TcpBind<'_>> {
        None
    }

    fn tcp_connect(&self) -> Option<Self::TcpConnect<'_>> {
        None
    }

    fn dns(&self) -> Option<Self::Dns<'_>> {
        None
    }
}

//! `NetStack` adapter: rs-matter-stack's UDP/TCP/DNS network stack over openthread.
//!
//! Matter-over-Thread is UDP-only. `OpenThread` already implements edge-nal
//! `UdpBind`/`UdpConnect` (openthread/src/enal.rs), so UDP maps directly; TCP and
//! DNS are unsupported here and use rs-matter-stack's `NoopNet` placeholder.

use openthread::OpenThread;
use rs_matter_stack::nal::noop::NoopNet;
use rs_matter_stack::nal::NetStack;

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
        = OpenThread<'a>
    where
        Self: 't;
    type UdpConnect<'t>
        = OpenThread<'a>
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
        Some(self.ot.clone())
    }

    fn udp_connect(&self) -> Option<Self::UdpConnect<'_>> {
        Some(self.ot.clone())
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

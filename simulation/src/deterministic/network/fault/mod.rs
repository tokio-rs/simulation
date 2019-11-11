use super::socket;
use super::Inner;
use std::net;
mod latency;
mod swizzle;
pub use latency::{LatencyFaultInjector, LatencyFaultInjectorConfig};
pub(crate) use swizzle::CloggedConnection;

const SWIZZLE_START_PROBABILITY: f64 = 0.01;
const SWIZZLE_SELECTION_PROBABILITY: f64 = 0.30;

#[derive(Debug, Clone)]
pub(crate) struct Connection {
    source: net::SocketAddr,
    dest: net::SocketAddr,
    client_fault_handle: socket::FaultyTcpStreamHandle,
    server_fault_handle: socket::FaultyTcpStreamHandle,
}

impl Connection {
    pub(crate) fn new(
        source: net::SocketAddr,
        dest: net::SocketAddr,
        client_fault_handle: socket::FaultyTcpStreamHandle,
        server_fault_handle: socket::FaultyTcpStreamHandle,
    ) -> Self {
        Self {
            source,
            dest,
            client_fault_handle,
            server_fault_handle,
        }
    }

    pub(crate) fn source(&self) -> net::SocketAddr {
        self.source
    }

    pub(crate) fn dest(&self) -> net::SocketAddr {
        self.dest
    }

    pub(crate) fn is_dropped(&self) -> bool {
        self.client_fault_handle.is_dropped() || self.server_fault_handle.is_dropped()
    }

    pub(crate) fn is_clogged(&self) -> bool {
        self.client_fault_handle.is_fully_clogged() && self.server_fault_handle.is_fully_clogged()
    }

    pub(crate) fn clog(&mut self) {
        self.client_fault_handle.clog_sends();
        self.client_fault_handle.clog_receives();
        self.server_fault_handle.clog_sends();
        self.server_fault_handle.clog_receives();
    }

    pub(crate) fn unclog(&mut self) {
        self.client_fault_handle.unclog_sends();
        self.client_fault_handle.unclog_receives();
        self.server_fault_handle.unclog_sends();
        self.server_fault_handle.unclog_receives();
    }
}

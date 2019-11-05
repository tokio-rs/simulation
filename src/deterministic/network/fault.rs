use super::socket;
use super::Inner;
use std::{cmp, hash, net, sync};
enum Method {
    SwizzleClog,
    RandomDisconnect,
}

struct NetworkFaultHandle {
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl NetworkFaultHandle {
    fn new(inner: sync::Arc<sync::Mutex<Inner>>) -> Self {
        Self { inner }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Connection {
    source: net::SocketAddr,
    dest: net::SocketAddr,
    client_fault_handle: socket::FaultyTcpStreamHandle,
    server_fault_handle: socket::FaultyTcpStreamHandle,
}

impl cmp::PartialEq for Connection {
    fn eq(&self, other: &Connection) -> bool {
        self.source.eq(&other.source) && self.dest.eq(&other.dest)
    }
}

impl cmp::Eq for Connection {}

impl hash::Hash for Connection {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
        self.dest.hash(state);
    }
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
}

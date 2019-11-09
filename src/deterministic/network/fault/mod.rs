use super::socket;
use super::Inner;
use crate::deterministic::{random::DeterministicRandomHandle, time::DeterministicTimeHandle};
use futures::{Future, FutureExt, Poll, Stream};
use std::{net, ops, pin::Pin, sync, task::Context, time::Duration};
mod swizzle;

const SWIZZLE_START_PROBABILITY: f64 = 0.01;
const SWIZZLE_SELECTION_PROBABILITY: f64 = 0.30;

/// SwizzleClogInjector is a network fault injector which follows the "swizzle clog" approach
/// popularized by FoundationDB.
///
/// The general algorithm consists of a series of dice rolls followed by actions which introduce
/// gradual ordered faults in a networked application. Connections are "clogged" between endpoints,
/// causing infinite delays for existing and new connections.
///
/// 1. Roll dice to decide if network swizzle should happen.
/// 2. Pick a random subset of connections and order them.
/// 3. Wait a random amount of time before setting the first connection as clogged.
/// 4. Repeat this process until all connections have been clogged.
/// 5. Once all connections have been clogged, unclog each connection in reverse order.
pub(crate) struct SwizzleClogInjector {
    started: bool,

    inner: sync::Arc<sync::Mutex<Inner>>,
    random: DeterministicRandomHandle,
    time: DeterministicTimeHandle,
    position: usize,
    to_swizzle: Vec<CloggedConnection>,
}

impl SwizzleClogInjector {
    pub(crate) fn new(
        inner: sync::Arc<sync::Mutex<Inner>>,
        random: DeterministicRandomHandle,
        time: DeterministicTimeHandle,
    ) -> Self {
        Self {
            started: false,
            inner,
            random,
            time,
            position: 0,
            to_swizzle: vec![],
        }
    }
}

impl SwizzleClogInjector {
    fn fault_started(&mut self) -> bool {
        if self.started {
            return true;
        }

        if self.random.should_fault(SWIZZLE_START_PROBABILITY) {
            self.started = true;
            return true;
        }
        false
    }

    fn ensure_swizzle_collection(&mut self) {
        if self.to_swizzle.is_empty() && self.started {
            let lock = self.inner.lock().unwrap();
            let mut to_swizzle = vec![];
            for connection in lock.connections.iter() {
                // pick connections to swizzle with a 30% chance
                if self.random.should_fault(0.30) {
                    let clog =
                        CloggedConnection::new(connection.source().ip(), connection.dest().ip());
                    to_swizzle.push(clog);
                }
            }
            self.position = to_swizzle.len();
            self.to_swizzle = to_swizzle;
        }
    }
}

impl Future for SwizzleClogInjector {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            // 1. Check if we can start the swizzle
            if !self.fault_started() {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            // 2. Pick a random subset of connections to swizzle.
            self.ensure_swizzle_collection();

            // 3. Take an element from the subset of connections and swizzle it
        }
    }
}

#[derive(Debug, Clone, PartialEq, Hash, Eq, Copy)]
pub(crate) struct CloggedConnection {
    source: net::IpAddr,
    dest: net::IpAddr,
}

impl CloggedConnection {
    pub(crate) fn source(&self) -> net::IpAddr {
        self.source
    }

    pub(crate) fn dest(&self) -> net::IpAddr {
        self.dest
    }

    pub(crate) fn new(source: net::IpAddr, dest: net::IpAddr) -> Self {
        Self { source, dest }
    }
}

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

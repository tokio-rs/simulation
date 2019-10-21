mod listen;
mod pipe;
mod socket;
mod stream;

use futures::{channel::mpsc, SinkExt};
pub(crate) use listen::MemoryListener;
pub(crate) use pipe::Pipe;
pub(crate) use socket::{new_pair, ClientSocket, ServerSocket};
use std::{collections::HashMap, io, net, num, ops, sync::Arc, time};
pub(crate) use stream::MemoryTcpStream;
use try_lock::TryLock;

/// Connections contains a mapping of bound ports and their corresponding acceptor channels.
///
/// Ports can be bound by registering a new acceptor channel, and acceptor channels can be
/// retrieved by port number to make new in-memory connections.
#[derive(Debug)]
pub(crate) struct Connections {
    /// listeners contains a mapping from port number to a channel where new
    /// sockets can be registered.
    listeners: HashMap<num::NonZeroU16, mpsc::Sender<ServerSocket>>,

    /// next_port is the next port which can be allocated.
    next_port: u16,
}

impl Connections {
    pub(crate) fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            next_port: 1,
        }
    }

    /// Check if the provided port is in use or not. If `port` is 0, assign a new
    /// port.
    fn free_port(&mut self, port: u16) -> Result<num::NonZeroU16, io::Error> {
        if let Some(port) = num::NonZeroU16::new(port) {
            if self.listeners.contains_key(&port) {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            return Ok(port);
        } else {
            // pick next available port
            loop {
                if let Some(port) = num::NonZeroU16::new(self.next_port) {
                    self.next_port += 1;
                    if !self.listeners.contains_key(&port) {
                        return Ok(port);
                    }
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        String::from("could not find a port to bind to"),
                    ));
                }
            }
        }
    }

    /// Returns the listener channel associated with the provided port (if there is one), otherwise
    /// returns an error.
    pub(crate) fn listener_channel(
        &mut self,
        port: num::NonZeroU16,
    ) -> Result<mpsc::Sender<ServerSocket>, io::Error> {
        self.listeners
            .get(&port)
            .map(Clone::clone)
            .ok_or(io::ErrorKind::AddrNotAvailable.into())
    }

    /// Registers a new listener channel for the specified port. If the specified port is 0, a random port will
    /// be selected and returned.
    pub(crate) fn register_listener_channel(
        &mut self,
        port: u16,
        chan: mpsc::Sender<ServerSocket>,
    ) -> Result<num::NonZeroU16, io::Error> {
        let port = self.free_port(port)?;
        self.listeners.insert(port, chan);
        Ok(port)
    }
}

/// Configuration for various fauilts which can be injected into the mock network.
#[derive(Debug, Clone)]
pub struct NetworkFaults {
    /// Toggle which allows enabling or disabling deterministic delay injection.
    pub enable_delays: bool,
    /// The range of duration for which a delay for a new connection can be injected.
    pub listener_connection_delay: ops::Range<time::Duration>,
    /// The probability of a new connection delay being injected, 0..1.
    pub listener_connection_delay_prob: f32,
    /// The range of duration for which a server socket read delay can be injected.
    pub server_socket_read_delay: ops::Range<time::Duration>,
    /// The probability of a server socket read delay being injected, 0..1.
    pub server_socket_read_delay_prob: f32,
    /// The range of duration for which a server socket write dealy can be injected.
    pub server_socket_write_delay: ops::Range<time::Duration>,
    /// The probability of a server socket write delay being injected, 0..1.
    pub server_socket_write_delay_prob: f32,

    env: crate::DeterministicRuntimeSchedulerRng,
}

impl NetworkFaults {
    fn new_default(env: crate::DeterministicRuntimeSchedulerRng) -> Self {
        Self {
            enable_delays: true,
            listener_connection_delay: time::Duration::from_millis(0)
                ..time::Duration::from_millis(10000),
            listener_connection_delay_prob: 0.10,
            server_socket_read_delay: time::Duration::from_millis(0)
                ..time::Duration::from_millis(5000),
            server_socket_read_delay_prob: 0.10,
            server_socket_write_delay: time::Duration::from_millis(0)
                ..time::Duration::from_millis(5000),
            server_socket_write_delay_prob: 0.10,
            env,
        }
    }

    pub(crate) fn listener_connection_delay(&self) -> Option<tokio::timer::Delay> {
        if self.enable_delays {
            self.env.maybe_random_delay(
                self.listener_connection_delay_prob,
                self.listener_connection_delay.start,
                self.listener_connection_delay.end,
            )
        } else {
            None
        }
    }

    pub(crate) fn socket_read_delay(&self) -> Option<tokio::timer::Delay> {
        if self.enable_delays {
            self.env.maybe_random_delay(
                self.server_socket_read_delay_prob,
                self.server_socket_read_delay.start,
                self.server_socket_read_delay.end,
            )
        } else {
            None
        }
    }
    pub(crate) fn socket_write_delay(&self) -> Option<tokio::timer::Delay> {
        if self.enable_delays {
            self.env.maybe_random_delay(
                self.server_socket_write_delay_prob,
                self.server_socket_write_delay.start,
                self.server_socket_write_delay.end,
            )
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MemoryNetwork {
    inner: Arc<TryLock<Connections>>,
    network_faults: NetworkFaults,
}

impl MemoryNetwork {
    pub(crate) fn new(env: crate::DeterministicRuntimeSchedulerRng) -> Self {
        let connections = Connections::new();
        Self {
            inner: Arc::new(TryLock::new(connections)),
            network_faults: NetworkFaults::new_default(env),
        }
    }

    pub(crate) fn network_faults(&self) -> NetworkFaults {
        self.network_faults.clone()
    }

    /// Opens an in-memory connection based on the port portion of the supplied SocketAddrs. Attempts each
    /// supplied SocketAddr in order, returning the first successful connection. If no successful connections
    /// could be made, the last error is returned instead.
    pub async fn connect<A: Into<net::SocketAddr>>(
        &self,
        addr: A,
    ) -> Result<MemoryTcpStream<ClientSocket>, io::Error> {
        loop {
            if let Some(mut inner) = self.inner.try_lock() {
                let addr: net::SocketAddr = addr.into();
                let (client_socket, server_socket) = new_pair(self.network_faults.clone(), addr);
                let port = addr.port();
                let port: num::NonZeroU16 =
                    num::NonZeroU16::new(port).ok_or(io::ErrorKind::InvalidInput)?;
                let mut chan = inner.listener_channel(port)?;
                chan.send(server_socket)
                    .await
                    .map_err(|_| io::ErrorKind::ConnectionRefused)?;

                // TODO: Figure out what to set the local addr to, I realize now I don't actually
                // know how this is supposed to work on Linux
                let tcp_stream = MemoryTcpStream::new_client(client_socket);

                return Ok(tcp_stream);
            }
        }
    }
    /// Creates a new InMemoryTcpListener which will be bound to the port portion of the specified address.
    /// Supplying a port of 0 will result in a random port being assigned.
    pub(crate) async fn bind<A>(&self, addr: A) -> Result<MemoryListener, io::Error>
    where
        A: Into<net::SocketAddr>,
    {
        loop {
            if let Some(mut inner) = self.inner.try_lock() {
                let mut addr: net::SocketAddr = addr.into();
                let (tx, rx) = mpsc::channel(1);
                let port = addr.port();
                let actual_port = inner.register_listener_channel(port, tx.clone())?;
                addr.set_port(actual_port.get());
                return Ok(MemoryListener::new(
                    self.network_faults.clone(),
                    rx,
                    addr.clone(),
                ));
            }
        }
    }
}

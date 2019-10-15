mod connection;
mod listen;
mod pipe;
mod socket;
mod stream;
use futures::{channel::mpsc, SinkExt};
pub(crate) use listen::MemoryListener;
pub(crate) use pipe::Pipe;
pub(crate) use socket::{new_pair, ClientSocket, ServerSocket};
use std::{io, net, num, sync::Arc};
pub(crate) use stream::MemoryTcpStream;
use try_lock::TryLock;

#[derive(Debug, Clone)]
pub(crate) struct InMemoryNetwork {
    env: crate::DeterministicRuntimeSchedulerRng,
    inner: Arc<TryLock<connection::Connections>>,
}

impl InMemoryNetwork {
    pub(crate) fn new(env: crate::DeterministicRuntimeSchedulerRng) -> Self {
        let connections = connection::Connections::new();
        Self {
            env,
            inner: Arc::new(TryLock::new(connections)),
        }
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
                let (client_socket, server_socket) = new_pair(self.env.clone(), addr);
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
                return Ok(MemoryListener::new(rx, addr.clone()));
            }
        }
    }
}

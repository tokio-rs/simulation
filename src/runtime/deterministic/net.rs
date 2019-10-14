mod connection;
mod listen;
mod pipe;
mod socket;
mod stream;
use futures::{channel::mpsc, SinkExt};
pub(crate) use listen::InMemoryListener;
pub(crate) use pipe::Pipe;
pub(crate) use socket::{new_pair, ClientSocket, ServerSocket};
use std::{collections::HashMap, io, net, num, sync::Arc};
pub(crate) use stream::InMemoryTcpStream;
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
    pub async fn connect<A: net::ToSocketAddrs>(
        &self,
        addrs: A,
    ) -> Result<InMemoryTcpStream<ClientSocket>, io::Error> {
        loop {
            if let Some(mut inner) = self.inner.try_lock() {
                let (client_socket, server_socket) = new_pair(self.env.clone());
                let addrs = addrs.to_socket_addrs()?;
                let mut last_err = None;
                for addr in addrs {
                    let port = addr.port();
                    let port = num::NonZeroU16::new(port).ok_or(io::ErrorKind::InvalidInput.into());
                    if let Err(e) = port {
                        last_err = Some(e);
                        continue;
                    }
                    let port = port.unwrap();
                    match inner.listener_channel(port) {
                        Ok(mut chan) => {
                            chan.send(server_socket)
                                .await
                                .map_err(|_| io::ErrorKind::ConnectionRefused)?;

                            // TODO: Figure out what to set the local addr to, I realize now I don't actually
                            // know how this is supposed to work on Linux
                            let tcp_stream = InMemoryTcpStream::new_client(
                                client_socket,
                                addr.clone(),
                                addr.clone(),
                            );

                            return Ok(tcp_stream);
                        }
                        Err(e) => last_err = Some(e),
                    };
                }
                if let Some(e) = last_err {
                    return Err(e);
                } else {
                    // no socket addrs were passed?
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "could not resolve to any addresses",
                    ));
                }
            }
        }
    }
    /// Creates a new InMemoryTcpListener which will be bound to the port portion of the specified address.
    /// Supplying a port of 0 will result in a random port being assigned.
    pub(crate) async fn bind<A: net::ToSocketAddrs>(
        &self,
        addrs: A,
    ) -> Result<InMemoryListener, io::Error> {
        loop {
            if let Some(mut inner) = self.inner.try_lock() {
                let addrs = addrs.to_socket_addrs()?;
                let (tx, rx) = mpsc::channel(1);
                let mut last_err = None;
                for addr in addrs {
                    let port = addr.port();
                    if let Err(e) = inner.register_listener_channel(port, tx.clone()) {
                        last_err = Some(e);
                        continue;
                    } else {
                        return Ok(InMemoryListener::new(rx, addr.clone()));
                    }
                }
                if let Some(e) = last_err {
                    return Err(e);
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "could not resolve to any addresses",
                    ));
                }
            }
        }
    }
}

mod pipe;
pub(crate) use pipe::Pipe;
mod socket;
pub(crate) use socket::{new_pair, ClientSocket, ServerSocket};

mod connection;

use futures::{channel::mpsc, SinkExt};
use std::{collections::HashMap, io, net, num, sync::Arc};
use try_lock::TryLock;

#[derive(Debug, Clone)]
pub(crate) struct InMemoryNetwork {
    env: crate::DeterministicRuntimeHandle,
    inner: Arc<TryLock<connection::Connections>>,
}

impl InMemoryNetwork {
    fn new(env: crate::DeterministicRuntimeHandle) -> Self {
        let connections = connection::Connections::new();
        Self {
            env,
            inner: Arc::new(TryLock::new(connections)),            
        }
    }
    /// Opens an in-memory connection based on the port portion of the supplied SocketAddrs. Attempts each 
    /// supplied SocketAddr in order, returning the first successful connection. If no successful connections
    /// could be made, the last error is returned instead. 
    async fn connect<A: net::ToSocketAddrs>(&mut self, addrs: A) -> Result<InMemoryTcpStream, io::Error> {
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
                            chan.send(server_socket).await.map_err(|_| io::ErrorKind::ConnectionRefused)?;
                            let tcp_stream = InMemoryTcpStream {
                                inner: client_socket,
                                local_addr: addr.clone(),
                                peer_addr: addr.clone(),
                            };
                            return Ok(tcp_stream)
                        },
                        Err(e) => last_err = Some(e)
                    };                    
                }
                if let Some(e) = last_err {
                    return Err(e)
                } else {
                    // no socket addrs were passed?
                    return Err(io::ErrorKind::InvalidInput.into())
                }
            }
        }
    }
    /// Creates a new InMemoryTcpListener which will be bound to the port portion of the specified address.
    /// Supplying a port of 0 will result in a random port being assigned.
    async fn listen<A: net::ToSocketAddrs>(&mut self, addrs: A) -> Result<InMemoryListener, io::Error> {
        loop {
            if let Some(mut inner) = self.inner.try_lock() {
                let addrs = addrs.to_socket_addrs()?;
                let (tx, rx) = mpsc::channel(1);
                let mut last_err = None;
                for addr in addrs {
                    let port = addr.port();
                    if let Err(e) = inner.register_listener_channel(port, tx.clone()) {
                        last_err = Some(e);
                        continue
                    } else {
                        return Ok(InMemoryListener{
                            rx,
                            local_addr: addr.clone()
                        })
                    }
                }
                if let Some(e) = last_err {
                    return Err(e)
                } else {
                    return Err(io::ErrorKind::InvalidInput.into())
                }
            }
        }
    }
}

struct InMemoryTcpStream {
    inner: ClientSocket,
    local_addr: net::SocketAddr,
    peer_addr: net::SocketAddr,
}


struct InMemoryListener {
    rx: mpsc::Receiver<ServerSocket>,
    local_addr: net::SocketAddr
}
//! Simulation of a global in-memory network.
//!
//! Maintains the set of all servers bound to in memory ports.
//! New connections can be made by enqueing a [`TcpStream`] for
//! the server listener to pickup. Closed connections are garbage
//! collected between calls to [`Park`].
//!
//! If there is an existing socket bound, calls to `Network::connect` will enqueue
//! a new connection to be picked up by the `Listener`. Once a listener is removed,
//! all enqueued connections are eventually dropped via GC.
//!
//! [`TcpStream`]:crate::TcpStream
//! [`Park`]: tokio_executor::park::Park
use super::Listener;
use super::SocketHalf;

use futures::{channel::mpsc, SinkExt};
use std::{collections, io, net, num, sync};

/// Inner state for the in memory network implementation and handles which interact with it.
#[derive(Debug)]
pub(crate) struct Inner {
    /// Mapping from bound port number to `Sender`.
    ///
    /// `SocketHalf`'s placed onto the `Sender` channel will be delivered
    /// to any server bound to this `Network`.
    listeners: collections::HashMap<num::NonZeroU16, mpsc::Sender<SocketHalf>>,

    /// Set of machines which have been connected, or have an outstanding network handle.
    connected: collections::HashSet<std::net::IpAddr>,

    /// Next available port to assign
    next_port: u16,
}

impl Inner {
    pub fn new() -> Self {
        Inner {
            listeners: collections::HashMap::new(),
            connected: collections::HashSet::new(),
            next_port: 0,
        }
    }
}

impl Inner {
    /// Check if the provided port is in use or not. If `port` is 0, assign a new
    /// port.
    fn free_port(&mut self, port: u16) -> Result<num::NonZeroU16, io::Error> {
        if let Some(port) = num::NonZeroU16::new(port) {
            if self.listeners.contains_key(&port) {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            Ok(port)
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

    /// Returns a new listener bound to the provided addr. If a listener is already bound, returns an error.
    /// To be assigned a listener with a random port, supply a addr with a port number of 0.
    fn bind_listener(&mut self, mut addr: net::SocketAddr) -> Result<Listener, io::Error> {
        let proposed_port = addr.port();
        let real_port = self.free_port(proposed_port)?;
        addr.set_port(real_port.into());
        let (listener_tx, listener_rx) = mpsc::channel(1);
        let listener = Listener::new(addr.clone(), listener_rx);
        self.listeners.insert(real_port, listener_tx);
        Ok(listener)
    }

    /// Returns a connection channel if one is bound to the provided addr. If there is no connection
    /// channel bound, return an error.
    fn connection_channel(
        &mut self,
        addr: net::SocketAddr,
    ) -> Result<mpsc::Sender<SocketHalf>, io::Error> {
        let server_port = num::NonZeroU16::new(addr.port()).ok_or(io::Error::new(
            io::ErrorKind::InvalidInput,
            "server port cannot be zero",
        ))?;
        let chan = self.listeners.get_mut(&server_port).ok_or(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "connection refused",
        ))?;
        Ok(chan.clone())
    }
}

/// NetworkHandle allows for binding and connecting to the in-memory network.
///
/// NetworkHandles are scoped to a particular IP address. `NetworkHandle::bind` calls will
/// return a listener which listenes for new connections on this IP address.
///
/// No two NetworkHandles will share the same IP address at any given time.
#[derive(Debug, Clone)]
pub struct NetworkHandle {
    ip_addr: net::IpAddr,
    inner: sync::Arc<sync::Mutex<Inner>>,
    next_socket_port: sync::Arc<sync::atomic::AtomicU16>,
}

impl NetworkHandle {
    pub(crate) fn new(ip_addr: net::IpAddr, inner: sync::Arc<sync::Mutex<Inner>>) -> Self {
        Self {
            ip_addr,
            inner,
            next_socket_port: sync::Arc::new(sync::atomic::AtomicU16::new(65535)),
        }
    }

    pub async fn bind(&self, port: u16) -> Result<Listener, io::Error> {
        let mut lock = self.inner.lock().unwrap();
        let server_socket = net::SocketAddr::new(self.ip_addr, port);
        let listener = lock.bind_listener(server_socket)?;
        Ok(listener)
    }

    pub async fn connect(&self, server_addr: net::SocketAddr) -> Result<SocketHalf, io::Error> {
        let client_addr = net::SocketAddr::new(self.ip_addr, self.new_socket_port());
        let client = self
            .enqueue_socket(server_addr, |srv_addr| {
                super::new_socket_pair(client_addr, srv_addr)
            })
            .await?;
        Ok(client)
    }

    pub fn ip_addr(&self) -> net::IpAddr {
        self.ip_addr
    }

    fn new_socket_port(&self) -> u16 {
        self.next_socket_port
            .fetch_sub(1, sync::atomic::Ordering::SeqCst)
    }

    fn connection_channel(
        &self,
        server_addr: net::SocketAddr,
    ) -> Result<mpsc::Sender<SocketHalf>, io::Error> {
        let mut lock = self.inner.lock().unwrap();
        let conn_chan = lock.connection_channel(server_addr)?;
        Ok(conn_chan)
    }

    async fn enqueue_socket<F>(
        &self,
        server_addr: net::SocketAddr,
        socket_fn: F,
    ) -> Result<SocketHalf, io::Error>
    where
        F: Fn(net::SocketAddr) -> (SocketHalf, SocketHalf),
    {
        let mut conn_chan = self.connection_channel(server_addr)?;
        let (client_socket, server_socket) = socket_fn(server_addr);

        conn_chan
            .send(server_socket)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused"))?;
        Ok(client_socket)
    }
}

#[cfg(test)]
mod tests {
    use super::super::Network;
    use super::*;
    use crate::deterministic::DeterministicRuntime;
    use crate::{Environment, TcpListener, TcpStream};
    use tokio::codec::{Framed, LinesCodec};
    use futures::{StreamExt, SinkExt};

    async fn handle_conn<T>(socket: T)
    where
        T: TcpStream,
    {
        let mut transport = Framed::new(socket, LinesCodec::new());
        while let Some(Ok(message)) = transport.next().await {
            assert_eq!(String::from("ping"), message);
            transport.send(String::from("pong")).await.unwrap();
        }
    }

    async fn serve<E, L>(handle: E, mut listener: L)
    where
        L: TcpListener,
        E: Environment,
    {
        while let Ok((new_conn, _)) = listener.accept().await {
            handle.spawn(handle_conn(new_conn))
        }
    }

    #[test]
    fn bind_and_dial() {
        let mut runtime = DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let network = Network::new_with_park(());
            let network_handle = network.handle("127.0.0.1".parse().unwrap()).unwrap();
            let network_handle2 = network.handle("10.0.0.1".parse().unwrap()).unwrap();
            let listener = network_handle.bind(9092).await.unwrap();
            let socket = listener.local_addr().unwrap();
            handle.spawn(serve(handle.clone(), listener));

            // Continously try to reconnect until the server binds
            let conn = network_handle2.connect(socket).await;
            if let Ok(conn) = conn {
                assert_eq!(net::Ipv4Addr::new(10, 0, 0, 1), conn.local_addr().ip());
                let mut transport = Framed::new(conn, LinesCodec::new());
                transport.send(String::from("ping")).await.unwrap();
                let response = transport.next().await.unwrap().unwrap();
                assert_eq!(response, String::from("pong"));
                return;
            }
        });
    }
}

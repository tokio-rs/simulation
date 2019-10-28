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
use crate::deterministic::network::Inner;
use futures::{channel::mpsc, SinkExt};
use std::{io, net, sync};

/// NetworkHandle allows for binding and connecting to the in-memory network.
///
/// NetworkHandle are scoped to a particular IP address. `NetworkHandle::bind` calls will
/// return a listener which listenes for new connections on this IP address.
///
/// No two NetworkHandle will share the same IP address at any given time.
#[derive(Debug, Clone)]
pub struct NetworkHandle {
    inner: sync::Arc<sync::Mutex<Inner<SocketHalf>>>,
    next_socket_port: sync::Arc<sync::atomic::AtomicU16>,
}

impl NetworkHandle {
    pub(crate) fn new(inner: sync::Arc<sync::Mutex<Inner<SocketHalf>>>) -> Self {
        Self {
            inner,
            next_socket_port: sync::Arc::new(sync::atomic::AtomicU16::new(65535)),
        }
    }

    pub async fn bind(
        &self,
        socket_addr: net::SocketAddr,
    ) -> Result<Listener<SocketHalf>, io::Error> {
        let mut lock = self.inner.lock().unwrap();
        let listener = lock.bind_listener(socket_addr)?;
        Ok(listener)
    }

    pub async fn connect(&self, server_addr: net::SocketAddr) -> Result<SocketHalf, io::Error> {
        let client_addr =
            net::SocketAddr::new(net::Ipv4Addr::UNSPECIFIED.into(), self.new_socket_port());
        let client = self
            .enqueue_socket(server_addr, |srv_addr| {
                super::new_socket_pair(client_addr, srv_addr)
            })
            .await?;
        Ok(client)
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
    use crate::deterministic::DeterministicRuntime;
    use crate::{Environment, TcpListener, TcpStream};
    use futures::{SinkExt, StreamExt};
    use tokio::codec::{Framed, LinesCodec};

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
            let network_handle = network.handle();
            let network_handle2 = network.handle();
            let listener = network_handle
                .bind("127.0.0.1:9092".parse().unwrap())
                .await
                .unwrap();
            let socket = listener.local_addr().unwrap();
            handle.spawn(serve(handle.clone(), listener));

            // Continously try to reconnect until the server binds
            let conn = network_handle2.connect(socket).await;
            if let Ok(conn) = conn {
                let mut transport = Framed::new(conn, LinesCodec::new());
                transport.send(String::from("ping")).await.unwrap();
                let response = transport.next().await.unwrap().unwrap();
                assert_eq!(response, String::from("pong"));
                return;
            }
        });
    }
}

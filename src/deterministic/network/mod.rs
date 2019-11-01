//! This is just one of those things where I don't really know
//! what the design should look like so I just slapped a bunch of
//! stuff together. Sorry.
use futures::channel::mpsc;
use futures::StreamExt;
use std::{io, net, sync, time::Duration};
mod handle;
mod inner;
mod socket;
use async_trait::async_trait;
pub use handle::NetworkHandle;
use inner::Inner;
pub use socket::{new_socket_pair, FaultyTcpStream, SocketHalf};

#[derive(Debug)]
/// Network for creating and managing a set of servers
/// and clients.
pub struct Network<P> {
    inner_park: P,
    inner: sync::Arc<sync::Mutex<Inner<SocketHalf>>>,
}

impl<P> Network<P> {
    pub fn handle(&self) -> NetworkHandle {
        // TODO: Throw error and register new handle IP
        let inner = sync::Arc::clone(&self.inner);
        NetworkHandle::new(inner)
    }
}

impl<P> tokio_executor::park::Park for Network<P>
where
    P: tokio_executor::park::Park,
{
    type Unpark = P::Unpark;
    type Error = P::Error;
    fn unpark(&self) -> Self::Unpark {
        self.inner_park.unpark()
    }
    fn park(&mut self) -> Result<(), Self::Error> {
        self.inner_park.park()
    }
    fn park_timeout(&mut self, duration: Duration) -> Result<(), Self::Error> {
        self.inner_park.park_timeout(duration)
    }
}

impl<P> Network<P> {
    pub fn new_with_park(park: P) -> Self {
        let inner = Inner::new();
        let inner = sync::Mutex::new(inner);
        let inner = sync::Arc::new(inner);
        Self {
            inner,
            inner_park: park,
        }
    }
}

pub struct Listener<T> {
    /// Channel containing incoming connections.
    accept: mpsc::Receiver<T>,
    local_addr: net::SocketAddr,
}

impl<T> Listener<T> {
    fn new(bind_addr: net::SocketAddr, rx: mpsc::Receiver<T>) -> Self {
        Self {
            accept: rx,
            local_addr: bind_addr,
        }
    }
}

#[async_trait]
impl<T> crate::TcpListener for Listener<T>
where
    T: crate::TcpStream,
{
    type Stream = T;
    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error> {
        if let Some(next_socket) = self.accept.next().await {
            let socket_addr = next_socket.local_addr()?;
            Ok((next_socket, socket_addr))
        } else {
            Err(io::ErrorKind::NotConnected.into())
        }
    }
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.local_addr)
    }
    fn ttl(&self) -> io::Result<u32> {
        Ok(0)
    }
    fn set_ttl(&self, _: u32) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use crate::TcpListener;
    use futures::{SinkExt, StreamExt};

    use tokio::codec::{Framed, LinesCodec};

    async fn handle_connection<T>(conn: T)
    where
        T: crate::TcpStream,
    {
        let mut transport = Framed::new(conn, LinesCodec::new());
        while let Some(Ok(msg)) = transport.next().await {
            let num: usize = msg.parse().unwrap();
            let new_num = num * 2;
            transport.send(new_num.to_string()).await.unwrap();
        }
    }

    async fn server(
        addr: net::SocketAddr,
        network: NetworkHandle,
        handle: crate::deterministic::DeterministicRuntimeHandle,
    ) {
        let mut listener = network
            .bind(addr)
            .await
            .expect("expected to be able to bind");
        while let Ok((new_conn, _)) = listener.accept().await {
            handle.spawn(handle_connection(new_conn));
        }
    }

    struct NoopPark;
    impl tokio_executor::park::Unpark for NoopPark {
        fn unpark(&self) {}
    }
    impl tokio_executor::park::Park for NoopPark {
        type Unpark = NoopPark;
        type Error = io::Error;
        fn unpark(&self) -> Self::Unpark {
            unimplemented!()
        }
        fn park(&mut self) -> Result<(), Self::Error> {
            unimplemented!()
        }
        fn park_timeout(&mut self, _duration: Duration) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[test]
    fn bind_and_connect() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        let network = Network::new_with_park(NoopPark);
        let network_handle = network.handle();
        runtime.block_on(async {
            // spawn server which binds to a port.
            let bind_addr = "127.0.0.1:9092".parse().unwrap();
            handle.spawn(server(bind_addr, network_handle.clone(), handle.clone()));
            let stream;
            loop {
                if let Ok(conn) = network_handle.connect(bind_addr).await {
                    stream = conn;
                    break;
                } else {
                    // continously force this task to the back of the queue if the server task has not spawned yet.
                    handle
                        .delay_from(std::time::Duration::from_millis(100))
                        .await;
                }
            }

            let mut transport = Framed::new(stream, LinesCodec::new());
            for idx in 0..100usize {
                transport.send(idx.to_string()).await.unwrap();
                let result: String = transport.next().await.unwrap().unwrap();
                assert_eq!(result.parse::<usize>().unwrap(), idx * 2);
            }
        });
    }
}

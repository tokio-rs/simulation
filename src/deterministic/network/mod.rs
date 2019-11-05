//! In memory network
//!
//! Servers can be registered with the network, allowing for new connections to
//! be accepted or rejected depending on the current fault state of the network.
//!
//! The network can inject partitions between machines.

use std::{io, net, sync};
mod listen;
mod socket;
mod inner;
mod fault;
pub(crate) use inner::Inner;
pub use listen::Listener;
use listen::ListenerState;
use socket::{FaultyTcpStream, SocketHalf};

pub type Socket = FaultyTcpStream<SocketHalf>;
pub struct DeterministicNetwork {
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl DeterministicNetwork {
    pub(crate) fn new(handle: crate::deterministic::DeterministicTimeHandle) -> DeterministicNetwork {
        let inner = Inner::new(handle);
        let inner = sync::Arc::new(sync::Mutex::new(inner));
        DeterministicNetwork { inner }
    }
    pub fn scoped<T>(&self, local_addr: T) -> DeterministicNetworkHandle
    where
        T: Into<net::IpAddr>,
    {
        DeterministicNetworkHandle::new(local_addr.into(), sync::Arc::clone(&self.inner))
    }
}

/// NetworkHandle is a scoped handle for binding and creating new connections.
/// Each NetworkHandle is scoped to a particular IP address, which is then used when
/// injecting faults.
#[derive(Debug, Clone)]
pub struct DeterministicNetworkHandle {
    local_addr: net::IpAddr,
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl DeterministicNetworkHandle {
    fn new(local_addr: net::IpAddr, inner: sync::Arc<sync::Mutex<Inner>>) -> Self {
        DeterministicNetworkHandle { local_addr, inner }
    }

    pub async fn bind(&self, mut bind_addr: net::SocketAddr) -> Result<Listener, io::Error> {
        bind_addr.set_ip(self.local_addr);
        let mut lock = self.inner.lock().unwrap();
        lock.listen(bind_addr)
    }

    pub async fn connect(&self, dest: net::SocketAddr) -> Result<FaultyTcpStream<SocketHalf>, io::Error> {
        let connfut = {
            let mut lock = self.inner.lock().unwrap();
            let ret = lock.connect(self.local_addr, dest);
            drop(lock);
            ret
        };
        connfut.await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Environment, TcpListener,
    };
    use futures::{StreamExt, SinkExt};
    use std::net;
    use tokio::codec::{Framed, LinesCodec};

    /// Starts a server which will forward messages to the next server in the ring.
    async fn serve_message_ring(
        network: DeterministicNetworkHandle,
        next_server: net::SocketAddr,
    ) -> Result<(), io::Error> {
        let bind_addr = "127.0.0.1:9092".parse().unwrap();
        let mut listener = network.bind(bind_addr).await?;
        while let Ok((conn, _)) = listener.accept().await {
            let mut client_transport = Framed::new(conn, LinesCodec::new());
            loop {
                if let Ok(stream) = network.connect(next_server).await {
                    let mut server_transport = Framed::new(stream, LinesCodec::new());
                    while let Some(Ok(message)) = client_transport.next().await {
                        let decoded: usize = message.parse().unwrap();
                        let new_message = format!("{}", decoded + 1);
                        server_transport
                            .send(new_message)
                            .await
                            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, ""))?;
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    /// Forms a ring of servers which pass a message to each other. The message is incremented in each server.
    /// Once the message has passed through 1000 servers, the test is finished.
    fn test_message_ring() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        let network = DeterministicNetwork::new(handle.time_handle());
        runtime.block_on(async {
            for oct in 0..100 {
                let scoped = network.scoped(net::Ipv4Addr::new(10, 0, 0, oct));
                handle.spawn(async move {
                    serve_message_ring(
                        scoped,
                        net::SocketAddr::new(net::Ipv4Addr::new(10, 0, 0, oct + 1).into(), 9092),
                    ).await.unwrap();
                });
            }
            let start_addr = net::SocketAddr::new(net::Ipv4Addr::new(10, 0, 0, 0).into(), 9092);
            let end_addr = net::SocketAddr::new(net::Ipv4Addr::new(10, 0, 0, 100).into(), 9092);


            let scoped = network.scoped(end_addr.ip());
            handle.delay_from(std::time::Duration::from_secs(10));

            let client = scoped.connect(start_addr).await.unwrap();
            let mut client_transport = Framed::new(client, LinesCodec::new());
            client_transport.send(String::from("1")).await.unwrap();
            let mut listener = scoped.bind(end_addr).await.unwrap();
            while let Ok((new_conn, _)) = listener.accept().await {
                let mut server_transport = Framed::new(new_conn, LinesCodec::new());
                while let Some(Ok(message))  = server_transport.next().await {
                    let decoded: usize = message.parse().unwrap();
                    if decoded > 1000 {
                        return
                    }
                    client_transport.send(message).await.unwrap();
                }
            }
        });
    }

    #[test]
    fn test_scoped_registration() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        let network = DeterministicNetwork::new(handle.time_handle());
        runtime.block_on(async {
            // create scoped network handle
            let network1 = network.scoped(net::Ipv4Addr::new(10, 0, 0, 1));
            let network2 = network.scoped(net::Ipv4Addr::new(10, 0, 0, 2));
            let common_addr = "127.0.0.1:9092".parse().unwrap();
            let network1_listener = network1.bind(common_addr).await.unwrap();
            let network2_listener = network2.bind(common_addr).await.unwrap();
            assert_ne!(
                network1_listener.local_addr().unwrap(),
                network2_listener.local_addr().unwrap(),
                "expected listener local addrs to be different"
            )
        });
    }
}

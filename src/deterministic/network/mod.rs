//! This is just one of those things where I don't really know
//! what the design should look like so I just slapped a bunch of
//! stuff together. Sorry.
use futures::channel::mpsc;
use futures::{Poll, SinkExt, Stream, StreamExt};
pub(crate) use pipe::Pipe;
use std::{
    collections::{hash_map::Entry, HashMap},
    io, net, num,
    pin::Pin,
    sync,
    task::Context,
    time::Duration,
};
use tokio_executor::park::Park;
mod pipe;
mod stream;
use async_trait::async_trait;
pub use stream::{ClientConnection, MemoryStream, ServerConnection};

#[derive(Debug)]
struct Inner {
    /// Next port which will be allocated
    next_port: u16,

    /// Map of active listeners to channels which new connections can be sent on.
    listeners: HashMap<num::NonZeroU16, mpsc::Sender<(stream::ServerConnection, net::SocketAddr)>>,

    /// Fault injectors corresponding to a connection.
    fault_injectors: HashMap<num::NonZeroU16, Vec<stream::MemoryConnectionFaultInjector>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            next_port: 1,
            listeners: HashMap::new(),
            fault_injectors: HashMap::new(),
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

    fn deregister_listener(&mut self, port: num::NonZeroU16) {
        self.listeners.remove(&port);
        if let Some(faults) = self.fault_injectors.get(&port) {
            for fault in faults {
                fault.disconnect();
            }
        }
        self.fault_injectors.remove(&port);
    }

    fn register_new_listener(
        &mut self,
        port: u16,
    ) -> Result<
        (
            num::NonZeroU16,
            mpsc::Receiver<(stream::ServerConnection, net::SocketAddr)>,
        ),
        io::Error,
    > {
        let port = self.free_port(port)?;
        let (tx, rx) = mpsc::channel(1);
        self.listeners.insert(port, tx);
        Ok((port, rx))
    }

    fn listener_channel(
        &self,
        server_port: num::NonZeroU16,
    ) -> Result<mpsc::Sender<(stream::ServerConnection, net::SocketAddr)>, io::Error> {
        self.listeners
            .get(&server_port)
            .cloned()
            .ok_or_else(|| io::ErrorKind::ConnectionRefused.into())
    }
}

pub struct Listener {
    ttl: u32,
    port: num::NonZeroU16,
    stream: mpsc::Receiver<(stream::ServerConnection, net::SocketAddr)>,
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl Stream for Listener {
    type Item = Result<stream::MemoryStream, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next = futures::ready!(self.stream.poll_next_unpin(cx));
        if let Some((sock, _)) = next {
            Poll::Ready(Some(Ok(sock)))
        } else {
            Poll::Ready(None)
        }
    }
}

#[async_trait]
impl crate::TcpListener for Listener {
    type Stream = stream::MemoryStream;
    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error> {
        if let Some(sock) = self.stream.next().await {
            Ok(sock)
        } else {
            Err(io::ErrorKind::NotConnected.into())
        }
    }
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(net::SocketAddr::new(
            net::Ipv4Addr::LOCALHOST.into(),
            self.port.get(),
        ))
    }
    fn ttl(&self) -> io::Result<u32> {
        Ok(self.ttl)
    }
    fn set_ttl(&self, _: u32) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.inner.lock().unwrap().deregister_listener(self.port)
    }
}

#[derive(Debug, Clone)]
pub struct NetworkHandle {
    fault_injector: super::FaultInjectorHandle,
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl NetworkHandle {
    fn new(
        inner: sync::Arc<sync::Mutex<Inner>>,
        fault_injector: super::FaultInjectorHandle,
    ) -> Self {
        Self {
            fault_injector,
            inner,
        }
    }
    pub async fn connect(
        &self,
        addr: net::SocketAddr,
    ) -> Result<stream::ClientConnection, io::Error> {
        let port: num::NonZeroU16 = num::NonZeroU16::new(addr.port())
            .ok_or_else(|| <io::ErrorKind as Into<io::Error>>::into(io::ErrorKind::InvalidInput))?;
        let mut channel = { self.inner.lock().unwrap().listener_channel(port)? };
        let (fault_handle, client, server) = stream::new_pair(self.fault_injector.clone(), port);
        channel
            .send((server, client.local_addr()))
            .await
            .map_err(|_| io::ErrorKind::ConnectionRefused)?;
        {
            let mut lock = self.inner.lock().unwrap();
            match lock.fault_injectors.entry(port) {
                Entry::Occupied(mut o) => o.get_mut().push(fault_handle),
                Entry::Vacant(v) => {
                    v.insert(vec![fault_handle]);
                }
            };
        }
        Ok(client)
    }

    pub fn bind(&self, addr: net::SocketAddr) -> Result<Listener, io::Error> {
        let mut lock = self.inner.lock().unwrap();
        let (port, listener_stream) = lock.register_new_listener(addr.port())?;
        Ok(Listener {
            ttl: 0,
            port,
            stream: listener_stream,
            inner: sync::Arc::clone(&self.inner),
        })
    }
}

pub(crate) struct Network<P> {
    park: P,
    inner: sync::Arc<sync::Mutex<Inner>>,
    fault_injector: super::FaultInjectorHandle,
}

impl<P> Park for Network<P>
where
    P: Park,
{
    type Unpark = P::Unpark;
    type Error = P::Error;
    fn unpark(&self) -> Self::Unpark {
        self.park.unpark()
    }
    fn park(&mut self) -> Result<(), Self::Error> {
        self.inject_faults();
        self.park.park()
    }
    fn park_timeout(&mut self, duration: Duration) -> Result<(), Self::Error> {
        self.inject_faults();
        self.park.park_timeout(duration)
    }
}

impl<P> Network<P>
where
    P: Park,
{
    pub(crate) fn new_with_park(park: P, fault_injector: super::FaultInjectorHandle) -> Network<P> {
        let inner = Inner {
            next_port: 1,
            listeners: HashMap::new(),
            fault_injectors: HashMap::new(),
        };
        let inner = sync::Arc::new(sync::Mutex::new(inner));
        Network {
            inner,
            park,
            fault_injector,
        }
    }

    pub(crate) fn handle(&self) -> NetworkHandle {
        NetworkHandle {
            fault_injector: self.fault_injector.clone(),
            inner: sync::Arc::clone(&self.inner),
        }
    }

    fn inject_faults(&self) {
        let mut lock = self.inner.lock().unwrap();
        for (_, v) in lock.fault_injectors.iter_mut() {
            if let Some(idx) = self
                .fault_injector
                .pick_rand_connection_disconnect(0..v.len())
            {
                let fault_injector = v.remove(idx);
                fault_injector.disconnect();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use crate::TcpListener;
    use futures::StreamExt;
    use std::sync;
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
        let mut listener = network.bind(addr).expect("expected to be able to bind");
        while let Ok((new_conn, _)) = listener.accept().await {
            handle.spawn(handle_connection(new_conn));
        }
    }

    #[test]
    fn bind_and_connect() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        let noop_fault_injector = crate::deterministic::FaultInjector::new_noop();
        let network_inner = Inner::new();
        let network_inner = sync::Arc::new(sync::Mutex::new(network_inner));
        let network_handle = NetworkHandle::new(network_inner, noop_fault_injector.handle());
        runtime.block_on(async {
            // spawn server which binds to a port.
            let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
            handle.spawn(server(addr, network_handle.clone(), handle.clone()));
            let stream;
            loop {
                if let Ok(conn) = network_handle.connect(addr).await {
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

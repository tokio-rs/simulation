//! InMemory TCPStream-like connection between a server and a client.
//! Supports injecting delay or disconnect faults specific to the client or server
//! side of a connection.
use futures::{FutureExt, Poll};
use std::{io, net, pin::Pin, sync, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_sync::AtomicWaker;

/// MemoryStream is a unidirectional in-memory byte stream supporting fault injection.
/// Each call to poll with attempt to inject delay faults. Upon the first call to poll,
/// a waker will be registered for a disconnect fault.
#[derive(Debug)]
pub struct MemoryStream {
    fault_injector: MemoryStreamFaultInjectorHandle,
    reader: tokio_io::split::ReadHalf<super::Pipe>,
    writer: tokio_io::split::WriteHalf<super::Pipe>,
    local_addr: net::SocketAddr,
    peer_addr: net::SocketAddr,
}

/// Wraps a FaultInjector to provide connection specific fault injection.
#[derive(Debug)]
struct MemoryStreamFaultInjector {
    /// Designates the types of fault errors which this fault injector will return.
    mode: Mode,

    /// Active delay faults are stored here. If there is an active delay, all writes and reads
    /// to the MemoryStream will be paused until this delay elapses.
    delay: Option<tokio_timer::Delay>,

    /// Wrapped fault injector, used to query for delay faults.
    fault_injector: crate::deterministic::FaultInjectorHandle,

    /// Disconnected fault injectors will return an appropriate disconnected error on calls to `poll_disconnected`,
    /// determined by the `Mode`.
    disconnected: bool,

    /// Waker to awake yielded tasks when a disconnect is triggered.
    waker: AtomicWaker,
}
/// Mode determines which types of errors to return upon polling a memory stream
/// with a fault injected. There are different types of errors for clients and servers.
#[derive(Debug)]
enum Mode {
    Client,
    Server,
}

/// Wraps the provided FaultInjectorHandle in a MemoryConnectionFaultInjector allowing for
/// injecting faults into an in-memory connection.
///
/// This fault injector allows injecting faults specific to the client or server side of a connection.
#[derive(Debug, Clone)]
pub(crate) struct MemoryConnectionFaultInjector {
    client: MemoryStreamFaultInjectorHandle,
    server: MemoryStreamFaultInjectorHandle,
}

impl MemoryConnectionFaultInjector {
    /// Returns a new `MemoryConnectionFaultInjector` wrapping the provided `FaultInjectorHandle`.
    ///
    /// [`FaultInjectorHandle`]:crate::next::FaultInjectorHandle
    /// [`MemoryConnectionFaultInjector`]:MemoryConnectionFaultInjector
    fn new(fault_injector: super::super::FaultInjectorHandle) -> Self {
        let client = MemoryStreamFaultInjectorHandle::new_with_fault_injector(
            fault_injector.clone(),
            Mode::Client,
        );
        let server =
            MemoryStreamFaultInjectorHandle::new_with_fault_injector(fault_injector, Mode::Server);
        Self { client, server }
    }

    /// Returns a handle to the fault injector corresponding to the client side of a MemoryConnection.
    /// This handle can be used to inject faults into client writes and server reads.
    fn client_handle(&self) -> MemoryStreamFaultInjectorHandle {
        self.client.clone()
    }

    /// Returns a handle to the fault injector corresponding to the server side of a MemoryConnection.    
    /// This handle can be used to inject faults into server writes and client reads.
    fn server_handle(&self) -> MemoryStreamFaultInjectorHandle {
        self.server.clone()
    }

    /// Disconnects the client, further client writes or server reads will return an error.
    pub(crate) fn disconnect_client(&self) {
        self.client.set_disconnected();
    }

    /// Disconnects the server, futher server writes or client reads will return an error.
    pub(crate) fn disconnect_server(&self) {
        self.server.set_disconnected();
    }

    /// Disconnects both the server and the client. Further reads and writes will return an error.
    pub(crate) fn disconnect(&self) {
        self.disconnect_client();
        self.disconnect_server();
    }
}

/// Handle to a `MemoryStreamFaultInjector`.
/// Supports both probability based delay faults as well as injecting disconnects.
#[derive(Debug, Clone)]
struct MemoryStreamFaultInjectorHandle {
    inner: sync::Arc<sync::Mutex<MemoryStreamFaultInjector>>,
}

impl MemoryStreamFaultInjectorHandle {
    /// Returns a new `MemoryStreamFaultInjectorHandle` which can be used to track delay faults and inject
    /// disconnects.
    fn new_with_fault_injector(
        fault_injector: super::super::FaultInjectorHandle,
        mode: Mode,
    ) -> Self {
        let state = MemoryStreamFaultInjector {
            mode,
            delay: None,
            fault_injector,
            disconnected: false,
            waker: AtomicWaker::new(),
        };
        let state = sync::Arc::new(sync::Mutex::new(state));
        Self { inner: state }
    }

    /// Poll any existing delay faults. If there is no existing delay faults, this method will return Poll::Ready(()) and attempt
    /// to get one from the wrapped fault injector for the next call to `poll_delay`.
    fn poll_delay(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let mut lock = self.inner.lock().unwrap();
        if let Some(mut delay) = lock.delay.take() {
            if let Poll::Pending = delay.poll_unpin(cx) {
                lock.delay.replace(delay);
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        } else {
            let new = lock.fault_injector.socket_read_delay();
            std::mem::replace(&mut lock.delay, new);
            Poll::Ready(())
        }
    }

    /// Poll for an injected disconnect fault. Calls to `poll_disconnected` will register a waker
    /// in order to respond to externally injected disconnects.
    fn poll_disconnected(&self, cx: &mut Context<'_>) -> Poll<io::Error> {
        let lock = self.inner.lock().unwrap();
        if lock.disconnected {
            match lock.mode {
                Mode::Client => return Poll::Ready(io::ErrorKind::ConnectionAborted.into()),
                Mode::Server => return Poll::Ready(io::ErrorKind::ConnectionAborted.into()),
            }
        }
        lock.waker.register_by_ref(cx.waker());
        Poll::Pending
    }

    /// Sets this fault injector to signal on `poll_disconnected`, waking the registered task.
    fn set_disconnected(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.disconnected = true;
        lock.waker.wake();
    }
}

/// An in-memory connection from a client to a server.
pub type ClientConnection = MemoryStream;

/// An in-memory connection from a server to a client.
pub type ServerConnection = MemoryStream;

impl crate::TcpStream for MemoryStream {
    fn local_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.local_addr)
    }
    fn peer_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.peer_addr)
    }
    fn shutdown(&self) -> io::Result<()> {
        self.fault_injector.set_disconnected();
        Ok(())
    }
}

/// Returns a new in-memory connection between a server and a client.
pub(crate) fn new_pair(
    fault_injector: super::super::FaultInjectorHandle,
    port: std::num::NonZeroU16,
) -> (
    MemoryConnectionFaultInjector,
    ClientConnection,
    ServerConnection,
) {
    let client_addr = net::SocketAddr::new(net::Ipv4Addr::LOCALHOST.into(), 0);
    let server_addr = net::SocketAddr::new(net::Ipv4Addr::LOCALHOST.into(), port.get());

    let client_pipe = super::Pipe::new();
    let server_pipe = super::Pipe::new();
    let (client_rx, client_tx) = tokio::io::split(client_pipe);
    let (server_rx, server_tx) = tokio::io::split(server_pipe);
    let fault_injector = MemoryConnectionFaultInjector::new(fault_injector);
    let server_stream = MemoryStream::new(
        fault_injector.server_handle(),
        client_rx,
        server_tx,
        server_addr,
        client_addr,
    );
    let client_stream = MemoryStream::new(
        fault_injector.client_handle(),
        server_rx,
        client_tx,
        client_addr,
        server_addr,
    );
    (fault_injector, client_stream, server_stream)
}

impl MemoryStream {
    fn new(
        fault_injector: MemoryStreamFaultInjectorHandle,
        reader: tokio_io::split::ReadHalf<super::Pipe>,
        writer: tokio_io::split::WriteHalf<super::Pipe>,
        local_addr: net::SocketAddr,
        peer_addr: net::SocketAddr,
    ) -> Self {
        MemoryStream {
            fault_injector,
            reader,
            writer,
            local_addr,
            peer_addr,
        }
    }

    pub fn local_addr(&self) -> net::SocketAddr {
        self.local_addr
    }
    pub fn peer_addr(&self) -> net::SocketAddr {
        self.local_addr
    }
}

impl AsyncRead for MemoryStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        futures::ready!(self.as_mut().fault_injector.poll_delay(cx));
        if let Poll::Ready(e) = self.as_ref().fault_injector.poll_disconnected(cx) {
            return Poll::Ready(Err(e));
        }
        let reader = Pin::new(&mut self.reader);
        reader.poll_read(cx, buf)
    }
}

impl AsyncWrite for MemoryStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        futures::ready!(self.as_mut().fault_injector.poll_delay(cx));
        if let Poll::Ready(e) = self.as_ref().fault_injector.poll_disconnected(cx) {
            return Poll::Ready(Err(e));
        }
        let writer = Pin::new(&mut self.writer);
        writer.poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        futures::ready!(self.as_mut().fault_injector.poll_delay(cx));
        if let Poll::Ready(e) = self.as_ref().fault_injector.poll_disconnected(cx) {
            return Poll::Ready(Err(e));
        }
        let writer = Pin::new(&mut self.writer);
        writer.poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        futures::ready!(self.as_mut().fault_injector.poll_delay(cx));
        let writer = Pin::new(&mut self.writer);
        writer.poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use futures::{SinkExt, StreamExt};
    use tokio::io::AsyncWriteExt;

    async fn pong_server(server: ServerConnection) -> Result<(), tokio::codec::LinesCodecError> {
        let mut transport = tokio::codec::Framed::new(server, tokio::codec::LinesCodec::new());
        while let Some(Ok(ping)) = transport.next().await {
            assert_eq!(String::from("ping"), ping);
            transport.send(String::from("pong")).await?;
        }
        Err(tokio::codec::LinesCodecError::Io(
            io::ErrorKind::BrokenPipe.into(),
        ))
    }

    #[test]
    /// Tests that messages can be sent and received using a pair of MemoryStreams.
    fn test_ping_pong() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let port = std::num::NonZeroU16::new(9092).unwrap();
            let noop_fault_injector = super::super::super::FaultInjector::new_noop();
            let (_, server_conn, client_conn) = new_pair(noop_fault_injector.handle(), port);
            handle.spawn(pong_server(server_conn).map(|_| ()));
            let mut transport =
                tokio::codec::Framed::new(client_conn, tokio::codec::LinesCodec::new());
            for _ in 0..100usize {
                transport.send(String::from("ping")).await.unwrap();
                let result = transport.next().await.unwrap().unwrap();
                assert_eq!(result, String::from("pong"));
            }
        });
    }

    #[test]
    /// Tests that disconnecting the server and client will cause both the server and client to fail further
    /// reads/writes with an error.
    fn test_disconnect() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let port = std::num::NonZeroU16::new(9092).unwrap();
            let noop_fault_injector = super::super::super::FaultInjector::new_noop();
            let (conn_handle, server_conn, client_conn) =
                new_pair(noop_fault_injector.handle(), port);
            let server_status = crate::spawn_with_result(&handle, pong_server(server_conn));
            let mut transport =
                tokio::codec::Framed::new(client_conn, tokio::codec::LinesCodec::new());
            for msg_num in 0..5 {
                if msg_num >= 3 {
                    conn_handle.disconnect();
                    assert!(transport.send(String::from("ping")).await.is_err());
                    assert!(transport.next().await.unwrap().is_err());
                } else {
                    transport.send(String::from("ping")).await.unwrap();
                    let result = transport.next().await.unwrap().unwrap();
                    assert_eq!(result, String::from("pong"));
                }
            }
            let server_status = server_status.await;
            assert!(
                server_status.is_err(),
                "expected server to terminate because the client connection was closed"
            );
        });
    }

    #[test]
    /// Tests that disconnecting a client will cause the server to wake and return an error.
    fn test_client_disconnect_wake() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let port = std::num::NonZeroU16::new(9092).unwrap();
            let noop_fault_injector = super::super::super::FaultInjector::new_noop();
            let (conn_handle, server_conn, _) = new_pair(noop_fault_injector.handle(), port);
            let server_status = crate::spawn_with_result(&handle, pong_server(server_conn));
            futures::pin_mut!(server_status);
            tokio_test::assert_pending!(futures::poll!(server_status.as_mut()), "expected the server status to be pending due to the MemoryConnection still being open");
            conn_handle.disconnect_client();
            let server_status = server_status.await;
            assert!(server_status.is_err(), "expected server to terminate because the client connection was closed");
        });
    }

    #[test]
    /// Tests that disconnecting a server will cause client writes to return an error.
    fn test_server_disconnect_wake() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let port = std::num::NonZeroU16::new(9092).unwrap();
            let noop_fault_injector = crate::deterministic::FaultInjector::new_noop();
            let (conn_handle, server_conn, mut client_conn) = new_pair(noop_fault_injector.handle(), port);
            let server_status = crate::spawn_with_result(&handle, pong_server(server_conn));
            futures::pin_mut!(server_status);
            tokio_test::assert_pending!(futures::poll!(server_status.as_mut()), "expected the server status to be pending due to the MemoryConnection still being open");
            conn_handle.disconnect_server();
            let result = client_conn.write_all(b"foo").await;
            assert!(result.is_err(), "expected write to fail because the server was disconnected");            
        });
    }
}

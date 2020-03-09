use super::Link;
use futures::{future::poll_fn, ready};
use std::{io, net, pin::Pin, sync, task::Context, task::Poll, time};
use tokio::io::{AsyncRead, AsyncWrite};

/// Inner shared state for two halves of a TcpStream.
#[derive(Debug)]
struct Shared {
    client_addr: net::SocketAddr,
    server_addr: net::SocketAddr,
    client_shutdown: Option<net::Shutdown>,
    server_shutdown: Option<net::Shutdown>,

    nodelay: bool,
    recv_buffer_size: usize,
    send_buffer_size: usize,
    keepalive: Option<time::Duration>,
    ttl: u32,
    linger: Option<time::Duration>,
}

#[derive(Debug)]
pub enum TcpStreamHandleError {
    #[allow(dead_code)]
    StreamDropped,
}

#[derive(Debug)]
pub struct SimulatedTcpStream {
    local_addr: net::SocketAddr,
    peer_addr: net::SocketAddr,
    shared: sync::Arc<sync::Mutex<Shared>>,
    link: Link,
    fault_injector: Option<FaultInjector>,
}

impl SimulatedTcpStream {
    fn new(
        local_addr: net::SocketAddr,
        peer_addr: net::SocketAddr,
        shared: sync::Arc<sync::Mutex<Shared>>,
        link: Link,
        fault_injector: Option<FaultInjector>,
    ) -> Self {
        Self {
            local_addr,
            peer_addr,
            shared,
            link,
            fault_injector,
        }
    }

    pub(crate) fn new_pair(
        client_addr: net::SocketAddr,
        server_addr: net::SocketAddr,
        fault_injector: Option<FaultInjector>,
    ) -> (Self, Self) {
        let (client_link, server_link) = Link::new_pair();
        let shared = Shared {
            client_addr,
            server_addr,
            client_shutdown: None,
            server_shutdown: None,
            nodelay: false,
            recv_buffer_size: 1024 * 8,
            send_buffer_size: 1024 * 8,
            keepalive: None,
            ttl: 30,
            linger: None,
        };
        let shared = sync::Arc::new(sync::Mutex::new(shared));
        let client = SimulatedTcpStream::new(
            client_addr,
            server_addr,
            sync::Arc::clone(&shared),
            client_link,
            fault_injector.clone(),
        );
        let server = SimulatedTcpStream::new(
            server_addr,
            client_addr,
            sync::Arc::clone(&shared),
            server_link,
            fault_injector.clone(),
        );
        (client, server)
    }

    pub fn local_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.local_addr)
    }

    pub fn peer_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.peer_addr)
    }

    pub fn poll_peek(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.link.poll_peek(cx, buf)
    }

    pub async fn peek(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        poll_fn(|cx| self.poll_peek(cx, buf)).await
    }

    pub fn shutdown(&self, how: net::Shutdown) -> io::Result<()> {
        if self.is_client() {
            self.shared.lock().unwrap().client_shutdown.replace(how);
        } else {
            self.shared.lock().unwrap().server_shutdown.replace(how);
        }
        Ok(())
    }

    pub fn nodelay(&self) -> io::Result<bool> {
        Ok(self.shared.lock().unwrap().nodelay)
    }

    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.shared.lock().unwrap().nodelay = nodelay;
        Ok(())
    }

    pub fn recv_buffer_size(&self) -> io::Result<usize> {
        Ok(self.shared.lock().unwrap().recv_buffer_size)
    }

    pub fn set_recv_buffer_size(&self, size: usize) -> io::Result<()> {
        self.shared.lock().unwrap().recv_buffer_size = size;
        Ok(())
    }

    pub fn send_buffer_size(&self) -> io::Result<usize> {
        Ok(self.shared.lock().unwrap().send_buffer_size)
    }

    pub fn set_send_buffer_size(&self, size: usize) -> io::Result<()> {
        self.shared.lock().unwrap().send_buffer_size = size;
        Ok(())
    }

    pub fn keepalive(&self) -> io::Result<Option<time::Duration>> {
        Ok(self.shared.lock().unwrap().keepalive)
    }

    pub fn set_keepalive(&self, keepalive: Option<time::Duration>) -> io::Result<()> {
        self.shared.lock().unwrap().keepalive = keepalive;
        Ok(())
    }

    pub fn ttl(&self) -> io::Result<u32> {
        Ok(self.shared.lock().unwrap().ttl)
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.shared.lock().unwrap().ttl = ttl;
        Ok(())
    }

    pub fn linger(&self) -> io::Result<Option<time::Duration>> {
        Ok(self.shared.lock().unwrap().linger)
    }

    pub fn set_linger(&self, dur: Option<time::Duration>) -> io::Result<()> {
        self.shared.lock().unwrap().linger = dur;
        Ok(())
    }
}

impl SimulatedTcpStream {
    fn is_client(&self) -> bool {
        self.shared.lock().unwrap().client_addr == self.local_addr
    }

    fn shutdown_status(&self) -> Option<net::Shutdown> {
        if self.is_client() {
            self.shared.lock().unwrap().client_shutdown
        } else {
            self.shared.lock().unwrap().server_shutdown
        }
    }
}

impl AsyncRead for SimulatedTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let from = self.local_addr;
        let to = self.peer_addr;
        if let Some(net::Shutdown::Read) = self.shutdown_status() {
            return Poll::Ready(Ok(0));
        }
        if let Some(ref mut fault_injector) = self.fault_injector {
            ready!(Pin::new(fault_injector).tcp_poll_read_delay(cx, from, to))?;
        }
        Pin::new(&mut self.link).poll_read(cx, buf)
    }
}

impl AsyncWrite for SimulatedTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let from = self.local_addr;
        let to = self.peer_addr;
        if let Some(net::Shutdown::Write) = self.shutdown_status() {
            return Poll::Ready(Err(io::ErrorKind::ConnectionAborted.into()));
        }
        if let Some(ref mut fault_injector) = self.fault_injector {
            ready!(Pin::new(fault_injector).tcp_poll_write_delay(cx, from, to))?;
        }
        Pin::new(&mut self.link).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.link).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.link).poll_shutdown(cx)
    }
}

use crate::fault::FaultInjector;
use pin_project::{pin_project, project};

#[pin_project]
#[derive(Debug)]
enum TcpStreamInner {
    Simulated(#[pin] SimulatedTcpStream),
    Tokio(#[pin] tokio::net::TcpStream),
}

#[pin_project]
#[derive(Debug)]
pub struct TcpStream {
    #[pin]
    inner: TcpStreamInner,
}

impl TcpStream {
    pub fn local_addr(&self) -> io::Result<net::SocketAddr> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.local_addr(),
            TcpStreamInner::Tokio(ref t) => t.local_addr(),
        }
    }

    pub fn peer_addr(&self) -> io::Result<net::SocketAddr> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.peer_addr(),
            TcpStreamInner::Tokio(ref t) => t.peer_addr(),
        }
    }

    pub fn poll_peek(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.inner {
            TcpStreamInner::Simulated(ref mut s) => s.poll_peek(cx, buf),
            TcpStreamInner::Tokio(ref mut t) => t.poll_peek(cx, buf),
        }
    }

    pub async fn peek(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        poll_fn(|cx| self.poll_peek(cx, buf)).await
    }

    pub fn shutdown(&self, how: net::Shutdown) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.shutdown(how),
            TcpStreamInner::Tokio(ref t) => t.shutdown(how),
        }
    }

    pub fn nodelay(&self) -> io::Result<bool> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.nodelay(),
            TcpStreamInner::Tokio(ref t) => t.nodelay(),
        }
    }

    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.set_nodelay(nodelay),
            TcpStreamInner::Tokio(ref t) => t.set_nodelay(nodelay),
        }
    }

    pub fn recv_buffer_size(&self) -> io::Result<usize> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.recv_buffer_size(),
            TcpStreamInner::Tokio(ref t) => t.recv_buffer_size(),
        }
    }

    pub fn set_recv_buffer_size(&self, size: usize) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.set_recv_buffer_size(size),
            TcpStreamInner::Tokio(ref t) => t.set_recv_buffer_size(size),
        }
    }

    pub fn send_buffer_size(&self) -> io::Result<usize> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.send_buffer_size(),
            TcpStreamInner::Tokio(ref t) => t.send_buffer_size(),
        }
    }

    pub fn set_send_buffer_size(&self, size: usize) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.set_send_buffer_size(size),
            TcpStreamInner::Tokio(ref t) => t.set_send_buffer_size(size),
        }
    }

    pub fn keepalive(&self) -> io::Result<Option<time::Duration>> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.keepalive(),
            TcpStreamInner::Tokio(ref t) => t.keepalive(),
        }
    }

    pub fn set_keepalive(&self, keepalive: Option<time::Duration>) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.set_keepalive(keepalive),
            TcpStreamInner::Tokio(ref t) => t.set_keepalive(keepalive),
        }
    }

    pub fn ttl(&self) -> io::Result<u32> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.ttl(),
            TcpStreamInner::Tokio(ref t) => t.ttl(),
        }
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.set_ttl(ttl),
            TcpStreamInner::Tokio(ref t) => t.set_ttl(ttl),
        }
    }

    pub fn linger(&self) -> io::Result<Option<time::Duration>> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.linger(),
            TcpStreamInner::Tokio(ref t) => t.linger(),
        }
    }

    /// Sets the linger duration of this socket by setting the `SO_LINGER`
    /// option.
    ///
    /// This option controls the action taken when a stream has unsent messages
    /// and the stream is closed. If `SO_LINGER` is set, the system
    /// shall block the process until it can transmit the data or until the
    /// time expires.
    ///
    /// If `SO_LINGER` is not specified, and the stream is closed, the system
    /// handles the call in a way that allows the process to continue as quickly
    /// as possible.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tokio::net::TcpStream;
    ///
    /// # async fn dox() -> Result<(), Box<dyn std::error::Error>> {
    /// let stream = TcpStream::connect("127.0.0.1:8080").await?;
    ///
    /// stream.set_linger(None)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_linger(&self, dur: Option<time::Duration>) -> io::Result<()> {
        match self.inner {
            TcpStreamInner::Simulated(ref s) => s.set_linger(dur),
            TcpStreamInner::Tokio(ref t) => t.set_linger(dur),
        }
    }
}

impl AsyncRead for TcpStreamInner {
    #[project]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        #[project]
        match self.project() {
            TcpStreamInner::Simulated(s) => s.poll_read(cx, buf),
            TcpStreamInner::Tokio(t) => t.poll_read(cx, buf),
        }
    }

    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [std::mem::MaybeUninit<u8>]) -> bool {
        match self {
            TcpStreamInner::Simulated(ref s) => s.prepare_uninitialized_buffer(buf),
            TcpStreamInner::Tokio(ref t) => t.prepare_uninitialized_buffer(buf),
        }
    }

    #[project]
    fn poll_read_buf<B: bytes::BufMut>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<io::Result<usize>>
    where
        Self: Sized,
    {
        #[project]
        match self.project() {
            TcpStreamInner::Simulated(s) => s.poll_read_buf(cx, buf),
            TcpStreamInner::Tokio(t) => t.poll_read_buf(cx, buf),
        }
    }
}

impl AsyncWrite for TcpStreamInner {
    #[project]
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        #[project]
        match self.project() {
            TcpStreamInner::Simulated(s) => s.poll_write(cx, buf),
            TcpStreamInner::Tokio(t) => t.poll_write(cx, buf),
        }
    }
    #[project]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        #[project]
        match self.project() {
            TcpStreamInner::Simulated(s) => s.poll_flush(cx),
            TcpStreamInner::Tokio(t) => t.poll_flush(cx),
        }
    }
    #[project]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        #[project]
        match self.project() {
            TcpStreamInner::Simulated(s) => s.poll_shutdown(cx),
            TcpStreamInner::Tokio(t) => t.poll_shutdown(cx),
        }
    }

    #[project]
    fn poll_write_buf<B: bytes::Buf>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<Result<usize, io::Error>>
    where
        Self: Sized,
    {
        #[project]
        match self.project() {
            TcpStreamInner::Simulated(s) => s.poll_write_buf(cx, buf),
            TcpStreamInner::Tokio(t) => t.poll_write_buf(cx, buf),
        }
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.project().inner.poll_read(cx, buf)
    }
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [std::mem::MaybeUninit<u8>]) -> bool {
        self.inner.prepare_uninitialized_buffer(buf)
    }
    fn poll_read_buf<B: bytes::BufMut>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<io::Result<usize>>
    where
        Self: Sized,
    {
        self.project().inner.poll_read_buf(cx, buf)
    }
}
impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        self.project().inner.poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.project().inner.poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.project().inner.poll_shutdown(cx)
    }
    fn poll_write_buf<B: bytes::Buf>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<Result<usize, io::Error>>
    where
        Self: Sized,
    {
        self.project().inner.poll_write_buf(cx, buf)
    }
}

impl From<SimulatedTcpStream> for TcpStream {
    fn from(item: SimulatedTcpStream) -> Self {
        TcpStream {
            inner: TcpStreamInner::Simulated(item),
        }
    }
}

impl From<tokio::net::TcpStream> for TcpStream {
    fn from(item: tokio::net::TcpStream) -> Self {
        TcpStream {
            inner: TcpStreamInner::Tokio(item),
        }
    }
}

use crate::runtime::deterministic::net::{MemoryTcpStream, ServerSocket};
use async_trait::async_trait;
use futures::{channel::mpsc, future::poll_fn, Poll, StreamExt};
use std::{io, net, net::SocketAddr, pin::Pin, task::Context};

/// An I/O object mocking a TCP socket listening for incoming connections.
///
/// MemoryListener is backed by a an in-memory network. New connections are
/// enqueued for the MemoryListener to process.
#[derive(Debug)]
pub struct MemoryListener {
    /// Incoming connections from the MemoryNetwork.
    new_sockets: mpsc::Receiver<ServerSocket>,
    /// The local address of this MemoryListener
    local_addr: net::SocketAddr,
    ttl: std::sync::atomic::AtomicU32,
}

impl MemoryListener {
    pub fn new(sockets_chan: mpsc::Receiver<ServerSocket>, addr: net::SocketAddr) -> Self {
        Self {
            new_sockets: sockets_chan,
            local_addr: addr,
            ttl: std::sync::atomic::AtomicU32::new(std::u32::MAX),
        }
    }
}

/// Stream returned by the `MemoryListener::incoming` function representing the
/// stream of sockets received from the listener.
#[must_use = "streams do nothing unless polled"]
#[derive(Debug)]
pub struct Incoming {
    inner: MemoryListener,
}

impl Incoming {
    pub(crate) fn new(listener: MemoryListener) -> Self {
        Self { inner: listener }
    }
}

impl futures::Stream for Incoming {
    type Item = io::Result<MemoryTcpStream<ServerSocket>>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let (sock, _) = futures::ready!(self.inner.poll_accept(cx))?;
        Poll::Ready(Some(Ok(sock)))
    }
}

impl MemoryListener {
    /// Accept a new incoming connection from this listener.
    ///
    /// The resulting `MemoryTcpStream` and remote peer's address will be returned.
    ///
    /// [`MemoryTcpStream`]: ../struct.MemoryTcpStream.html
    pub async fn accept(&mut self) -> io::Result<(MemoryTcpStream<ServerSocket>, SocketAddr)> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }

    pub(crate) fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<(MemoryTcpStream<ServerSocket>, SocketAddr)>> {
        if let Some(next) = futures::ready!(self.new_sockets.poll_next_unpin(cx)) {
            let addr = next.peer_addr();
            let stream = MemoryTcpStream::new_server(next);
            Poll::Ready(Ok((stream, addr)))
        } else {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
    }

    pub fn local_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.local_addr)
    }

    pub fn incoming(self) -> Incoming {
        Incoming::new(self)
    }
    pub fn ttl(&self) -> io::Result<u32> {
        return Ok(self.ttl.load(std::sync::atomic::Ordering::SeqCst));
    }
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.ttl.store(ttl, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl crate::TcpListener for MemoryListener {
    type Stream = MemoryTcpStream<ServerSocket>;
    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error> {
        MemoryListener::accept(self).await
    }

    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        MemoryListener::local_addr(self)
    }
    fn ttl(&self) -> io::Result<u32> {
        MemoryListener::ttl(self)
    }
    fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        MemoryListener::set_ttl(self, ttl)
    }
}

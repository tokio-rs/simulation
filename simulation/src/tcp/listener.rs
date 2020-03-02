use crate::tcp::TcpStream;
use futures::{future::poll_fn, ready, Future, Stream};
use std::{
    io, net,
    pin::Pin,
    sync,
    task::{Context, Poll},
};

use tokio::sync::mpsc;
/// Fault injection state for the TcpListener
#[derive(Debug)]
struct Shared {}
#[derive(Debug)]
pub struct TcpListener {
    local_addr: net::SocketAddr,
    incoming: mpsc::Receiver<TcpStream>,
    shared: sync::Arc<sync::Mutex<Shared>>,
}

impl Incoming<'_> {
    pub(crate) fn new(listener: &mut TcpListener) -> Incoming<'_> {
        Incoming { inner: listener }
    }

    #[doc(hidden)] // TODO: dox
    pub fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<TcpStream>> {
        let (socket, _) = ready!(self.inner.poll_accept(cx))?;
        Poll::Ready(Ok(socket))
    }
}

impl Stream for Incoming<'_> {
    type Item = io::Result<TcpStream>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let (socket, _) = ready!(self.inner.poll_accept(cx))?;
        Poll::Ready(Some(Ok(socket)))
    }
}

/// Stream returned by the `TcpListener::incoming` function representing the
/// stream of sockets received from a listener.
#[must_use = "streams do nothing unless polled"]
#[derive(Debug)]
pub struct Incoming<'a> {
    inner: &'a mut TcpListener,
}

impl TcpListener {
    pub(crate) fn new(local_addr: net::SocketAddr) -> (Self, TcpListenerHandle) {
        let (tx, rx) = mpsc::channel(1);
        let shared = Shared {};
        let shared = sync::Arc::new(sync::Mutex::new(shared));
        let weak = sync::Arc::downgrade(&shared);
        (
            Self {
                local_addr,
                shared,
                incoming: rx,
            },
            TcpListenerHandle {
                shared: weak,
                sender: tx,
            },
        )
    }

    pub fn local_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.local_addr)
    }

    pub async fn accept(&mut self) -> io::Result<(TcpStream, net::SocketAddr)> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }

    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(TcpStream, net::SocketAddr), io::Error>> {
        let stream =
            ready!(Pin::new(&mut self.incoming).poll_next(cx)).ok_or(io::ErrorKind::NotFound)?;
        let addr = stream.peer_addr()?;
        Poll::Ready(Ok((stream, addr)))
    }

    pub fn incoming(&mut self) -> Incoming<'_> {
        Incoming::new(self)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        Ok(0)
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct TcpListenerHandle {
    shared: sync::Weak<sync::Mutex<Shared>>,
    sender: mpsc::Sender<TcpStream>,
}

impl TcpListenerHandle {
    pub fn dropped(&self) -> bool {
        self.shared.upgrade().is_none()
    }

    pub fn poll_enqueue_incoming(
        &mut self,
        cx: &mut Context<'_>,
        stream: TcpStream,
    ) -> Poll<Result<(), io::Error>> {
        let future = self.sender.send(stream);
        futures::pin_mut!(future);
        ready!(future.poll(cx)).map_err(|_| io::ErrorKind::ConnectionRefused)?;
        Poll::Ready(Ok(()))
    }
}

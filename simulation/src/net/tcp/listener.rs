use super::{SimulatedTcpStream, TcpStream};
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
struct Shared {
    ttl: u32,
}
#[derive(Debug)]
pub struct SimulatedTcpListener {
    local_addr: net::SocketAddr,
    incoming: mpsc::Receiver<SimulatedTcpStream>,
    shared: sync::Arc<sync::Mutex<Shared>>,
}

impl Incoming<'_> {
    pub(crate) fn new(listener: &mut SimulatedTcpListener) -> Incoming<'_> {
        Incoming { inner: listener }
    }

    #[doc(hidden)] // TODO: dox
    pub fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<SimulatedTcpStream>> {
        let (socket, _) = ready!(self.inner.poll_accept(cx))?;
        Poll::Ready(Ok(socket))
    }
}

impl Stream for Incoming<'_> {
    type Item = io::Result<SimulatedTcpStream>;

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
    inner: &'a mut SimulatedTcpListener,
}

impl SimulatedTcpListener {
    pub(crate) fn new(local_addr: net::SocketAddr) -> (Self, SimulatedTcpListenerHandle) {
        let (tx, rx) = mpsc::channel(1);
        let shared = Shared { ttl: 0 };
        let shared = sync::Arc::new(sync::Mutex::new(shared));
        let weak = sync::Arc::downgrade(&shared);
        (
            Self {
                local_addr,
                shared,
                incoming: rx,
            },
            SimulatedTcpListenerHandle {
                shared: weak,
                sender: tx,
            },
        )
    }

    pub(crate) fn local_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.local_addr)
    }

    pub(crate) async fn accept(&mut self) -> io::Result<(SimulatedTcpStream, net::SocketAddr)> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }

    pub(crate) fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(SimulatedTcpStream, net::SocketAddr), io::Error>> {
        let stream =
            ready!(Pin::new(&mut self.incoming).poll_next(cx)).ok_or(io::ErrorKind::NotFound)?;
        let addr = stream.peer_addr()?;
        Poll::Ready(Ok((stream, addr)))
    }

    pub(crate) fn incoming(&mut self) -> Incoming<'_> {
        Incoming::new(self)
    }

    pub(crate) fn ttl(&self) -> io::Result<u32> {
        Ok(self.shared.lock().unwrap().ttl)
    }

    pub(crate) fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.shared.lock().unwrap().ttl = ttl;
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub(crate) struct SimulatedTcpListenerHandle {
    shared: sync::Weak<sync::Mutex<Shared>>,
    sender: mpsc::Sender<SimulatedTcpStream>,
}

impl SimulatedTcpListenerHandle {
    pub(crate) fn dropped(&self) -> bool {
        self.shared.upgrade().is_none()
    }

    pub(crate) fn poll_enqueue_incoming(
        &mut self,
        cx: &mut Context<'_>,
        stream: SimulatedTcpStream,
    ) -> Poll<Result<(), io::Error>> {
        let future = self.sender.send(stream);
        futures::pin_mut!(future);
        ready!(future.poll(cx)).map_err(|_| io::ErrorKind::ConnectionRefused)?;
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug)]
enum TcpListenerInner {
    Simulated(SimulatedTcpListener),
    Tokio(tokio::net::TcpListener),
}
pub struct TcpListener {
    inner: TcpListenerInner,
}

impl TcpListener {
    pub(crate) fn local_addr(&self) -> io::Result<net::SocketAddr> {
        match self.inner {
            TcpListenerInner::Simulated(ref s) => s.local_addr(),
            TcpListenerInner::Tokio(ref t) => t.local_addr(),
        }
    }

    pub(crate) async fn accept(&mut self) -> io::Result<(TcpStream, net::SocketAddr)> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }

    pub(crate) fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(TcpStream, net::SocketAddr), io::Error>> {
        match self.inner {
            TcpListenerInner::Simulated(ref mut s) => {
                let stream = ready!(s.poll_accept(cx));
                match stream {
                    Ok((stream, addr)) => Poll::Ready(Ok((stream.into(), addr))),
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
            TcpListenerInner::Tokio(ref mut t) => {
                let stream = ready!(t.poll_accept(cx));
                match stream {
                    Ok((stream, addr)) => Poll::Ready(Ok((stream.into(), addr))),
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
        }
    }

    pub(crate) fn ttl(&self) -> io::Result<u32> {
        match self.inner {
            TcpListenerInner::Simulated(ref s) => s.ttl(),
            TcpListenerInner::Tokio(ref t) => t.ttl(),
        }
    }

    pub(crate) fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        match self.inner {
            TcpListenerInner::Simulated(ref s) => s.set_ttl(ttl),
            TcpListenerInner::Tokio(ref t) => t.set_ttl(ttl),
        }
    }
}

impl From<SimulatedTcpListener> for TcpListener {
    fn from(item: SimulatedTcpListener) -> Self {
        TcpListener {
            inner: TcpListenerInner::Simulated(item),
        }
    }
}

impl From<tokio::net::TcpListener> for TcpListener {
    fn from(item: tokio::net::TcpListener) -> Self {
        TcpListener {
            inner: TcpListenerInner::Tokio(item),
        }
    }
}

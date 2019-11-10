use async_trait::async_trait;
use std::{io, net, time};
use futures::Future;
use tokio::io::{AsyncRead, AsyncWrite};

pub trait TcpStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {
    fn local_addr(&self) -> io::Result<net::SocketAddr>;
    fn peer_addr(&self) -> io::Result<net::SocketAddr>;
}

#[async_trait]
pub trait TcpListener {
    type Stream: TcpStream + Send + 'static;

    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error>;

    fn local_addr(&self) -> Result<net::SocketAddr, io::Error>;

    fn ttl(&self) -> io::Result<u32>;

    fn set_ttl(&self, ttl: u32) -> io::Result<()>;
}

#[async_trait]
pub trait Network {
    type TcpStream: TcpStream + Send + 'static + Unpin;
    type TcpListener: TcpListener + Send + 'static + Unpin;

    async fn bind<A>(&self, addr: A) -> io::Result<Self::TcpListener>
    where
        A: Into<net::SocketAddr> + Send + Sync;

    async fn connect<A>(&self, addr: A) -> io::Result<Self::TcpStream>
    where
        A: Into<net::SocketAddr> + Send + Sync;
}

pub trait Scheduler {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static;
    /// Return the time now according to the executor.
    fn now(&self) -> time::Instant;
    /// Returns a delay future which completes after the provided instant.
    fn delay(&self, deadline: time::Instant) -> tokio_timer::Delay;
    /// Returns a delay future which completes at some time from now.
    fn delay_from(&self, from_now: time::Duration) -> tokio_timer::Delay {
        let now = self.now();
        self.delay(now + from_now)
    }
    /// Creates a timeout future which which will execute T until the timeout elapses.
    fn timeout<T>(&self, value: T, timeout: time::Duration) -> tokio_timer::Timeout<T>;
}

pub trait Platform: Unpin + Sized + Clone + Send + Scheduler + Network + 'static {}
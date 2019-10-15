//! This crate provides an abstraction over the [tokio] [CurrentThread] runtime
//! which allows for simulating applications.
//!
//! The [Environment] trait provides an abstraction over [Delay] and [Timeout].
//! This allows for applications to be generic over [DeterministicRuntime] or
//! [SingleThreadedRuntime].
//!
//! [DeterministicRuntime] will automatically advance a mocked clock if there is
//! no more work to do, up until the next timeout. This results in applications which
//! can be decoupled from time, facilitating fast integration/simulation tests.
//!
//! ```rust
//! use std::time;
//! use simulation::{DeterministicRuntime, Environment};
//!
//! async fn delayed<E>(handle: E) where E: Environment {
//!    let start_time = handle.now();
//!    handle.delay_from(time::Duration::from_secs(30)).await;
//!    println!("that was fast!");
//!    assert_eq!(handle.now(), start_time + time::Duration::from_secs(30));
//! }
//!
//! #[test]
//! fn time() {
//!     let mut runtime = DeterministicRuntime::new();
//!     let handle = runtime.handle();
//!     runtime.block_on(async move {
//!         delayed(handle).await;
//!     });
//! }
//! ```
//!
//! [tokio]: https://github.com/tokio-rs
//! [CurrentThread]:[tokio_executor::current_thread::CurrentThread]
//! [Delay]:[tokio_timer::Delay]
//! [Timeout]:[tokio_timer::Timeout]

use async_trait::async_trait;
use futures::{channel::oneshot, Future, FutureExt, Poll};
use std::{io, net, pin::Pin, task::Context, time};
mod runtime;
pub(crate) use runtime::DeterministicRuntimeSchedulerRng;
pub use runtime::{
    DeterministicRuntime, DeterministicRuntimeHandle, SingleThreadedRuntime,
    SingleThreadedRuntimeHandle,
};
use tokio::io::{AsyncRead, AsyncWrite};

#[async_trait]
pub trait Environment: Unpin + Sized + Clone + Send {
    type TcpStream: TcpStream + Send + 'static;
    type TcpListener: TcpListener + Send + 'static;

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
    /// Creates a timeout future which will execute blah blah
    fn timeout<T>(&self, value: T, timeout: time::Duration) -> tokio_timer::Timeout<T>;

    async fn bind<'a, A>(&'a self, addr: A) -> Result<Self::TcpListener, io::Error>
    where
        A: Into<net::SocketAddr> + Send + Sync;
    async fn connect<'a, A>(&'a self, addr: A) -> Result<Self::TcpStream, io::Error>
    where
        A: Into<net::SocketAddr> + Send + Sync;
}

pub trait TcpStream: AsyncRead + AsyncWrite + Unpin {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error>;
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error>;
    fn shutdown(&self) -> Result<(), io::Error>;
}

pub trait TcpListener {
    type Stream: TcpStream + Send;
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error>;
    fn poll_accept(&mut self, cx: &mut Context<'_>) -> Poll<Result<Self::Stream, io::Error>>;

    fn accept<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Stream, io::Error>> + 'a + Send>>
    where
        Self: Send,
    {
        let fut = futures::future::poll_fn(move |cx| self.poll_accept(cx));
        return Pin::from(Box::new(fut));
    }
}

pub fn spawn_with_result<F, E, U>(env: &E, future: F) -> impl Future<Output = Option<U>>
where
    F: Future<Output = U> + Send + 'static,
    U: Send + 'static,
    E: Environment,
{
    let (tx, rx) = oneshot::channel();
    env.spawn(async {
        tx.send(future.await).unwrap_or(());
    });
    Box::new(rx).map(|v| v.ok())
}

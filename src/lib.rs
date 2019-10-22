#![allow(dead_code)]
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
//! Simulation additionally provides facilities for networking and fault injection.
//!
//! ```rust
//! use crate::{Environment, TcpListener};
//!    use futures::{SinkExt, StreamExt};
//!    use std::{io, net, time};
//!    use tokio::codec::{Framed, LinesCodec};
//!
//!    /// Start a client request handler which will write greetings to clients.
//!    async fn handle<E>(env: E, socket: <E::TcpListener as TcpListener>::Stream, addr: net::SocketAddr)
//!    where
//!        E: Environment,
//!    {
//!        // delay the response, in deterministic mode this will immediately progress time.
//!        env.delay_from(time::Duration::from_secs(1));
//!        println!("handling connection from {:?}", addr);
//!        let mut transport = Framed::new(socket, LinesCodec::new());
//!        if let Err(e) = transport.send(String::from("Hello World!")).await {
//!            println!("failed to send response: {:?}", e);
//!        }
//!    }
//!
//!    /// Start a server which will bind to the provided addr and repyl to clients.
//!    async fn server<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
//!    where
//!        E: Environment,
//!    {
//!        let mut listener = env.bind(addr).await?;
//!
//!        while let Ok((socket, addr)) = listener.accept().await {
//!            let request = handle(env.clone(), socket, addr);
//!            env.spawn(request)
//!        }
//!        Ok(())
//!    }
//!
//!
//!    /// Create a client which will read a message from the server
//!    async fn client<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
//!    where
//!        E: Environment,
//!    {
//!        let conn = env.connect(addr).await?;
//!        let mut transport = Framed::new(conn, LinesCodec::new());
//!        let result = transport.next().await.unwrap().unwrap();
//!        assert_eq!(result, "Hello world!");
//!        Ok(())
//!    }
//! ```
//!
//! [tokio]: https://github.com/tokio-rs
//! [CurrentThread]:[tokio_executor::current_thread::CurrentThread]
//! [Delay]:[tokio_timer::Delay]
//! [Timeout]:[tokio_timer::Timeout]

use async_trait::async_trait;
use futures::{Future, FutureExt};
use std::{io, net, time};
use tokio::io::{AsyncRead, AsyncWrite};

pub mod deterministic;
pub mod singlethread;

mod example {
    use crate::{Environment, TcpListener};
    use futures::{SinkExt, StreamExt};
    use std::{io, net, time};
    use tokio::codec::{Framed, LinesCodec};

    /// Start a client request handler which will write greetings to clients.
    async fn handle<E>(env: E, socket: <E::TcpListener as TcpListener>::Stream, addr: net::SocketAddr)
    where
        E: Environment,
    {
        // delay the response, in deterministic mode this will immediately progress time.
        env.delay_from(time::Duration::from_secs(1));
        println!("handling connection from {:?}", addr);
        let mut transport = Framed::new(socket, LinesCodec::new());
        if let Err(e) = transport.send(String::from("Hello World!")).await {
            println!("failed to send response: {:?}", e);
        }
    }

    /// Start a server which will bind to the provided addr and repyl to clients.
    async fn server<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
    where
        E: Environment,
    {
        let mut listener = env.bind(addr).await?;

        while let Ok((socket, addr)) = listener.accept().await {
            let request = handle(env.clone(), socket, addr);
            env.spawn(request)
        }
        Ok(())
    }


    /// Create a client which will read a message from the server
    async fn client<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
    where
        E: Environment,
    {
        let conn = env.connect(addr).await?;
        let mut transport = Framed::new(conn, LinesCodec::new());
        let result = transport.next().await.unwrap().unwrap();
        assert_eq!(result, "Hello world!");
        Ok(())
    }

    fn main() {
        // Runtimes, networking and fault injection can be swapped out.
        // let mut runtime = crate::singlethread::SingleThreadedRuntime::new().unwrap();
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        let addr: net::SocketAddr = "127.0.0.1:9090".parse().unwrap();
        runtime.block_on(server(handle, addr)).unwrap();
    }
}

#[derive(Debug)]
pub enum Error {
    Spawn {
        source: tokio_executor::SpawnError,
    },
    RuntimeBuild {
        source: io::Error,
    },
    CurrentThreadRun {
        source: tokio_executor::current_thread::RunError,
    },
}

#[async_trait]
pub trait Environment: Unpin + Sized + Clone + Send + 'static {
    type TcpStream: TcpStream + Send + 'static + Unpin;
    type TcpListener: TcpListener + Send + 'static + Unpin;

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

    async fn bind<'a, A>(&'a self, addr: A) -> io::Result<Self::TcpListener>
    where
        A: Into<net::SocketAddr> + Send + Sync;
    async fn connect<'a, A>(&'a self, addr: A) -> io::Result<Self::TcpStream>
    where
        A: Into<net::SocketAddr> + Send + Sync;
}

pub trait TcpStream: AsyncRead + AsyncWrite + Unpin {
    fn local_addr(&self) -> io::Result<net::SocketAddr>;
    fn peer_addr(&self) -> io::Result<net::SocketAddr>;
    fn shutdown(&self) -> io::Result<()>;
}

#[async_trait]
pub trait TcpListener {
    type Stream: TcpStream + Send;
    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error>;
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error>;
    fn ttl(&self) -> io::Result<u32>;
    fn set_ttl(&self, ttl: u32) -> io::Result<()>;
}

pub fn spawn_with_result<F, E, U>(env: &E, future: F) -> impl Future<Output = U>
where
    F: Future<Output = U> + Send + 'static,
    U: Send + 'static,
    E: Environment,
{
    let (remote, handle) = future.remote_handle();
    env.spawn(remote);
    Box::new(handle)
}

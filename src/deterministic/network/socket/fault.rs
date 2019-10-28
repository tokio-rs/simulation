//! Facilities for injecting faults into [`TcpStream`]'s and the in memory
//! network.

use crate::{deterministic::DeterministicRuntimeHandle, Environment, TcpStream};
use futures::{FutureExt, Poll, Stream, StreamExt};
use std::time;
use std::{io, net, pin::Pin, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::timer::Delay;

#[derive(Debug, Clone)]
pub enum Fault {
    Delay(time::Duration),
    Disconnect,
}

pub struct FaultyTcpStream<T> {
    handle: DeterministicRuntimeHandle,
    disconnected: bool,
    delay: Option<Delay>,
    inner: T,
    faults: Pin<Box<dyn Stream<Item = Fault> + Send + 'static>>,
}

impl<T> FaultyTcpStream<T> {
    /// Wrap the provided TcpStream with fault injection support. Calls to poll_* will
    /// first attempt to inject a fault supplied by fault_stream.
    pub fn wrap(
        handle: DeterministicRuntimeHandle,
        fault_stream: Pin<Box<dyn Stream<Item = Fault> + Send + 'static>>,
        inner: T,
    ) -> FaultyTcpStream<T> {
        FaultyTcpStream {
            handle,
            disconnected: false,
            delay: None,
            inner,
            faults: fault_stream,
        }
    }

    /// Poll for either a delay or a disconnect fault. Returns Ok(()) if there is no fault.
    fn poll_fault(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        loop {
            if self.disconnected {
                return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
            }
            // if there is a delay set, poll it until the delay elapses.
            if let Some(ref mut delay) = self.delay {
                futures::ready!(delay.poll_unpin(cx));
                self.delay.take();
                return Poll::Ready(Ok(()));
            }
            // check if there are any new faults to inject
            match self.faults.poll_next_unpin(cx) {
                Poll::Ready(Some(Fault::Delay(duration))) => {
                    let new_delay = self.handle.delay_from(duration);
                    self.delay.replace(new_delay);
                }
                Poll::Ready(Some(Fault::Disconnect)) => {
                    self.disconnected = true;
                }
                _ => return Poll::Ready(Ok(())),
            }
        }
    }
}

impl<T> AsyncRead for FaultyTcpStream<T>
where
    T: TcpStream,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(e) = futures::ready!(self.poll_fault(cx)) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T> AsyncWrite for FaultyTcpStream<T>
where
    T: TcpStream,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if let Err(e) = futures::ready!(self.poll_fault(cx)) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if let Err(e) = futures::ready!(self.poll_fault(cx)) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        if let Err(e) = futures::ready!(self.poll_fault(cx)) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<T> TcpStream for FaultyTcpStream<T>
where
    T: TcpStream,
{
    fn local_addr(&self) -> io::Result<net::SocketAddr> {
        T::local_addr(&self.inner)
    }
    fn peer_addr(&self) -> io::Result<net::SocketAddr> {
        T::peer_addr(&self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deterministic::network;
    use crate::Environment;
    use futures::channel::mpsc;
    use futures::{stream, SinkExt, StreamExt};
    use std::time;
    use tokio::codec::{Framed, LinesCodec};

    #[test]
    /// Test that injecting delay and disconnect faults causes the socket to delay and disconnect reads.
    fn faults() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let server_addr = "127.0.0.1:9092".parse().unwrap();
            let client_addr = "127.0.0.1:35255".parse().unwrap();
            let (client_conn, server_conn) = network::new_socket_pair(client_addr, server_addr);
            let fault_stream = Box::pin(stream::iter(vec![
                Fault::Delay(time::Duration::from_secs(10)),
                Fault::Disconnect,
            ]));
            let client_conn = FaultyTcpStream::wrap(handle.clone(), fault_stream, client_conn);

            // spawn a server future which returns a message
            handle.spawn(async move {
                let mut transport = Framed::new(server_conn, LinesCodec::new());
                while let Ok(_) = transport.send(String::from("Hello Future!")).await {}
            });

            let mut transport = Framed::new(client_conn, LinesCodec::new());
            let start_time = handle.now();
            let result = transport.next().await.unwrap().unwrap();
            assert_eq!(result, String::from("Hello Future!"));

            let elapsed = handle.now() - start_time;
            assert!(elapsed >= time::Duration::from_secs(10));

            let result = transport.next().await.unwrap();
            assert!(
                result.is_err(),
                "expected final stream item to cause a disconnect"
            );
        });
    }

    #[test]
    /// Test that injecting no faults allows the socket to behave normally.
    fn inactive_faults() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let server_addr = "127.0.0.1:9092".parse().unwrap();
            let client_addr = "127.0.0.1:35255".parse().unwrap();
            let (client_conn, server_conn) = network::new_socket_pair(client_addr, server_addr);

            let (_fault_tx, fault_rx) = mpsc::channel(1);
            let client_conn =
                FaultyTcpStream::wrap(handle.clone(), Box::pin(fault_rx), client_conn);
            // spawn a server future which returns a message
            handle.spawn(async move {
                let mut transport = Framed::new(server_conn, LinesCodec::new());
                while let Ok(_) = transport.send(String::from("Hello Future!")).await {}
            });
            let mut transport = Framed::new(client_conn, LinesCodec::new());
            let result = transport.next().await.unwrap().unwrap();
            assert_eq!(result, String::from("Hello Future!"));
        });
    }

    #[test]
    /// Test that injecting a disconnect fault unblocks poll.
    fn disconnect_unblocks() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let server_addr = "127.0.0.1:9092".parse().unwrap();
            let client_addr = "127.0.0.1:35255".parse().unwrap();
            // need to keep _server_conn in scope so that actual disconnects due to drop are not confused with injected ones
            let (client_conn, _server_conn) = network::new_socket_pair(client_addr, server_addr);
            let (mut fault_tx, fault_rx) = mpsc::channel(1);
            let client_conn =
                FaultyTcpStream::wrap(handle.clone(), Box::pin(fault_rx), client_conn);

            let mut transport = Framed::new(client_conn, LinesCodec::new());
            // ensure the transport returns nothing
            tokio_test::assert_pending!(futures::poll!(transport.next()));
            // spawn a future to inject a disconnect fault.
            handle.spawn(async move {
                fault_tx.send(Fault::Disconnect).await.unwrap();
            });
            let result = transport.next().await.unwrap();
            assert!(
                result.is_err(),
                "expected future to resolve in disconnect error"
            );
        });
    }
}

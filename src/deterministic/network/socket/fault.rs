//! Fault injection for AsyncRead/AsyncWrite types.

use crate::TcpStream;
use futures::{task::Waker, FutureExt, Poll};
use std::time;
use std::{io, net, pin::Pin, sync, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::timer::Delay;

#[derive(Debug)]
struct FaultState {
    send_latency: time::Duration,
    send_delay: Delay,
    receive_latency: time::Duration,
    receive_delay: Delay,
    send_clogged: bool,
    send_waker: Option<Waker>,
    receive_clogged: bool,
    receive_waker: Option<Waker>,
    disconnected: bool,
}

#[derive(Debug, Clone)]
pub struct FaultyTcpStreamHandle {
    inner: sync::Arc<sync::Mutex<FaultState>>,
}

impl FaultyTcpStreamHandle {
    pub fn is_dropped(&self) -> bool {
        sync::Arc::strong_count(&self.inner) <= 1
    }
    pub fn disconnect(&self) {
        self.inner.lock().unwrap().disconnected = true;
    }
    pub fn set_send_latency(&self, duration: time::Duration) {
        self.inner.lock().unwrap().send_latency = duration;
    }
    pub fn set_receive_latency(&self, duration: time::Duration) {
        self.inner.lock().unwrap().receive_latency = duration;
    }

    pub fn is_fully_clogged(&self) -> bool {
        let lock = self.inner.lock().unwrap();
        lock.send_clogged || lock.receive_clogged
    }

    pub fn clog_sends(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.send_clogged = true;
        if let Some(v) = lock.send_waker.take() {
            v.wake()
        }
    }
    pub fn clog_receives(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.receive_clogged = true;
        if let Some(v) = lock.receive_waker.take() {
            v.wake()
        }
    }
    pub fn unclog_sends(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.send_clogged = false;
        if let Some(v) = lock.send_waker.take() {
            v.wake()
        }
    }
    pub fn unclog_receives(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.receive_clogged = false;
        if let Some(v) = lock.receive_waker.take() {
            v.wake()
        }
    }
}

#[derive(Debug)]
pub struct FaultyTcpStream<T> {
    handle: crate::deterministic::DeterministicTimeHandle,
    inner: T,
    fault_state: sync::Arc<sync::Mutex<FaultState>>,
}

impl<T> FaultyTcpStream<T> {
    /// Wrap the provided TcpStream with fault injection support. Calls to poll_* will
    /// first attempt to inject a fault supplied by fault_stream.
    pub fn wrap(
        handle: crate::deterministic::DeterministicTimeHandle,
        inner: T,
    ) -> (FaultyTcpStream<T>, FaultyTcpStreamHandle) {
        let send_latency = time::Duration::from_millis(0);
        let send_delay = handle.delay_from(send_latency);
        let receive_latency = time::Duration::from_millis(0);
        let receive_delay = handle.delay_from(send_latency);
        let fault_state = FaultState {
            send_latency,
            send_delay,
            receive_latency,
            receive_delay,
            send_clogged: false,
            send_waker: None,
            receive_clogged: false,
            receive_waker: None,
            disconnected: false,
        };
        let fault_state = sync::Arc::new(sync::Mutex::new(fault_state));

        let wrapped_stream = FaultyTcpStream {
            handle,
            inner,
            fault_state: sync::Arc::clone(&fault_state),
        };
        let handle = FaultyTcpStreamHandle {
            inner: sync::Arc::clone(&fault_state),
        };
        (wrapped_stream, handle)
    }

    fn poll_send_delay(&self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let mut lock = self.fault_state.lock().unwrap();
        let send_latency = lock.send_latency;
        if lock.disconnected {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        // If sends are clogged, register a waker to be notified when sends are unclogged
        // and return pending.
        if lock.send_clogged {
            lock.send_waker.replace(cx.waker().clone());
            return Poll::Pending;
        }
        // Poll the send latency future until it passes. Once it passes, reset the delay to ensure
        // that future calls to poll_send_delay also reflect the latency.
        let deadline = lock.send_delay.deadline();
        futures::ready!(lock.send_delay.poll_unpin(cx));
        lock.send_delay.reset(deadline + send_latency);
        // since the latency delay has elapsed, the socket is not disconnected, and it's not clogged, we can
        // return Ready.
        Poll::Ready(Ok(()))
    }

    fn poll_receive_delay(&self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let mut lock = self.fault_state.lock().unwrap();
        let receive_latency = lock.receive_latency;
        if lock.disconnected {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        // If receives are clogged, register a waker to be notified when receives are unclogged
        // and return pending.
        if lock.receive_clogged {
            lock.receive_waker.replace(cx.waker().clone());
            return Poll::Pending;
        }
        // Poll the receive latency future until it passes. Once it passes, reset the delay to ensure
        // that future calls to poll_receive_delay also reflect the latency.
        let deadline = lock.receive_delay.deadline();
        futures::ready!(lock.receive_delay.poll_unpin(cx));
        lock.receive_delay.reset(deadline + receive_latency);
        // since the latency delay has elapsed, the socket is not disconnected, and it's not clogged, we can
        // return Ready.
        Poll::Ready(Ok(()))
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
        if let Err(e) = futures::ready!(self.poll_receive_delay(cx)) {
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
        if let Err(e) = futures::ready!(self.poll_send_delay(cx)) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if let Err(e) = futures::ready!(self.poll_send_delay(cx)) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        if let Err(e) = futures::ready!(self.poll_send_delay(cx)) {
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
    use crate::deterministic::network::socket::new_socket_pair;
    use crate::Environment;

    use futures::{SinkExt, StreamExt};
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
            let (client_conn, server_conn) = new_socket_pair(client_addr, server_addr);
            let (client_conn, client_handle) =
                FaultyTcpStream::wrap(handle.time_handle(), client_conn);
            client_handle.set_receive_latency(time::Duration::from_secs(10));

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
            client_handle.disconnect();

            let result = transport.next().await.unwrap();
            assert!(
                result.is_err(),
                "expected final stream item to cause a disconnect"
            );
        });
    }

    #[test]
    /// tests that send and receives can be clogged/unclogged
    #[allow(unused_must_use)]
    fn clogging() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let server_addr = "127.0.0.1:9092".parse().unwrap();
            let client_addr = "127.0.0.1:35255".parse().unwrap();
            let (client_conn, server_conn) = new_socket_pair(client_addr, server_addr);
            let (client_conn, client_handle) =
                FaultyTcpStream::wrap(handle.time_handle(), client_conn);
            // clog both sends and receives
            client_handle.clog_receives();
            client_handle.clog_sends();

            // spawn a server future which returns a message when it receives a message
            handle.spawn(async move {
                let mut transport = Framed::new(server_conn, LinesCodec::new());
                while let Some(Ok(_)) = transport.next().await {
                    transport.send(String::from("Hello Future!")).await.unwrap();
                }
            });

            let mut transport = Framed::new(client_conn, LinesCodec::new());
            let send = transport.send(String::from("ping"));
            futures::pin_mut!(send);
            tokio_test::assert_pending!(
                futures::poll!(send.as_mut()),
                "expected clogged socket to be pending"
            );
            client_handle.unclog_sends();
            tokio_test::assert_ready!(
                futures::poll!(send),
                "expected socket send to be ready after unclogging sends"
            );

            let receive = transport.next();
            futures::pin_mut!(receive);
            tokio_test::assert_pending!(
                futures::poll!(receive.as_mut()),
                "expected clogged socket to be pending"
            );
            client_handle.unclog_receives();
            let result = receive.await.unwrap().unwrap();
            assert_eq!(result, String::from("Hello Future!"));
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
            let (client_conn, server_conn) = new_socket_pair(client_addr, server_addr);
            let (client_conn, _) = FaultyTcpStream::wrap(handle.time_handle(), client_conn);
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
            let (client_conn, _server_conn) = new_socket_pair(client_addr, server_addr);
            let (client_conn, client_handle) =
                FaultyTcpStream::wrap(handle.time_handle(), client_conn);

            let mut transport = Framed::new(client_conn, LinesCodec::new());
            // ensure the transport returns nothing
            tokio_test::assert_pending!(futures::poll!(transport.next()));
            // spawn a future to inject a disconnect fault.
            client_handle.disconnect();
            let result = transport.next().await.unwrap();
            assert!(
                result.is_err(),
                "expected future to resolve in disconnect error"
            );
        });
    }
}

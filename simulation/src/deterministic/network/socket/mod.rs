use bytes::{Buf, Bytes, IntoBuf};
use futures::{channel::mpsc, Future, Poll, Sink, SinkExt, Stream};
use std::{fmt, io, net, pin::Pin, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
pub mod fault;
pub use fault::{FaultyTcpStream, FaultyTcpStreamHandle};
use tracing::{span, trace, Level};

/// Returns a client/server socket pair, along with a SocketHandle which can be used to close
/// either side of the socket halfs.
pub fn new_socket_pair(
    client_addr: net::SocketAddr,
    server_addr: net::SocketAddr,
) -> (SocketHalf, SocketHalf) {
    let (client_tx, client_rx) = mpsc::channel(8);
    let (server_tx, server_rx) = mpsc::channel(8);
    let client_socket = SocketHalf::new(client_addr, server_addr, client_tx, server_rx);
    let server_socket = SocketHalf::new(server_addr, client_addr, server_tx, client_rx);
    (client_socket, server_socket)
}

pub struct SocketHalf {
    tx: mpsc::Sender<Bytes>,
    rx: mpsc::Receiver<Bytes>,
    staged: Option<Bytes>,
    shutdown: bool,
    local_addr: net::SocketAddr,
    peer_addr: net::SocketAddr,
}

impl fmt::Debug for SocketHalf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SocketHalf {{ local_addr: {}, peer_addr: {}, shutdown: {}, staged: {:?} }}",
            self.local_addr,
            self.peer_addr,
            self.shutdown,
            self.staged.as_ref().map(|b| b.len())
        )
    }
}

impl SocketHalf {
    fn new(
        local_addr: net::SocketAddr,
        peer_addr: net::SocketAddr,
        tx: mpsc::Sender<Bytes>,
        rx: mpsc::Receiver<Bytes>,
    ) -> Self {
        Self {
            tx,
            rx,
            staged: None,
            shutdown: false,
            local_addr,
            peer_addr,
        }
    }
    pub fn local_addr(&self) -> net::SocketAddr {
        self.local_addr
    }
    pub fn peer_addr(&self) -> net::SocketAddr {
        self.peer_addr
    }
    pub(crate) fn connected(&self) -> bool {
        !self.tx.is_closed()
    }
    /// Attempt to read any staged bytes into `dst`. Returns the number of bytes read, or None if
    /// no bytes were staged.
    fn read_staged(&mut self, dst: &mut [u8]) -> Option<usize> {
        if let Some(mut bytes) = self.staged.take() {
            debug_assert!(!bytes.is_empty(), "staged bytes should not be empty");
            let to_write = std::cmp::min(dst.len(), bytes.len());
            let b = bytes.split_to(to_write);
            let mut b = b.into_buf();
            b.copy_to_slice(&mut dst[..to_write]);
            if !bytes.is_empty() {
                self.staged.replace(bytes);
            }
            Some(to_write)
        } else {
            None
        }
    }
}

impl AsyncRead for SocketHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dst: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // span! macro seems to trip up clippy here
        #![allow(clippy::cognitive_complexity)]
        span!(Level::TRACE, "AsyncRead::poll_read", "{:?}", self).in_scope(|| loop {
            trace!("attempting to read {} bytes", dst.len());
            if let Some(bytes_read) = self.read_staged(dst) {
                trace!("read {} bytes", bytes_read);
                return Poll::Ready(Ok(bytes_read));
            }

            trace!("no bytes staged");
            let stream = Pin::new(&mut self.rx);
            match futures::ready!(stream.poll_next(cx)) {
                Some(new_bytes) => {
                    trace!("found staged bytes");
                    self.staged.replace(new_bytes)
                }
                None => {
                    trace!("socket disconnected");
                    return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
                }
            };
        })
    }
}

impl AsyncWrite for SocketHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        span!(Level::TRACE, "AsyncWrite::poll_write", "{:?}", self).in_scope(|| {
            let size = buf.len();
            let bytes: Bytes = buf.into();
            trace!("writing {} bytes", size);
            let send = self.tx.send(bytes);
            futures::pin_mut!(send);
            match futures::ready!(send.poll(cx)) {
                Ok(()) => Poll::Ready(Ok(size)),
                Err(_) => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            }
        })
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        span!(Level::TRACE, "AsyncWrite::poll_flush", "{:?}", self).in_scope(|| {
            trace!("flushing");
            let stream = &mut self.tx;
            futures::pin_mut!(stream);
            stream
                .poll_flush(cx)
                .map_err(|_| io::ErrorKind::BrokenPipe.into())
        })
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        span!(Level::TRACE, "AsyncWrite::poll_flush", "{:?}", self).in_scope(|| {
            trace!("shutting down");
            Pin::new(&mut self.tx)
                .poll_close(cx)
                .map_err(|_| io::ErrorKind::BrokenPipe.into())
        })
    }
}

impl crate::TcpStream for SocketHalf {
    fn local_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.local_addr)
    }
    fn peer_addr(&self) -> io::Result<net::SocketAddr> {
        Ok(self.peer_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use futures::{FutureExt, SinkExt, StreamExt};

    async fn pong_server(
        server: SocketHalf,
        stop: Option<usize>,
    ) -> Result<(), tokio::codec::LinesCodecError> {
        let mut transport = tokio::codec::Framed::new(server, tokio::codec::LinesCodec::new());
        let mut itercnt = 0;
        while let Some(Ok(ping)) = transport.next().await {
            itercnt += 1;
            if let Some(stop) = stop {
                if stop == itercnt {
                    return Ok(());
                }
            }
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
        let handle = runtime.localhost_handle();
        runtime.block_on(async {
            let server_addr = "127.0.0.1:9092".parse().unwrap();
            let client_addr = "127.0.0.1:35255".parse().unwrap();
            let (client_conn, server_conn) = new_socket_pair(client_addr, server_addr);
            handle.spawn(pong_server(server_conn, None).map(|_| ()));
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
        let handle = runtime.localhost_handle();
        runtime.block_on(async {
            let server_addr = "127.0.0.1:9092".parse().unwrap();
            let client_addr = "127.0.0.1:35255".parse().unwrap();
            let (server_conn, client_conn) = new_socket_pair(client_addr, server_addr);
            // create a server which will exit after receiving 3 messages
            let server_status =
                crate::spawn_with_result(&handle, pong_server(server_conn, Some(3)));
            let mut transport =
                tokio::codec::Framed::new(client_conn, tokio::codec::LinesCodec::new());

            for msg_num in 0..5usize {
                let send_result = transport.send(String::from("ping")).await;
                match msg_num {
                    num if num < 2 => {
                        // since the server closes at 3 requests, 2 or less should be fine
                        assert!(send_result.is_ok(), "expected sends to succeed because the server is not closed");
                        let receive_result = transport.next().await;
                        assert_eq!(receive_result.unwrap().unwrap(), String::from("pong"), "expected received to succeed");
                    }
                    num if num == 2 => {
                        assert!(send_result.is_ok(), "expected send to succeed");
                        assert!(transport.next().await.unwrap().is_err(), "msg num 2 should cause the server to close, resulting in an err returned by the receive")
                    }
                    _ => {
                        assert!(send_result.is_err(), "now that the server is closed, sends should always fail");
                    }
                }
            }
            server_status.await.unwrap();
        });
    }
}

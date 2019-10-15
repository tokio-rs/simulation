use futures::Poll;
use std::{io, net, pin::Pin, sync::Arc, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
use try_lock::TryLock;

pub(crate) fn new_pair(
    env: crate::DeterministicRuntimeSchedulerRng,
    server_addr: net::SocketAddr,
) -> (ClientSocket, ServerSocket) {
    let inner = Inner {
        client: super::Pipe::new(env.clone()),
        server: super::Pipe::new(env.clone()),
    };
    let state = Arc::new(TryLock::new(inner));
    let client = ClientSocket {
        inner: Arc::clone(&state),
    };
    let server = ServerSocket {
        inner: Arc::clone(&state),
        local_addr: server_addr,
        peer_addr: net::SocketAddr::new(net::IpAddr::V4(net::Ipv4Addr::LOCALHOST), 0),
    };
    (client, server)
}

#[derive(Debug)]
struct Inner {
    client: super::Pipe,
    server: super::Pipe,
}

type State = Arc<TryLock<Inner>>;

#[derive(Debug)]
pub struct ClientSocket {
    inner: State,
}

impl ClientSocket {
    pub(crate) fn shutdown(&self) {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                state.client.shutdown();
                state.server.shutdown();
            }
        }
    }
}

impl AsyncRead for ClientSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // client sockets read from the server pipe
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let server_pipe = &mut state.server;
                futures::pin_mut!(server_pipe);
                return server_pipe.poll_read(cx, buf);
            }
        }
    }
}

impl AsyncWrite for ClientSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let client_pipe = &mut state.client;
                futures::pin_mut!(client_pipe);
                return client_pipe.poll_write(cx, buf);
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let client_pipe = &mut state.client;
                futures::pin_mut!(client_pipe);
                return client_pipe.poll_flush(cx);
            }
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let client_pipe = &mut state.client;
                futures::pin_mut!(client_pipe);
                return client_pipe.poll_shutdown(cx);
            }
        }
    }
}

#[derive(Debug)]
pub struct ServerSocket {
    inner: State,
    local_addr: net::SocketAddr,
    peer_addr: net::SocketAddr,
}

impl ServerSocket {
    pub fn shutdown(&self) {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                state.client.shutdown();
                state.server.shutdown();
            }
        }
    }
    pub fn local_addr(&self) -> net::SocketAddr {
        self.local_addr
    }
    pub fn peer_addr(&self) -> net::SocketAddr {
        self.peer_addr
    }
}

impl AsyncRead for ServerSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // server sockets read from the client pipe
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let client_pipe = &mut state.client;
                futures::pin_mut!(client_pipe);
                return client_pipe.poll_read(cx, buf);
            }
        }
    }
}

impl AsyncWrite for ServerSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let server_pipe = &mut state.server;
                futures::pin_mut!(server_pipe);
                return server_pipe.poll_write(cx, buf);
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let server_pipe = &mut state.server;
                futures::pin_mut!(server_pipe);
                return server_pipe.poll_flush(cx);
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        loop {
            if let Some(mut state) = self.inner.try_lock() {
                let server_pipe = &mut state.server;
                futures::pin_mut!(server_pipe);
                return server_pipe.poll_shutdown(cx);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Environment;
    use futures::{SinkExt, StreamExt};

    async fn pong_server(server: ServerSocket) {
        let mut transport = tokio::codec::Framed::new(server, tokio::codec::LinesCodec::new());
        while let Some(Ok(ping)) = transport.next().await {
            assert_eq!(String::from("ping"), ping);
            transport.send(String::from("pong")).await.unwrap();
        }
    }

    #[test]
    fn pingpong() {
        let mut runtime = crate::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let (client, server) =
                new_pair(handle.scheduler_rng(), net::SocketAddr::new(net::Ipv4Addr::LOCALHOST.into(), 9092));
            handle.spawn(pong_server(server));
            let mut transport = tokio::codec::Framed::new(client, tokio::codec::LinesCodec::new());
            for _ in 0..100 {
                transport.send(String::from("ping")).await.unwrap();
                let result = transport.next().await.unwrap().unwrap();
                assert_eq!(result, String::from("pong"));
            }
        });
    }
}

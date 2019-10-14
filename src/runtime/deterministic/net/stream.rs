use super::{ClientSocket, ServerSocket};
use futures::Poll;
use std::{io, net, pin::Pin, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};

pub struct InMemoryTcpStream<S> {
    local_addr: net::SocketAddr,
    peer_addr: net::SocketAddr,
    socket: S,
}

impl InMemoryTcpStream<ClientSocket> {
    pub(crate) fn new_client(
        socket: ClientSocket,
        local_addr: net::SocketAddr,
        peer_addr: net::SocketAddr,
    ) -> Self {
        Self {
            local_addr,
            peer_addr,
            socket,
        }
    }
}

impl InMemoryTcpStream<ServerSocket> {
    pub(crate) fn new_server(socket: ServerSocket, local_addr: net::SocketAddr) -> Self {
        Self {
            local_addr: local_addr.clone(),
            // TODO: this is definitely wrong
            peer_addr: local_addr,
            socket,
        }
    }
}

impl<S> AsyncRead for InMemoryTcpStream<S>
where
    S: AsyncRead + Unpin + Send,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let socket = &mut self.socket;
        futures::pin_mut!(socket);
        socket.poll_read(cx, buf)
    }
}
impl<S> AsyncWrite for InMemoryTcpStream<S>
where
    S: AsyncWrite + Unpin + Send,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let socket = &mut self.socket;
        futures::pin_mut!(socket);
        socket.poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let socket = &mut self.socket;
        futures::pin_mut!(socket);
        socket.poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        let socket = &mut self.socket;
        futures::pin_mut!(socket);
        socket.poll_shutdown(cx)
    }
}

impl crate::TcpStream for InMemoryTcpStream<ClientSocket> {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.local_addr.clone())
    }
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.peer_addr.clone())
    }
    fn shutdown(&self) -> Result<(), io::Error> {
        self.socket.shutdown();
        Ok(())
    }
}

impl crate::TcpStream for InMemoryTcpStream<ServerSocket> {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.local_addr.clone())
    }
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.peer_addr.clone())
    }
    fn shutdown(&self) -> Result<(), io::Error> {
        self.socket.shutdown();
        Ok(())
    }
}

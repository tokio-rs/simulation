use super::{ClientSocket, ServerSocket};
use futures::Poll;
use std::{io, net, pin::Pin, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};

pub struct MemoryTcpStream<S> {
    socket: S,
}

impl MemoryTcpStream<ClientSocket> {
    pub(crate) fn new_client(socket: ClientSocket) -> Self {
        Self { socket }
    }
}

impl MemoryTcpStream<ServerSocket> {
    pub(crate) fn new_server(socket: ServerSocket) -> Self {
        Self { socket }
    }
}

impl<S> AsyncRead for MemoryTcpStream<S>
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
impl<S> AsyncWrite for MemoryTcpStream<S>
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

impl crate::TcpStream for MemoryTcpStream<ClientSocket> {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        unimplemented!()
    }
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error> {
        unimplemented!()
    }
    fn shutdown(&self) -> Result<(), io::Error> {
        self.socket.shutdown();
        Ok(())
    }
}

impl crate::TcpStream for MemoryTcpStream<ServerSocket> {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        unimplemented!()
    }
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error> {
        unimplemented!()
    }
    fn shutdown(&self) -> Result<(), io::Error> {
        self.socket.shutdown();
        Ok(())
    }
}

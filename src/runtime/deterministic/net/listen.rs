use super::ServerSocket;
use futures::{channel::mpsc, Poll, StreamExt};
use std::{io, net, pin::Pin, task::Context};

pub struct InMemoryListener {
    rx: mpsc::Receiver<ServerSocket>,
    local_addr: net::SocketAddr,
}

impl InMemoryListener {
    pub(crate) fn new(rx: mpsc::Receiver<ServerSocket>, local_addr: net::SocketAddr) -> Self {
        Self { rx, local_addr }
    }
}

impl crate::TcpListener for InMemoryListener {
    type Stream = super::InMemoryTcpStream<ServerSocket>;
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.local_addr)
    }
    fn poll_accept(&mut self, cx: &mut Context<'_>) -> Poll<Result<Self::Stream, io::Error>> {
        let server_socket = futures::ready!(self.rx.poll_next_unpin(cx));
        if let Some(socket) = server_socket {
            return Poll::Ready(Ok(super::InMemoryTcpStream::new_server(
                socket,
                self.local_addr,
            )));
        } else {
            // TODO: Is this the correct error?
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
    }
}

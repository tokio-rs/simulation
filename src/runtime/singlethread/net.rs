use futures::Poll;
use std::{io, net, pin::Pin, task::Context};

impl crate::TcpStream for tokio::net::TcpStream {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        return self.local_addr();
    }
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error> {
        return self.peer_addr();
    }
    fn shutdown(&self) -> Result<(), io::Error> {
        unimplemented!()
    }
}

impl crate::TcpListener for tokio::net::TcpListener {
    type Stream = tokio::net::TcpStream;
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        self.local_addr()
    }
    fn poll_accept(&mut self, cx: &mut Context<'_>) -> Poll<Result<Self::Stream, io::Error>> {
        unimplemented!()
    }
}

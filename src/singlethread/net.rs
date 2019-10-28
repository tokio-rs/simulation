use async_trait::async_trait;
use std::{io, net};
use tokio::net::{TcpListener, TcpStream};

impl crate::TcpStream for TcpStream {
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        self.local_addr()
    }
    fn peer_addr(&self) -> Result<net::SocketAddr, io::Error> {
        self.peer_addr()
    }
}

#[async_trait]
impl crate::TcpListener for TcpListener {
    type Stream = tokio::net::TcpStream;
    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error> {
        tokio::net::TcpListener::accept(self).await
    }
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        tokio::net::TcpListener::local_addr(self)
    }
    fn ttl(&self) -> io::Result<u32> {
        tokio::net::TcpListener::ttl(self)
    }
    fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        tokio::net::TcpListener::set_ttl(self, ttl)
    }
}

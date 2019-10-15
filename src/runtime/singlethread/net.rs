use async_trait::async_trait;
use std::{io, net};

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

#[async_trait]
impl crate::TcpListener for tokio::net::TcpListener {
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

use futures::{Future, FutureExt, Poll};
use std::{io, net, task::Context};

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
        let fut = tokio::net::TcpListener::accept(self);
        futures::pin_mut!(fut);
        Poll::Ready(futures::ready!(fut.poll(cx)).map(|(sock, _)| sock))
    }
}

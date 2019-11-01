use crate::TcpStream;
use async_trait::async_trait;
use futures::{channel::mpsc, StreamExt};
use std::{io, net};
use super::{FaultyTcpStream, SocketHalf};

#[derive(Debug)]
/// ListenerState represents both the bound and unbound state of a Listener.
/// This allows supporting late binding of Listeners to sockets.
pub(crate) enum ListenerState {
    Unbound { tx: mpsc::Sender<FaultyTcpStream<SocketHalf>>, rx: mpsc::Receiver<FaultyTcpStream<SocketHalf>>},
    Bound { tx: mpsc::Sender<FaultyTcpStream<SocketHalf>> }
}

pub struct Listener {
    local_addr: net::SocketAddr,
    incoming: mpsc::Receiver<FaultyTcpStream<SocketHalf>>,
}

impl Listener {
    pub fn new(local_addr: net::SocketAddr, incoming: mpsc::Receiver<FaultyTcpStream<SocketHalf>>) -> Self {
        Self {
            local_addr,
            incoming,
        }
    }
}

#[async_trait]
impl crate::TcpListener for Listener {
    type Stream = FaultyTcpStream<SocketHalf>;
    async fn accept(&mut self) -> Result<(Self::Stream, net::SocketAddr), io::Error> {
        if let Some(next) = self.incoming.next().await {
            let addr = next.peer_addr()?;
            Ok((next, addr))
        } else {
            Err(io::ErrorKind::NotConnected.into())
        }
    }
    fn local_addr(&self) -> Result<net::SocketAddr, io::Error> {
        Ok(self.local_addr)
    }
    fn ttl(&self) -> io::Result<u32> {
        Ok(0)
    }
    fn set_ttl(&self, _: u32) -> io::Result<()> {
        Ok(())
    }
}

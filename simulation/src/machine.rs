//! Simulation state for a logical machine in a simulation.
use crate::tcp::{TcpListener, TcpListenerHandle, TcpStream, TcpStreamHandle};
use std::{collections, io, net, num, string};
#[derive(Debug)]
pub(crate) struct LogicalMachine {
    hostname: String,
    ipaddr: net::IpAddr,
    tags: collections::HashMap<String, String>,
    acceptors: collections::HashMap<num::NonZeroU16, TcpListenerHandle>,
    connections: Vec<TcpStreamHandle>,
}

impl LogicalMachine {
    pub(crate) fn new<T>(hostname: T) -> Self
    where
        T: string::ToString,
    {
        Self {
            hostname: hostname.to_string(),
            ipaddr: net::Ipv4Addr::LOCALHOST.into(),
            tags: collections::HashMap::new(),
            acceptors: collections::HashMap::new(),
            connections: vec![],
        }
    }

    pub(crate) fn new_with_tags<T>(
        hostname: String,
        ipaddr: net::IpAddr,
        tags: collections::HashMap<String, String>,
    ) -> Self
    where
        T: string::ToString,
    {
        Self {
            hostname: hostname.to_string(),
            ipaddr: ipaddr,
            tags,
            acceptors: collections::HashMap::new(),
            connections: vec![],
        }
    }

    pub(crate) fn hostname(&self) -> &str {
        self.hostname.as_ref()
    }

    pub(crate) fn ipaddr(&self) -> net::IpAddr {
        self.ipaddr
    }

    pub(crate) fn bind_acceptor(&mut self, port: u16) -> Result<TcpListener, io::Error> {
        self.gc_connections();
        let port = self.allocate_port(port)?;
        let (acceptor, handle) = TcpListener::new(net::SocketAddr::new(self.ipaddr, port.get()));
        self.acceptors.insert(port, handle);
        Ok(acceptor)
    }

    pub(crate) async fn connect(
        &mut self,
        client_addr: net::SocketAddr,
        port: num::NonZeroU16,
    ) -> Result<TcpStream, io::Error> {
        self.gc_connections();
        let mut acceptor = self
            .acceptors
            .get(&port)
            .cloned()
            .ok_or(io::ErrorKind::ConnectionRefused)?;
        let server_addr = net::SocketAddr::new(self.ipaddr(), port.get());
        let (client, server, handle) = TcpStream::new_pair(client_addr, server_addr);
        acceptor.enqueue_incoming(server).await?;
        self.connections.push(handle);
        Ok(client)
    }

    fn allocate_port(&mut self, port: u16) -> Result<num::NonZeroU16, io::Error> {
        let mut candidate_port = port;
        loop {
            if let Some(valid_port) = num::NonZeroU16::new(candidate_port) {
                if self.acceptors.contains_key(&valid_port) {
                    return Err(io::ErrorKind::AddrInUse.into());
                } else {
                    return Ok(valid_port);
                }
            } else {
                candidate_port = u16::checked_add(candidate_port, 1).ok_or(io::Error::new(
                    io::ErrorKind::Other,
                    format!("no more ports available for machine {}", self.hostname),
                ))?;
            }
        }
    }

    fn gc_connections(&mut self) {
        self.acceptors.retain(|_, v| !v.dropped());
        self.connections.retain(|v| !v.dropped());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio::{runtime::Runtime, sync::oneshot};
    use tokio_util::codec::{Framed, LinesCodec};

    #[test]
    fn bind_acceptor() -> Result<(), Box<dyn std::error::Error>> {
        let mut machine = LogicalMachine::new("localhost");
        let acceptor1 = machine.bind_acceptor(1)?;
        let acceptor2 = machine.bind_acceptor(2)?;

        assert_eq!(
            acceptor1.local_addr(),
            std::net::SocketAddr::new(machine.ipaddr(), 1)
        );
        assert_eq!(
            acceptor2.local_addr(),
            std::net::SocketAddr::new(machine.ipaddr(), 2)
        );
        assert!(
            machine.bind_acceptor(1).is_err(),
            "expected binding to an existing port to return an error"
        );
        Ok(())
    }

    #[test]
    fn bind_connect() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = Runtime::new()?;

        runtime.block_on(async {
            let mut machine = LogicalMachine::new("localhost");
            let mut listener = machine.bind_acceptor(0)?;
            let server_port = listener.local_addr().port();

            let (mut server_started, mut start) = oneshot::channel::<()>();

            tokio::spawn(async move {
                start.close();
                for conn in listener
                    .incoming()
                    .next()
                    .await
                    .expect("listener disconnected")
                {
                    let mut conn = Framed::new(conn, LinesCodec::new());
                    conn.send("hello".to_owned())
                        .await
                        .expect("could not respond");
                }
            });
            server_started.closed().await;

            let client_addr = net::SocketAddr::new(net::Ipv4Addr::LOCALHOST.into(), 9999);
            let conn: TcpStream = machine
                .connect(client_addr, num::NonZeroU16::new(server_port).unwrap())
                .await
                .expect("could not connect");
            let mut conn = Framed::new(conn, LinesCodec::new());
            let response = conn.next().await.unwrap().unwrap();
            assert_eq!(response, "hello".to_owned());
            Ok(())
        })
    }
}

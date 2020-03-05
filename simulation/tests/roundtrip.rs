extern crate simulation;
use futures::SinkExt;
use futures::StreamExt;
use simulation::{
    net::tcp::{TcpListener, TcpStream},
    Simulation,
};
use std::io;
use tokio_util::codec::{Framed, LinesCodec};

struct Client {
    inner: Framed<TcpStream, LinesCodec>,
}

impl Client {
    async fn connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let inner = Framed::new(stream, LinesCodec::new());
        Ok(Client { inner })
    }

    async fn send_msg(&mut self, msg: &str) -> io::Result<()> {
        self.inner
            .send(String::from(msg))
            .await
            .map_err(|_| io::ErrorKind::ConnectionRefused)?;
        Ok(())
    }

    async fn send_ping(&mut self) -> io::Result<()> {
        self.send_msg("ping").await
    }
}

/// Establishes a TcpListener which will listen for incoming connections and
/// issue a ping to the provided next.
async fn forward_to(next_addr: &str) {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    simulation::spawn(async move {
        let mut listener = TcpListener::bind(9092).await.unwrap();
        while let Ok((socket, addr)) = listener.accept().await {
            let mut transport = Framed::new(socket, LinesCodec::new());

            let inner_tx = tx.clone();
            simulation::spawn(async move {
                while let Some(Ok(msg)) = transport.next().await {
                    println!("forwarding message from {:?}", addr);
                    inner_tx.clone().send(msg).await.unwrap();
                }
            });
        }
    });

    // "sleep" according to virtual time for a bit to allow the server to come up
    // for other nodes in the ring.
    tokio::time::delay_for(std::time::Duration::from_secs(5)).await;
    let mut client = Client::connect(next_addr).await.unwrap();

    // send an inital message to provide stimuli for the message ring
    client.send_ping().await.unwrap();

    while let Some(_) = rx.next().await {
        client.send_ping().await.unwrap();
    }
}

/// Round trip messages around a ring. Each function creates a logical machine
/// which listens for new connection and forwards a "ping" message to the next server
/// upon recipt of a message.
///
///
#[test]
fn roundtrip_ring() {
    Simulation::new(0)
        // 4 node ring
        .machine("node0", forward_to("node1:9092"))
        .machine("node1", forward_to("node2:9092"))
        .machine("node2", forward_to("node3:9092"))
        .machine("node3", forward_to("node0:9092"))
        // Independent cycle
        .machine("node4", forward_to("node5:9092"))
        .machine("node5", forward_to("node4:9092"))
        .run();
}

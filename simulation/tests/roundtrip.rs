extern crate simulation;

use futures::{SinkExt, StreamExt};

use simulation::{
    net::tcp::{TcpListener, TcpStream},
    Simulation,
};

use std::{io, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    time::delay_for,
};
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
            .send(msg.to_owned())
            .await
            .map_err(|_| io::ErrorKind::ConnectionRefused)?;
        Ok(())
    }

    async fn send_ping(&mut self) -> io::Result<()> {
        self.send_msg("ping").await
    }

    async fn send_stop(&mut self) -> io::Result<()> {
        self.send_msg("stop").await
    }
}

enum Message {
    Ping,
    Stop,
}

async fn handler(socket: TcpStream, mut tx: mpsc::Sender<Message>) {
    let mut transport = Framed::new(socket, LinesCodec::new());
    while let Some(Ok(msg)) = transport.next().await {
        match msg.as_str() {
            "ping" => {
                let _ = tx.send(Message::Ping).await;
            }
            "stop" => {
                let _ = tx.send(Message::Stop).await;
                return;
            }
            _ => panic!("unknown message"),
        };
    }
}

async fn server(tx: mpsc::Sender<Message>, stop_rx: oneshot::Receiver<()>) {
    let mut listener = TcpListener::bind(9092).await.expect("failed to bind");
    futures::pin_mut!(stop_rx);

    loop {
        tokio::select! {
            _ = stop_rx.as_mut() => {
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((socket, _)) => {
                        simulation::spawn(handler(socket, tx.clone()))
                    },
                    _ => {
                        break;
                    }
                };
            }
        };
    }
}

/// Establishes a TcpListener which will listen for incoming connections and
/// issue a ping to the provided next. It will forward 100 messages around the ring
/// before waiting on the barrier.
async fn ring_node(next_addr: String) {
    let (stop_tx, stop_rx) = oneshot::channel::<()>();

    let (tx, mut rx) = mpsc::channel::<Message>(1);
    simulation::spawn(server(tx, stop_rx));
    // wait for the server to come up.
    delay_for(Duration::from_secs(1)).await;
    let mut client = Client::connect(next_addr.as_str())
        .await
        .expect("failed to connect");
    loop {
        tokio::select! {
            msg = rx.next() => {
                match msg {
                    Some(Message::Ping) => {
                        let _ = client.send_ping().await;
                    }
                    Some(Message::Stop) => {
                        stop_tx.send(()).expect("failed to signal stop");
                        let _ = client.send_stop().await;
                        break;
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }
}

/// Round trip messages around a ring. Each function creates a logical machine
/// which listens for new connection and forwards a "ping" message to the next server
/// upon recipt of a message.
#[test]
fn roundtrip_ring() {
    Simulation::new(32)
        // 4 node ring
        .machine("node0", ring_node(String::from("node1:9092")))
        .machine("node1", ring_node(String::from("node2:9092")))
        .machine("node2", ring_node(String::from("node3:9092")))
        .machine("node3", ring_node(String::from("node0:9092")))
        .simulate(async {
            let start_time = tokio::time::Instant::now();
            println!("starting simulation run");
            // Wait for a second for the server to come up.
            delay_for(Duration::from_secs(1)).await;
            println!("sending first message");
            // provide initial event to start the ring
            let mut client = Client::connect("node0:9092").await.unwrap();
            client
                .send_ping()
                .await
                .expect("failed to send initial event");
            println!("sent first message");
            // let the message ring run for an hour
            delay_for(Duration::from_secs(60 * 60)).await;
            client.send_stop().await.unwrap();

            let end_time = tokio::time::Instant::now();
            println!("elapsed {:?}", end_time.duration_since(start_time));
        });
}

//! Demonstrates using the simulation framework to find a race condition in a distributed system.
//! The particular race condition is a message reordering issue between two processes. A central
//! banking server is in charge of maintaining the balance of an account. Alice and Bob both need
//! to make several withdrawals from their shared account.
//!
//! The system demonstrated here is composed of 3 connected components.
//! 1. A banking server, which maintains the current balance for a users account.
//! 2. An ATM process for Alice, which attempts to withdraw 1 dollar every 500ms.
//! 3. An ATM process for Bob, which attempts to withdraw 2 dollars every 200ms.

use futures::{channel::oneshot, SinkExt, StreamExt};
use simulation::{
    DeterministicRuntime, DeterministicRuntimeHandle, Environment, TcpListener, TcpStream,
};
use std::{
    net::Ipv4Addr,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::codec::{Framed, LinesCodec};

#[derive(Debug)]
enum Error {
    IO(std::io::Error),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::IO(e)
    }
}

enum BankOperations {
    Deposit { amount: usize },
    Withdraw { amount: usize },
    BalanceRequest,
    BalanceResponse { balance: usize },
}

impl BankOperations {
    fn parse(s: String) -> BankOperations {
        let parts: Vec<&str> = s.split(" ").collect();
        if parts[0] == "deposit" {
            return BankOperations::Deposit {
                amount: parts[1].parse().unwrap(),
            };
        }
        if parts[0] == "withdraw" {
            return BankOperations::Withdraw {
                amount: parts[1].parse().unwrap(),
            };
        }
        if parts[0] == "balancereq" {
            return BankOperations::BalanceRequest;
        }
        if parts[0] == "balanceresp" {
            return BankOperations::BalanceResponse {
                balance: parts[1].parse().unwrap(),
            };
        }
        panic!("unexpected message")
    }
    fn to_message(&self) -> String {
        use BankOperations::*;
        match self {
            Deposit { amount } => format!("deposit {}", amount),
            Withdraw { amount } => format!("withdraw {}", amount),
            BalanceRequest => format!("balancereq"),
            BalanceResponse { balance } => format!("balanceresp {}", balance),
        }
    }
}

async fn banking_server(
    handle: DeterministicRuntimeHandle,
    bind_addr: std::net::SocketAddr,
    accepting: oneshot::Sender<bool>,
) {
    let user_balance = std::sync::Arc::new(std::sync::Mutex::new(100usize));

    let mut listener = handle.bind(bind_addr).await.unwrap();
    accepting.send(true).unwrap();

    while let Ok(new_connection) = listener.accept().await {
        println!("new connection from {:?}", new_connection.peer_addr());
        let framed_read = Framed::new(new_connection, LinesCodec::new());
        let (mut sink, mut stream) = framed_read.split();

        // spawn a new worker to handle the connection
        let balance_handle = Arc::clone(&user_balance);
        handle.spawn(async move {
            while let Some(Ok(req)) = stream.next().await {
                let message = BankOperations::parse(req);
                println!("recieved message {}", message.to_message());
                match message {
                    BankOperations::Deposit { amount } => {
                        let mut lock = balance_handle.lock().unwrap();
                        *lock += amount;
                    }
                    BankOperations::Withdraw { amount } => {
                        let mut lock = balance_handle.lock().unwrap();
                        assert!(*lock > 0, "overdraft detected!");
                        *lock -= amount;
                    }
                    BankOperations::BalanceRequest => {
                        let current_balance = {
                            let lock = balance_handle.lock().unwrap();
                            *lock
                        };
                        let message = BankOperations::BalanceResponse {
                            balance: current_balance,
                        }
                        .to_message();
                        sink.send(message).await.unwrap();
                    }
                    _ => unreachable!(),
                }
            }
        });
    }
}

/// atm starts a process with withdraws `withdraw` dollars from the bank every `period`.
async fn atm<E>(handle: E, period: Duration, withdraw: usize, socket: E::TcpStream)
where
    E: simulation::Environment,
{
    let mut transport = Framed::new(socket, LinesCodec::new());
    loop {
        // first make a balance request
        transport
            .send(BankOperations::BalanceRequest.to_message())
            .await
            .unwrap();
        // retrieve balance response
        if let BankOperations::BalanceResponse { balance } =
            BankOperations::parse(transport.next().await.unwrap().unwrap())
        {
            println!("balance response {}", balance);
            // if we have money in the account, withdraw it!
            if balance > withdraw {
                transport
                    .send(BankOperations::Withdraw { amount: withdraw }.to_message())
                    .await
                    .unwrap();
            } else {
                println!("done!");
                break;
            }
        } else {
            panic!("unexpected response")
        }
        // sleep until the next withdraw.
        handle.delay_from(period).await;
    }
}


/// Run our simulated bank with various seeds from 1..10
/// to find a seed which causes an overdraft.
/// 
/// Particularly, seed #2 causes a message ordering which results 
/// in an overdraft.
fn main() {
    for x in 0..10 {
        println!("--- seed --- {}", x);
        let mut runtime = DeterministicRuntime::new_with_seed(x).unwrap();
        let handle = runtime.handle();
        let bank_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9092);

        runtime.block_on(async {
            let server_handle = handle.clone();
            // setup a channel to notify us when a bind happens
            let (bind_tx, bind_rx) = oneshot::channel();
            handle.spawn(async move {
                banking_server(server_handle, bank_addr.clone(), bind_tx).await;
            });
            bind_rx.await.unwrap();

            let socket = handle.connect(bank_addr).await.unwrap();
            let r1 = simulation::spawn_with_result(
                &handle,
                atm(handle.clone(), Duration::from_millis(200), 2, socket),
            );
            let socket = handle.connect(bank_addr).await.unwrap();
            let r2 = simulation::spawn_with_result(
                &handle,
                atm(handle.clone(), Duration::from_millis(500), 1, socket),
            );
            r1.await;
            r2.await;
        });
    }
}

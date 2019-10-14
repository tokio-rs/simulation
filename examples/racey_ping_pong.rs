//! Demonstrates using the simulation framework to find a race condition in a distributed system.
//! The particular race condition is a message reordering issue between two processes. A central
//! banking server is in charge of maintaining a users balance, but two seperate ATM's are attempting
//! to withdraw money via a simple protocol where balance is checked before a withdraw is issued.
//!
//! Periodically, money is deposited into the account
//!
//! The system demonstrated here is composed of 3 connected components.
//! 1. A banking server, which maintains the current balance for a users account.
//! 2. An ATM process, which

use futures::{SinkExt, StreamExt, channel::oneshot};
use simulation::{
    DeterministicRuntime, DeterministicRuntimeHandle, Environment, TcpListener, TcpStream,
};
use std::sync::{Arc};
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

async fn banking_server(handle: DeterministicRuntimeHandle, accepting: oneshot::Sender<bool>) {
    let user_balance = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let mut listener = handle.bind("localhost:9092").await.unwrap();
    accepting.send(true);
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
fn main() {
    let mut runtime = DeterministicRuntime::new_with_seed(10).unwrap();
    let handle = runtime.handle();

    runtime.block_on(async {
        let server_handle = handle.clone();
        // setup a channel to notify us when a bind happens
        let (bind_tx, bind_rx) = oneshot::channel();
        handle.spawn(async move{
          banking_server(server_handle, bind_tx).await;
        });
        bind_rx.await.unwrap();
        let socket = handle.connect("localhost:9092").await.unwrap();
        let mut transport = Framed::new(socket, LinesCodec::new());
        transport.send(String::from("deposit 100")).await.unwrap();
        transport.send(String::from("balancereq")).await.unwrap();
        let response = transport.next().await.unwrap().unwrap();
        println!("response {}", response);
    });
}

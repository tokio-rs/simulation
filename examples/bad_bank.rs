//! Demonstrates using the simulation framework to find a race condition in a distributed system.
//! The particular race condition is a message reordering issue between two processes. A central
//! banking server is in charge of maintaining the balance of an account. Alice and Bob both need
//! to make several withdrawals from their shared account.
//!
//! The system demonstrated here is composed of 3 connected components.
//! 1. A banking server, which maintains the current balance for a users account.
//! 2. An ATM process for Alice, which attempts to withdraw 1 dollar every 500ms.
//! 3. An ATM process for Bob, which attempts to withdraw 2 dollars every 200ms.

use bytes::BytesMut;
use futures::{channel::oneshot, FutureExt, SinkExt, StreamExt};
use simulation::{deterministic::DeterministicRuntime, Environment, TcpListener};
use std::{
    io,
    net::Ipv4Addr,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::codec::{Framed, LinesCodec};

enum BankOperations {
    Deposit { amount: usize },
    Withdraw { amount: usize },
    BalanceRequest,
    BalanceResponse { balance: usize },
}

struct Codec {
    inner: LinesCodec,
}

impl Codec {
    fn wrap(c: LinesCodec) -> Self {
        Self { inner: c }
    }
}

impl tokio::codec::Decoder for Codec {
    type Item = BankOperations;
    type Error = io::Error;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(line) = self
            .inner
            .decode(src)
            .map_err(|_| io::ErrorKind::InvalidData)?
        {
            let parts: Vec<&str> = line.split(" ").collect();
            if parts[0] == "deposit" {
                return Ok(Some(BankOperations::Deposit {
                    amount: parts[1].parse().unwrap(),
                }));
            }
            if parts[0] == "withdraw" {
                return Ok(Some(BankOperations::Withdraw {
                    amount: parts[1].parse().unwrap(),
                }));
            }
            if parts[0] == "balancereq" {
                return Ok(Some(BankOperations::BalanceRequest));
            }
            if parts[0] == "balanceresp" {
                return Ok(Some(BankOperations::BalanceResponse {
                    balance: parts[1].parse().unwrap(),
                }));
            }
            return Err(io::ErrorKind::InvalidData.into());
        } else {
            return Ok(None);
        }
    }
}

impl tokio::codec::Encoder for Codec {
    type Item = BankOperations;
    type Error = io::Error;
    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        use BankOperations::*;
        let encoded = match item {
            Deposit { amount } => format!("deposit {}", amount),
            Withdraw { amount } => format!("withdraw {}", amount),
            BalanceRequest => format!("balancereq"),
            BalanceResponse { balance } => format!("balanceresp {}", balance),
        };
        return self
            .inner
            .encode(encoded, dst)
            .map_err(|_| io::ErrorKind::InvalidData.into());
    }
}

async fn banking_server<E>(
    handle: E,
    bind_addr: std::net::SocketAddr,
    accepting: oneshot::Sender<bool>,
) where
    E: Environment,
{
    let user_balance = std::sync::Arc::new(std::sync::Mutex::new(1000usize));

    let mut listener = handle.bind(bind_addr).await.unwrap();
    accepting.send(true).unwrap();

    while let Ok((new_connection, _)) = listener.accept().await {
        let framed_read = Framed::new(new_connection, Codec::wrap(LinesCodec::new()));
        let (mut sink, mut stream) = framed_read.split();

        // spawn a new worker to handle the connection
        let balance_handle = Arc::clone(&user_balance);
        handle.spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                match message {
                    BankOperations::Deposit { amount } => {
                        let mut lock = balance_handle.lock().unwrap();
                        *lock += amount;
                    }
                    BankOperations::Withdraw { amount } => {
                        let mut lock = balance_handle.lock().unwrap();
                        if *lock - amount <= 0 {
                            assert!(false, "overdraft detected!");
                        }

                        *lock -= amount;
                    }
                    BankOperations::BalanceRequest => {
                        let current_balance = {
                            let lock = balance_handle.lock().unwrap();
                            *lock
                        };
                        let message = BankOperations::BalanceResponse {
                            balance: current_balance,
                        };
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
    let mut transport = Framed::new(socket, Codec::wrap(LinesCodec::new()));
    loop {
        println!("atming");
        // first make a balance request
        transport
            .send(BankOperations::BalanceRequest)
            .await
            .unwrap();
        // retrieve balance response
        if let BankOperations::BalanceResponse { balance } =
            transport.next().await.unwrap().unwrap()
        {
            // if we have money in the account, withdraw it!
            if balance > withdraw {
                transport
                    .send(BankOperations::Withdraw { amount: withdraw })
                    .await
                    .unwrap();
            } else {
                break;
            }
        } else {
            panic!("unexpected response")
        }
        // sleep until the next withdraw.
        handle.delay_from(period).await;
    }
}

fn simulate(seed: u64) -> std::time::Duration {
    // A SingleThreaded runtime can be swapped in at will.
    // let mut runtime = SingleThreadedRuntime::new().unwrap();
    let mut runtime = DeterministicRuntime::new_with_seed(seed).unwrap();
    let handle = runtime.handle();
    let bank_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9092);
    let start_time = handle.now();
    runtime.block_on(async {
        let server_handle = handle.clone();
        // setup a channel to notify us when a bind happens
        let (bind_tx, bind_rx) = oneshot::channel();
        let mut server_fut = simulation::spawn_with_result(&handle, async move {
            banking_server(server_handle, bank_addr.clone(), bind_tx).await;
        })
        .fuse();
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

        let mut fut = futures::future::join_all(vec![r1, r2]).fuse();
        futures::select!(
            _ = fut => {
                println!("clients finished")
            }
            _ = server_fut => {
                println!("bank overdrafted on seed {}", seed)
            }
        );
    });
    let end_time = handle.now();
    end_time - start_time
}

/// Run our simulated bank with various seeds from 1..10
/// to find a seed which causes an overdraft.
///
/// Particularly, seed #1 causes a message ordering which results
/// in an overdraft, while seed #0 does not.
fn main() {
    for seed in 1..10 {
        println!("--- seed --- {}", seed);
        let true_start_time = std::time::Instant::now();
        let simulation_duration = simulate(seed);
        let true_duration = std::time::Instant::now() - true_start_time;
        println!(
            "real-time: {:?}\n simulated-time: {:?}",
            true_duration, simulation_duration
        )
    }
}

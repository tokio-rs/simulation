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
use futures::{FutureExt, SinkExt, StreamExt};
use simulation::{deterministic::DeterministicRuntime, Environment, TcpListener};
use std::{
    io,
    net,
    sync,
    time,
};
use tokio::codec::{Framed, LinesCodec};

#[derive(Debug)]
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
            let parts: Vec<&str> = line.split(' ').collect();
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
            Err(io::ErrorKind::InvalidData.into())
        } else {
            Ok(None)
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
            BalanceRequest => "balancereq".to_string(),
            BalanceResponse { balance } => format!("balanceresp {}", balance),
        };
        self.inner
            .encode(encoded, dst)
            .map_err(|_| io::ErrorKind::InvalidData.into())
    }
}

struct BankingServer<E> {
    env: E,
    bank_balance: sync::Arc<sync::atomic::AtomicIsize>
}

async fn handle_new_connection<S>(bank_balance: sync::Arc<sync::atomic::AtomicIsize>, stream: S)
    where S: simulation::TcpStream
{
    let mut transport = Framed::new(stream, Codec::wrap(LinesCodec::new()));
    let user_balance = sync::Arc::clone(&bank_balance);
    while let Some(Ok(message)) = transport.next().await {
        match message {
            BankOperations::Deposit { amount } => {
                user_balance.fetch_add(amount as isize, sync::atomic::Ordering::SeqCst);
            }
            BankOperations::Withdraw { amount } => {
                if user_balance.load(sync::atomic::Ordering::SeqCst) <= 0  {
                    panic!("overdraft detected!");
                }
                user_balance.fetch_sub(amount as isize, sync::atomic::Ordering::SeqCst);
            }
            BankOperations::BalanceRequest => {
                let current_balance = user_balance.load(sync::atomic::Ordering::SeqCst);
                let message = BankOperations::BalanceResponse {
                    balance: current_balance as usize,
                };
                transport.send(message).await.unwrap();
            }
            _ => unreachable!(),
        }
    }
    println!("BankingServer connection closed");
}

impl<E> BankingServer<E> where E: Environment + Send + Sync + Unpin {
    fn new(handle: E, initial_balance: usize) -> BankingServer<E> {
        Self {
            env: handle,
            bank_balance: sync::Arc::new(sync::atomic::AtomicIsize::new(initial_balance as isize))
        }
    }

    async fn serve(self, port: u16) -> Result<(), io::Error> {
        let mut listener = self.env.bind(net::SocketAddr::new(net::Ipv4Addr::LOCALHOST.into(), port)).await?;
        while let Ok((new_connection, _)) = listener.accept().await {
            self.env.spawn(handle_new_connection(self.bank_balance.clone(), new_connection))
        }
        println!("BankingServer shut down");
        Ok(())
    }
}

/// atm starts a process with withdraws `withdraw` dollars from the bank every `period`.
async fn atm<E>(handle: E, period: time::Duration, withdraw: usize)
where
    E: simulation::Environment,
{
    'outer: loop {
        let bank_server_addr: net::SocketAddr = "127.0.0.1:9092".parse().unwrap();
        if let Ok(socket) = handle.connect(bank_server_addr).await {
            let mut transport = Framed::new(socket, Codec::wrap(LinesCodec::new()));
            if let Ok(()) = transport.send(BankOperations::BalanceRequest).await {
                if let Some(Ok(BankOperations::BalanceResponse { balance })) = transport.next().await {
                    if balance > withdraw {
                        if let Ok(()) = transport.send(BankOperations::Withdraw { amount: withdraw }).await {
                            handle.delay_from(period).await;
                        } else {
                            println!("error sending withdraw request, reconnecting");
                            continue 'outer;
                        }
                    } else {
                        println!("account empty, complete!");
                        return;
                    }
                } else {
                    println!("error reading balance response, reconnecting");
                    continue 'outer;
                }
            } else {
                println!("error sending balance request, reconnecting");
                continue 'outer;
            }

        } else {
            println!("error connecting to bank server, retrying...");
            continue;
        }
    }
}

fn simulate(seed: u64) -> std::time::Duration {
    // A SingleThreaded runtime can be swapped in at will.
    // let mut runtime = SingleThreadedRuntime::new().unwrap();
    let mut runtime = DeterministicRuntime::new_with_seed(seed).unwrap();
    let handle = runtime.handle();
    let start_time = handle.now();
    runtime.block_on(async {
        let server_handle = handle.clone();
        let banking_server = BankingServer::new(server_handle.clone(), 100);
        let mut server_fut = simulation::spawn_with_result(&handle, async move {
            banking_server.serve(9092).await.unwrap()
        })
        .fuse();

        let r1 = simulation::spawn_with_result(
            &handle,
            atm(handle.clone(), time::Duration::from_millis(200), 2),
        );

        let r2 = simulation::spawn_with_result(
            &handle,
            atm(handle.clone(), time::Duration::from_millis(500), 1),
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
    for seed in 0..10 {
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

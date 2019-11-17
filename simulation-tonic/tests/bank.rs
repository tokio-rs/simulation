//! Bank simulation showing how the simulation framework can detect a message
//! reordering bug.
use simulation::{deterministic::DeterministicRuntime, Environment, TcpListener};
pub mod bank {
    tonic::include_proto!("bank");
}
use bank::{
    client::BankClient,
    server::{Bank, BankServer},
    withdraw_response::WithdrawStatus,
    BalanceQueryRequest, BalanceQueryResponse, DepositRequest, DepositResponse, WithdrawRequest,
    WithdrawResponse,
};
use std::{collections, net, time};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tower_service::Service;

#[derive(Default)]
struct BankHandler {
    /// mapping from user account to balance
    vault: Mutex<collections::HashMap<i32, i32>>,
}

impl BankHandler {
    fn new() -> Self {
        Self {
            vault: Mutex::new(collections::HashMap::new()),
        }
    }
}

#[tonic::async_trait]
impl Bank for BankHandler {
    async fn balance_query(
        &self,
        request: Request<BalanceQueryRequest>,
    ) -> Result<Response<BalanceQueryResponse>, Status> {
        let lock = self.vault.lock().await;
        let balance = lock.get(&request.get_ref().account_id).unwrap_or(&0);
        Ok(Response::new(BalanceQueryResponse {
            account_balance: *balance,
        }))
    }

    async fn withdraw(
        &self,
        request: Request<WithdrawRequest>,
    ) -> Result<Response<WithdrawResponse>, Status> {
        let mut lock = self.vault.lock().await;
        let account_id = request.get_ref().account_id;
        let withdraw_amount = request.get_ref().amount;
        let balance = *lock.get(&account_id).unwrap_or(&0);
        let status = if withdraw_amount > balance {
            lock.insert(account_id, 0);
            WithdrawStatus::Overdraft
        } else {
            lock.insert(account_id, balance - withdraw_amount);
            WithdrawStatus::Success
        };

        Ok(Response::new(WithdrawResponse {
            status: status.into(),
        }))
    }

    async fn deposit(
        &self,
        request: Request<DepositRequest>,
    ) -> Result<Response<DepositResponse>, Status> {
        let mut lock = self.vault.lock().await;
        let account_id = request.get_ref().account_id;
        let deposit_amount = request.get_ref().amount;
        let balance = *lock.get(&account_id).unwrap_or(&0);
        let new_balance = deposit_amount + balance;
        lock.insert(account_id, new_balance);
        Ok(Response::new(DepositResponse { new_balance }))
    }
}

async fn start_server<E>(env: E)
where
    E: Environment,
    E::TcpListener: Sync,
    <E::TcpListener as TcpListener>::Stream: Sync,
{
    let bank_handler = BankHandler::new();
    let bind_addr: net::SocketAddr = "127.0.0.1:9092".parse().unwrap();
    let listener = env.bind(bind_addr).await.unwrap();
    let listener = listener.into_stream();
    tonic::transport::Server::builder()
        .add_service(BankServer::new(bank_handler))
        .serve_from_stream(listener)
        .await
        .unwrap();
}

struct Client<E> {
    handle: E,
    inner: BankClient<
        simulation_tonic::AddOrigin<hyper::client::conn::SendRequest<tonic::body::BoxBody>>,
    >,
}

impl<E> Client<E>
where
    E: Environment + Send + Sync + 'static,
{
    async fn new(handle: E, addr: net::SocketAddr) -> Self {
        let connector = simulation_tonic::Connector::new(handle.clone());
        let mut connector = hyper::client::service::Connect::new(
            connector,
            hyper::client::conn::Builder::new().http2_only(true).clone(),
        );
        let svc = connector.call(addr).await.unwrap();
        let client = BankClient::new(simulation_tonic::AddOrigin::new(
            svc,
            hyper::Uri::from_static("http://127.0.0.1:9092"),
        ));
        Client {
            handle: handle.clone(),
            inner: client,
        }
    }

    async fn query_balance(&mut self, account_id: i32) -> i32 {

        loop {
            let request = BalanceQueryRequest { account_id };
            match self.handle.timeout(self.inner.balance_query(request), time::Duration::from_secs(5)).await {
                Ok(Ok(response)) => return response.get_ref().account_balance,
                Ok(Err(e)) => panic!("server error: {}", e),
                Err(_) => {
                    self.handle.delay_from(time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn deposit(&mut self, account_id: i32, amount: i32) -> i32 {
        loop {
            let request = DepositRequest { account_id, amount };
            match self
                .handle
                .timeout(self.inner.deposit(request), time::Duration::from_secs(5))
                .await
            {
                Ok(Ok(response)) => {
                    return response.get_ref().new_balance;
                }
                Ok(Err(e)) => panic!("server error: {}", e),
                Err(_) => {
                    // after a timeout, wait for a bit and try to resend
                    self.handle.delay_from(time::Duration::from_secs(1)).await;
                }
            }
        }
    }
}

fn run_bank_simulation(seed: u64) {
    let mut runtime = DeterministicRuntime::new_with_seed(seed).unwrap();
    let latency_fault = runtime.latency_fault();
    let handle = runtime.localhost_handle();
    runtime.block_on(async {
        handle.spawn(latency_fault.run());
        handle.spawn(start_server(handle.clone()));
        let mut client = Client::new(handle.clone(), "127.0.0.1:9092".parse().unwrap()).await;
        client.deposit(1, 100).await;
        assert_eq!(client.query_balance(1).await, 100);
    });
}

#[test]
#[should_panic]
fn test() {
    println!("starting test");
    for seed in 0..100 {
        println!("attempting seed {}", seed);
        run_bank_simulation(seed);
    }
}

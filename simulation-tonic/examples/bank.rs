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
use simulation_tonic::AddOrigin;
use std::{collections, net, time};
use tonic::{Request, Response, Status};
use tracing_subscriber;
use std::fmt::{Debug, Write};
use tracing_subscriber::fmt::time::FormatTime;
use std::time::UNIX_EPOCH;

#[derive(Debug)]
enum Error {
    Rpc(Status),
    Overdraft,
}

impl From<Status> for Error {
    fn from(e: Status) -> Self {
        Error::Rpc(e)
    }
}

#[derive(Default)]
struct BankHandler {
    /// mapping from user account to balance
    vault: tokio::sync::Mutex<collections::HashMap<i32, i32>>,
}

impl BankHandler {
    fn new() -> Self {
        Self {
            vault: tokio::sync::Mutex::new(collections::HashMap::new()),
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
    let listener = TcpListener::into_stream(listener);
    let service = BankServer::new(bank_handler);
    tonic::transport::Server::builder()
        .add_service(service)
        .serve_from_stream(listener)
        .await
        .unwrap();
}

type Connect<E> = hyper::client::service::Connect<simulation_tonic::Connector<E>, tonic::body::BoxBody, net::SocketAddr>;
type Layer<E> = tower::timeout::Timeout<tower_reconnect::Reconnect<Connect<E>, std::net::SocketAddr>>;

struct Client<E>
where
    E: Environment + Send + Sync + 'static,
{
    handle: E,
    inner: BankClient<AddOrigin<Layer<E>>>,
}

impl<E> Debug for Client<E> where E: Environment + Send + Sync + 'static{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "Client")
    }
}

impl<E> Client<E>
where
    E: Environment + Send + Sync + 'static + Clone,
{
    async fn new(handle: E, addr: net::SocketAddr) -> Self {
        let connector = simulation_tonic::Connector::new(handle.clone());
        let connection = hyper::client::conn::Builder::new().http2_only(true).clone();
        let service = hyper::client::service::Connect::new(connector, connection);
        let service = tower_reconnect::Reconnect::new(service, addr);
        let svc = tower::ServiceBuilder::new()
            .layer(simulation_tonic::AddOriginLayer::new(
                hyper::Uri::from_static("http://127.0.0.1:9092"),
            ))
            .timeout(time::Duration::from_secs(5))
            .service(service);
        let client = BankClient::new(svc);

        Client { inner: client, handle }
    }
    #[tracing_attributes::instrument]
    async fn query_balance(&mut self, account_id: i32) -> Result<i32, Error> {
        loop {
            match self.inner
                .balance_query(BalanceQueryRequest { account_id })
                .await
                .map(|r| r.get_ref().account_balance) {
                Err(_e) => self.handle.delay_from(time::Duration::from_secs(1)).await,
                Ok(result) => return Ok(result)
            }
        }
    }
    #[tracing_attributes::instrument]
    async fn deposit(&mut self, account_id: i32, amount: i32) -> Result<i32, Error> {
        loop {
            match self.inner
                .deposit(DepositRequest { account_id, amount })
                .await
                .map(|r| r.get_ref().new_balance) {
                Err(_e) => self.handle.delay_from(time::Duration::from_secs(1)).await,
                Ok(result) => return Ok(result)
            }
        }
    }

    #[tracing_attributes::instrument]
    async fn withdraw(&mut self, account_id: i32, amount: i32) -> Result<(), Error> {
        loop {
            let request = WithdrawRequest { account_id, amount };
            match self.inner.withdraw(request).await {
                Ok(response) => {
                    if response.get_ref().status == 0 {
                        return Ok(());
                    } else {
                        return Err(Error::Overdraft);
                    }
                }
                Err(_e) => self.handle.delay_from(time::Duration::from_secs(1)).await,
            }
        }
    }
}

// Creates a workload which periodically withdraws money from the provided account.
async fn withdraw_worker<E>(
    handle: E,
    server_addr: net::SocketAddr,
    account_id: i32,
    period: time::Duration,
) -> Result<(), Error>
where
    E: Environment + Send + Sync + 'static,
{
    let mut client = Client::new(handle.clone(), server_addr).await;
    loop {
        let balance = client.query_balance(account_id).await?;
        if balance > 0 {
            client.withdraw(account_id, 1).await?;
        } else {
            return Ok(());
        }
        handle.delay_from(period).await;
    }
}

struct TracingClock<E> {
    base: time::Instant,
    inner: E
}

impl<E> TracingClock<E> where E: Environment {
    fn new(inner: E) -> Self {
        let base = inner.now();
        Self {base, inner}
    }
}

impl<E> FormatTime for TracingClock<E> where E: Environment {
    fn format_time(&self, w: &mut std::fmt::Write) -> Result<(), std::fmt::Error> {
        let since_base = self.inner.now().duration_since(self.base);
        write!(w, "SimTime {}", since_base.as_millis())
    }
}

fn run_bank_simulation(seed: u64) {
    let mut runtime = DeterministicRuntime::new_with_seed(seed).unwrap();
    let latency_fault = runtime.latency_fault();
    let handle = runtime.localhost_handle();
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_timer(TracingClock::new(handle.clone()))
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing::info_span!("DeterministicRuntime", seed = seed).in_scope(|| {
        let start_time = handle.now();
        runtime.block_on(async {
            handle.spawn(latency_fault.run());
            let server_addr: net::SocketAddr = "127.0.0.1:9092".parse().unwrap();
            handle.spawn(start_server(handle.clone()));
            let mut client = Client::new(handle.clone(), server_addr).await;
            client.deposit(1, 100).await.unwrap();
            if let Err(e) = futures::future::try_join(
                withdraw_worker(handle.clone(), server_addr, 1, time::Duration::from_millis(10)),
                withdraw_worker(handle.clone(), server_addr, 1, time::Duration::from_millis(5)),
            )
                .await
            {
                tracing::error!("Error running simulation: {:?}", e);
            }
        });
    })
}

const SKIP: &'static [u64] = &[
    8, // https://github.com/hyperium/h2/issues/417
    202, // https://github.com/hyperium/h2/issues/417
    295, // https://github.com/hyperium/h2/issues/417
    325, // https://github.com/hyperium/h2/issues/417
    430, // https://github.com/hyperium/h2/issues/417
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        let seed = args[1].parse().unwrap();
        run_bank_simulation(seed);
    } else {
        for seed in (*SKIP.last().unwrap_or(&0))..50000 {
            if SKIP.contains(&seed) {
                continue
            }
            run_bank_simulation(seed);
        }
    }

}

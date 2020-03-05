//! Simulation contains the state for a simulation run. A simulation
//! run is a determinstic test run with variance introduced via a seed.
use crate::{
    net::tcp::{SimulatedTcpListener, SimulatedTcpStream},
    state::{LogicalMachine, LogicalMachineId},
};
use futures::stream::FuturesUnordered;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    future::Future,
    io, net,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{runtime::Runtime, stream::StreamExt, task::JoinHandle};

#[derive(Debug)]
struct State {
    seed: u64,
    next_machineid: u64,
    machines: HashMap<LogicalMachineId, LogicalMachine>,
    hostnames: HashSet<String>,
}

impl State {
    fn new(seed: u64) -> Self {
        State {
            seed,
            next_machineid: 0,
            machines: HashMap::new(),
            hostnames: HashSet::new(),
        }
    }

    fn next_machineid(&mut self) -> LogicalMachineId {
        if self.next_machineid == std::u64::MAX {
            todo!("handle garbage collection of machine ids");
        }
        let new = self.next_machineid;
        self.next_machineid += 1;
        LogicalMachineId::new(new)
    }

    fn unused_ipaddr(&self) -> net::IpAddr {
        let all = self
            .machines
            .values()
            .map(|m| m.localaddr())
            .collect::<Vec<_>>();
        crate::util::find_unused_ipaddr(&all)
    }

    /// Register a new [LogicalMachine] under this [Simulation] for the provided
    /// hostname.
    ///
    /// [LogicalMachineId]:struct.LogicalMachineId.html
    /// [Simulation]:struct.Simulation.html
    fn register_machine<S: Into<String>>(&mut self, hostname: S) -> &mut LogicalMachine {
        let id = self.next_machineid();
        let hostname: String = hostname.into();
        if self.hostnames.contains(&hostname) {
            panic!("cannot register the same hostname twice");
        }
        let ipaddr = self.unused_ipaddr();
        let machine = LogicalMachine::new(id, hostname.clone(), ipaddr);
        self.machines.insert(id, machine);
        self.hostnames.insert(hostname);
        self.machines.get_mut(&id).unwrap()
    }

    /// Returns a reference to the [LogicalMachine] associated with the provided
    /// [LogicalMachineId]
    ///
    /// [LogicalMachine]:struct.LogicalMachine.html
    /// [LogicalMachineId]:struct.LogicalMachineId.html
    fn machine(&mut self, id: LogicalMachineId) -> &mut LogicalMachine {
        // This should never fail as machines are never removed once created.
        self.machines.get_mut(&id).expect("logical machine lost")
    }

    /// Resolve a hostname to a [LogicalMachineId].    
    ///
    /// [LogicalMachineId]:struct.LogicalMachineId.html
    fn resolve<S: Into<String>>(&self, hostname: S) -> Option<LogicalMachineId> {
        let hostname = hostname.into();
        for (id, machine) in self.machines.iter() {
            if machine.hostname() == hostname {
                return Some(*id);
            }
        }
        None
    }

    fn lookup(&self, addr: net::SocketAddr) -> Option<LogicalMachineId> {
        for (id, machine) in self.machines.iter() {
            if machine.localaddr() == addr.ip() {
                return Some(*id);
            }
        }
        None
    }
}

#[derive(Debug)]
pub struct Simulation {
    runtime: Runtime,
    handles: FuturesUnordered<JoinHandle<()>>,
    state: Arc<Mutex<State>>,
}

impl Simulation {
    pub fn new(seed: u64) -> Self {
        let runtime = tokio::runtime::Builder::new()
            .enable_time()
            .freeze_time()
            .basic_scheduler()
            .build()
            .unwrap();
        let inner = State::new(seed);
        let state = Arc::new(Mutex::new(inner));
        Self {
            runtime,
            state,
            handles: FuturesUnordered::new(),
        }
    }

    pub fn handle(&self) -> SimulationHandle {
        let rt_handle = self.runtime.handle().clone();
        let state = Arc::clone(&self.state);
        SimulationHandle { rt_handle, state }
    }

    /// Construct and run a future under a new simulation context.
    pub fn machine<F, S>(&mut self, hostname: S, f: F) -> &mut Self
    where
        F: Future<Output = ()> + Send + 'static,
        S: Into<String>,
    {
        let future = {
            let mut state = self.state.lock().unwrap();
            let machine = state.register_machine(hostname);

            machine.register_task(f)
        };

        let handle = self.runtime.spawn(future);
        self.handles.push(handle);
        self
    }

    /// Run the simulation, waiting on termination of all simulation contexts.
    pub fn run(&mut self) {
        let handle = self.handle();
        with_handle(handle, || {
            let handles = &mut self.handles;
            self.runtime
                .block_on(async { for _ in handles.next().await {} })
        });
    }

    /// Run the simulation using the default "localhost" context.
    pub fn simulate<F>(&mut self, f: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let future = {
            let mut state = self.state.lock().unwrap();
            let machine = state.register_machine("localhost");
            let handle = self.handle();
            machine.register_task(f)
        };
        let handle = self.handle();
        with_handle(handle, || self.runtime.block_on(future))
    }
}

thread_local! {
    static CURRENT_HANDLE: RefCell<Option<SimulationHandle>> = RefCell::new(None);
}

fn with_handle<F, U>(handle: SimulationHandle, f: F) -> U
where
    F: FnOnce() -> U,
{
    struct DropGuard;
    impl Drop for DropGuard {
        fn drop(&mut self) {
            CURRENT_HANDLE.with(|cx| cx.borrow_mut().take());
        }
    }
    CURRENT_HANDLE.with(|cx| *cx.borrow_mut() = Some(handle));
    let _guard = DropGuard;
    f()
}

#[derive(Debug, Clone)]
pub struct SimulationHandle {
    rt_handle: tokio::runtime::Handle,
    state: Arc<Mutex<State>>,
}

impl SimulationHandle {
    pub(crate) fn opt() -> Option<Self> {
        CURRENT_HANDLE.with(|cx| cx.borrow().clone())
    }

    pub fn current() -> Self {
        CURRENT_HANDLE
            .with(|cx| cx.borrow().clone())
            .expect("must be called from within simulation")
    }

    fn machine_id(&self) -> LogicalMachineId {
        let current =
            crate::state::task::current_taskid().expect("called outside of machine context");
        current.machine()
    }

    pub fn hostname(&self) -> String {
        let machineid = self.machine_id();
        let mut lock = self.state.lock().unwrap();
        let machine = lock.machine(machineid);
        machine.hostname()
    }

    pub fn bind(&self, port: u16) -> SimulatedTcpListener {
        let machineid = self.machine_id();
        let mut lock = self.state.lock().unwrap();
        let machine = lock.machine(machineid);
        machine.bind(port)
    }

    pub fn poll_connect(
        &self,
        cx: &mut Context<'_>,
        addr: net::SocketAddr,
    ) -> Poll<Result<SimulatedTcpStream, io::Error>> {
        let mut lock = self.state.lock().unwrap();
        let local_machine_id = self.machine_id();

        if let Some(remote_machine_id) = lock.lookup(addr) {
            // Safety: We ensure that the inidices we're borrowing here are unique.
            let (local_machine, remote_machine): (&mut LogicalMachine, &mut LogicalMachine) = unsafe {
                assert_ne!(local_machine_id, remote_machine_id);
                let local_machine = lock.machine(local_machine_id) as *mut _;
                let remote_machine = lock.machine(remote_machine_id) as *mut _;
                (&mut *local_machine, &mut *remote_machine)
            };
            local_machine.poll_connect(cx, addr.port(), remote_machine)
        } else {
            Poll::Ready(Err(io::ErrorKind::ConnectionRefused.into()))
        }
    }

    pub async fn connect(
        &self,
        addr: std::net::SocketAddr,
    ) -> Result<SimulatedTcpStream, io::Error> {
        futures::future::poll_fn(|cx| self.poll_connect(cx, addr)).await
    }

    pub fn resolve<S: Into<String>>(&self, hostname: S) -> Option<net::IpAddr> {
        let mut lock = self.state.lock().unwrap();
        if let Some(machineid) = lock.resolve(hostname) {
            return Some(lock.machine(machineid).localaddr());
        }
        None
    }
}

impl crate::api::ExecutorHandle for SimulationHandle {
    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let machineid = self.machine_id();
        let mut lock = self.state.lock().unwrap();
        let machine = lock.machine(machineid);
        tokio::spawn(machine.register_task(future))
    }

    fn spawn_local<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let machineid = self.machine_id();
        let mut lock = self.state.lock().unwrap();
        let machine = lock.machine(machineid);
        tokio::task::spawn_local(machine.register_task(future))
    }

    fn spawn_blocking<F, R>(&self, f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let machineid = self.machine_id();
        let mut lock = self.state.lock().unwrap();
        let machine = lock.machine(machineid);
        // Just block the event loop here to ensure deterministic execution is
        // maintained. It might be worth considering emitting a warning in the future
        // though, as this could hinder simulation speed.
        tokio::spawn(machine.register_task(async { f() }))
    }
}

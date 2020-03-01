#![allow(dead_code, unused_variables, unreachable_pub)]

/*
use std::{collections, io, net, num, string::ToString, sync};
mod machine;
pub mod tcp;
mod util;
use tokio::task::JoinHandle;
use machine::LogicalMachine;
pub(crate) use machine::SimTask;
pub use machine::{current_machineid, LogicalMachineId};
use std::future::Future;

#[derive(Debug)]
pub(self) struct State {
    next_machine_id: usize,
    id_to_machine: collections::HashMap<LogicalMachineId, LogicalMachine>,
    hostname_to_machineid: collections::HashMap<String, LogicalMachineId>,
    ipaddr_to_machineid: collections::HashMap<net::IpAddr, LogicalMachineId>,
}

impl State {
    /// Iterates through all logical machines looking for an unused ip address.
    fn unused_ipaddr(&mut self) -> net::IpAddr {
        let ipaddrs = self
            .id_to_machine
            .values()
            .map(|v| v.ipaddr())
            .collect::<Vec<_>>();
        util::find_unused_ipaddr(&ipaddrs)
    }
}

impl State {
    fn register_machine<T>(&mut self, hostname: T) -> LogicalMachineId
    where
        T: ToString,
    {
        let id = LogicalMachineId::new(self.next_machine_id);
        self.next_machine_id += 1;
        let machine_ipaddr = self.unused_ipaddr();
        let machine = LogicalMachine::new(id, hostname.to_string(), machine_ipaddr);
        self.id_to_machine.insert(id, machine);
        self.hostname_to_machineid.insert(hostname.to_string(), id);
        self.ipaddr_to_machineid.insert(machine_ipaddr, id);
        id
    }
}

/// Contains all state for a simulation run.
#[derive(Debug)]
pub struct Simulation {
    inner: sync::Arc<sync::Mutex<State>>,
}

impl Simulation {
    pub fn new() -> Self {
        let state = State {
            next_machine_id: 0,
            id_to_machine: collections::HashMap::new(),
            hostname_to_machineid: collections::HashMap::new(),
            ipaddr_to_machineid: collections::HashMap::new(),
        };
        let inner = sync::Arc::new(sync::Mutex::new(state));
        let mut simulation = Self { inner };
        // register a default logical machine at LogicalMachineId 0;
        let machineid = simulation.register_machine("localhost");
        machine::set_current_machineid(machineid);
        simulation
    }

    // Register a new logical machine with the simulation.
    fn register_machine<T>(&mut self, hostname: T) -> LogicalMachineId
    where
        T: ToString,
    {
        let mut lock = self.inner.lock().unwrap();
        lock.register_machine(hostname)
    }

    pub fn handle(&self) -> SimulationHandle {
        let inner = sync::Arc::clone(&self.inner);
        SimulationHandle { inner }
    }
}

#[derive(Debug, Clone)]
pub struct SimulationHandle {
    inner: sync::Arc<sync::Mutex<State>>,
}

impl SimulationHandle {
    pub fn resolve_tuple(
        
        &self,
        &(addr, port): &(&str, u16),
    ) -> Result<std::vec::IntoIter<net::SocketAddr>, io::Error> {
        let lock = self.inner.lock().unwrap();
        let target_machineid = lock.hostname_to_machineid.get(addr).ok_or(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "host could not be reached",
        ))?;
        let machine = lock
            .id_to_machine
            .get(target_machineid)
            .expect("expected machine set to never be modified");
        let target_ipaddr = machine.ipaddr();
        let target_socketaddr = net::SocketAddr::new(target_ipaddr.into(), port);
        Ok(vec![target_socketaddr].into_iter())
    }

    /// Bind a new TcpListener to the provided machineid port. A port value of 0
    /// will bind to a random port.
    ///
    /// Returns a TcpListener if the machineid is present and binding is successful.
    pub fn bind(&self, port: u16) -> io::Result<tcp::SimTcpListener> {
        let current_machineid = current_machineid();
        let mut lock = self.inner.lock().unwrap();
        let machine = lock
            .id_to_machine
            .get_mut(&current_machineid)
            .expect("could not find associated logical machine");
        machine.bind_listener(port)
    }

    pub async fn connect(&self, addr: std::net::SocketAddr) -> io::Result<tcp::SimTcpStream> {
        let current_machineid = current_machineid();

        let (ipaddr, port) = (addr.ip(), addr.port());
        let machineid = {
            let lock = self.inner.lock().unwrap();
            lock.ipaddr_to_machineid
                .get(&ipaddr)
                .ok_or(io::ErrorKind::ConnectionReset)
                .map(|v| *v)
        }?;

        // TODO: Use the correct error for a 0 port
        let port = num::NonZeroU16::new(port).ok_or(io::ErrorKind::InvalidInput)?;

        let fut = {
            let mut lock = self.inner.lock().unwrap();
            let target_machine = lock.id_to_machine.get(&current_machineid).unwrap();
            let machine = lock.id_to_machine.get_mut(&machineid).unwrap();
            machine.connect(port)
        };
        fut.await
    }

    fn register_machine<T>(&self, hostname: T) -> LogicalMachineId
    where
        T: ToString,
    {
        let mut lock = self.inner.lock().unwrap();
        lock.register_machine(hostname)
    
    }
}

*/
#![allow(dead_code, unused_variables)]
use std::{collections, io, net, num, sync};
mod machine;
pub mod tcp;
use machine::{LogicalMachine, LogicalMachineId};
mod util;

struct State {
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

/// Contains all state for a simulation run.
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
        Self { inner }
    }

    // Register a new logical machine with the simulation.
    fn register<T>(&mut self, hostname: T) -> MachineContext
    where
        T: std::string::ToString,
    {
        let mut lock = self.inner.lock().unwrap();
        let id = LogicalMachineId::new(lock.next_machine_id);
        lock.next_machine_id += 1;
        let machine_ipaddr = lock.unused_ipaddr();
        let machine = LogicalMachine::new_with_tags(
            id,
            hostname.to_string(),
            machine_ipaddr,
            collections::HashMap::new(),
        );
        lock.id_to_machine.insert(id, machine);
        lock.hostname_to_machineid.insert(hostname.to_string(), id);
        lock.ipaddr_to_machineid.insert(machine_ipaddr, id);
        MachineContext {
            machineid: id,
            inner: sync::Arc::clone(&self.inner),
        }
    }
}

pub struct MachineContext {
    machineid: LogicalMachineId,
    inner: sync::Arc<sync::Mutex<State>>,
}

pub trait ResolveMachine {
    fn resolve(
        &self,
        state: &MachineContext,
    ) -> Result<(LogicalMachineId, num::NonZeroU16), io::Error>;
}

impl<T: ResolveMachine + ?Sized> ResolveMachine for &T {
    fn resolve(
        &self,
        state: &MachineContext,
    ) -> Result<(LogicalMachineId, num::NonZeroU16), io::Error> {
        (**self).resolve(state)
    }
}

impl ResolveMachine for net::SocketAddr {
    fn resolve(
        &self,
        state: &MachineContext,
    ) -> Result<(LogicalMachineId, num::NonZeroU16), io::Error> {
        let lock = state.inner.lock().unwrap();
        let id = lock
            .ipaddr_to_machineid
            .get(&self.ip())
            .ok_or(io::ErrorKind::NotFound)?;
        let port = num::NonZeroU16::new(self.port())
            .ok_or(io::Error::new(io::ErrorKind::Other, "port cannot be zero"))?;
        Ok((*id, port))
    }
}

impl ResolveMachine for (net::IpAddr, u16) {
    fn resolve(
        &self,
        state: &MachineContext,
    ) -> Result<(LogicalMachineId, num::NonZeroU16), io::Error> {
        let addr = Some(net::SocketAddr::from(*self)).ok_or(io::ErrorKind::InvalidData)?;
        addr.resolve(state)
    }
}

impl ResolveMachine for str {
    fn resolve(
        &self,
        state: &MachineContext,
    ) -> Result<(LogicalMachineId, num::NonZeroU16), io::Error> {
        let res: Result<net::SocketAddr, _> = self.parse();
        if let Ok(addr) = res {
            return addr.resolve(state);
        }
        let split = self.split(':').collect::<Vec<_>>();
        if split.len() != 2 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("failed to parse hostport {}", self),
            ));
        } else {
            let hostname = split[0];
            let port: u16 = split[1]
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "failed to parse port"))?;
            let port = num::NonZeroU16::new(port)
                .ok_or(io::Error::new(io::ErrorKind::Other, "port cannot be zero"))?;
            let lock = state.inner.lock().unwrap();
            let id = lock
                .hostname_to_machineid
                .get(hostname)
                .ok_or(io::Error::new(
                    io::ErrorKind::Other,
                    "could not resolve hostname",
                ))?;
            Ok((*id, port))
        }
    }
}

impl ResolveMachine for String {
    fn resolve(
        &self,
        state: &MachineContext,
    ) -> Result<(LogicalMachineId, num::NonZeroU16), io::Error> {
        (&self[..]).resolve(state)
    }
}

impl MachineContext {
    /// Bind a new TcpListener to the provided machineid port. A port value of 0
    /// will bind to a random port.
    ///
    /// Returns a TcpListener if the machineid is present and binding is successful.
    pub fn bind(&self, port: u16) -> io::Result<tcp::TcpListener> {
        let mut lock = self.inner.lock().unwrap();
        let machine = lock
            .id_to_machine
            .get_mut(&self.machineid)
            .expect("could not find associated logical machine");
        machine.bind_listener(port)
    }

    pub async fn connect<T>(&self, addr: T) -> io::Result<tcp::TcpStream>
    where
        T: ResolveMachine,
    {
        let (machineid, port) = addr.resolve(self)?;

        let fut = {
            let mut lock = self.inner.lock().unwrap();
            let client_ipaddr = lock.id_to_machine.get(&self.machineid).unwrap().ipaddr();
            let client_addr = net::SocketAddr::new(client_ipaddr, 9999);
            let machine = lock.id_to_machine.get_mut(&machineid).unwrap();
            machine.connect(client_addr, port)
        };
        fut.await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio::runtime::Runtime;
    use tokio_util::codec::{Framed, LinesCodec};

    #[test]
    fn bind_connect() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime: Runtime = Runtime::new()?;
        let mut simulation: Simulation = Simulation::new();
        let client: MachineContext = simulation.register("client.svc.local");
        let server: MachineContext = simulation.register("server.svc.local");
        runtime.block_on(async {
            let mut listener = server.bind(9092)?;
            tokio::spawn(async move {
                while let Some(Ok(conn)) = listener.incoming().next().await {
                    let mut transport = Framed::new(conn, LinesCodec::new());
                    transport.send("hello world!".to_owned()).await.unwrap();
                }
            });
            let conn = client.connect("server.svc.local:9092").await?;
            let mut transport = Framed::new(conn, LinesCodec::new());
            let response = transport.next().await.unwrap().unwrap();
            assert_eq!("hello world!", response);
            Ok(())
        })
    }
}

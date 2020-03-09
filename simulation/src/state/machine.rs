//! LogicalMachine contains the state associated with a single
//! logical machine for a simulation run.

use crate::fault::FaultInjector;
use crate::net::tcp::{SimulatedTcpListener, SimulatedTcpListenerHandle, SimulatedTcpStream};
use crate::state::{task::wrap_task, LogicalMachineId, LogicalTaskId};
use futures::ready;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io, net,
    num::NonZeroU16,
    task::{Context, Poll},
};

#[derive(Debug)]
pub struct LogicalMachine {
    id: LogicalMachineId,
    next_taskid: u64,
    hostname: String,
    localaddr: net::IpAddr,
    bound: HashMap<NonZeroU16, SimulatedTcpListenerHandle>,
    fault_injector: Option<FaultInjector>,
}

impl LogicalMachine {
    /// Construct a new [LogicalMachine] keyed by the provided id and hostname.
    ///
    /// [LogicalMachine]:
    pub(crate) fn new<S: Into<String>>(
        id: LogicalMachineId,
        hostname: S,
        addr: net::IpAddr,
        fault_injector: Option<FaultInjector>,
    ) -> Self {
        Self {
            id,
            next_taskid: 0,
            hostname: hostname.into(),
            localaddr: addr,
            bound: HashMap::new(),
            fault_injector,
        }
    }

    /// Construct a new [LogicalTaskId] associated with this machine. Each call to this
    /// function will produce a unique [LogicalTaskId].
    ///
    /// # Panic
    ///
    /// Panics if this [LogicalMachine] has allocated more than u64::MAX [LogicalTaskId]'s.
    ///
    /// [LogicalMachine]:struct.LogicalMachine.html
    /// [LogicalTaskId]:struct.LogicalTaskId.html
    fn new_taskid(&mut self) -> LogicalTaskId {
        if self.next_taskid == std::u64::MAX {
            todo!("handle garbage collection of task ids")
        }
        let new = self.id.new_task(self.next_taskid);
        self.next_taskid += 1;
        new
    }

    pub(crate) fn register_task<F>(&mut self, future: F) -> impl Future<Output = F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let taskid = self.new_taskid();
        let fault_injector = self.fault_injector.clone();
        let task = wrap_task(taskid, fault_injector, future);
        return task;
    }

    fn unused_port(&self) -> NonZeroU16 {
        let mut start = 0;
        let occupied = self
            .bound
            .keys()
            .into_iter()
            .map(|v| v.get())
            .collect::<HashSet<_>>();
        loop {
            if start == 0 {
                panic!("out of available ports")
            }
            if !occupied.contains(&start) {
                return NonZeroU16::new(start).unwrap();
            }
            start -= 1;
        }
    }

    /// Returns the hostname associated with this logical machine.
    pub(crate) fn hostname(&self) -> String {
        self.hostname.clone()
    }

    pub(crate) fn bind(&mut self, port: u16) -> SimulatedTcpListener {
        self.garbage_collect();
        let port = if let Some(port) = NonZeroU16::new(port) {
            port
        } else {
            self.unused_port()
        };

        let socketaddr = net::SocketAddr::new(self.localaddr, port.get());
        let (listener, handle) = SimulatedTcpListener::new(socketaddr);
        // TODO: handle binding to the same port twice
        self.bound.insert(port, handle);
        listener
    }

    /// Connect to the remote machine.
    pub(crate) fn poll_connect(
        &mut self,
        cx: &mut Context<'_>,
        port: u16, //TODO: we need a "register" kind of approach here
        // where we don't pass in the entire remote machine but we pass in
        // what's needed from it.W
        remote: &mut LogicalMachine,
    ) -> Poll<Result<SimulatedTcpStream, io::Error>> {
        self.garbage_collect();
        let local_addr = net::SocketAddr::new(self.localaddr(), 9999);
        let remote_addr = net::SocketAddr::new(remote.localaddr(), port);
        let fault_injector = self.fault_injector.clone();
        let (client, server) =
            SimulatedTcpStream::new_pair(local_addr, remote_addr, fault_injector);
        if let Some(remote_port) = NonZeroU16::new(port) {
            if let Some(remote) = remote.bound.get_mut(&remote_port) {
                ready!(remote.poll_enqueue_incoming(cx, server))?;
                Poll::Ready(Ok(client))
            } else {
                Poll::Ready(Err(io::ErrorKind::ConnectionRefused.into()))
            }
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot connect to 0 port",
            )))
        }
    }

    pub(crate) fn localaddr(&self) -> net::IpAddr {
        self.localaddr
    }

    pub(crate) fn garbage_collect(&mut self) {
        let mut garbage = vec![];
        for (k, v) in self.bound.iter() {
            if v.dropped() {
                garbage.push(*k)
            }
        }
        for port in garbage {
            self.bound.remove(&port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_machine() {
        let id = LogicalMachineId::new(0);
        let mut machine = LogicalMachine::new(id, "client", net::Ipv4Addr::LOCALHOST.into(), None);
        let future = machine
            .register_task(async { assert!(crate::state::task::current_taskid().is_some()) });
        let mut runtime = tokio::runtime::Builder::new()
            .basic_scheduler()
            .build()
            .unwrap();
        runtime.block_on(future);
    }
}

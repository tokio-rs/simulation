//! LogicalMachine contains the state associated with a single
//! logical machine for a simulation run.
use crate::state::{task::wrap_task, LogicalMachineId, LogicalTaskHandle, LogicalTaskId};
use crate::tcp;
use crate::tcp::TcpListenerHandle;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net,
    num::NonZeroU16,
};

#[derive(Debug)]
pub struct LogicalMachine {
    id: LogicalMachineId,
    next_taskid: u64,
    hostname: String,
    localaddr: net::IpAddr,
    bound: HashMap<NonZeroU16, TcpListenerHandle>,
    tasks: HashMap<LogicalTaskId, LogicalTaskHandle>,
}

impl LogicalMachine {
    /// Construct a new [LogicalMachine] keyed by the provided id and hostname.
    ///
    /// [LogicalMachine]:
    pub(crate) fn new<S: Into<String>>(
        id: LogicalMachineId,
        hostname: S,
        addr: net::IpAddr,
    ) -> Self {
        Self {
            id,
            next_taskid: 0,
            hostname: hostname.into(),
            localaddr: addr,
            bound: HashMap::new(),
            tasks: HashMap::new(),
        }
    }

    /// Returns the [LogicalMachineId] associated with this [LogicalMachine].
    ///
    /// [LogicalMachineId]:struct.LogicalMachineId.html
    /// [LogicalMachine]:struct.LogicalMachine.html
    pub(crate) fn id(&self) -> LogicalMachineId {
        self.id
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
        let (handle, task) = wrap_task(taskid, future);
        self.tasks.insert(taskid, handle);
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

    pub(crate) fn bind(&mut self, port: u16) -> tcp::TcpListener {
        let port = if let Some(port) = NonZeroU16::new(port) {
            port
        } else {
            self.unused_port()
        };

        let socketaddr = net::SocketAddr::new(self.localaddr, port.get());
        let (listener, handle) = tcp::TcpListener::new(socketaddr);
        // TODO: handle binding to the same port twice
        self.bound.insert(port, handle);
        listener
    }

    pub(crate) fn localaddr(&self) -> net::IpAddr {
        self.localaddr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_machine() {
        let id = LogicalMachineId::new(0);
        let mut machine = LogicalMachine::new(id, "client", net::Ipv4Addr::LOCALHOST.into());
        let future = machine
            .register_task(async { assert!(crate::state::task::current_taskid().is_some()) });
        let mut runtime = tokio::runtime::Builder::new()
            .basic_scheduler()
            .build()
            .unwrap();
        runtime.block_on(future);
    }
}

//! LogicalMachine contains the state associated with a single
//! logical machine for a simulation run.
use crate::state::{task::wrap_task, LogicalMachineId, LogicalTaskHandle, LogicalTaskId};
use crate::tcp::TcpListenerHandle;
use core::future::Future;
use core::num::NonZeroU16;
use std::collections::HashMap;

#[derive(Debug)]
pub struct LogicalMachine {
    id: LogicalMachineId,
    next_taskid: u64,
    hostname: String,
    bound: HashMap<NonZeroU16, TcpListenerHandle>,
    tasks: HashMap<LogicalTaskId, LogicalTaskHandle>,
}

impl LogicalMachine {
    /// Construct a new [LogicalMachine] keyed by the provided id and hostname.
    ///
    /// [LogicalMachine]:
    pub(crate) fn new<S: Into<String>>(id: LogicalMachineId, hostname: S) -> Self {
        Self {
            id,
            next_taskid: 0,
            hostname: hostname.into(),
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

    /// Returns the hostname associated with this logical machine.
    pub(crate) fn hostname(&self) -> String {
        self.hostname.clone()
    }
}

#[cfg(test)]
mod tests {}

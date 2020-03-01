//! LogicalMachine contains the state associated with a single
//! logical machine for a simulation run.
use crate::state::{task::wrap_task, LogicalMachineId, LogicalTaskHandle, LogicalTaskId};
use crate::tcp::TcpListenerHandle;
use core::future::Future;
use core::num::NonZeroU16;
use std::collections::HashMap;
use tokio::task::JoinHandle;

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

    fn register_task<F>(&mut self, future: F) -> crate::state::LogicalTaskWrapper<F>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let taskid = self.new_taskid();
        let (handle, task) = wrap_task(taskid, future);
        self.tasks.insert(taskid, handle);
        return task;
    }

    /// Spawns a task on this [LogicalMachine].
    ///
    /// The task will have an associated handle registered with this [LogicalMachine]
    /// which can be used to dynamically manipulate the running task.
    ///
    /// [LogicalMachine]:struct.LogicalMachine.html
    /// [LogicalTaskId]:struct.LogicalTaskId.html
    pub(crate) fn spawn_task<F>(&mut self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task = self.register_task(future);
        tokio::spawn(task)
    }

    /// Spawns a task on this [LogicalMachine].
    ///
    /// The task will have an associated handle registered with this [LogicalMachine]
    /// which can be used to dynamically manipulate the running task.    
    ///
    /// [LogicalMachine]:struct.LogicalMachine.html
    /// [LogicalTaskId]:struct.LogicalTaskId.html    
    pub(crate) fn spawn_local_task<F>(&mut self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let task = self.register_task(future);
        tokio::task::spawn_local(task)
    }

    /// Spawns a closure on this [LogicalMachine]. The closure
    ///
    /// The task will have an associated handle registered with this [LogicalMachine]
    /// which can be used to dynamically manipulate the running task.
    ///
    /// [LogicalMachine]:struct.LogicalMachine.html
    /// [LogicalTaskId]:struct.LogicalTaskId.html    
    pub(crate) fn spawn_blocking_task<F, R>(&mut self, f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        // In simulation mode, just block. This way we can take advantage
        // of task poll delays to introduce a bit of reordering, and for most operations,
        // this should allow the simulation to remain deterministic.
        let future = async { f() };
        let task = self.register_task(future);
        tokio::task::spawn(task)
    }

    /// Returns the hostname associated with this logical machine.
    pub(crate) fn hostname(&self) -> String {
        self.hostname.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::task::current_taskid;
    use tokio::runtime::Builder;

    /// Test that tasks spawned on a logical machine inherit the logical
    /// machines id.
    #[test]
    fn task_spawn_machine_context() {
        let mut rt = Builder::new().basic_scheduler().build().unwrap();
        rt.block_on(async {
            let id = LogicalMachineId::new(42);
            let mut machine = LogicalMachine::new(id, "hostname");
            let spawn1 = machine
                .spawn_task(async { current_taskid() })
                .await
                .unwrap()
                .unwrap();
            let spawn2 = machine
                .spawn_task(async { current_taskid() })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(spawn1.machine(), machine.id());
            assert_eq!(spawn2.machine(), machine.id());
            assert_ne!(spawn1, spawn2, "task ids should be unique");
        });
    }
}

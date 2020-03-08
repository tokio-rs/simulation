/// Identifier used to associate resources with a particular [LogicalMachine].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogicalMachineId(u64);

impl LogicalMachineId {
    /// Construct a new [LogicalMachineId].
    pub(crate) fn new(id: u64) -> Self {
        Self(id)
    }
    /// Create a new [LogicalTaskId] which belongs to this [LogicalMachineId]    
    pub(crate) fn new_task(&self, id: u64) -> LogicalTaskId {
        LogicalTaskId::new(*self, id)
    }
}

/// Identifier used to associate a task with a [LogicalMachine];
///
/// Each [LogicalTaskId] has a parent [LogicalMachineId] which can
/// be used to associate a task with a logical machine.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogicalTaskId(LogicalMachineId, u64);

impl LogicalTaskId {
    /// Construct a new [LogicalTaskId] which is a child of the provided [LogicalMachineId].
    pub(crate) fn new(machine: LogicalMachineId, id: u64) -> Self {
        LogicalTaskId(machine, id)
    }
    /// Get the [LogicalMachineId] which is the parent of this [LogicalTaskId].
    pub(crate) fn machine(&self) -> LogicalMachineId {
        self.0
    }
}

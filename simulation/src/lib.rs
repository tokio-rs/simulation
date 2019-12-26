#![allow(dead_code, unused_variables)]
use std::{collections, num};
mod machine;
pub mod tcp;

/// Contains all state for a simulation run.
pub struct Simulation {
    next_machine_id: usize,
    machines: collections::HashMap<MachineId, Machine>,
}

struct Machine {
    next_port: u16,
    tags: collections::HashMap<String, String>,
    acceptors: collections::HashMap<num::NonZeroU16, Acceptor>,
}

struct Acceptor {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct MachineId(usize);

impl Simulation {
    // Register a new logical machine with the simulation.
    fn register(
        &mut self,
        hostname: String,
        tags: collections::HashMap<String, String>,
    ) -> MachineId {
        let id = MachineId(self.next_machine_id);
        self.next_machine_id += 1;
        id
    }
}

mod deterministic;
mod singlethread;

pub use deterministic::{DeterministicRuntime, DeterministicRuntimeHandle};
pub use singlethread::{SingleThreadedRuntime, SingleThreadedRuntimeHandle};
use std::io;

#[derive(Debug)]
pub enum Error {
    Spawn {
        source: tokio_executor::SpawnError,
    },
    RuntimeBuild {
        source: io::Error,
    },
    CurrentThreadRun {
        source: tokio_executor::current_thread::RunError,
    },
}

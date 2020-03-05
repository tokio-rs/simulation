//! Simulation is a wrapper around Tokio which supports building
//! applications amenable to FoundationDB style simulation testing.
#![allow(dead_code, unused_imports, unused_variables)]
mod api;
pub use api::ExecutorHandle;
pub mod net;
mod spawn;
mod state;
mod util;
pub use spawn::spawn;
pub use state::{Simulation, SimulationHandle};
pub mod task {
    pub use crate::spawn::spawn_blocking;
    pub use crate::spawn::spawn_local;
}

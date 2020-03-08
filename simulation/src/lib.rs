//! Simulation is a wrapper around Tokio which supports building
//! applications amenable to FoundationDB style simulation testing.
mod api;
pub use api::ExecutorHandle;
pub mod net;
mod spawn;
mod state;
mod util;
pub use net::tcp::SimulatedTcpStream;
pub use spawn::spawn;
pub use state::LogicalTaskHandle;
pub use state::{Simulation, SimulationHandle};
pub mod task {
    pub use crate::spawn::spawn_blocking;
    pub use crate::spawn::spawn_local;
}
pub mod fault;

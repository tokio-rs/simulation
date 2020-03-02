mod link;
mod listener;
mod stream;

use link::Link;
pub use listener::TcpListener;
pub(crate) use listener::{SimulatedTcpListener, SimulatedTcpListenerHandle};
use std::net;
pub use stream::TcpStream;
pub(crate) use stream::{SimulatedTcpStream, SimulatedTcpStreamHandle};

pub trait Resolveable {
    fn resolve(&self) -> net::SocketAddr;
}

impl TcpStream {
    /*
    pub async fn connect<R: Resolveable>(addr: R) -> Self {
        let addr = addr.resolve();
        if let Some(handle) = crate::state::SimulationHandle::opt() {
            handle.connect()
        }
    }
    */
}

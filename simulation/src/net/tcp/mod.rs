mod link;
mod listener;
mod stream;

use link::Link;
pub use listener::TcpListener;
pub(crate) use listener::{SimulatedTcpListener, SimulatedTcpListenerHandle};
use std::future::Future;
use std::{io, net, vec};
pub use stream::TcpStream;
pub(crate) use stream::{SimulatedTcpStream, SimulatedTcpStreamHandle};

fn resolve_simulation_addr(
    addr: &str,
    handle: crate::SimulationHandle,
) -> io::Result<vec::IntoIter<net::SocketAddr>> {
    fn err(msg: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, msg)
    }
    let mut parts_iter = addr.rsplitn(2, ':');
    let port_str = parts_iter.next().ok_or(err("invalid socket address"))?;
    let host = parts_iter.next().ok_or(err("invalid socket address"))?;
    let port: u16 = port_str.parse().map_err(|_| err("invalid port value"))?;
    let ipaddr = handle
        .resolve(host)
        .ok_or(io::Error::new(io::ErrorKind::NotFound, ""))?;
    let addr = net::SocketAddr::new(ipaddr, port);
    Ok(vec![addr].into_iter())
}

impl TcpStream {
    pub async fn connect(addr: &str) -> Result<Self, io::Error> {
        if let Some(handle) = crate::state::SimulationHandle::opt() {
            for addr in resolve_simulation_addr(addr, handle.clone())? {
                return Ok(handle.connect(addr).await?.into());
            }
            return Err(io::ErrorKind::NotFound.into());
        } else {
            tokio::net::TcpStream::connect(addr).await.map(Into::into)
        }
    }
}

impl TcpListener {
    pub async fn bind(port: u16) -> Result<Self, io::Error> {
        if let Some(handle) = crate::state::SimulationHandle::opt() {
            return Ok(handle.bind(port).into());
        } else {
            let addr = net::SocketAddr::new(net::Ipv4Addr::LOCALHOST.into(), port);
            Ok(tokio::net::TcpListener::bind(addr).await?.into())
        }
    }
}

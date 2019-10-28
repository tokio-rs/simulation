//! Inner state for the in memory network. This is shared between
//! network handles and allows for injecting faults at specific points in time.
use super::Listener;
use futures::channel::mpsc;
use std::{collections, io, net, num};

/// Inner state for the in memory network implementation and handles which interact with it.
#[derive(Debug)]
pub(crate) struct Inner<T> {
    /// Mapping from bound port number to `Sender`.
    ///
    /// `SocketHalf`'s placed onto the `Sender` channel will be delivered
    /// to any server bound to this `Network`.
    listeners: collections::HashMap<num::NonZeroU16, mpsc::Sender<T>>,

    /// Set of machines which have been connected, or have an outstanding network handle.
    connected: collections::HashSet<std::net::IpAddr>,

    /// Next available port to assign
    next_port: u16,
}

impl<T> Inner<T> {
    pub(crate) fn new() -> Self {
        Inner {
            listeners: collections::HashMap::new(),
            connected: collections::HashSet::new(),
            next_port: 0,
        }
    }
}

impl<T> Inner<T> {
    pub(crate) fn register_connected(&mut self, addr: std::net::IpAddr) -> Result<(), io::Error> {
        if self.connected.contains(&addr) {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("addr {} is already registered", addr),
            ))
        } else {
            Ok(())
        }
    }

    /// Check if the provided port is in use or not. If `port` is 0, assign a new
    /// port.
    fn free_port(&mut self, port: u16) -> Result<num::NonZeroU16, io::Error> {
        if let Some(port) = num::NonZeroU16::new(port) {
            if self.listeners.contains_key(&port) {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            Ok(port)
        } else {
            // pick next available port
            loop {
                if let Some(port) = num::NonZeroU16::new(self.next_port) {
                    self.next_port += 1;
                    if !self.listeners.contains_key(&port) {
                        return Ok(port);
                    }
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        String::from("could not find a port to bind to"),
                    ));
                }
            }
        }
    }

    /// Returns a new listener bound to the provided addr. If a listener is already bound, returns an error.
    /// To be assigned a listener with a random port, supply a addr with a port number of 0.
    pub(crate) fn bind_listener(
        &mut self,
        mut addr: net::SocketAddr,
    ) -> Result<Listener<T>, io::Error> {
        let proposed_port = addr.port();
        let real_port = self.free_port(proposed_port)?;
        addr.set_port(real_port.into());
        let (listener_tx, listener_rx) = mpsc::channel(1);
        let listener = Listener::new(addr, listener_rx);
        self.listeners.insert(real_port, listener_tx);
        Ok(listener)
    }

    /// Returns a connection channel if one is bound to the provided addr. If there is no connection
    /// channel bound, return an error.
    pub(crate) fn connection_channel(
        &mut self,
        addr: net::SocketAddr,
    ) -> Result<mpsc::Sender<T>, io::Error> {
        let server_port = num::NonZeroU16::new(addr.port()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "server port cannot be zero")
        })?;
        let chan = self.listeners.get_mut(&server_port).ok_or_else(|| {
            io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused")
        })?;
        Ok(chan.clone())
    }
}

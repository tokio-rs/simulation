use super::{ClientSocket, ServerSocket};
use futures::{channel::mpsc, SinkExt};
use std::{collections::HashMap, io, net, num, sync::Arc};
use try_lock::TryLock;

#[derive(Debug)]
pub(crate) struct Connections {
    /// listeners contains a mapping from port number to a channel where new
    /// sockets can be registered.
    listeners: HashMap<num::NonZeroU16, mpsc::Sender<ServerSocket>>,

    /// next_port is the next port which can be allocated.
    next_port: u16,
}

impl Connections {
    pub(crate) fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            next_port: 1,
        }
    }
    /// Check if the provided port is in use or not. If `port` is 0, assign a new
    /// port.
    fn free_port(&mut self, port: u16) -> Result<num::NonZeroU16, io::Error> {
        if let Some(port) = num::NonZeroU16::new(port) {
            if self.listeners.contains_key(&port) {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            return Ok(port);
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
    /// Returns the listener channel associated with the provided port (if there is one), otherwise
    /// returns an error.
    pub(crate) fn listener_channel(
        &mut self,
        port: num::NonZeroU16,
    ) -> Result<mpsc::Sender<ServerSocket>, io::Error> {
        self.listeners
            .get(&port)
            .map(Clone::clone)
            .ok_or(io::ErrorKind::AddrNotAvailable.into())
    }
    /// Registers a new listener channel for the specified port. If the specified port is 0, a random port will
    /// be selected and returned.
    pub(crate) fn register_listener_channel(
        &mut self,
        port: u16,
        chan: mpsc::Sender<ServerSocket>,
    ) -> Result<num::NonZeroU16, io::Error> {
        let port = self.free_port(port)?;
        self.listeners.insert(port, chan);
        Ok(port)
    }
}

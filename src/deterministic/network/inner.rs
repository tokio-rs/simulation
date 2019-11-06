use super::fault::{CloggedConnection, Connection};
use super::{socket, FaultyTcpStream, Listener, ListenerState, SocketHalf};
use futures::{channel::mpsc, Future, SinkExt};
use std::{
    collections::{self, hash_map::Entry},
    io, net,
};

#[derive(Debug)]
pub(crate) struct Inner {
    handle: crate::deterministic::DeterministicTimeHandle,
    pub(crate) connections: Vec<Connection>,
    clogged: collections::HashSet<CloggedConnection>,
    endpoints: collections::HashMap<net::SocketAddr, ListenerState>,
}

impl Inner {
    pub(crate) fn new(handle: crate::deterministic::DeterministicTimeHandle) -> Self {
        Inner {
            handle,
            connections: vec![],
            clogged: collections::HashSet::new(),
            endpoints: collections::HashMap::new(),
        }
    }
    fn register_new_connection_pair(
        &mut self,
        source: net::SocketAddr,
        dest: net::SocketAddr,
    ) -> Result<(FaultyTcpStream<SocketHalf>, FaultyTcpStream<SocketHalf>), io::Error> {
        if self
            .connections
            .iter()
            .map(|c| c.source())
            .collect::<Vec<_>>()
            .contains(&source)
        {
            return Err(io::ErrorKind::AddrInUse.into());
        }

        let (client, server) = socket::new_socket_pair(source, dest);
        let (client, client_fault_handle) =
            socket::FaultyTcpStream::wrap(self.handle.clone(), client);
        let (server, server_fault_handle) =
            socket::FaultyTcpStream::wrap(self.handle.clone(), server);
        let mut connection =
            Connection::new(source, dest, client_fault_handle, server_fault_handle);
        if self.should_clog(source, dest) {
            connection.clog();
        }
        self.connections.push(connection);
        Ok((client, server))
    }
    // find an unused socket port for the provided ipaddr.
    fn unused_socket_port(&self, addr: net::IpAddr) -> u16 {
        let mut start = 65535;
        let occupied: collections::HashSet<u16> = self
            .connections
            .iter()
            .filter(|v| v.source().ip() == addr)
            .map(|v| v.source().port())
            .collect();
        loop {
            if !occupied.contains(&start) {
                return start;
            }
            if start == 0 {}
            start -= 1;
        }
    }

    fn gc_dropped(&mut self) {
        let mut connections = vec![];
        for connection in self.connections.iter() {
            if !connection.is_dropped() {
                connections.push(connection.clone());
            }
        }
        self.connections = connections;
    }

    pub fn connect(
        &mut self,
        source: net::IpAddr,
        dest: net::SocketAddr,
    ) -> impl Future<Output = Result<socket::FaultyTcpStream<SocketHalf>, io::Error>> {
        self.gc_dropped();
        let free_socket_port = self.unused_socket_port(source);
        let source_addr = net::SocketAddr::new(source, free_socket_port);
        let registration = self.register_new_connection_pair(source_addr, dest);

        let mut channel;
        match self.endpoints.entry(dest) {
            Entry::Vacant(v) => {
                let (tx, rx) = mpsc::channel(1);
                let state = ListenerState::Unbound { tx: tx.clone(), rx };
                channel = tx;
                v.insert(state);
            }
            Entry::Occupied(o) => match o.get() {
                ListenerState::Bound { tx } => channel = tx.clone(),
                ListenerState::Unbound { tx, .. } => channel = tx.clone(),
            },
        }

        async move {
            let (client, server) = registration?;
            match channel.send(server).await {
                Ok(_) => Ok(client),
                Err(_) => Err(io::ErrorKind::ConnectionRefused.into()),
            }
        }
    }

    pub fn listen(&mut self, bind_addr: net::SocketAddr) -> Result<Listener, io::Error> {
        self.gc_dropped();
        match self.endpoints.remove(&bind_addr) {
            Some(listener_state) => {
                if let ListenerState::Unbound { tx, rx } = listener_state {
                    let listener = Listener::new(bind_addr, rx);
                    let new_state = ListenerState::Bound { tx };
                    self.endpoints.insert(bind_addr, new_state);
                    Ok(listener)
                } else {
                    self.endpoints.insert(bind_addr, listener_state);
                    Err(io::ErrorKind::AddrInUse.into())
                }
            }
            _ => {
                let (tx, rx) = mpsc::channel(1);
                let state = ListenerState::Bound { tx };
                self.endpoints.insert(bind_addr, state);
                let listener = Listener::new(bind_addr, rx);
                Ok(listener)
            }
        }
    }

    /// Determines if a connection should be clogged based on the state of clogged connections.
    fn should_clog(&self, source: net::SocketAddr, dest: net::SocketAddr) -> bool {
        let source_ip = source.ip();
        let dest_ip = dest.ip();
        for connection in self.clogged.iter() {
            if connection.source() == source_ip && connection.dest() == dest_ip {
                return true;
            }
        }
        return false;
    }

    /// Clog all new connections from one IP to another. If there are any existing connections, they
    /// are also clogged.
    fn clog_connection(&mut self, clog: CloggedConnection) {
        let clog_source = clog.source();
        let clog_dest = clog.dest();
        self.clogged.insert(clog);
        for connection in self.connections.iter_mut() {
            let source_ip = connection.source().ip();
            let dest_ip = connection.dest().ip();
            if source_ip == clog_source && dest_ip == clog_dest {
                connection.clog();
            }
        }
    }

    /// Unclog all new connection between two IP addresses. If there are any existing connections which
    /// are clogged, they are unclogged.
    fn unclog_connection(&mut self, unclog: CloggedConnection) {
        let clog_source = unclog.source();
        let clog_dest = unclog.dest();
        self.clogged.remove(&unclog);
        for connection in self.connections.iter_mut() {
            let source_ip = connection.source().ip();
            let dest_ip = connection.dest().ip();
            if source_ip == clog_source && dest_ip == clog_dest {
                connection.unclog();
            }
        }
    }
}

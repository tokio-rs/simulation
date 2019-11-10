//! Machine abstraction, allowing manipulation of machine properties.
use futures::{Future, Poll};
use std::{net, pin::Pin, task::Context};

pub struct Machine {
    /// Hostname for this machine instance.
    hostname: String,
    /// Ip Address for this machine instance.
    ip_addr: net::IpAddr,
}

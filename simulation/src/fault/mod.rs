use crate::state::LogicalTaskId;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{
    future::Future,
    io, net,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{delay_for, Delay};

#[derive(Debug, Clone, Copy)]
struct Config {
    /// The maximum duration a AsyncRead can be delayed.
    ///
    /// Defaults to 60 seconds.
    tcp_max_read_delay: Duration,
    /// The maximum duration an AsyncWrite can be delayed
    ///
    /// Defaults to 60 seconds.
    tcp_max_write_delay: Duration,

    /// The probability that a tcp read or write will result in a disconnect.
    ///
    /// Defaults to 0.05, or 5% of reads/writes will cause a disconnect.
    tcp_read_write_disconnect_prob: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tcp_max_read_delay: Duration::from_secs(60),
            tcp_max_write_delay: Duration::from_secs(60),
            tcp_read_write_disconnect_prob: 0.005,
        }
    }
}

#[derive(Debug)]
struct State {
    config: Config,
    rng: SmallRng,
}

impl State {
    fn new(seed: u64) -> State {
        let rng = SeedableRng::seed_from_u64(seed);
        State {
            rng,
            config: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FaultController {
    state: Arc<Mutex<State>>,
}

impl FaultController {
    pub(crate) fn new(seed: u64) -> FaultController {
        let state = State::new(seed);
        return Self {
            state: Arc::new(Mutex::new(state)),
        };
    }

    pub(crate) fn fault_injector(&self) -> FaultInjector {
        let state = Arc::clone(&self.state);
        FaultInjector {
            state,
            read_delay: None,
            write_delay: None,
            disconnected: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct FaultInjector {
    state: Arc<Mutex<State>>,

    read_delay: Option<Delay>,
    write_delay: Option<Delay>,
    disconnected: bool,
}

impl Clone for FaultInjector {
    fn clone(&self) -> Self {
        FaultInjector {
            state: Arc::clone(&self.state),
            read_delay: None,
            write_delay: None,
            disconnected: false,
        }
    }
}

impl FaultInjector {
    fn disconnected(&self) -> bool {
        if self.disconnected {
            return true;
        }
        let mut lock = self.state.lock().unwrap();
        let disconnect_prob = lock.config.tcp_read_write_disconnect_prob;
        if lock.rng.gen_bool(disconnect_prob) {
            return true;
        }
        false
    }

    fn tcp_new_read_delay(&self) -> Delay {
        let new_durations = {
            let mut lock = self.state.lock().unwrap();
            let max_delay = lock.config.tcp_max_read_delay;
            lock.rng.gen_range(Duration::from_secs(0), max_delay)
        };
        delay_for(new_durations)
    }

    fn tcp_new_write_delay(&self) -> Delay {
        let new_durations = {
            let mut lock = self.state.lock().unwrap();
            let max_delay = lock.config.tcp_max_write_delay;
            lock.rng.gen_range(Duration::from_secs(0), max_delay)
        };
        delay_for(new_durations)
    }

    /// Poll the current read delay if there is one. If the previous read
    /// delay has elapsed, a new one will be set.
    pub(crate) fn tcp_poll_read_delay(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _: net::SocketAddr,
        _: net::SocketAddr,
    ) -> Poll<io::Result<()>> {
        if self.disconnected() {
            self.as_mut().disconnected = true;
            return Poll::Ready(Err(io::ErrorKind::ConnectionAborted.into()));
        }
        if let Some(ref mut delay) = self.read_delay {
            let delay = Pin::new(delay);
            futures::ready!(delay.poll(cx));
        }
        let delay = self.tcp_new_read_delay();
        self.read_delay.replace(delay);
        Poll::Ready(Ok(()))
    }

    /// Poll the current write delay if there is one. If the previous write
    /// delay has elapsed, a new one will be set.
    pub(crate) fn tcp_poll_write_delay(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _: net::SocketAddr,
        _: net::SocketAddr,
    ) -> Poll<io::Result<()>> {
        if self.disconnected() {
            self.as_mut().disconnected = true;
            return Poll::Ready(Err(io::ErrorKind::ConnectionAborted.into()));
        }
        if let Some(ref mut delay) = self.write_delay {
            let delay = Pin::new(delay);
            futures::ready!(delay.poll(cx));
        }
        let delay = self.tcp_new_write_delay();
        self.write_delay.replace(delay);
        Poll::Ready(Ok(()))
    }

    pub(crate) fn update_task_poll_priority(&self, _: LogicalTaskId) -> Option<u16> {
        None
    }
}

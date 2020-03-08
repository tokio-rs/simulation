use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{delay_for, Delay};

#[derive(Debug)]
struct Config {
    /// The maximum duration a AsyncRead can be delayed.
    max_read_delay: Duration,
    /// The maximum duration an AsyncWrite can be delayed
    max_write_delay: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_read_delay: Duration::from_secs(60),
            max_write_delay: Duration::from_secs(60),
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
        }
    }
}

#[derive(Debug)]
pub(crate) struct FaultInjector {
    state: Arc<Mutex<State>>,

    read_delay: Option<Delay>,
    write_delay: Option<Delay>,
}

impl Clone for FaultInjector {
    fn clone(&self) -> Self {
        FaultInjector {
            state: Arc::clone(&self.state),
            read_delay: None,
            write_delay: None,
        }
    }
}

impl FaultInjector {
    fn new_read_delay(&self) -> Delay {
        let new_durations = {
            let mut lock = self.state.lock().unwrap();
            let max_delay = lock.config.max_read_delay;
            lock.rng.gen_range(Duration::from_secs(0), max_delay)
        };
        delay_for(new_durations)
    }

    fn new_write_delay(&self) -> Delay {
        let new_durations = {
            let mut lock = self.state.lock().unwrap();
            let max_delay = lock.config.max_write_delay;
            lock.rng.gen_range(Duration::from_secs(0), max_delay)
        };
        delay_for(new_durations)
    }

    /// Poll the current read delay if there is one. If the previous read
    /// delay has elapsed, a new one will be set.
    pub(crate) fn poll_read_delay(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(ref mut delay) = self.read_delay {
            let delay = Pin::new(delay);
            futures::ready!(delay.poll(cx));
        }

        let delay = self.new_read_delay();
        self.read_delay.replace(delay);
        Poll::Ready(Ok(()))
    }

    /// Poll the current write delay if there is one. If the previous write
    /// delay has elapsed, a new one will be set.
    pub(crate) fn poll_write_delay(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(ref mut delay) = self.write_delay {
            let delay = Pin::new(delay);
            futures::ready!(delay.poll(cx));
        }

        let delay = self.new_write_delay();
        self.write_delay.replace(delay);
        Poll::Ready(Ok(()))
    }
}

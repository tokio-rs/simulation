//! Fault injection controller.
use rand::{rngs, Rng};
use std::{ops, sync, time};
use tokio_timer::clock::Now;

/// Configuration for various fauilts which can be injected into the mock network.
#[derive(Debug, Clone)]
pub struct Config {
    /// The range of duration for which a delay for a new connection can be injected.
    pub listener_connection_delay: ops::Range<time::Duration>,
    /// The probability of a new connection delay being injected, 0..1.
    pub listener_connection_delay_prob: f64,
    /// The range of duration for which a server socket read delay can be injected.
    pub socket_read_delay: ops::Range<time::Duration>,
    /// The probability of a server socket read delay being injected, 0..1.
    pub socket_read_delay_prob: f64,
    /// The range of duration for which a server socket write dealy can be injected.
    pub socket_write_delay: ops::Range<time::Duration>,
    /// The probability of a server socket write delay being injected, 0..1.
    pub socket_write_delay_prob: f64,

    pub disconnect_prob: f64,
}

impl Config {
    pub(crate) fn new() -> Self {
        Self {
            listener_connection_delay: time::Duration::from_millis(0)
                ..time::Duration::from_millis(10000),
            listener_connection_delay_prob: 0.10,
            socket_read_delay: time::Duration::from_millis(0)..time::Duration::from_millis(5000),
            socket_read_delay_prob: 0.10,
            socket_write_delay: time::Duration::from_millis(0)..time::Duration::from_millis(5000),
            socket_write_delay_prob: 0.10,
            disconnect_prob: 0.01,
        }
    }
}

#[derive(Debug)]
enum State {
    Real {
        timer_handle: tokio_timer::timer::Handle,
        now: super::time::Now,
        rng: rngs::SmallRng,
    },
    Noop,
}

impl State {
    fn should_fault(&mut self, probability: f64) -> bool {
        match self {
            State::Real { rng, .. } => rng.gen_bool(probability),
            State::Noop => false,
        }
    }

    fn new_delay(&mut self, range: ops::Range<time::Duration>) -> tokio_timer::Delay {
        match self {
            State::Real {
                timer_handle,
                now,
                rng,
            } => {
                let now = now.now();
                let duration = rng.gen_range(range.start, range.end);
                timer_handle.delay(now + duration)
            }
            State::Noop => unreachable!(),
        }
    }

    fn maybe_new_delay(
        &mut self,
        probability: f64,
        range: ops::Range<time::Duration>,
    ) -> Option<tokio_timer::Delay> {
        if self.should_fault(probability) {
            let delay = self.new_delay(range);
            Some(delay)
        } else {
            None
        }
    }

    fn random_idx(&mut self, probability: f64, range: ops::Range<usize>) -> Option<usize> {
        if self.should_fault(probability) {
            match self {
                State::Real { rng, .. } => Some(rng.gen_range(range.start, range.end)),
                State::Noop => None,
            }
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct FaultInjector {
    config: Config,
    inner: sync::Arc<sync::Mutex<State>>,
}

impl FaultInjector {
    pub(crate) fn new_noop() -> Self {
        FaultInjector {
            config: Config::new(),
            inner: sync::Arc::new(sync::Mutex::new(State::Noop)),
        }
    }
    pub(crate) fn new(
        seed: u64,
        timer_handle: tokio_timer::timer::Handle,
        now: super::time::Now,
    ) -> FaultInjector {
        let state = State::Real {
            timer_handle,
            now,
            rng: rand::SeedableRng::seed_from_u64(seed),
        };
        let state = sync::Arc::new(sync::Mutex::new(state));
        FaultInjector {
            config: Config::new(),
            inner: state,
        }
    }
    pub(crate) fn handle(&self) -> FaultInjectorHandle {
        FaultInjectorHandle::new(self.config.clone(), sync::Arc::clone(&self.inner))
    }
}

#[derive(Debug, Clone)]
pub struct FaultInjectorHandle {
    config: Config,
    inner: sync::Arc<sync::Mutex<State>>,
}

impl FaultInjectorHandle {
    fn new(config: Config, inner: sync::Arc<sync::Mutex<State>>) -> Self {
        Self { config, inner }
    }

    pub(crate) fn listener_delay(&self) -> Option<tokio_timer::Delay> {
        self.inner.lock().unwrap().maybe_new_delay(
            self.config.listener_connection_delay_prob,
            self.config.listener_connection_delay.clone(),
        )
    }

    pub(crate) fn socket_read_delay(&self) -> Option<tokio_timer::Delay> {
        self.inner.lock().unwrap().maybe_new_delay(
            self.config.socket_read_delay_prob,
            self.config.socket_read_delay.clone(),
        )
    }

    pub(crate) fn socket_write_delay(&self) -> Option<tokio_timer::Delay> {
        self.inner.lock().unwrap().maybe_new_delay(
            self.config.socket_write_delay_prob,
            self.config.socket_write_delay.clone(),
        )
    }

    pub(crate) fn pick_rand_connection_disconnect(
        &self,
        range: ops::Range<usize>,
    ) -> Option<usize> {
        self.inner
            .lock()
            .unwrap()
            .random_idx(self.config.disconnect_prob, range)
    }
}

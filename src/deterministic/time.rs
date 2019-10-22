//! A mock source of time, allowing for determinstic control of the progress
//! of time.
use std::{sync, time};

#[derive(Debug)]
struct State {
    /// Time basis for which mock time is derived.
    base: time::Instant,
    /// The amount of mock time which has elapsed.
    advance: time::Duration,
}

impl State {
    fn advance(&mut self, duration: time::Duration) {
        self.advance += duration;
    }

    fn now(&self) -> time::Instant {
        self.base + self.advance
    }
}

/// A mock source of time, providing deterministic control of time.
#[derive(Debug, Clone)]
pub(crate) struct Time {
    inner: sync::Arc<sync::Mutex<State>>,
}

impl Default for Time {
    fn default() -> Self {
        let state = State {
            base: time::Instant::now(),
            advance: time::Duration::from_millis(0),
        };
        Self {
            inner: sync::Arc::new(sync::Mutex::new(state)),
        }
    }
}

impl Time {
    pub(crate) fn new() -> Self {
        Default::default()
    }
    /// Advances the internal clock for the provided duration.
    pub(crate) fn advance(&self, duration: time::Duration) {
        self.inner.lock().unwrap().advance(duration);
    }
    /// Return time now.
    pub(crate) fn now(&self) -> time::Instant {
        self.inner.lock().unwrap().now()
    }

    /// Creates an instance of `Now` from this deterministic time source.
    ///
    /// [`Now`]:[tokio_timer::clock::Now]
    pub(crate) fn clone_now(&self) -> Now {
        Now::new(sync::Arc::clone(&self.inner))
    }

    /// Returns a new instance of `tokio_timer::clock::Clock` which wraps
    /// this determinstic time source.
    pub(crate) fn clone_tokio_clock(&self) -> tokio_timer::clock::Clock {
        tokio_timer::clock::Clock::new_with_now(self.clone_now())
    }

    /// Wrap the provided `Park` instance in a new `Park`, which instantly
    /// advances the determinstic time source on `Park::park_with_timeout`.
    ///
    /// [`Park`]:[tokio_executor::park::Park]
    pub(crate) fn wrap_park<P>(&self, park: P) -> Park<P>
    where
        P: tokio_executor::park::Park,
    {
        Park::wrap(sync::Arc::clone(&self.inner), park)
    }
}

/// `Now` implementation wrapping the deterministic `Time` source
///
/// [`Now`]:[tokio_timer::clock::Now]
/// [`Time`]:[Time]
#[derive(Debug, Clone)]
pub(crate) struct Now {
    inner: sync::Arc<sync::Mutex<State>>,
}

impl Now {
    fn new(state: sync::Arc<sync::Mutex<State>>) -> Self {
        Self { inner: state }
    }
}

impl tokio_timer::clock::Now for Now {
    fn now(&self) -> time::Instant {
        let l = self.inner.lock().unwrap();
        l.base + l.advance
    }
}

#[allow(deprecated)]
impl tokio_timer::timer::Now for Now {
    fn now(&mut self) -> time::Instant {
        tokio_timer::clock::Now::now(self)
    }
}

#[derive(Debug)]
pub(crate) struct Park<P> {
    inner: sync::Arc<sync::Mutex<State>>,
    inner_park: P,
}

impl<P> Park<P> {
    fn wrap(state: sync::Arc<sync::Mutex<State>>, park: P) -> Self {
        Self {
            inner: state,
            inner_park: park,
        }
    }
}

impl<P> tokio_executor::park::Park for Park<P>
where
    P: tokio_executor::park::Park,
{
    type Unpark = P::Unpark;
    type Error = P::Error;
    fn unpark(&self) -> Self::Unpark {
        self.inner_park.unpark()
    }
    fn park(&mut self) -> Result<(), Self::Error> {
        self.inner_park.park()
    }
    fn park_timeout(&mut self, duration: time::Duration) -> Result<(), Self::Error> {
        let mut lock = self.inner.lock().unwrap();
        lock.advance(duration);
        self.inner_park.park_timeout(time::Duration::from_millis(0))
    }
}

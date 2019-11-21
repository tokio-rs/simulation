//! A mock source of time, allowing for determinstic control of the progress
//! of time.
use std::{sync, time};

#[derive(Debug)]
struct Inner {
    /// Time basis for which mock time is derived.
    base: time::Instant,
    /// The amount of mock time which has elapsed.
    advance: time::Duration,
}

impl Inner {
    fn new() -> Self {
        Self {
            base: time::Instant::now(),
            advance: time::Duration::from_millis(0),
        }
    }

    fn advance(&mut self, duration: time::Duration) {
        self.advance += duration;
    }

    fn now(&self) -> time::Instant {
        self.base + self.advance
    }
}

/// A mock source of time, providing deterministic control of time.
#[derive(Debug)]
pub struct DeterministicTime<P> {
    park: tokio_timer::Timer<DeterministicPark<P>, Now>,
    inner: sync::Arc<sync::Mutex<Inner>>,
    timer_handle: tokio_timer::timer::Handle,
}

impl<P> DeterministicTime<P>
where
    P: tokio_executor::park::Park,
{
    /// Wrap the provided `Park` instance with DeterministicTime, which instantly
    /// advances the determinstic time source on `Park::park_with_timeout`.
    ///
    /// [`Park`]:[tokio_executor::park::Park]
    pub fn new_with_park(park: P) -> Self {
        let inner = Inner::new();
        let inner = sync::Arc::new(sync::Mutex::new(inner));
        let now = Now::new(sync::Arc::clone(&inner));
        let inner_park = DeterministicPark::new(park, sync::Arc::clone(&inner));
        let timer = tokio_timer::Timer::new_with_now(inner_park, now);
        let timer_handle = timer.handle();
        Self {
            inner,
            park: timer,
            timer_handle,
        }
    }

    pub fn handle(&self) -> DeterministicTimeHandle {
        let inner = sync::Arc::clone(&self.inner);
        DeterministicTimeHandle {
            inner,
            timer_handle: self.timer_handle.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicTimeHandle {
    inner: sync::Arc<sync::Mutex<Inner>>,
    timer_handle: tokio_timer::timer::Handle,
}

impl DeterministicTimeHandle {
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

    pub fn delay(&self, deadline: time::Instant) -> tokio_timer::Delay {
        self.timer_handle.delay(deadline)
    }

    pub fn delay_from(&self, duration: time::Duration) -> tokio_timer::Delay {
        self.timer_handle.delay(self.now() + duration)
    }

    pub fn timeout<T>(&self, value: T, timeout: time::Duration) -> tokio_timer::Timeout<T> {
        self.timer_handle.timeout(value, timeout)
    }

    pub fn clone_timer_handle(&self) -> tokio_timer::timer::Handle {
        self.timer_handle.clone()
    }
}

#[derive(Debug)]
struct DeterministicPark<P> {
    park: P,
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl<P> DeterministicPark<P> {
    fn new(park: P, inner: sync::Arc<sync::Mutex<Inner>>) -> Self {
        Self { park, inner }
    }
}

impl<P> tokio_executor::park::Park for DeterministicPark<P>
where
    P: tokio_executor::park::Park,
{
    type Unpark = P::Unpark;
    type Error = P::Error;
    fn unpark(&self) -> Self::Unpark {
        self.park.unpark()
    }
    fn park(&mut self) -> Result<(), Self::Error> {
        self.park.park()
    }
    fn park_timeout(&mut self, duration: time::Duration) -> Result<(), Self::Error> {
        let mut lock = self.inner.lock().unwrap();
        lock.advance(duration);
        self.park.park_timeout(time::Duration::from_millis(0))
    }
}

impl<P> tokio_executor::park::Park for DeterministicTime<P>
where
    P: tokio_executor::park::Park,
{
    type Unpark = P::Unpark;
    type Error = P::Error;
    fn unpark(&self) -> Self::Unpark {
        self.park.unpark()
    }
    fn park(&mut self) -> Result<(), Self::Error> {
        self.park.park()
    }
    fn park_timeout(&mut self, duration: time::Duration) -> Result<(), Self::Error> {
        self.park.park_timeout(duration)
    }
}

/// `Now` implementation wrapping the deterministic `Time` source
///
/// [`Now`]:[tokio_timer::clock::Now]
/// [`Time`]:[Time]
#[derive(Debug, Clone)]
pub(crate) struct Now {
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl Now {
    fn new(state: sync::Arc<sync::Mutex<Inner>>) -> Self {
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

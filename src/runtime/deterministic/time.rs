use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_executor::park::{Park, Unpark};
use tokio_timer::clock::{Clock, Now};
use try_lock::TryLock;

#[derive(Clone, Debug)]
pub(crate) struct MockClock {
    inner: Inner,
    clock: Clock,
}

impl MockClock {
    pub(crate) fn wrap_park<P>(park: P) -> (Self, MockPark<P>)
    where
        P: Park,
    {
        let state = State {
            base: Instant::now(),
            advance: Duration::from_millis(0),
        };
        let state = Arc::new(TryLock::new(state));
        let now = MockNow {
            inner: Arc::clone(&state),
        };
        let clock = Clock::new_with_now(now);
        let wrapped_park = MockPark {
            inner: Arc::clone(&state),
            inner_park: park,
        };
        (
            Self {
                inner: Arc::clone(&state),
                clock,
            },
            wrapped_park,
        )
    }
    pub(crate) fn now(&self) -> Instant {
        loop {
            if let Some(lock) = self.inner.try_lock() {
                return lock.now();
            }
        }
    }

    pub(crate) fn get_clock(&self) -> Clock {
        self.clock.clone()
    }

    pub fn enter<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let clock = self.get_clock();
        ::tokio_timer::clock::with_default(&clock, || f())
    }
}
#[derive(Debug)]
struct State {
    base: Instant,
    advance: Duration,
}

impl State {
    fn now(&self) -> Instant {
        self.base + self.advance
    }
    fn advance(&mut self, duration: Duration) {
        self.advance += duration;
    }
}

type Inner = Arc<TryLock<State>>;

#[derive(Debug, Clone)]
pub(crate) struct MockNow {
    inner: Inner,
}

impl Now for MockNow {
    fn now(&self) -> Instant {
        loop {
            if let Some(lock) = self.inner.try_lock() {
                return lock.now();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MockPark<P> {
    inner: Inner,
    inner_park: P,
}

impl<P> Park for MockPark<P>
where
    P: Park,
{
    type Unpark = MockUnpark;
    type Error = P::Error;
    fn unpark(&self) -> Self::Unpark {
        let unpark = self.inner_park.unpark();
        MockUnpark {
            unpark: Box::new(unpark),
            _inner: self.inner.clone(),
        }
    }
    fn park(&mut self) -> Result<(), Self::Error> {
        self.inner_park.park()
    }
    fn park_timeout(&mut self, duration: Duration) -> Result<(), Self::Error> {
        loop {
            if let Some(mut lock) = self.inner.try_lock() {
                lock.advance(duration);
                return self.inner_park.park_timeout(Duration::from_millis(0));
            }
        }
    }
}

pub(crate) struct MockUnpark {
    unpark: Box<dyn Unpark>,
    _inner: Inner,
}

impl Unpark for MockUnpark {
    fn unpark(&self) {
        self.unpark.unpark()
    }
}

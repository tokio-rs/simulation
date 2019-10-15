//! This module provides a deterministic runtime for Rust. [`DeterministicRuntime`] provides
//! a single threaded runtime with a mocked notion of time. This allows for an executor to
//! advance the clock when there is no more work to do.
//!
//! `DeterministicRuntimeHandle` allows for creating things like TCP connections or Delays which
//! are backed by deterministic implementations. A seedable RNG is also provided to facilitate
//! probabilistic fault injection.
//!
//! There are two types of determinstic components which are both wrapped by the `DeterministicRuntimeHandle`.
//! The `DeterministicRuntimeSchedulerRng` contains base components which need to be shared between simulated
//! systems and higher order components alike. `DeterministicRuntimeSchedulerRng` contains the facilities for
//! simulating scheduling and time.
//!
//! `DeterministicRuntimeHandle` wraps the `DeterministicRuntimeSchedulerRng` and further provides deterministic
//! networking with probabilistic fault injection.
use crate::{runtime::Error, Environment};
use async_trait::async_trait;
use futures::Future;
use rand::{rngs, Rng};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_executor::current_thread::{CurrentThread, TaskExecutor};
use tokio_net::driver::Reactor;
use tokio_timer::timer;
use try_lock::TryLock;

mod net;
mod time;

#[derive(Debug, Clone)]
pub struct DeterministicRuntimeSchedulerRngState {
    reactor_handle: tokio_net::driver::Handle,
    executor_handle: tokio_executor::current_thread::Handle,
    clock: time::MockClock,
    timer_handle: timer::Handle,
    rng: Option<rngs::SmallRng>,
}

/// DeterministicRuntimeSchedulerRng contains the base deterministic components for constructing
/// a runtime. It is seperated out from the DeterminsticRuntimeHandle to allow building higher
/// order deterministic components, like networking.
#[derive(Debug, Clone)]
pub struct DeterministicRuntimeSchedulerRng {
    state: Arc<TryLock<DeterministicRuntimeSchedulerRngState>>,
}

impl DeterministicRuntimeSchedulerRngState {
    /// Decides to preform an action based on the provided probability if there is an RNG seed
    /// provided to the backing DeterministicRuntime.
    /// If there is none, then this function will always return false.
    fn should_perform_random_action(&mut self, probability: f32) -> bool {
        if let Some(ref mut rng) = self.rng {
            let rand: f32 = rng.gen_range(0.0, 1.0);
            if probability >= rand {
                return true;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }
}

impl DeterministicRuntimeSchedulerRng {
    fn new(
        reactor_handle: tokio_net::driver::Handle,
        executor_handle: tokio_executor::current_thread::Handle,
        clock: time::MockClock,
        timer_handle: timer::Handle,
        rng: Option<rngs::SmallRng>,
    ) -> Self {
        let state = DeterministicRuntimeSchedulerRngState {
            reactor_handle,
            executor_handle,
            clock,
            timer_handle,
            rng,
        };
        let state = Arc::new(TryLock::new(state));
        Self { state }
    }

    /// Returns a randomized delay between the provided from and to durations based on the provided probability of the delay
    /// occuring.
    pub(crate) fn maybe_random_delay(
        &self,
        probability: f32,
        from: Duration,
        to: Duration,
    ) -> Option<tokio::timer::Delay> {
        loop {            
            if let Some(mut lock) = self.state.try_lock() {
                if lock.should_perform_random_action(probability) {
                    if let Some(ref mut rng) = lock.rng {
                        let duration = rng.gen_range(from.as_millis(), to.as_millis());
                        let duration = Duration::from_millis(duration as u64);
                        let delay = lock.timer_handle.delay(lock.clock.now() + duration);
                        return Some(delay);
                    }
                    return None;
                }
                return None;
            }
        }
    }

    pub(crate) fn executor_handle(&self) -> tokio_executor::current_thread::Handle {
        loop {            
            if let Some(lock) = self.state.try_lock() {
                return lock.executor_handle.clone();
            }
        }
    }
    pub(crate) fn clock(&self) -> time::MockClock {
        loop {            
            if let Some(lock) = self.state.try_lock() {
                return lock.clock.clone();
            }
        }
    }

    pub(crate) fn timer_handle(&self) -> timer::Handle {
        loop {
            if let Some(lock) = self.state.try_lock() {
                return lock.timer_handle.clone();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicRuntimeHandle {
    scheduler_rng: DeterministicRuntimeSchedulerRng,
    network: net::MemoryNetwork,
}

impl DeterministicRuntimeHandle {
    /// Returns the backing DeterministicRuntimeSchedulerRng which contains a subset of runtime functionality.
    pub fn network_faults(&self) -> net::NetworkFaults {
        self.network.network_faults()
    }
}

#[async_trait]
impl Environment for DeterministicRuntimeHandle {
    type TcpStream = net::MemoryTcpStream<net::ClientSocket>;
    type TcpListener = net::MemoryListener;

    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.scheduler_rng
            .executor_handle()
            .spawn(future)
            .expect("failed to spawn task")
    }
    fn now(&self) -> Instant {
        self.scheduler_rng.clock().now()
    }
    fn delay(&self, deadline: Instant) -> tokio::timer::Delay {
        self.scheduler_rng.timer_handle().delay(deadline)
    }
    fn timeout<T>(&self, value: T, timeout: Duration) -> tokio::timer::Timeout<T> {
        self.scheduler_rng.timer_handle().timeout(value, timeout)
    }
    async fn bind<'a, A>(&'a self, addr: A) -> Result<Self::TcpListener, io::Error>
    where
        A: Into<std::net::SocketAddr> + Send + Sync,
    {
        self.network.bind(addr).await
    }
    async fn connect<'a, A>(&'a self, addr: A) -> Result<Self::TcpStream, io::Error>
    where
        A: Into<std::net::SocketAddr> + Send + Sync,
    {
        self.network.connect(addr).await
    }
}

/// DeterminaticRuntime is a runtime.
pub struct DeterministicRuntime {
    reactor_handle: tokio_net::driver::Handle,
    timer_handle: tokio_timer::timer::Handle,
    clock: time::MockClock,
    executor: CurrentThread<timer::Timer<time::MockPark<Reactor>>>,
    handle: DeterministicRuntimeHandle,
}

impl DeterministicRuntime {
    pub fn new() -> Result<Self, Error> {
        DeterministicRuntime::new_inner(None)
    }
    pub fn new_with_seed(rng_seed: u64) -> Result<Self, Error> {
        DeterministicRuntime::new_inner(Some(rng_seed))
    }

    fn new_inner(rng_seed: Option<u64>) -> Result<Self, Error> {
        let reactor = Reactor::new().map_err(|source| Error::RuntimeBuild { source })?;
        let reactor_handle = reactor.handle();
        let (clock, park) = time::MockClock::wrap_park(reactor);
        let timer = tokio_timer::timer::Timer::new_with_now(park, clock.get_clock());
        let timer_handle = timer.handle();
        let executor = CurrentThread::new_with_park(timer);

        let scheduler_rng = DeterministicRuntimeSchedulerRng::new(
            reactor_handle.clone(),
            executor.handle(),
            clock.clone(),
            timer_handle.clone(),
            rng_seed.map(|s| rand::SeedableRng::seed_from_u64(s)),
        );

        let network = net::MemoryNetwork::new(scheduler_rng.clone());
        let handle = DeterministicRuntimeHandle {
            scheduler_rng,
            network,
        };
        let runtime = DeterministicRuntime {
            reactor_handle,
            timer_handle,
            clock,
            executor,
            handle,
        };
        return Ok(runtime);
    }

    pub fn handle(&self) -> DeterministicRuntimeHandle {
        self.handle.clone()
    }

    pub fn spawn<F>(&mut self, future: F) -> &mut Self
    where
        F: Future<Output = ()> + 'static,
    {
        self.executor.spawn(future);
        self
    }

    pub fn run(&mut self) -> Result<(), Error> {
        self.enter(|executor| executor.run())
            .map_err(|source| Error::CurrentThreadRun { source })
    }

    pub fn block_on<F>(&mut self, f: F) -> F::Output
    where
        F: Future,
    {
        self.enter(|executor| executor.block_on(f))
    }

    fn enter<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut CurrentThread<timer::Timer<time::MockPark<Reactor>>>) -> R,
    {
        let DeterministicRuntime {
            ref reactor_handle,
            ref mut clock,
            ref mut executor,
            ref timer_handle,
            ..
        } = *self;
        let _reactor = tokio_net::driver::set_default(&reactor_handle);
        let _guard = tokio_timer::timer::set_default(timer_handle);
        clock.enter(move || {
            let mut default_executor = TaskExecutor::current();
            tokio_executor::with_default(&mut default_executor, || f(executor))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Test that delays accurately advance the clock.    
    fn delays() {
        let mut runtime = DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let start_time = handle.now();
            handle.delay_from(Duration::from_secs(30)).await;
            let end_time = handle.now();
            assert!(end_time > start_time);
            assert_eq!(end_time - Duration::from_secs(30), start_time)
        });
    }

    #[test]
    /// Test that waiting on delays across spawned tasks results in the clock
    /// being advanced in accordance with the length of the delay.
    fn ordering() {
        let mut runtime = DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let delay1 = handle.delay_from(Duration::from_secs(10));
            let delay2 = handle.delay_from(Duration::from_secs(30));

            let handle1 = handle.clone();
            let completed_at1 = crate::spawn_with_result(&handle1.clone(), async move {
                delay1.await;
                handle1.now()
            })
            .await;

            let handle2 = handle.clone();
            let completed_at2 = crate::spawn_with_result(&handle2.clone(), async move {
                delay2.await;
                handle2.now()
            })
            .await;
            assert!(completed_at1 < completed_at2)
        });
    }

    #[test]
    /// Test that the Tokio global timer and clock are both set correctly.
    fn globals() {
        let mut runtime = DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let start_time = tokio_timer::clock::now();
            assert_eq!(handle.now(), tokio_timer::clock::now());
            let delay = tokio::timer::delay_for(Duration::from_secs(10));
            delay.await;
            assert_eq!(
                start_time + Duration::from_secs(10),
                tokio_timer::clock::now()
            );
        });
    }
}

//! Determinstic scheduling, IO and fault injection for Tokio
//!
//! The goal of this crate is to provide FoundationDB style simulation
//! testing for all.
//!
//! There are 3 layers on which the `DeterministicRuntime` is built.
//!
//! - `DeterministicRandom` allows for accessing a deterministic source of randomness.
//! - `DeterministicScheduling` allows for spawning and scheduling tasks deterministicially,
//!    and provides a deterministic time source.
//! - `DeterministicNetwork` provides a process wide networking in memory networking implementation.
//!
use crate::Error;
use async_trait::async_trait;
use futures::Future;
use std::{
    io, net,
    time::{Duration, Instant},
};

mod fault;
pub use fault::{FaultInjector, FaultInjectorHandle};
mod network;
mod network2;
mod time;
mod rand;
pub use network::{Listener, SocketHalf};
pub(crate) use time::DeterministicTime;

#[derive(Debug, Clone)]
pub struct DeterministicRuntimeHandle {
    reactor_handle: tokio_net::driver::Handle,
    time_handle: time::DeterministicTimeHandle,
    timer_handle: tokio_timer::timer::Handle,
    fault_injector: FaultInjectorHandle,
    network_handle: network::NetworkHandle,
    executor_handle: tokio_executor::current_thread::Handle,
}

impl DeterministicRuntimeHandle {
    pub fn now(&self) -> Instant {
        self.time_handle.now()
    }
}

#[async_trait]
impl crate::Environment for DeterministicRuntimeHandle {
    type TcpStream = network::SocketHalf;
    type TcpListener = network::Listener<SocketHalf>;
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.executor_handle.spawn(future).expect("failed to spawn");
    }
    fn now(&self) -> Instant {
        self.time_handle.now()
    }
    fn delay(&self, deadline: Instant) -> tokio_timer::Delay {
        self.timer_handle.delay(deadline)
    }
    fn timeout<T>(&self, value: T, timeout: Duration) -> tokio_timer::Timeout<T> {
        self.timer_handle.timeout(value, timeout)
    }
    async fn bind<A>(&self, addr: A) -> io::Result<Self::TcpListener>
    where
        A: Into<net::SocketAddr> + Send + Sync,
    {
        self.network_handle.bind(addr.into()).await
    }
    async fn connect<A>(&self, addr: A) -> io::Result<Self::TcpStream>
    where
        A: Into<net::SocketAddr> + Send + Sync,
    {
        self.network_handle.connect(addr.into()).await
    }
}

type Executor = tokio_executor::current_thread::CurrentThread<
    network::Network<tokio_timer::timer::Timer<time::DeterministicTime<tokio_net::driver::Reactor>, time::Now>>,
>;

pub struct DeterministicRuntime {
    executor: Executor,
    handle: DeterministicRuntimeHandle,
    reactor_handle: tokio_net::driver::Handle,
    timer_handle: tokio_timer::timer::Handle,
}

impl DeterministicRuntime {
    pub fn new() -> Result<Self, Error> {
        DeterministicRuntime::new_with_seed(0)
    }
    pub fn new_with_seed(seed: u64) -> Result<Self, Error> {
        let reactor =
            tokio_net::driver::Reactor::new().map_err(|source| Error::RuntimeBuild { source })?;
        let reactor_handle = reactor.handle();
        let time = DeterministicTime::new_with_park(reactor);
        let time_handle = time.handle();
        let timer = tokio_timer::Timer::new_with_now(time, time_handle.clone_now());
        let timer_handle = timer.handle();
        let fault_injector =
            fault::FaultInjector::new(seed, timer_handle.clone(), time_handle.clone_now());
        let fault_injector_handle = fault_injector.handle();
        let network = network::Network::new_with_park(timer);
        let network_handle = network.handle();
        let executor = tokio_executor::current_thread::CurrentThread::new_with_park(network);
        let handle = DeterministicRuntimeHandle {
            reactor_handle: reactor_handle.clone(),
            time_handle: time_handle,
            timer_handle: timer_handle.clone(),
            fault_injector: fault_injector_handle,
            network_handle: network_handle,
            executor_handle: executor.handle(),
        };
        Ok(DeterministicRuntime {
            executor,
            handle,
            reactor_handle,
            timer_handle,
        })
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
        F: FnOnce(&mut Executor) -> R,
    {
        let DeterministicRuntime {
            ref reactor_handle,
            ref mut executor,
            ref timer_handle,
            ref handle,
            ..
        } = *self;
        let clock = tokio_timer::clock::Clock::new_with_now(handle.time_handle.clone_now());
        let _reactor = tokio_net::driver::set_default(&reactor_handle);
        let _guard = tokio_timer::timer::set_default(timer_handle);
        tokio_timer::clock::with_default(&clock, || {
            let mut default_executor = tokio_executor::current_thread::TaskExecutor::current();
            tokio_executor::with_default(&mut default_executor, || f(executor))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;

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

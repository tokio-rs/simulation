//! Determinstic scheduling, IO and fault injection for Tokio
//!
//! The goal of this crate is to provide FoundationDB style simulation
//! testing for all.
//!

use async_trait::async_trait;
pub(crate) use fault::FaultInjectorHandle;
use futures::Future;
use std::{
    io, net,
    time::{Duration, Instant},
};
use crate::runtime::Error;
pub(crate) use time::Time;
use tokio_executor::park::Park;

mod fault;
mod network;
mod time;

#[derive(Debug, Clone)]
pub struct DeterministicRuntimeHandle {
    reactor: tokio_net::driver::Handle,
    time: Time,
    timer: tokio_timer::timer::Handle,
    fault_injector: FaultInjectorHandle,
    network: network::NetworkHandle,
    executor: tokio_executor::current_thread::Handle,
}

#[async_trait]
impl crate::Environment for DeterministicRuntimeHandle {
    type TcpStream = network::ClientConnection;
    type TcpListener = network::Listener;
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        unimplemented!()
    }
    fn now(&self) -> Instant {
        unimplemented!()
    }
    fn delay(&self, deadline: Instant) -> tokio_timer::Delay {
        unimplemented!()
    }
    fn timeout<T>(&self, value: T, timeout: Duration) -> tokio_timer::Timeout<T> {
        unimplemented!()
    }
    async fn bind<'a, A>(&'a self, addr: A) -> io::Result<Self::TcpListener>
    where
        A: Into<net::SocketAddr> + Send + Sync,
    {
        unimplemented!()
    }
    async fn connect<'a, A>(&'a self, addr: A) -> io::Result<Self::TcpStream>
    where
        A: Into<net::SocketAddr> + Send + Sync,
    {
        unimplemented!()
    }
}

type Executor = tokio_executor::current_thread::CurrentThread<
    network::Network<tokio_timer::timer::Timer<time::Park<tokio_net::driver::Reactor>, time::Now>>,
>;

pub struct DeterministicRuntime {
    executor: Executor,
    handle: DeterministicRuntimeHandle,
    reactor_handle: tokio_net::driver::Handle,
    timer_handle: tokio_timer::timer::Handle,
    clock: tokio_timer::clock::Clock, 
}

impl DeterministicRuntime {
    pub fn new(seed: u64) -> Self {
        let reactor = tokio_net::driver::Reactor::new().unwrap();
        let reactor_handle = reactor.handle();
        let time = Time::new();
        let reactor = time.wrap_park(reactor);
        let timer = tokio_timer::Timer::new_with_now(reactor, time.clone_now());
        let timer_handle = timer.handle();
        let clock = tokio_timer::clock::Clock::new_with_now(time.clone_now());
        let fault_injector =
            fault::FaultInjector::new(seed, timer_handle.clone(), time.clone_now());
        let fault_injector_handle = fault_injector.handle();
        let network = network::Network::new_with_park(timer, fault_injector_handle.clone());
        let network_handle = network.handle();
        let executor = tokio_executor::current_thread::CurrentThread::new_with_park(network);
        let handle = DeterministicRuntimeHandle {
            reactor: reactor_handle.clone(),
            time: time.clone(),
            timer: timer_handle.clone(),
            fault_injector: fault_injector_handle.clone(),
            network: network_handle,
            executor: executor.handle(),
        };
        DeterministicRuntime { executor, handle, reactor_handle, timer_handle, clock }
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
            ref mut clock,
            ref mut executor,
            ref timer_handle,
            ..
        } = *self;
        
        let _reactor = tokio_net::driver::set_default(&reactor_handle);
        let _guard = tokio_timer::timer::set_default(timer_handle);
        tokio_timer::clock::with_default(&clock, || {
            let mut default_executor = tokio_executor::current_thread::TaskExecutor::current();
            tokio_executor::with_default(&mut default_executor, || f(executor))
        })
    }

}

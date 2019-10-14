use crate::runtime::Error;
use async_trait::async_trait;
use futures::Future;
use std::{io, net::ToSocketAddrs, pin::Pin, task::Context, time};
use tokio_executor::current_thread;
use tokio_net::driver::Reactor;
use tokio_timer::{clock::Clock, timer};
mod net;
#[derive(Debug, Clone)]
pub struct SingleThreadedRuntimeHandle {
    executor_handle: current_thread::Handle,
    clock_handle: Clock,
    timer_handle: timer::Handle,
}

#[async_trait]
impl crate::Environment for SingleThreadedRuntimeHandle {
    type TcpStream = tokio::net::TcpStream;
    type TcpListener = tokio::net::TcpListener;
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.executor_handle
            .spawn(future)
            .expect("failed to spawn task")
    }
    fn now(&self) -> time::Instant {
        self.clock_handle.now()
    }
    fn delay(&self, deadline: time::Instant) -> tokio::timer::Delay {
        self.timer_handle.delay(deadline)
    }
    fn timeout<T>(&self, value: T, timeout: time::Duration) -> tokio::timer::Timeout<T> {
        self.timer_handle.timeout(value, timeout)
    }
    async fn bind<'a, A>(&'a self, addrs: A) -> Result<Self::TcpListener, io::Error>
    where
        A: ToSocketAddrs + Send { unimplemented!() }
    async fn connect<'a, A>(&'a self, addrs: A) -> Result<Self::TcpStream, io::Error>
    where
        A: ToSocketAddrs + Send { unimplemented!() }
}

pub struct SingleThreadedRuntime {
    reactor_handle: tokio_net::driver::Handle,
    timer_handle: tokio_timer::timer::Handle,
    clock: Clock,
    executor: current_thread::CurrentThread<timer::Timer<Reactor>>,
}

impl SingleThreadedRuntime {
    pub fn new() -> Result<Self, Error> {
        let reactor = Reactor::new().map_err(|source| Error::RuntimeBuild { source })?;
        let reactor_handle = reactor.handle();
        let clock = Clock::new();
        let timer = tokio_timer::Timer::new_with_now(reactor, clock.clone());
        let timer_handle = timer.handle();
        let executor = current_thread::CurrentThread::new_with_park(timer);
        let runtime = SingleThreadedRuntime {
            reactor_handle,
            timer_handle,
            clock,
            executor,
        };
        Ok(runtime)
    }

    pub fn handle(&self) -> SingleThreadedRuntimeHandle {
        let executor_handle = self.executor.handle();
        let clock_handle = self.clock.clone();
        let timer_handle = self.timer_handle.clone();
        SingleThreadedRuntimeHandle {
            executor_handle,
            clock_handle,
            timer_handle,
        }
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
        F: FnOnce(&mut current_thread::CurrentThread<timer::Timer<Reactor>>) -> R,
    {
        let SingleThreadedRuntime {
            ref reactor_handle,
            ref timer_handle,
            ref clock,
            ref mut executor,
        } = *self;
        let _reactor = tokio_net::driver::set_default(&reactor_handle);
        let clock = clock;
        tokio_timer::clock::with_default(&clock, || {
            let _timer = tokio_timer::timer::set_default(&timer_handle);
            let mut default_executor = tokio_executor::current_thread::TaskExecutor::current();
            tokio_executor::with_default(&mut default_executor, || f(executor))
        })
    }
}

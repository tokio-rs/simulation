use crate::{runtime::Error, Environment};
use futures::Future;
use std::time;

use tokio_executor::current_thread::{CurrentThread, TaskExecutor};
use tokio_net::driver::Reactor;
use tokio_timer::timer;

mod mocktime;

#[derive(Debug, Clone)]
pub struct DeterministicRuntimeHandle {
    reactor_handle: tokio_net::driver::Handle,
    executor_handle: tokio_executor::current_thread::Handle,
    clock: mocktime::MockClock,
    timer_handle: timer::Handle,
}

impl DeterministicRuntimeHandle {}

impl Environment for DeterministicRuntimeHandle {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.executor_handle
            .spawn(future)
            .expect("failed to spawn task")
    }
    fn now(&self) -> time::Instant {
        self.clock.now()
    }
    fn delay(&self, deadline: time::Instant) -> tokio::timer::Delay {        
        self.timer_handle.delay(deadline)
    }
    fn timeout<T>(&self, value: T, timeout: time::Duration) -> tokio::timer::Timeout<T> {
        self.timer_handle.timeout(value, timeout)
    }
}

/// DeterminaticRuntime is a runtime.
pub struct DeterministicRuntime {
    reactor_handle: tokio_net::driver::Handle,
    timer_handle: tokio_timer::timer::Handle,
    clock: mocktime::MockClock,
    executor: CurrentThread<timer::Timer<mocktime::MockPark<Reactor>>>,
}

impl DeterministicRuntime {
    pub fn new() -> Result<Self, Error> {
        let reactor = Reactor::new().map_err(|source| Error::RuntimeBuild { source })?;
        let reactor_handle = reactor.handle();
        let (clock, park) = mocktime::MockClock::wrap_park(reactor);
        let timer = tokio_timer::timer::Timer::new_with_now(park, clock.get_clock());                
        let timer_handle = timer.handle();
        let executor = CurrentThread::new_with_park(timer);
        let runtime = DeterministicRuntime {
            reactor_handle,
            timer_handle,
            clock,
            executor,
        };
        return Ok(runtime);
    }

    pub fn handle(&self) -> DeterministicRuntimeHandle {
        let executor_handle = self.executor.handle().clone();
        let clock = self.clock.clone();
        let timer_handle = self.timer_handle.clone();
        let reactor_handle = self.reactor_handle.clone();
        DeterministicRuntimeHandle {
            reactor_handle,
            executor_handle,
            clock,
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
        F: FnOnce(&mut CurrentThread<timer::Timer<mocktime::MockPark<Reactor>>>) -> R,
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
    use std::time;

    #[test]
    /// Test that delays accurately advance the clock.    
    fn delays() {
        let mut runtime = DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let start_time = handle.now();
            handle.delay_from(time::Duration::from_secs(30)).await;
            let end_time = handle.now();
            assert!(end_time > start_time);
            assert_eq!(end_time - time::Duration::from_secs(30), start_time)
        });
    }

    #[test]
    /// Test that waiting on delays across spawned tasks results in the clock
    /// being advanced in accordance with the length of the delay.
    fn ordering() {
        let mut runtime = DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let delay1 = handle.delay_from(time::Duration::from_secs(10));
            let delay2 = handle.delay_from(time::Duration::from_secs(30));

            let handle1 = handle.clone();
            let completed_at1 = crate::spawn_with_result(&handle1.clone(), async move {
                delay1.await;
                handle1.now()
            }).await;

            let handle2 = handle.clone();
            let completed_at2 = crate::spawn_with_result(&handle2.clone(), async move {
                delay2.await;
                handle2.now()
            }).await;
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
            let delay = tokio::timer::delay_for(time::Duration::from_secs(10));
            delay.await;
            assert_eq!(start_time + time::Duration::from_secs(10), tokio_timer::clock::now());
        });
    }
}

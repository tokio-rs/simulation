use crate::ExecutorHandle;
use crate::SimulationHandle;
use std::future::Future;
use tokio::task::JoinHandle;

pub fn spawn<T>(future: T) -> JoinHandle<T::Output>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    if let Some(handle) = SimulationHandle::opt() {
        handle.spawn(future)
    } else {
        tokio::spawn(future)
    }
}

pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if let Some(handle) = SimulationHandle::opt() {
        handle.spawn_blocking(f)
    } else {
        tokio::task::spawn_blocking(f)
    }
}

pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    if let Some(handle) = SimulationHandle::opt() {
        handle.spawn_local(future)
    } else {
        tokio::task::spawn_local(future)
    }
}

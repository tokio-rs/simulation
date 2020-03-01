use std::future::Future;
use tokio::task::JoinHandle;
mod concrete;
mod simulation;

pub use concrete::TokioExecutorHandle;

pub trait ExecutorHandle {
    /// Spawns a future onto the Tokio runtime.
    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;

    /// Spawns a !Send task onto the local task set.
    ///
    /// This task is guranteed to be run on the current thread.
    fn spawn_local<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static;

    /// Runs the provided closure on a thread where blocking is acceptable.
    fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static;
}

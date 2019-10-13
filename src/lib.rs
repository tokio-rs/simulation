#![allow(unused_imports)]
//! This crate provides an abstraction over the [tokio] [CurrentThread] runtime
//! which allows for simulating applications.
//!
//! The [Environment] trait provides an abstraction over [Delay] and [Timeout].
//! This allows for applications to be generic over [DeterministicRuntime] or
//! [SingleThreadedRuntime].
//!
//! [DeterministicRuntime] will automatically advance a mocked clock if there is
//! no more work to do, up until the next timeout. This results in applications which
//! can be decoupled from time, facilitating fast integration/simulation tests.
//!
//! ```rust
//! use std::time;
//! use simulation::{DeterministicRuntime, Environment};
//!
//! async fn delayed<E>(handle: E) where E: Environment {
//!    let start_time = handle.now();
//!    handle.delay_from(time::Duration::from_secs(30)).await;
//!    println!("that was fast!");
//!    assert_eq!(handle.now(), start_time + time::Duration::from_secs(30));
//! }
//!
//! #[test]
//! fn time() {
//!     let mut runtime = DeterministicRuntime::new();
//!     let handle = runtime.handle();
//!     runtime.block_on(async move {
//!         delayed(handle).await;
//!     });
//! }
//! ```
//!
//! [tokio]: https://github.com/tokio-rs
//! [CurrentThread]:[tokio_executor::current_thread::CurrentThread]
//! [Delay]:[tokio_timer::Delay]
//! [Timeout]:[tokio_timer::Timeout]

use futures::{channel::oneshot, Future, FutureExt};
use std::time;
mod runtime;
pub use runtime::{
    DeterministicRuntime, DeterministicRuntimeHandle, SingleThreadedRuntime,
    SingleThreadedRuntimeHandle,
};

pub trait Environment: Unpin + Sized + Clone + Send {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static;
    /// Return the time now according to the executor.
    fn now(&self) -> time::Instant;
    /// Returns a delay future which completes after the provided instant.
    fn delay(&self, deadline: time::Instant) -> tokio_timer::Delay;
    /// Returns a delay future which completes at some time from now.
    fn delay_from(&self, from_now: time::Duration) -> tokio_timer::Delay {
        let now = self.now();
        self.delay(now + from_now)
    }
    /// Creates a timeout future which will execute blah blah
    fn timeout<T>(&self, value: T, timeout: time::Duration) -> tokio_timer::Timeout<T>;
}

pub fn spawn_with_result<F, E, U>(env: &E, future: F) -> impl Future<Output = Option<U>>
where
    F: Future<Output = U> + Send + 'static,
    U: Send + 'static,
    E: Environment,
{
    let (tx, rx) = oneshot::channel();
    env.spawn(async {
        tx.send(future.await).unwrap_or(());
    });
    Box::new(rx).map(|v| v.ok())
}

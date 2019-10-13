# simulation

This crate provides an abstraction over the [tokio] [CurrentThread] runtime
which allows for simulating applications.

The [Environment] trait provides an abstraction over [Delay] and [Timeout].
This allows for applications to be generic over [DeterministicRuntime] or
[SingleThreadedRuntime].

[DeterministicRuntime] will automatically advance a mocked clock if there is
no more work to do, up until the next timeout. This results in applications which
can be decoupled from time, facilitating fast integration/simulation tests.

```rust
use std::time;
use simulation::{DeterministicRuntime, Environment};

async fn delayed<E>(handle: E) where E: Environment {
   let start_time = handle.now();
   handle.delay_from(time::Duration::from_secs(30)).await;
   println!("that was fast!");
   assert_eq!(handle.now(), start_time + time::Duration::from_secs(30));
}

#[test]
fn time() {
    let mut runtime = DeterministicRuntime::new();
    let handle = runtime.handle();
    runtime.block_on(async move {
        delayed(handle).await;
    });
}
```

[tokio]: https://github.com/tokio-rs
[CurrentThread]:[tokio_executor::current_thread::CurrentThread]
[Delay]:[tokio_timer::Delay]
[Timeout]:[tokio_timer::Timeout]

License: MIT

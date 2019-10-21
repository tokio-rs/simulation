# Simulation
Simulation provides abstractions for building asynchronous networked applications in Rust with support for [FoundationDB] style deterministic simulation testing. The provided abstractions allow application authors to create construct components or whole systems which are generic over sources of nondeterminism like scheduling, time or the network.

Simulation provides determinstic implementations of scheduling, time and networking along with fault injection driven by a seedable RNG. The end result is the ability to simulate and inject faults into an application, or a set of interconnected applications deterministically.


### Runtime
Simulation is built on top of [Tokio]. Currently there are two runtime implementations, `DeterministicRuntime` and `SingleThreadedRuntime`. 

```rust
use std::time;
use simulation::{DeterministicRuntime, Environment, SingleThreadedRuntime};

async fn delayed<E>(handle: E) where E: Environment {
   let start_time = handle.now();
   handle.delay_from(time::Duration::from_secs(30)).await;
   println!("that was fast!");
   assert_eq!(handle.now(), start_time + time::Duration::from_secs(30));
}

fn main() {
    //let mut runtime = SingleThreadedRuntime::new(); SingleThreadedRuntime can be swapped in
    let mut runtime = DeterministicRuntime::new();
    let handle = runtime.handle();
    runtime.block_on(async move {
        delayed(handle).await;
    });
}
```

In a [Tokio] application, tasks typically execute until there 






[FoundationDB]: https://apple.github.io/foundationdb/index.html
[Tokio]: https://github.com/tokio-rs/tokio
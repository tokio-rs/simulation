# simulation

This crate provides an abstraction over the [tokio] [CurrentThread] runtime
which allows for simulating applications.

The [Environment] trait provides an abstraction over [Delay] and [Timeout].
This allows for applications to be generic over [DeterministicRuntime] or
[SingleThreadedRuntime].

[DeterministicRuntime] will automatically advance a mocked clock if there is
no more work to do, up until the next timeout. This results in applications which
can be decoupled from time, facilitating fast integration/simulation tests.

Simulation additionally provides facilities for networking and fault injection.

```rust
use crate::{Environment, TcpListener};
   use futures::{SinkExt, StreamExt};
   use std::{io, net, time};
   use tokio::codec::{Framed, LinesCodec};

   /// Start a client request handler which will write greetings to clients.
   async fn handle<E>(env: E, socket: <E::TcpListener as TcpListener>::Stream, addr: net::SocketAddr)
   where
       E: Environment,
   {
       // delay the response, in deterministic mode this will immediately progress time.
       env.delay_from(time::Duration::from_secs(1));
       println!("handling connection from {:?}", addr);
       let mut transport = Framed::new(socket, LinesCodec::new());
       if let Err(e) = transport.send(String::from("Hello World!")).await {
           println!("failed to send response: {:?}", e);
       }
   }

   /// Start a server which will bind to the provided addr and repyl to clients.
   async fn server<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
   where
       E: Environment,
   {
       let mut listener = env.bind(addr).await?;

       while let Ok((socket, addr)) = listener.accept().await {
           let request = handle(env.clone(), socket, addr);
           env.spawn(request)
       }
       Ok(())
   }


   /// Create a client which will read a message from the server
   async fn client<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
   where
       E: Environment,
   {
       let conn = env.connect(addr).await?;
       let mut transport = Framed::new(conn, LinesCodec::new());
       let result = transport.next().await.unwrap().unwrap();
       assert_eq!(result, "Hello world!");
       Ok(())
   }
```

[tokio]: https://github.com/tokio-rs
[CurrentThread]:[tokio_executor::current_thread::CurrentThread]
[Delay]:[tokio_timer::Delay]
[Timeout]:[tokio_timer::Timeout]

License: MIT

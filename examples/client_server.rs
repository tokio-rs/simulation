use futures::{SinkExt, StreamExt};
use simulation::{Environment, TcpListener};
use std::{io, net, time};
use tokio::codec::{Framed, LinesCodec};
/// Start a client request handler which will write greetings to clients.
async fn handle_request<E>(
    env: E,
    socket: <E::TcpListener as TcpListener>::Stream,
    addr: net::SocketAddr,
) where
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
        let request = handle_request(env.clone(), socket, addr);
        env.spawn(request)
    }
    Ok(())
}

/// Create a client which will read a message from the server
async fn client<E>(env: E, addr: net::SocketAddr) -> Result<(), io::Error>
where
    E: Environment,
{
    loop {
        match env.connect(addr).await {
            Err(_) => {
                env.delay_from(time::Duration::from_secs(1)).await;
                continue;
            }
            Ok(conn) => {
                let mut transport = Framed::new(conn, LinesCodec::new());
                let result = transport.next().await.unwrap().unwrap();
                assert_eq!(result, "Hello World!");
                println!("Success!");
                return Ok(());
            }
        }
    }
}

fn main() {
    let mut runtime = simulation::deterministic::DeterministicRuntime::new_with_seed(1).unwrap();
    let handle = runtime.handle();
    runtime.block_on(async {
        let bind_addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let server = server(handle.clone(), bind_addr);
        handle.spawn(async move {
            server.await.unwrap();
        });
        client(handle, bind_addr).await.unwrap();
    })
}

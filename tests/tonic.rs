use tonic::{transport::Server, Request, Response, Status};
use simulation::deterministic::DeterministicRuntime;
use std::{net};
use simulation::{Environment, TcpListener};


pub mod hello_world {
    tonic::include_proto!("helloworld");
}

use hello_world::{
    server::{Greeter, GreeterServer},
    HelloReply, HelloRequest,
};


#[derive(Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        println!("Got a request: {:?}", request);

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[test]
fn hyper_request_response() {
    let mut runtime = DeterministicRuntime::new().unwrap();
    let handle = runtime.localhost_handle();

    runtime.block_on(async move {
        let greeter = MyGreeter::default();
        let bind_addr: net::SocketAddr = "127.0.0.1:9092".parse().unwrap();
        let listener = handle.bind(bind_addr).await.unwrap();
        let listener = listener.into_stream();
        Server::builder()
            .add_service(GreeterServer::new(greeter))
            .serve_from_stream(listener).await.unwrap();
    });
}

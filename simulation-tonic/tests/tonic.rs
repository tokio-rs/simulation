use simulation::deterministic::DeterministicRuntime;
use simulation::{Environment, TcpListener};
use simulation_tonic::{AddOrigin, Connector};
use std::net;
use tonic::{transport::Server, Request, Response, Status};
use tower_service::Service;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

use hello_world::{
    client::GreeterClient,
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
        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[test]
fn hyper_request_response() {
    let mut runtime = DeterministicRuntime::new().unwrap();
    let latency_fault = runtime.latency_fault();
    let handle = runtime.localhost_handle();

    runtime.block_on(async move {
        handle.spawn(latency_fault.run());
        let server_handle = handle.clone();
        let bind_addr: net::SocketAddr = "127.0.0.1:9092".parse().unwrap();
        // spawn a server
        handle.spawn(async move {
            let greeter = MyGreeter::default();

            let listener = server_handle.bind(bind_addr).await.unwrap();
            let listener = listener.into_stream();
            Server::builder()
                .add_service(GreeterServer::new(greeter))
                .serve_from_stream(listener)
                .await
                .unwrap();
        });
        let connector = Connector::new(handle.clone());
        let mut connector = hyper::client::service::Connect::new(
            connector,
            hyper::client::conn::Builder::new().http2_only(true).clone(),
        );
        let svc = connector
            .call("127.0.0.1:9092".parse().unwrap())
            .await
            .unwrap();
        let mut client = GreeterClient::new(AddOrigin::new(
            svc,
            hyper::Uri::from_static("http://127.0.0.1:9092"),
        ));
        let response = client
            .say_hello(HelloRequest {
                name: "simulation".into(),
            })
            .await
            .unwrap()
            .into_inner();

        assert_eq!(response.message, "Hello simulation!");
    });
}

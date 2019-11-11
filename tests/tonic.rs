use simulation::deterministic::DeterministicRuntime;
use simulation::{Environment, TcpListener};
use std::net;
use tonic::{transport::Server, Request, Response, Status};
use tower_service::Service;
use std::{
    net::{SocketAddr, ToSocketAddrs},
    pin::Pin,
    future::Future,
    task::{Context, Poll},
};

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

use hello_world::{
    server::{Greeter, GreeterServer},
    client::{GreeterClient},
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

        let connector = Connector {
            inner: handle,
        };
        let mut connector = hyper::client::service::Connect::new(connector, hyper::client::conn::Builder::new().http2_only(true).clone());

        let svc = connector.call("127.0.0.1:9092".to_socket_addrs().unwrap().next().unwrap()).await.unwrap();

        let mut client = GreeterClient::new(add_origin::AddOrigin::new(svc, hyper::Uri::from_static("http://127.0.0.1:9092")));

        client.say_hello(HelloRequest { name: "hello".into() }).await.unwrap();
    });
}

struct Connector<T> {
    inner: T,
}

impl<T> tower_service::Service<SocketAddr> for Connector<T>
    where T: Environment + Send + Sync + 'static,
{
    type Response = T::TcpStream;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Ok(()).into()
    }

    fn call(&mut self, addr: SocketAddr) -> Self::Future {
        let handle = self.inner.clone();

        Box::pin(async move {
            let conn = handle.connect(addr).await.unwrap();

            Ok(conn)
        })
    }
}

mod add_origin {
    use http::{Request, Uri};
    use std::task::{Context, Poll};
    use tower_service::Service;

    // From tonic/src/transport/service/add_origin.rs
    #[derive(Debug)]
    pub(crate) struct AddOrigin<T> {
        inner: T,
        origin: Uri,
    }

    impl<T> AddOrigin<T> {
        pub(crate) fn new(inner: T, origin: Uri) -> Self {
            Self { inner, origin }
        }
    }

    impl<T, ReqBody> Service<Request<ReqBody>> for AddOrigin<T>
    where
        T: Service<Request<ReqBody>>,
    {
        type Response = T::Response;
        type Error = T::Error;
        type Future = T::Future;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(cx)
        }

        fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
            // Split the request into the head and the body.
            let (mut head, body) = req.into_parts();

            // Split the request URI into parts.
            let mut uri: http::uri::Parts = head.uri.into();
            let set_uri = self.origin.clone().into_parts();

            // Update the URI parts, setting hte scheme and authority
            uri.scheme = Some(set_uri.scheme.expect("expected scheme").clone());
            uri.authority = Some(set_uri.authority.expect("expected authority").clone());

            // Update the the request URI
            head.uri = http::Uri::from_parts(uri).expect("valid uri");

            let request = Request::from_parts(head, body);

            self.inner.call(request)
        }
    }
}

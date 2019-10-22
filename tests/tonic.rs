use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Error, Response, server::accept::Accept};
use simulation::{deterministic::DeterministicRuntime, Environment};
use std::{net, pin::Pin, task::Context, io};

use futures::{Poll, StreamExt};

struct HyperAccept {
    inner: simulation::deterministic::Listener
}

impl Accept for HyperAccept {
    type Conn = simulation::deterministic::ServerConnection;
    type Error = io::Error;
    fn poll_accept(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Option<Result<Self::Conn, Self::Error>>> { 
            self.inner.poll_next_unpin(cx)
    }
}

#[test]
fn foo() {
    let mut runtime = DeterministicRuntime::new();
    let handle = runtime.handle();
    runtime.block_on(async {
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let listener: simulation::deterministic::Listener = handle.bind(addr).await.unwrap();
        let http = hyper::server::conn::Http::new();

        let make_service = make_service_fn(move |_| {
            async move {
                Ok::<_, Error>(service_fn(move |_| {
                    async move {
                        Ok::<_, Error>(Response::new(Body::from(format!(
                            "Hello Deterministic world!"
                        ))))
                    }
                }))
            }
        });
        let accept = HyperAccept{inner: listener};
        hyper::server::Builder::new(accept, http)
            
            .serve(make_service).await.unwrap();
    });
}

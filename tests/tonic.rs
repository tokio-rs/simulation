use futures::{Future, FutureExt};
use hyper::service::{make_service_fn, service_fn};
use hyper::{server::accept::Accept, Body, Error, Response};
use simulation::{deterministic::DeterministicRuntime, Environment};
use std::{io, net, pin::Pin, task::Context};

use futures::Poll;
#[derive(Clone)]
struct HyperExecutor<T> {
    inner: T,
}

impl<T, F> tokio_executor::TypedExecutor<F> for HyperExecutor<T>
where
    F: Future<Output = ()> + Send + 'static,
    T: simulation::Environment,
{
    fn spawn(&mut self, future: F) -> Result<(), tokio_executor::SpawnError> {
        <T as Environment>::spawn(&self.inner, Box::pin(future));
        Ok(())
    }
}

struct HyperAccept<T>
where
    T: simulation::TcpListener,
{
    inner: T,
}

struct HyperConnect<T> {
    inner: T,
}

struct HyperConnectFuture<T> {
    addr: net::SocketAddr,
    inner: T,
}

impl<T> Future for HyperConnectFuture<T>
where
    T: Environment,
{
    type Output = Result<(T::TcpStream, hyper::client::connect::Connected), io::Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match futures::ready!(self.inner.connect(self.addr).poll_unpin(cx)) {
            Ok(conn) => {
                let connected = hyper::client::connect::Connected::new();
                return Poll::Ready(Ok((conn, connected)));
            }
            Err(e) => return Poll::Ready(Err(e)),
        };
    }
}

impl<T> hyper::client::connect::Connect for HyperConnect<T>
where
    T: Environment + Send + Sync + Unpin + 'static,
    T::TcpStream: Unpin,
{
    type Transport = T::TcpStream;
    type Error = std::io::Error;
    type Future = HyperConnectFuture<T>;
    fn connect(&self, dst: hyper::client::connect::Destination) -> Self::Future {
        let host = dst.host();
        let port = dst.port().expect("expected to find a port");
        HyperConnectFuture {
            inner: self.inner.clone(),
            addr: format!("{}:{}", host, port).parse().unwrap(),
        }
    }
}

impl<T> Accept for HyperAccept<T>
where
    T: simulation::TcpListener + Unpin,
{
    type Conn = T::Stream;
    type Error = io::Error;
    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let accept = self.inner.accept();
        futures::pin_mut!(accept);

        match futures::ready!(accept.poll(cx)) {
            Ok((sock, _)) => Poll::Ready(Some(Ok(sock))),
            Err(e) => Poll::Ready(Some(Err(e))),
        }
    }
}

#[test]
fn hyper_request_response() {
    let mut runtime = DeterministicRuntime::new().unwrap();
    let handle = runtime.handle();
    runtime.block_on(async move {
        let server_addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let server_handle = handle.clone();
        handle.clone().spawn(async move {
            let listener = server_handle.bind(server_addr).await.unwrap();
            let make_service = make_service_fn(move |_| {
                async move {
                    Ok::<_, Error>(service_fn(move |_| {
                        async move {
                            Ok::<_, Error>(Response::new(Body::from(
                                "Hello Deterministic world!\n".to_string(),
                            )))
                        }
                    }))
                }
            });
            let accept = HyperAccept { inner: listener };
            hyper::Server::builder(accept)
                .executor(HyperExecutor {
                    inner: server_handle.clone(),
                })
                .serve(make_service)
                .await
                .unwrap();
        });
        let connector = HyperConnect {
            inner: handle.clone(),
        };
        let builder = hyper::client::Client::builder();
        let client = builder.build(connector);
        let request = hyper::Request::builder()
            .uri("http://127.0.0.1:8080/foo")
            .method("GET")
            .body(hyper::body::Body::default())
            .unwrap();
        let response = client.request(request).await.unwrap();
        let mut body = response.into_body();

        while let Some(Ok(resp)) = body.next().await {
            let bytes = resp.into_bytes();
            let message = std::str::from_utf8(&bytes[..]).unwrap();
            assert_eq!(message, "Hello Deterministic world!\n");
            return;
        }
        assert!(false, "expected to read response message");
    });
}

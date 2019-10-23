use futures::Future;
use hyper::service::{make_service_fn, service_fn};
use hyper::{server::accept::Accept, Body, Error, Response};
use simulation::{
    deterministic::DeterministicRuntime, singlethread::SingleThreadedRuntime, Environment,
};
use std::{io, net, pin::Pin, task::Context};

use futures::{Poll, StreamExt};
#[derive(Clone)]
struct HyperExecutor<T> {
    inner: T,
}

impl<T> tokio_executor::Executor for HyperExecutor<T>
where
    T: simulation::Environment,
{
    fn spawn(
        &mut self,
        future: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) -> Result<(), tokio_executor::SpawnError> {
        <T as Environment>::spawn(&self.inner, future);
        Ok(())
    }
}

struct HyperAccept<T>
where
    T: simulation::TcpListener,
{
    inner: T,
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
            Ok((sock, _)) => return Poll::Ready(Some(Ok(sock))),
            Err(e) => return Poll::Ready(Some(Err(e))),
        }
    }
}

#[test]
fn foo() {
    let mut runtime = DeterministicRuntime::new().unwrap();
    let handle = runtime.handle();
    runtime.block_on(async {
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let listener = handle.bind(addr).await.unwrap();
        let http = hyper::server::conn::Http::new();
        let make_service = make_service_fn(move |_| {
            async move {
                Ok::<_, Error>(service_fn(move |_| {
                    async move {
                        Ok::<_, Error>(Response::new(Body::from(format!(
                            "Hello Deterministic world!\n"
                        ))))
                    }
                }))
            }
        });

        let accept = HyperAccept { inner: listener };
        let executor = HyperExecutor {
            inner: handle.clone(),
        };
        hyper::server::Builder::new(accept, http)
            .executor(executor)
            .serve(make_service)
            .await
            .unwrap();
    });
}

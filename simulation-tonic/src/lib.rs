pub use add_origin::AddOrigin;
use futures::{Future, Poll};
use simulation::Environment;
use std::{io, net, pin::Pin, task::Context};

pub struct Connector<T> {
    inner: T,
}

impl<T> Connector<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

type ServiceFuture<R, E> = Pin<Box<dyn Future<Output = Result<R, E>> + Send + 'static>>;

impl<T> tower_service::Service<net::SocketAddr> for Connector<T>
where
    T: Environment + Send + Sync + 'static,
{
    type Response = T::TcpStream;
    type Error = io::Error;
    type Future = ServiceFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Ok(()).into()
    }

    fn call(&mut self, addr: net::SocketAddr) -> Self::Future {
        let handle = self.inner.clone();
        Box::pin(async move { handle.connect(addr).await })
    }
}

mod add_origin {
    use http::{Request, Uri};
    use std::task::{Context, Poll};
    use tower_service::Service;

    // From tonic/src/transport/service/add_origin.rs
    #[derive(Debug)]
    pub struct AddOrigin<T> {
        inner: T,
        origin: Uri,
    }

    impl<T> AddOrigin<T> {
        pub fn new(inner: T, origin: Uri) -> Self {
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

            // Update the URI parts, setting http scheme and authority
            uri.scheme = Some(set_uri.scheme.expect("expected scheme"));
            uri.authority = Some(set_uri.authority.expect("expected authority"));

            // Update the the request URI
            head.uri = http::Uri::from_parts(uri).expect("valid uri");

            let request = Request::from_parts(head, body);

            self.inner.call(request)
        }
    }
}

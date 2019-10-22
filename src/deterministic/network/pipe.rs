use bytes::{Buf, BytesMut, IntoBuf};
use futures::Poll;
use std::{io, pin::Pin, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_sync::AtomicWaker;

/// In-memory AsyncRead/AsyncWrite.
#[derive(Debug)]
pub(crate) struct Pipe {
    buf: Option<BytesMut>,
    waker: AtomicWaker,
    dropped: bool,
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.dropped = true;
        self.waker.wake()
    }
}

impl Pipe {
    pub(crate) fn new() -> Self {
        Self {
            buf: None,
            waker: AtomicWaker::new(),
            dropped: false,
        }
    }
}

impl AsyncRead for Pipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dst: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.dropped {
            return Poll::Ready(Ok(0));
        }

        if let Some(mut bytes) = self.buf.take() {
            debug_assert!(!bytes.is_empty());
            let amt = std::cmp::min(bytes.len(), dst.len());
            let b = bytes.split_to(amt).freeze();
            let mut b = b.into_buf();
            b.copy_to_slice(&mut dst[..amt]);
            if bytes.is_empty() {
                self.buf.take();
            } else {
                self.buf.replace(bytes);
            }
            self.waker.wake();
            Poll::Ready(Ok(amt))
        } else {
            self.waker.register_by_ref(cx.waker());
            Poll::Pending
        }
    }
}

impl AsyncWrite for Pipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if self.dropped {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if self.buf.is_some() {
            self.waker.register_by_ref(cx.waker());
            Poll::Pending
        } else {
            self.buf = Some(buf.into());
            self.waker.wake();
            Poll::Ready(Ok(buf.len()))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if self.dropped {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.dropped = true;
        self.waker.wake();
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    /// Tests that a pipe can transport values.
    fn bounded_pipe() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new_with_seed(3).unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let rw = Pipe::new();
            let (mut r, mut w) = tokio::io::split(rw);
            handle.spawn(async move {
                w.write_all(b"foo").await.unwrap();
                w.write_all(b"bar").await.unwrap();
                w.write_all(b"baz").await.unwrap();
            });
            let mut target = vec![0; 9];
            r.read_exact(&mut target).await.unwrap();
            assert_eq!(std::str::from_utf8(&target[..]).unwrap(), "foobarbaz");
        })
    }

    #[test]
    /// When a pipe is shutdown, the write side should return an error on subsequent writes, and the read
    /// side should always return Ok(0).
    /// TODO: Check if this logic matches TcpStream
    fn shutdown_pipe() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let rw = Pipe::new();
        runtime.block_on(async {
            let (mut r, mut w) = tokio::io::split(rw);
            w.write(b"foo").await.unwrap();
            w.shutdown().await.unwrap();
            assert!(
                w.write_all(b"foo").await.is_err(),
                "expected write to fail after shutdown"
            );
            let mut target = vec![0; 0];
            assert_eq!(
                r.read(&mut target[..]).await.unwrap(),
                0,
                "expected to read 0 bytes from shutdown pipe"
            );
        })
    }
}

use bytes::{Buf, BytesMut, IntoBuf};
use futures::{FutureExt, Poll};
use std::{io, pin::Pin, task::Context};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_sync::AtomicWaker;

#[derive(Debug)]
pub(crate) struct Pipe {
    faults: super::NetworkFaults,
    read_delay: Option<tokio::timer::Delay>,
    write_delay: Option<tokio::timer::Delay>,
    buf: Option<BytesMut>,
    waker: AtomicWaker,
    shutdown: bool,
}

impl Pipe {
    pub(crate) fn new(faults: super::NetworkFaults) -> Self {
        Self {
            faults,
            read_delay: None,
            write_delay: None,
            buf: None,
            waker: AtomicWaker::new(),
            shutdown: false,
        }
    }
    pub(crate) fn shutdown(&mut self) {
        self.shutdown = true;
        self.waker.wake();
    }
}

impl AsyncRead for Pipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dst: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.shutdown {
            return Poll::Ready(Ok(0));
        }

        // check if there is an existing read delay
        if let Some(mut delay) = self.read_delay.take() {
            // try to poll the delay
            if let Poll::Pending = delay.poll_unpin(cx) {
                self.read_delay.replace(delay);
                return Poll::Pending;
            }
        } else {
            // no poll delay found, lets roll for another one with a 25% probability of being delayed on the next poll
            // for anywhere from 100 to 5000 milliseconds.
            if let Some(mut new_delay) = self.faults.socket_read_delay() {
                if let Poll::Pending = new_delay.poll_unpin(cx) {
                    self.read_delay.replace(new_delay);
                    return Poll::Pending;
                }
            }
        }

        if let Some(mut bytes) = self.buf.take() {
            debug_assert!(bytes.len() > 0);
            let amt = std::cmp::min(bytes.len(), dst.len());
            let b = bytes.split_to(amt).freeze();
            let mut b = b.into_buf();
            b.copy_to_slice(&mut dst[..amt]);
            if bytes.len() == 0 {
                self.buf.take();
            } else {
                self.buf.replace(bytes);
            }
            self.waker.wake();
            return Poll::Ready(Ok(amt));
        } else {
            self.waker.register_by_ref(cx.waker());
            return Poll::Pending;
        }
    }
}

impl AsyncWrite for Pipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if self.shutdown {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }

        // check if there is an existing write delay
        if let Some(mut delay) = self.write_delay.take() {
            // try to poll the delay
            if let Poll::Pending = delay.poll_unpin(cx) {
                self.write_delay.replace(delay);
                return Poll::Pending;
            }
        } else {
            // no poll delay found, lets roll for another one with a 25% probability of being delayed on the next poll
            // for anywhere from 100 to 5000 milliseconds.
            if let Some(mut new_delay) = self.faults.socket_write_delay() {
                if let Poll::Pending = new_delay.poll_unpin(cx) {
                    self.write_delay.replace(new_delay);
                    return Poll::Pending;
                }
            }
        }

        if let Some(_) = self.buf {
            self.waker.register_by_ref(cx.waker());
            return Poll::Pending;
        } else {
            self.buf = Some(buf.into());
            self.waker.wake();
            return Poll::Ready(Ok(buf.len()));
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if self.shutdown {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.shutdown = true;
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
    /// Tests that a pipe can transport values, also demonstrates the probabilistic delay/fault injection capabilites
    /// of the pipe. 
    fn bounded_pipe() {
        let mut runtime = crate::DeterministicRuntime::new_with_seed(3).unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let time_before = handle.now();
            let rw = Pipe::new(handle.network_faults());
            let (mut r, mut w) = tokio::io::split(rw);
            handle.spawn(async move {
                w.write_all("foo".as_bytes()).await.unwrap();
                w.write_all("bar".as_bytes()).await.unwrap();
                w.write_all("baz".as_bytes()).await.unwrap();
            });
            let mut target = vec![0; 9];
            r.read_exact(&mut target).await.unwrap();
            assert_eq!(std::str::from_utf8(&target[..]).unwrap(), "foobarbaz");
            let time_after = handle.now();
            assert!(
                time_after > time_before,
                "expected with a seed of 3, the timer would advance"
            );
        })
    }

    #[test]
    /// When a pipe is shutdown, the write side should return an error on subsequent writes, and the read
    /// side should always return Ok(0).
    /// TODO: Check if this logic matches TcpStream
    fn shutdown_pipe() {
        let mut runtime = crate::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        let rw = Pipe::new(handle.network_faults());
        runtime.block_on(async {
            let (mut r, mut w) = tokio::io::split(rw);
            w.write("foo".as_bytes()).await.unwrap();
            w.shutdown().await.unwrap();
            assert!(
                w.write_all("foo".as_bytes()).await.is_err(),
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

use bytes::{Buf, Bytes, BytesMut, IntoBuf};
use futures::{channel::mpsc, stream::FusedStream, Poll, StreamExt, FutureExt};
use std::{collections::VecDeque, io, net, num, pin::Pin, sync::Arc, task::Context, time};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_sync::AtomicWaker;
use try_lock::TryLock;

struct Inner {
    client_to_server: Pipe,
    server_to_client: Pipe,
}

struct Pipe {
    env: crate::DeterministicRuntimeHandle,
    read_delay: Option<tokio::timer::Delay>,
    write_delay: Option<tokio::timer::Delay>,
    buf: Option<BytesMut>,
    waker: AtomicWaker,
    dropped: bool,
}

impl Pipe {
    fn new(env: crate::DeterministicRuntimeHandle) -> Self {
        Self {
            env,
            read_delay: None,
            write_delay: None,
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
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
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
            // for anywhere from 100 to 1000 milliseconds.
            if let Some(mut new_delay) = self.env.maybe_random_delay(0.25, time::Duration::from_millis(100), time::Duration::from_millis(1000)) {
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
        if self.dropped {
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
            // for anywhere from 100 to 1000 milliseconds.
            if let Some(mut new_delay) = self.env.maybe_random_delay(0.25, time::Duration::from_millis(100), time::Duration::from_millis(1000)) {
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
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        self.dropped = true;
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn bounded_pipe() {
        let mut runtime = crate::DeterministicRuntime::new_with_seed(2).unwrap();
        let handle = runtime.handle();
        runtime.block_on(async {
            let time_before = handle.now();
            let rw = Pipe::new(handle.clone());
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
            assert!(time_after > time_before, "expected with a seed of 2, the timer would advance");            
        })
    }
}

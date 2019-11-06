use super::CloggedConnection;
/// Swizzle is a network fault generator which follows the "swizzle clog" approach
/// popularized by FoundationDB.
///
/// The general algorithm consists of a series of dice rolls followed by actions which introduce
/// gradual ordered faults in a networked application. Connections are "clogged" between endpoints,
/// causing infinite delays for existing and new connections.
///
/// 1. Roll dice to decide if network swizzle should happen.
/// 2. Pick a random subset of connections and order them.
/// 3. Wait a random amount of time before setting the first connection as clogged.
/// 4. Repeat this process until all connections have been clogged.
/// 5. Once all connections have been clogged, unclog each connection in reverse order.
use crate::deterministic::{random::DeterministicRandomHandle, DeterministicTimeHandle};
use futures::{FutureExt, Poll, Stream};
use std::{ops, pin::Pin, task::Context, time::Duration};

/// Duration between clogging actions when swizzling.
const SWIZZLE_PROGRESSION_INTERVAL: ops::Range<Duration> =
    Duration::from_secs(0)..Duration::from_secs(10);

/// Duration to wait between completing clogging of all connection, and beginning to
/// unclog connections.
const SWIZZLE_UNSWIZZLE_PAUSE: ops::Range<Duration> =
    Duration::from_secs(0)..Duration::from_secs(120);

/// State of the SwizzleClog operation. State progression is as follows:
/// ```
///     Idle  --------> Clog
///                     |
///                     v
///     ReverseIdle <-- ReverseIdle
/// ```
///
/// In between each state transition, various timeouts or dice rolls must be satsified to
/// allow progression.
#[derive(Debug)]
enum State {
    /// Idle, no clog is present. To progress, a successful dice roll is required.
    Idle,

    /// Clog, the swizzle clog workload is progressing through the `to_clog` set. To progress,
    /// the `to_clog` set must be empty and there must be no outstanding delays.
    Clog,

    /// ReverseIdle, all connections which need to be clogged have been clogged. To progress,
    /// any outstanding delays must have elapsed.
    ReverseIdle,

    /// Begin unclogging all clogged connections. To complete, there must be no connections remaining
    /// in the `clogged` set, and no outstanding timeouts.
    Unclog,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SwizzleAction {
    Clog(CloggedConnection),
    Unclog(CloggedConnection),
}

/// i love this word
struct Swizzler {
    random: DeterministicRandomHandle,
    time: DeterministicTimeHandle,
    state: State,
    delay: Option<tokio::timer::Delay>,
    to_clog: Vec<CloggedConnection>,
    clogged: Vec<CloggedConnection>,
}

impl Swizzler {
    fn new(
        random: DeterministicRandomHandle,
        time: DeterministicTimeHandle,
        connections: Vec<CloggedConnection>,
    ) -> Self {
        Self {
            random,
            time,
            state: State::Idle,
            delay: None,
            to_clog: connections,
            clogged: vec![],
        }
    }

    fn ensure_delay(&mut self, range: std::ops::Range<Duration>) {
        if self.delay.is_none() {
            let duration = self.random.gen_range(range);
            let delay = self.time.delay_from(duration);
            self.delay.replace(delay);
        }
    }

    /// Poll any swizzler timeout, removing the timeout once elpased. If there is no timeout,
    /// immediately returns `Poll::Ready`
    fn poll_delay(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if let Some(delay) = self.delay.as_mut() {
            futures::ready!(delay.poll_unpin(cx));
            self.delay.take();
            return Poll::Ready(());
        }
        return Poll::Ready(());
    }
}

impl Stream for Swizzler {
    type Item = SwizzleAction;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match this.state {
                State::Idle => {
                    // Idle, pick a time in the future to start the swizzle clog oepration.
                    this.ensure_delay(Duration::from_secs(0)..Duration::from_secs(120));
                    futures::ready!(this.poll_delay(cx));
                    // delay has elapsed, transition state
                    this.ensure_delay(SWIZZLE_PROGRESSION_INTERVAL);
                    this.state = State::Clog;
                }
                State::Clog => {
                    if this.to_clog.is_empty() {
                        // all connections have been clogged, transition to the next state if there is no delay.
                        futures::ready!(this.poll_delay(cx));
                        this.ensure_delay(SWIZZLE_UNSWIZZLE_PAUSE);
                        this.clogged.reverse();
                        this.state = State::ReverseIdle;
                    } else {
                        // wait out any pending delays
                        futures::ready!(this.poll_delay(cx));
                        // pick the next connection to clog
                        let next = this.to_clog.remove(0);
                        this.clogged.push(next.clone());
                        // set a new delay after putting the picked connection into the clogged set
                        this.ensure_delay(SWIZZLE_PROGRESSION_INTERVAL);
                        return Poll::Ready(Some(SwizzleAction::Clog(next)));
                    }
                }
                State::ReverseIdle => {
                    debug_assert!(
                        this.to_clog.is_empty(),
                        "expected the to_clog set to be empty"
                    );
                    // once the reverse pause is complete, transition to unclogging.
                    futures::ready!(this.poll_delay(cx));
                    this.ensure_delay(SWIZZLE_PROGRESSION_INTERVAL);
                    this.state = State::Unclog;
                }
                State::Unclog => {
                    if this.clogged.is_empty() {
                        // no more work to do, wait out any remaining delays and transition back to idle.
                        futures::ready!(this.poll_delay(cx));
                        return Poll::Ready(None);
                    } else {
                        futures::ready!(this.poll_delay(cx));
                        let next = this.clogged.remove(0);
                        this.ensure_delay(SWIZZLE_PROGRESSION_INTERVAL);
                        return Poll::Ready(Some(SwizzleAction::Unclog(next)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn swizzle_clog_generator() {
        let mut runtime = crate::deterministic::DeterministicRuntime::new().unwrap();
        let handle = runtime.handle();
        runtime.block_on(async move {
            let time_handle = handle.time_handle();
            let random_handle = handle.random_handle();
            let conn1 =
                CloggedConnection::new("10.0.0.1".parse().unwrap(), "10.0.0.2".parse().unwrap());
            let conn2 =
                CloggedConnection::new("10.0.0.2".parse().unwrap(), "10.0.0.3".parse().unwrap());
            let conn3 =
                CloggedConnection::new("10.0.0.3".parse().unwrap(), "10.0.0.1".parse().unwrap());

            let mut swizzler = Swizzler::new(random_handle, time_handle, vec![conn1, conn2, conn3]);
            let mut results = vec![];
            while let Some(next) = swizzler.next().await {
                results.push(next);
            }
            assert_eq!(
                results.len(),
                6,
                "expected 6 actions from 3 connections, clog and unclog"
            );
            assert_eq!(
                vec![
                    SwizzleAction::Clog(conn1),
                    SwizzleAction::Clog(conn2),
                    SwizzleAction::Clog(conn3),
                    SwizzleAction::Unclog(conn3),
                    SwizzleAction::Unclog(conn2),
                    SwizzleAction::Unclog(conn1)
                ],
                results
            );
        })
    }
}

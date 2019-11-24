//! Fault injector which periodically adjusts socket latency.
use super::Inner;
use crate::deterministic::{DeterministicRandomHandle, DeterministicTimeHandle};
use std::{ops, sync, time};

pub struct LatencyFaultInjectorConfig {
    client_latency_range: ops::Range<time::Duration>,
    server_latency_range: ops::Range<time::Duration>,
}

pub struct LatencyFaultInjector {
    inner: sync::Arc<sync::Mutex<Inner>>,
    random_handle: DeterministicRandomHandle,
    time_handle: DeterministicTimeHandle,
    config: LatencyFaultInjectorConfig,
}

impl LatencyFaultInjector {
    pub(crate) fn from_config(
        inner: sync::Arc<sync::Mutex<Inner>>,
        random_handle: DeterministicRandomHandle,
        time_handle: DeterministicTimeHandle,
        config: LatencyFaultInjectorConfig,
    ) -> Self {
        Self {
            inner,
            random_handle,
            time_handle,
            config,
        }
    }

    pub(crate) fn new(
        inner: sync::Arc<sync::Mutex<Inner>>,
        random_handle: DeterministicRandomHandle,
        time_handle: DeterministicTimeHandle,
    ) -> Self {
        Self {
            inner,
            random_handle,
            time_handle,
            config: LatencyFaultInjectorConfig {
                client_latency_range: time::Duration::from_secs(0)..time::Duration::from_secs(100),
                server_latency_range: time::Duration::from_secs(0)..time::Duration::from_secs(100),
            },
        }
    }

    /// Consumes this fault injector and begins injecting randomized latency into both client and server connections..
    pub async fn run(self) {
        loop {
            let min_interval = time::Duration::from_secs(1);
            let max_interval = time::Duration::from_secs(5);
            let wait_interval = self.random_handle.gen_range(min_interval..max_interval);
            self.time_handle.delay_from(wait_interval).await;
            // every second, adjust latencies across all connections.
            self.time_handle
                .delay_from(time::Duration::from_secs(1))
                .await;
            if self.random_handle.should_fault(0.1) {
                self.inject_latency();
            }
            if self.random_handle.should_fault(0.1) {
                self.unclog_clog();
            }
        }
    }

    fn unclog_clog(&self) {
        let mut lock = self.inner.lock().unwrap();
        let clogged_connections = lock.clogged_connections();
        for clogged in clogged_connections {
            if self.random_handle.should_fault(0.1) {
                tracing::debug!("unclogging {:?}", clogged);
                lock.unclog_connection(clogged);
            }
        }

        let mut to_clog = vec![];
        for connection in &lock.connections {
            if self.random_handle.should_fault(0.01) {
                to_clog.push(super::CloggedConnection::new(
                    connection.source.ip(),
                    connection.dest.ip(),
                ));
            }
        }
        for clog in to_clog {
            tracing::debug!("clogging {:?}", clog);
            lock.clog_connection(clog)
        }
    }

    /// Generate a new client latency value for the provided config.
    fn client_latency(&self) -> time::Duration {
        self.random_handle
            .gen_range(self.config.client_latency_range.clone())
    }

    /// Generate a new server latency value for the provided config.
    fn server_latency(&self) -> time::Duration {
        self.random_handle
            .gen_range(self.config.server_latency_range.clone())
    }

    /// Iterate through all connections, setting a random latency value for both server and client send/receive calls.
    fn inject_latency(&self) {
        let mut lock = self.inner.lock().unwrap();
        for connection in lock.connections.iter_mut() {
            connection
                .client_fault_handle
                .set_receive_latency(self.client_latency());
            connection
                .client_fault_handle
                .set_send_latency(self.client_latency());
            connection
                .server_fault_handle
                .set_receive_latency(self.server_latency());
            connection
                .server_fault_handle
                .set_send_latency(self.server_latency());
        }
    }
}

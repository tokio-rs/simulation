//! State for tasks which are running under simulation.
use crate::fault::FaultInjector;
use crate::state::LogicalTaskId;
use core::cell::RefCell;
use core::future::Future;
use pin_project::pin_project;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

#[derive(Debug, Clone)]
pub(super) struct LogicalTask {
    /// The unique identifier associated with this task.
    id: LogicalTaskId,

    /// Simulate tasks executing at different speeds.
    ///
    /// A task with a poll_priority of 0 will execute immediately,
    /// while a task with a poll_priority of 4 will be forced to cycle
    /// through the Executors task queue 4 times before being polled once.
    ///
    /// This effectively results in a task wit ha poll_priorty of 4 running 4x slower
    /// than a task with a poll priority of 0.
    poll_priority: u16,

    /// The amount of times the wrapped future will return Poll::Pending before
    /// polling the inner future. This will be reset to poll_priority once it
    /// reaches 0.
    poll_delay_remaining: u16,

    fault_injector: Option<FaultInjector>,
}

impl LogicalTask {
    fn new(id: LogicalTaskId, fault_injector: Option<FaultInjector>) -> Self {
        Self {
            id,
            poll_priority: 0,
            poll_delay_remaining: 0,
            fault_injector,
        }
    }
}

thread_local! {
    static CURRENT_TASK: RefCell<Option<LogicalTaskId>> = RefCell::new(None)
}

pub(super) fn current_taskid() -> Option<LogicalTaskId> {
    CURRENT_TASK.with(|cx| *cx.borrow())
}

fn with_current_taskid<F, U>(taskid: LogicalTaskId, f: F) -> U
where
    F: FnOnce() -> U,
{
    struct DropGuard;
    impl Drop for DropGuard {
        fn drop(&mut self) {
            CURRENT_TASK.with(|cx| cx.borrow_mut().take());
        }
    }
    CURRENT_TASK.with(|cx| *cx.borrow_mut() = Some(taskid));
    let _guard = DropGuard;
    f()
}

#[pin_project]
pub(super) struct LogicalTaskWrapper<F> {
    inner: Arc<Mutex<LogicalTask>>,
    #[pin]
    task: F,
}

impl<F> LogicalTaskWrapper<F> {
    fn refresh_poll_priority(&self) {
        let mut lock = self.inner.lock().unwrap();
        let task_id = lock.id;
        if let Some(new) = lock
            .fault_injector
            .as_ref()
            .and_then(|fi| fi.update_task_poll_priority(task_id))
        {
            lock.poll_delay_remaining = new;
            lock.poll_priority = new;
        }
    }
}

impl<F> Future for LogicalTaskWrapper<F>
where
    F: Future,
{
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.refresh_poll_priority();
        let this = self.project();
        let id = { this.inner.lock().unwrap().id };
        with_current_taskid(id, move || {
            let mut lock = this.inner.lock().unwrap();
            if lock.poll_delay_remaining == 0 {
                lock.poll_delay_remaining = lock.poll_priority;
                this.task.poll(cx)
            } else {
                lock.poll_delay_remaining -= 1;
                let waker = cx.waker();
                waker.wake_by_ref();
                Poll::Pending
            }
        })
    }
}

pub(super) fn wrap_task<F>(
    id: LogicalTaskId,
    fault_injector: Option<FaultInjector>,
    f: F,
) -> LogicalTaskWrapper<F>
where
    F: Future,
{
    let inner = LogicalTask::new(id, fault_injector);
    let inner = Arc::new(Mutex::new(inner));
    let task = LogicalTaskWrapper {
        inner: Arc::clone(&inner),
        task: f,
    };

    task
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LogicalMachineId;
    use std::task::Context;

    /// Test that a TaskID is properly set on poll and unset after.
    #[test]
    fn task_poll_sets_taskid() {
        let machine = LogicalMachineId::new(42);
        let taskid = machine.new_task(42);
        let task = async { assert_eq!(Some(taskid), current_taskid()) };
        let task = wrap_task(taskid, None, task);
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());

        futures::pin_mut!(task);
        assert_eq!(
            task.poll(&mut cx),
            Poll::Ready(()),
            "expected poll to succeed"
        );
        assert_eq!(None, current_taskid(), "expected taskid to be unset");
    }
}

//! State for tasks which are running under simulation.
use crate::state::LogicalTaskId;
use core::cell::RefCell;
use core::future::Future;
use pin_project::pin_project;
use std::{
    pin::Pin,
    sync::{Arc, Mutex, Weak},
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
}

impl LogicalTask {
    fn new(id: LogicalTaskId) -> Self {
        Self {
            id,
            poll_priority: 0,
            poll_delay_remaining: 0,
        }
    }
}

#[derive(Debug)]
pub struct LogicalTaskHandle {
    inner: Weak<Mutex<LogicalTask>>,
}

impl LogicalTaskHandle {
    /// Set the number of times the logical task associated with this
    /// handle
    pub fn set_poll_priority(&self, prio: u16) -> Option<()> {
        Weak::upgrade(&self.inner).map(|l| {
            let mut lock = l.lock().unwrap();
            lock.poll_priority = prio;
            lock.poll_delay_remaining = prio;
        })
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

impl<F> Future for LogicalTaskWrapper<F>
where
    F: Future,
{
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
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

pub(super) fn wrap_task<F>(id: LogicalTaskId, f: F) -> (LogicalTaskHandle, LogicalTaskWrapper<F>)
where
    F: Future,
{
    let inner = LogicalTask::new(id);
    let inner = Arc::new(Mutex::new(inner));
    let handle = LogicalTaskHandle {
        inner: Arc::downgrade(&inner),
    };
    let task = LogicalTaskWrapper {
        inner: Arc::clone(&inner),
        task: f,
    };

    (handle, task)
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
        let (_, task) = wrap_task(taskid, task);
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());

        futures::pin_mut!(task);
        assert_eq!(
            task.poll(&mut cx),
            Poll::Ready(()),
            "expected poll to succeed"
        );
        assert_eq!(None, current_taskid(), "expected taskid to be unset");
    }

    #[test]
    fn task_poll_priority() {
        let machine = LogicalMachineId::new(42);
        let taskid = machine.new_task(42);
        let task = async { assert_eq!(Some(taskid), current_taskid()) };
        let (handle, task) = wrap_task(taskid, task);
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        futures::pin_mut!(task);
        handle.set_poll_priority(2);
        // A poll priority of 2 should result in the task returning
        // Poll::Pending twice before the underlying task is polled.
        assert_eq!(task.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(task.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(task.as_mut().poll(&mut cx), Poll::Ready(()));
    }

    /// Test that the method for waking a task with a nonzero poll priority works with Tokio's basic
    /// scheduler.
    #[test]
    fn task_poll_priority_wakes_task() {
        let mut rt = tokio::runtime::Builder::new()
            .basic_scheduler()
            .build()
            .unwrap();
        rt.block_on(async {
            let machine = LogicalMachineId::new(42);
            let taskid = machine.new_task(42);
            let task = async { assert_eq!(Some(taskid), current_taskid()) };
            let (handle, task) = wrap_task(taskid, task);
            futures::pin_mut!(task);
            handle.set_poll_priority(2);
            task.await;
        })
    }

    /// Test that tasks with a higher poll priority always complete after tasks with a lower poll priority.
    #[test]
    fn test_poll_priority_imbalance() {
        let mut rt = tokio::runtime::Builder::new()
            .basic_scheduler()
            .enable_time()
            .freeze_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let machine = LogicalMachineId::new(42);
            let taskid = machine.new_task(42);
            let task = async { assert_eq!(Some(taskid), current_taskid()) };
            let (handle, task_slow) = wrap_task(taskid, task);
            handle.set_poll_priority(2);
            let task_fast = async { tokio::time::delay_for(std::time::Duration::from_secs(1)) };
            tokio::select! {
                _ = task_slow => {
                    assert!(false, "expected fast task to complete first")
                }
                _ = task_fast => {
                    return;
                }
            }
        })
    }
}

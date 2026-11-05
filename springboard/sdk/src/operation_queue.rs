//! `OperationQueue` parity: runs closures (operations) on background threads, at most
//! `max_concurrent_operation_count` at a time, in the order they were added. A suspended queue holds its operations
//! until it is resumed. `cancel_all_operations` drops the operations that have not started; the running ones finish.
//! Dependencies between operations and quality-of-service classes are not modelled; `dispatch` covers those cases.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The reference operation queue's default concurrency: no fixed limit, which here means one per CPU core.
pub const DEFAULT_MAX_CONCURRENT_OPERATIONS: usize = 0;

struct State {
    pending: VecDeque<Job>,
    running: usize,
    max_concurrent: usize,
    suspended: bool,
}

impl State {
    fn limit(&self) -> usize {
        if self.max_concurrent == DEFAULT_MAX_CONCURRENT_OPERATIONS {
            thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        } else {
            self.max_concurrent
        }
    }
}

struct Shared {
    state: Mutex<State>,
    idle: Condvar,
}

/// `OperationQueue`: a queue of closures that a bounded number of worker threads drain.
pub struct OperationQueue {
    shared: Arc<Shared>,
}

impl Default for OperationQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationQueue {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    pending: VecDeque::new(),
                    running: 0,
                    max_concurrent: DEFAULT_MAX_CONCURRENT_OPERATIONS,
                    suspended: false,
                }),
                idle: Condvar::new(),
            }),
        }
    }

    /// `maxConcurrentOperationCount`: the number of operations that may run at once. 0 means the default.
    pub fn set_max_concurrent_operation_count(&self, max: usize) {
        let mut s = self.shared.state.lock().unwrap();
        s.max_concurrent = max;
        drop(s);
        self.start_ready();
    }

    pub fn max_concurrent_operation_count(&self) -> usize {
        self.shared.state.lock().unwrap().max_concurrent
    }

    /// `isSuspended`: true while the queue holds its operations back.
    pub fn is_suspended(&self) -> bool {
        self.shared.state.lock().unwrap().suspended
    }

    /// `isSuspended = true` or `false`. Resuming starts the operations that were held.
    pub fn set_suspended(&self, suspended: bool) {
        self.shared.state.lock().unwrap().suspended = suspended;
        self.start_ready();
    }

    /// `addOperation(_:)`: queues `op`. It runs as soon as a worker slot is free and the queue is not suspended.
    pub fn add_operation<F: FnOnce() + Send + 'static>(&self, op: F) {
        self.shared.state.lock().unwrap().pending.push_back(Box::new(op));
        self.start_ready();
    }

    /// `operationCount`: the operations that have not finished, running or waiting.
    pub fn operation_count(&self) -> usize {
        let s = self.shared.state.lock().unwrap();
        s.pending.len() + s.running
    }

    /// `cancelAllOperations()`: drops every operation that has not started. Running operations are not stopped.
    pub fn cancel_all_operations(&self) {
        self.shared.state.lock().unwrap().pending.clear();
        self.shared.idle.notify_all();
    }

    /// `waitUntilAllOperationsAreFinished()`: blocks until nothing is running and nothing can run. A suspended queue
    /// with waiting operations keeps this call blocked until it is resumed.
    pub fn wait_until_all_operations_are_finished(&self) {
        let mut s = self.shared.state.lock().unwrap();
        while s.running > 0 || (!s.suspended && !s.pending.is_empty()) {
            s = self.shared.idle.wait(s).unwrap();
        }
    }

    /// Starts workers for waiting operations while there are free slots and the queue is not suspended.
    fn start_ready(&self) {
        loop {
            let job = {
                let mut s = self.shared.state.lock().unwrap();
                if s.suspended || s.pending.is_empty() || s.running >= s.limit() {
                    return;
                }
                s.running += 1;
                s.pending.pop_front().unwrap()
            };
            let shared = Arc::clone(&self.shared);
            thread::spawn(move || {
                job();
                let mut s = shared.state.lock().unwrap();
                s.running -= 1;
                shared.idle.notify_all();
                drop(s);
                let queue = OperationQueue { shared: Arc::clone(&shared) };
                queue.start_ready();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runs_every_operation_within_the_concurrency_limit() {
        let queue = OperationQueue::new();
        queue.set_max_concurrent_operation_count(2);
        assert_eq!(queue.max_concurrent_operation_count(), 2);
        let done = Arc::new(AtomicUsize::new(0));
        let now = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        for _ in 0..10 {
            let (done, now, peak) = (Arc::clone(&done), Arc::clone(&now), Arc::clone(&peak));
            queue.add_operation(move || {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                thread::sleep(std::time::Duration::from_millis(5));
                now.fetch_sub(1, Ordering::SeqCst);
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        queue.wait_until_all_operations_are_finished();
        assert_eq!(done.load(Ordering::SeqCst), 10);
        assert!(peak.load(Ordering::SeqCst) <= 2);
        assert_eq!(queue.operation_count(), 0);
    }

    #[test]
    fn a_suspended_queue_holds_its_operations_until_resumed() {
        let queue = OperationQueue::new();
        queue.set_suspended(true);
        assert!(queue.is_suspended());
        let done = Arc::new(AtomicUsize::new(0));
        let d = Arc::clone(&done);
        queue.add_operation(move || {
            d.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(queue.operation_count(), 1);
        assert_eq!(done.load(Ordering::SeqCst), 0);
        queue.set_suspended(false);
        queue.wait_until_all_operations_are_finished();
        assert_eq!(done.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_drops_operations_that_have_not_started() {
        let queue = OperationQueue::new();
        queue.set_suspended(true);
        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let d = Arc::clone(&done);
            queue.add_operation(move || {
                d.fetch_add(1, Ordering::SeqCst);
            });
        }
        queue.cancel_all_operations();
        assert_eq!(queue.operation_count(), 0);
        queue.set_suspended(false);
        queue.wait_until_all_operations_are_finished();
        assert_eq!(done.load(Ordering::SeqCst), 0);
    }
}

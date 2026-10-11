//! Worker threads whose results wake the shell's poll loop through a socket
//! pair, so the UI never blocks on the network or on image decoding.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};

/// A queued job; None when it panicked and has no fallback result.
type Job<T> = Box<dyn FnOnce() -> Option<T> + Send>;

/// Jobs waiting for a thread; the front is served first.
struct Queue<T> {
    state: Mutex<QueueState<T>>,
    ready: Condvar,
}

struct QueueState<T> {
    jobs: VecDeque<Job<T>>,
    closed: bool,
}

/// Pokes the poll loop from any thread.
#[derive(Clone)]
pub struct Waker(Arc<Mutex<UnixStream>>);

impl Waker {
    pub fn wake(&self) {
        // a full pipe means a wake-up is already pending
        let _ = self.0.lock().unwrap().write(&[1]);
    }
}

pub struct Pool<T> {
    queue: Arc<Queue<T>>,
    done: Arc<Mutex<Vec<T>>>,
    waker: Waker,
    reader: UnixStream,
}

impl<T: Send + 'static> Pool<T> {
    pub fn new(threads: usize) -> std::io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let queue = Arc::new(Queue { state: Mutex::new(QueueState { jobs: VecDeque::new(), closed: false }), ready: Condvar::new() });
        let done = Arc::new(Mutex::new(Vec::new()));
        let waker = Waker(Arc::new(Mutex::new(writer)));
        for _ in 0..threads.max(1) {
            let (queue, done, waker) = (queue.clone(), done.clone(), waker.clone());
            crate::bare::sys::spawn_named("deildu-job", move || Self::work(queue, done, waker));
        }
        Ok(Pool { queue, done, waker, reader })
    }

    fn work(queue: Arc<Queue<T>>, done: Arc<Mutex<Vec<T>>>, waker: Waker) {
        loop {
            let job = {
                let mut state = queue.state.lock().unwrap();
                loop {
                    if let Some(job) = state.jobs.pop_front() {
                        break job;
                    }
                    if state.closed {
                        return;
                    }
                    state = queue.ready.wait(state).unwrap();
                }
            };
            // a panicking job must neither kill the thread nor leave its caller waiting
            if let Some(result) = job() {
                done.lock().unwrap().push(result);
                waker.wake();
            }
        }
    }

    fn push(&self, front: bool, job: Job<T>) {
        let mut state = self.queue.state.lock().unwrap();
        if front {
            state.jobs.push_front(job);
        } else {
            state.jobs.push_back(job);
        }
        self.queue.ready.notify_one();
    }

    /// Queues work for the next free thread.
    pub fn spawn(&self, job: impl FnOnce() -> T + Send + 'static) {
        self.push(false, Box::new(move || catch_unwind(AssertUnwindSafe(job)).ok()));
    }

    /// Like `spawn`, but when the job panics `fallback` makes its (failed) result,
    /// so whoever waits on it hears back.
    pub fn spawn_or(&self, job: impl FnOnce() -> T + Send + 'static, fallback: impl FnOnce() -> T + Send + 'static) {
        self.push(false, Box::new(move || Some(catch_unwind(AssertUnwindSafe(job)).unwrap_or_else(|_| fallback()))));
    }

    /// Like `spawn_or`, but ahead of everything queued: for work the user waits on.
    pub fn spawn_front_or(&self, job: impl FnOnce() -> T + Send + 'static, fallback: impl FnOnce() -> T + Send + 'static) {
        self.push(true, Box::new(move || Some(catch_unwind(AssertUnwindSafe(job)).unwrap_or_else(|_| fallback()))));
    }

    /// The fd that becomes readable when results (or wake-ups) are pending.
    pub fn fd(&self) -> std::ffi::c_int {
        self.reader.as_raw_fd()
    }

    pub fn waker(&self) -> Waker {
        self.waker.clone()
    }

    /// Clears pending wake-ups and returns finished results in completion order.
    pub fn drain(&mut self) -> Vec<T> {
        let mut sink = [0u8; 64];
        while matches!(self.reader.read(&mut sink), Ok(n) if n > 0) {}
        std::mem::take(&mut *self.done.lock().unwrap())
    }

    /// Results pending without touching the pipe.
    pub fn pending(&self) -> usize {
        self.done.lock().unwrap().len()
    }
}

impl<T> Drop for Pool<T> {
    fn drop(&mut self) {
        self.queue.state.lock().unwrap().closed = true;
        self.queue.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_readable(fd: std::ffi::c_int) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if crate::bare::sys::wait(&[fd], 50)[0] {
                return true;
            }
        }
        false
    }

    #[test]
    fn results_arrive_through_the_wake_fd() {
        let mut pool = Pool::new(2).unwrap();
        assert!(!crate::bare::sys::wait(&[pool.fd()], 0)[0], "quiet at start");
        pool.spawn(|| 40 + 2);
        assert!(wait_readable(pool.fd()));
        assert_eq!(pool.drain(), vec![42]);
        assert!(!crate::bare::sys::wait(&[pool.fd()], 0)[0], "drain clears the pipe");
        assert!(pool.drain().is_empty());
    }

    #[test]
    fn waker_pokes_without_a_result() {
        let mut pool: Pool<u8> = Pool::new(1).unwrap();
        let w = pool.waker();
        std::thread::spawn(move || w.wake()).join().unwrap();
        assert!(wait_readable(pool.fd()));
        assert!(pool.drain().is_empty());
        assert_eq!(pool.pending(), 0);
    }

    #[test]
    fn a_panicking_job_reports_its_fallback_and_the_thread_survives() {
        let mut pool = Pool::new(1).unwrap();
        pool.spawn_or(|| panic!("decoder blew up"), || -1);
        pool.spawn(|| panic!("no fallback"));
        pool.spawn(|| 7);
        let mut got = vec![];
        let start = Instant::now();
        while got.len() < 2 && start.elapsed() < Duration::from_secs(5) {
            wait_readable(pool.fd());
            got.extend(pool.drain());
        }
        assert_eq!(got, vec![-1, 7]);
    }

    #[test]
    fn front_jobs_run_before_queued_ones() {
        let mut pool = Pool::new(1).unwrap();
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let g = gate.clone();
        pool.spawn(move || {
            drop(g.lock().unwrap());
            0
        });
        pool.spawn(|| 1);
        pool.spawn(|| 2);
        pool.spawn_front_or(|| 3, || -1);
        drop(held);
        let mut got = vec![];
        let start = Instant::now();
        while got.len() < 4 && start.elapsed() < Duration::from_secs(5) {
            wait_readable(pool.fd());
            got.extend(pool.drain());
        }
        got.retain(|&v| v != 0);
        assert_eq!(got, vec![3, 1, 2]);
    }

    #[test]
    fn many_jobs_complete_on_several_threads() {
        let mut pool = Pool::new(3).unwrap();
        for i in 0..20 {
            pool.spawn(move || {
                std::thread::sleep(Duration::from_millis(5));
                i
            });
        }
        let mut got = vec![];
        let start = Instant::now();
        while got.len() < 20 && start.elapsed() < Duration::from_secs(5) {
            wait_readable(pool.fd());
            got.extend(pool.drain());
        }
        got.sort();
        assert_eq!(got, (0..20).collect::<Vec<_>>());
    }
}

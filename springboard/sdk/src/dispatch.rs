//! Grand Central Dispatch parity: `DispatchQueue`, `DispatchGroup`, `DispatchWorkItem`, QoS.
//!
//! One lazily started, process-wide pool backs every queue. Idle workers block on a `Condvar` (and exit after 30 s
//! idle); nothing spins or polls. Background and Utility workers lower their priority through
//! `springboard::gfx::affinity::background_thread()` (nice +10). The main queue runs closures on the app loop
//! thread: `DispatchQueue::main().async_` pushes and writes an eventfd the runtime loop waits on, and the loop
//! drains it before it draws.
//!
//! Width cap: the pool runs at most `available_parallelism` jobs at once across all lanes. Higher QoS is served
//! first; a lane borrows width the higher lanes leave idle, and Background always gets one slot when it has work.
//!
//! Closures are `FnOnce() + Send + 'static`. A panicking closure is caught and logged; the worker survives.

use std::cmp::Ordering as Ord_;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send + 'static>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(job: Job) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
        eprintln!("dispatch: a closure panicked");
    }
}

/// Quality of service (`DispatchQoS`): selects the worker lane and its thread priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Qos {
    UserInteractive,
    UserInitiated,
    Utility,
    Background,
}

impl Qos {
    fn lane(self) -> usize {
        self as usize
    }
    fn name(self) -> &'static str {
        match self {
            Qos::UserInteractive => "user-interactive",
            Qos::UserInitiated => "user-initiated",
            Qos::Utility => "utility",
            Qos::Background => "background",
        }
    }
}

const LANES: usize = 4;
const IDLE_EXIT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Lane {
    jobs: VecDeque<Job>,
    idle: usize,
    threads: usize,
}

struct Timer {
    deadline: Instant,
    /// the latest it may fire: `deadline + leeway`
    latest: Instant,
    seq: u64,
    queue: DispatchQueue,
    job: Option<Job>,
}
impl PartialEq for Timer {
    fn eq(&self, o: &Self) -> bool {
        self.deadline == o.deadline && self.seq == o.seq
    }
}
impl Eq for Timer {}
impl PartialOrd for Timer {
    fn partial_cmp(&self, o: &Self) -> Option<Ord_> {
        Some(self.cmp(o))
    }
}
impl Ord for Timer {
    // reversed: BinaryHeap pops the earliest deadline
    fn cmp(&self, o: &Self) -> Ord_ {
        (o.deadline, o.seq).cmp(&(self.deadline, self.seq))
    }
}

#[derive(Default)]
struct State {
    lanes: [Lane; LANES],
    timers: BinaryHeap<Timer>,
    timer_thread: bool,
    /// jobs running now, all lanes: never above `Pool::cap`
    active: usize,
    running: [usize; LANES],
}

struct Pool {
    state: Mutex<State>,
    cv: [Condvar; LANES],
    timer_cv: Condvar,
    cap: usize,
    seq: AtomicU64,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        state: Mutex::new(State::default()),
        cv: Default::default(),
        timer_cv: Condvar::new(),
        cap: thread::available_parallelism().map_or(2, |n| n.get()).max(1),
        seq: AtomicU64::new(0),
    })
}

impl Pool {
    fn submit(&'static self, qos: Qos, job: Job) {
        let l = qos.lane();
        let mut st = lock(&self.state);
        st.lanes[l].jobs.push_back(job);
        let lane = &mut st.lanes[l];
        if lane.idle > 0 {
            self.cv[l].notify_one();
        }
        if lane.jobs.len() > lane.idle && lane.threads < self.cap {
            lane.threads += 1;
            let ok = thread::Builder::new().name(format!("dispatch-{}", qos.name())).spawn(move || self.worker(qos)).is_ok();
            if !ok {
                st.lanes[l].threads -= 1;
            }
        }
    }

    fn worker(&'static self, qos: Qos) {
        if matches!(qos, Qos::Utility | Qos::Background) {
            springboard::gfx::affinity::background_thread();
        }
        let l = qos.lane();
        let mut st = lock(&self.state);
        loop {
            if self.may_run(&st, l) {
                let job = st.lanes[l].jobs.pop_front().unwrap();
                st.active += 1;
                st.running[l] += 1;
                drop(st);
                run(job);
                st = lock(&self.state);
                st.active -= 1;
                st.running[l] -= 1;
                // a slot opened: wake the best waiting lane first
                for k in 0..LANES {
                    if st.lanes[k].idle > 0 && !st.lanes[k].jobs.is_empty() {
                        self.cv[k].notify_one();
                    }
                }
                continue;
            }
            st.lanes[l].idle += 1;
            let (g, to) = self.cv[l].wait_timeout(st, IDLE_EXIT).unwrap_or_else(|e| e.into_inner());
            st = g;
            st.lanes[l].idle -= 1;
            if to.timed_out() && st.lanes[l].jobs.is_empty() {
                st.lanes[l].threads -= 1;
                return;
            }
        }
    }

    /// Width cap: all lanes together run at most `cap` jobs. Lane `l` may take a slot when no higher lane has work
    /// waiting (it borrows idle width); Background with nothing running takes one regardless, so it never starves.
    fn may_run(&self, st: &State, l: usize) -> bool {
        if st.lanes[l].jobs.is_empty() || st.active >= self.cap {
            return false;
        }
        let starved_bg = l == Qos::Background.lane() && st.running[l] == 0;
        starved_bg || (0..l).all(|h| st.lanes[h].jobs.is_empty())
    }

    fn after(&'static self, d: Duration, leeway: Duration, queue: DispatchQueue, job: Job) {
        let mut st = lock(&self.state);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + d;
        st.timers.push(Timer { deadline, latest: deadline + leeway, seq, queue, job: Some(job) });
        if !st.timer_thread {
            st.timer_thread = true;
            if thread::Builder::new().name("dispatch-timer".into()).spawn(move || self.timer_loop()).is_err() {
                st.timer_thread = false;
            }
        }
        self.timer_cv.notify_one();
    }

    fn timer_loop(&'static self) {
        let mut st = lock(&self.state);
        loop {
            let now = Instant::now();
            match st.timers.peek().map(|t| t.deadline) {
                None => {
                    // nothing pending: exit when idle so no thread lingers; `after` restarts it
                    let (g, to) = self.timer_cv.wait_timeout(st, IDLE_EXIT).unwrap_or_else(|e| e.into_inner());
                    st = g;
                    if to.timed_out() && st.timers.is_empty() {
                        st.timer_thread = false;
                        return;
                    }
                }
                Some(_) => {
                    // Timer coalescing: wake once, at the earliest `latest` of all pending timers, and fire every timer
                    // whose deadline has passed by then. With no leeway this is the plain earliest-deadline wait.
                    let wake = st.timers.iter().map(|t| t.latest).min().unwrap_or(now);
                    if wake > now {
                        st = self.timer_cv.wait_timeout(st, wake - now).unwrap_or_else(|e| e.into_inner()).0;
                        continue;
                    }
                    let mut due = Vec::new();
                    while st.timers.peek().is_some_and(|t| t.deadline <= now) {
                        due.extend(st.timers.pop());
                    }
                    drop(st);
                    for mut t in due {
                        if let Some(job) = t.job.take() {
                            t.queue.enqueue(job);
                        }
                    }
                    st = lock(&self.state);
                }
            }
        }
    }
}

// ---- main queue ------------------------------------------------------------------------------------------------

mod main_q {
    use super::*;
    extern "C" {
        fn eventfd(init: u32, flags: i32) -> i32;
        fn read(fd: i32, buf: *mut u8, n: usize) -> isize;
        fn write(fd: i32, buf: *const u8, n: usize) -> isize;
    }
    const EFD_CLOEXEC: i32 = 0o2000000;
    const EFD_NONBLOCK: i32 = 0o4000;

    pub static JOBS: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
    pub static THREAD: OnceLock<ThreadId> = OnceLock::new();

    pub fn fd() -> i32 {
        static FD: OnceLock<i32> = OnceLock::new();
        // SAFETY: plain syscall; a negative result means "no wake-up descriptor" and the loop skips it.
        *FD.get_or_init(|| unsafe { eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC) })
    }
    pub fn wake() {
        let fd = fd();
        if fd >= 0 {
            let one = 1u64.to_ne_bytes();
            // SAFETY: eight bytes from a live buffer to our own eventfd; EAGAIN (counter full) already means "awake".
            unsafe { write(fd, one.as_ptr(), 8) };
        }
    }
    pub fn clear() {
        let fd = fd();
        if fd >= 0 {
            let mut b = [0u8; 8];
            // SAFETY: eight bytes into a live buffer; non-blocking, so an empty counter just returns EAGAIN.
            unsafe { read(fd, b.as_mut_ptr(), 8) };
        }
    }
}

/// The descriptor the app loop waits on for main-queue work (negative if the eventfd could not be created).
pub(crate) fn main_fd() -> i32 {
    main_q::fd()
}

/// Runs the queued main-queue closures on the calling (loop) thread. Returns true if any ran, so the loop marks a
/// frame needed. The first caller becomes "the main thread" for `sync`. Tests that drive an app without its loop call
/// this to run the main queue, like `RunLoop.main.run(until:)` in an XCTest.
pub fn drain_main() -> bool {
    let _ = main_q::THREAD.set(thread::current().id());
    main_q::clear();
    let mut ran = false;
    loop {
        // take one batch; closures queued while running re-arm the eventfd and form the next batch
        let batch: Vec<Job> = lock(&main_q::JOBS).drain(..).collect();
        if batch.is_empty() {
            return ran;
        }
        ran = true;
        for job in batch {
            run(job);
        }
    }
}

/// True on the main-queue thread, or when no main loop has started (tests, tools): then the caller's thread is main.
pub(crate) fn on_main_or_unset() -> bool {
    main_q::THREAD.get().map_or(true, |t| *t == thread::current().id())
}

fn on_main_thread() -> bool {
    main_q::THREAD.get() == Some(&thread::current().id())
}

// ---- queues ----------------------------------------------------------------------------------------------------

struct Serial {
    label: String,
    qos: Qos,
    state: Mutex<(VecDeque<Job>, bool)>,
}

#[derive(Clone)]
enum Kind {
    Main,
    Global(Qos),
    Serial(Arc<Serial>),
}

/// `DispatchQueue`: the main queue, a global concurrent queue per QoS, or a private serial queue.
#[derive(Clone)]
pub struct DispatchQueue(Kind);

impl std::fmt::Debug for DispatchQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Kind::Main => write!(f, "DispatchQueue(main)"),
            Kind::Global(q) => write!(f, "DispatchQueue(global {})", q.name()),
            Kind::Serial(s) => write!(f, "DispatchQueue(serial {:?})", s.label),
        }
    }
}

impl DispatchQueue {
    /// `DispatchQueue.main`: closures run on the app loop thread, before the next draw.
    pub fn main() -> Self {
        DispatchQueue(Kind::Main)
    }
    /// `DispatchQueue.global(qos:)`: concurrent.
    pub fn global(qos: Qos) -> Self {
        DispatchQueue(Kind::Global(qos))
    }
    /// `DispatchQueue(label:)`: serial, FIFO, user-initiated QoS.
    pub fn serial(label: &str) -> Self {
        Self::serial_qos(label, Qos::UserInitiated)
    }
    /// `DispatchQueue(label:qos:)`.
    pub fn serial_qos(label: &str, qos: Qos) -> Self {
        DispatchQueue(Kind::Serial(Arc::new(Serial { label: label.to_string(), qos, state: Mutex::new((VecDeque::new(), false)) })))
    }

    /// The queue's label.
    pub fn label(&self) -> &str {
        match &self.0 {
            Kind::Main => "main",
            Kind::Global(q) => q.name(),
            Kind::Serial(s) => &s.label,
        }
    }

    /// `async`: returns at once; the closure runs later on this queue.
    pub fn async_<F: FnOnce() + Send + 'static>(&self, f: F) {
        self.enqueue(Box::new(f));
    }

    fn enqueue(&self, job: Job) {
        match &self.0 {
            Kind::Main => {
                lock(&main_q::JOBS).push_back(job);
                main_q::wake();
            }
            Kind::Global(q) => pool().submit(*q, job),
            Kind::Serial(s) => {
                let start = {
                    let mut st = lock(&s.state);
                    st.0.push_back(job);
                    !std::mem::replace(&mut st.1, true)
                };
                if start {
                    let s = s.clone();
                    pool().submit(s.qos, Box::new(move || drain_serial(s)));
                }
            }
        }
    }

    /// `asyncAfter(deadline:)`: never runs before `delay` has passed. One timer thread waits on a `Condvar`.
    pub fn async_after<F: FnOnce() + Send + 'static>(&self, delay: Duration, f: F) {
        pool().after(delay, Duration::ZERO, self.clone(), Box::new(f));
    }

    /// `asyncAfter` with timer leeway (`DispatchSource` timer `leeway`, `Timer.tolerance`): runs in `[delay, delay + leeway]`.
    /// The timer thread wakes once for every pending timer whose window allows it, so tolerant timers coalesce with
    /// each other instead of each causing a wake-up.
    pub fn async_after_leeway<F: FnOnce() + Send + 'static>(&self, delay: Duration, leeway: Duration, f: F) {
        pool().after(delay, leeway, self.clone(), Box::new(f));
    }

    /// `sync`: runs the closure on this queue and waits for its result. A concurrent queue runs it on the calling
    /// thread. Panics when called on the main queue from the main thread, and deadlocks (as in GCD) when a serial
    /// queue syncs onto itself.
    pub fn sync<T: Send + 'static, F: FnOnce() -> T + Send + 'static>(&self, f: F) -> T {
        match &self.0 {
            Kind::Global(_) => f(),
            Kind::Main if on_main_thread() => panic!("DispatchQueue::main().sync called on the main thread (deadlock)"),
            _ => {
                let (tx, rx) = mpsc::channel();
                self.async_(move || {
                    let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
                });
                match rx.recv() {
                    Ok(Ok(v)) => v,
                    Ok(Err(p)) => std::panic::resume_unwind(p),
                    Err(_) => panic!("dispatch: queue dropped the sync closure"),
                }
            }
        }
    }

    /// `async(group:)`: the group counts the closure until it finishes.
    pub fn async_group<F: FnOnce() + Send + 'static>(&self, group: &DispatchGroup, f: F) {
        group.enter();
        let g = group.clone();
        self.async_(move || {
            run(Box::new(f));
            g.leave();
        });
    }

    /// `async(execute: workItem)`: skipped if the item was cancelled before it starts.
    pub fn async_item(&self, item: &DispatchWorkItem) {
        let it = item.clone();
        self.async_(move || it.perform());
    }

    /// `asyncAndWait(execute:)`: submits `f` and returns once it has finished. The caller waits, but the closure runs
    /// on this queue, also for the global queues (unlike `sync`, which runs it on the caller there). A panicking
    /// closure is logged by the pool and still counts as finished. Waiting from a serial queue onto itself deadlocks
    /// as in GCD, and the main queue refuses a wait from the main thread, as `sync` does.
    pub fn async_and_wait<F: FnOnce() + Send + 'static>(&self, f: F) {
        if matches!(self.0, Kind::Main) && on_main_thread() {
            panic!("DispatchQueue::main().asyncAndWait called on the main thread (deadlock)");
        }
        let item = DispatchWorkItem::new(f);
        self.async_item(&item);
        item.wait(None);
    }
}

fn drain_serial(s: Arc<Serial>) {
    loop {
        let job = {
            let mut st = lock(&s.state);
            match st.0.pop_front() {
                Some(j) => j,
                None => {
                    st.1 = false;
                    return;
                }
            }
        };
        run(job);
    }
}

// ---- group -----------------------------------------------------------------------------------------------------

#[derive(Default)]
struct GroupState {
    count: usize,
    notifies: Vec<(DispatchQueue, Job)>,
}

/// `DispatchGroup`: wait for, or be told when, a set of tasks has finished.
#[derive(Clone, Default)]
pub struct DispatchGroup(Arc<(Mutex<GroupState>, Condvar)>);

impl DispatchGroup {
    pub fn new() -> Self {
        Self::default()
    }
    /// `enter()`: one more task is outstanding.
    pub fn enter(&self) {
        lock(&self.0 .0).count += 1;
    }
    /// `leave()`: a task finished; the last one fires the `notify` closures and wakes `wait`. Extra calls are ignored.
    pub fn leave(&self) {
        let fire = {
            let mut st = lock(&self.0 .0);
            if st.count == 0 {
                return;
            }
            st.count -= 1;
            if st.count > 0 {
                return;
            }
            self.0 .1.notify_all();
            std::mem::take(&mut st.notifies)
        };
        for (q, job) in fire {
            q.enqueue(job);
        }
    }
    /// `notify(queue:)`: runs `f` on `queue` once the group is empty (at once if it already is).
    pub fn notify<F: FnOnce() + Send + 'static>(&self, queue: &DispatchQueue, f: F) {
        let mut st = lock(&self.0 .0);
        if st.count == 0 {
            drop(st);
            queue.async_(f);
        } else {
            st.notifies.push((queue.clone(), Box::new(f)));
        }
    }
    /// `wait(timeout:)`: blocks until the group is empty. `None` waits forever. Returns false on timeout.
    pub fn wait(&self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut st = lock(&self.0 .0);
        while st.count > 0 {
            match deadline {
                None => st = self.0 .1.wait(st).unwrap_or_else(|e| e.into_inner()),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return false;
                    }
                    st = self.0 .1.wait_timeout(st, d - now).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
        }
        true
    }
}

// ---- work item -------------------------------------------------------------------------------------------------

struct ItemInner {
    cancelled: AtomicBool,
    job: Mutex<Option<Job>>,
    done: (Mutex<bool>, Condvar),
}

/// `DispatchWorkItem`: a closure that can be cancelled before it starts and waited on.
#[derive(Clone)]
pub struct DispatchWorkItem(Arc<ItemInner>);

impl DispatchWorkItem {
    pub fn new<F: FnOnce() + Send + 'static>(f: F) -> Self {
        DispatchWorkItem(Arc::new(ItemInner { cancelled: AtomicBool::new(false), job: Mutex::new(Some(Box::new(f))), done: (Mutex::new(false), Condvar::new()) }))
    }
    /// `cancel()`: a work item that has not started never runs; one already running is not interrupted.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        // a cancelled item that never starts still releases its waiters
        if lock(&self.0.job).take().is_some() {
            self.finish();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }
    /// `perform()`: runs the closure on the calling thread unless cancelled (or already run).
    pub fn perform(&self) {
        let job = if self.is_cancelled() { None } else { lock(&self.0.job).take() };
        if let Some(job) = job {
            run(job);
            self.finish();
        }
    }
    fn finish(&self) {
        *lock(&self.0.done.0) = true;
        self.0.done.1.notify_all();
    }
    /// `wait(timeout:)`: until the item has run or been cancelled. Returns false on timeout.
    pub fn wait(&self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut done = lock(&self.0.done.0);
        while !*done {
            match deadline {
                None => done = self.0.done.1.wait(done).unwrap_or_else(|e| e.into_inner()),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return false;
                    }
                    done = self.0.done.1.wait_timeout(done, d - now).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn leeway_timers_run_within_their_window() {
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        DispatchQueue::global(Qos::Utility).async_after_leeway(Duration::from_millis(20), Duration::from_millis(60), move || {
            let _ = tx.send(start.elapsed());
        });
        let waited = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(waited >= Duration::from_millis(20) && waited <= Duration::from_millis(500), "{waited:?}");
    }

    use super::*;
    use std::sync::atomic::AtomicUsize;

    const T: Option<Duration> = Some(Duration::from_secs(5));

    #[test]
    fn async_and_wait_returns_only_after_the_closure_finished() {
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d = done.clone();
        DispatchQueue::global(Qos::Utility).async_and_wait(move || {
            std::thread::sleep(Duration::from_millis(30));
            d.store(true, Ordering::SeqCst);
        });
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    fn async_and_wait_runs_the_closure_on_the_queue_thread() {
        let q = DispatchQueue::serial("t.wait.thread");
        let caller = std::thread::current().id();
        let seen = Arc::new(Mutex::new(None));
        let s = seen.clone();
        q.async_and_wait(move || *s.lock().unwrap() = Some(std::thread::current().id()));
        let ran_on = (*seen.lock().unwrap()).expect("the closure ran");
        assert_ne!(ran_on, caller);
    }

    #[test]
    fn async_and_wait_on_a_serial_queue_keeps_submission_order() {
        let q = DispatchQueue::serial("t.wait.order");
        let out = Arc::new(Mutex::new(Vec::new()));
        for i in 1..=3 {
            let o = out.clone();
            q.async_(move || o.lock().unwrap().push(i));
        }
        let o = out.clone();
        q.async_and_wait(move || o.lock().unwrap().push(4));
        assert_eq!(*out.lock().unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn async_and_wait_counts_a_panicking_closure_as_finished() {
        let q = DispatchQueue::serial("t.wait.panic");
        q.async_and_wait(|| panic!("expected test panic"));
        let n = Arc::new(AtomicUsize::new(0));
        let c = n.clone();
        q.async_and_wait(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn async_and_wait_returns_after_each_job_in_turn() {
        let n = Arc::new(AtomicUsize::new(0));
        for step in 1..=5 {
            let c = n.clone();
            DispatchQueue::global(Qos::Utility).async_and_wait(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(n.load(Ordering::SeqCst), step);
        }
    }

    #[test]
    fn serial_queue_runs_in_order() {
        let q = DispatchQueue::serial("t.order");
        let out = Arc::new(Mutex::new(Vec::new()));
        let g = DispatchGroup::new();
        for i in 0..50 {
            let out = out.clone();
            q.async_group(&g, move || lock(&out).push(i));
        }
        assert!(g.wait(T));
        assert_eq!(*lock(&out), (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn global_queue_runs_concurrently() {
        // two closures that each wait for the other only finish if they run at the same time
        if pool().cap < 2 {
            return;
        }
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let g = DispatchGroup::new();
        for _ in 0..2 {
            let b = barrier.clone();
            DispatchQueue::global(Qos::UserInitiated).async_group(&g, move || {
                b.wait();
            });
        }
        assert!(g.wait(T));
    }

    #[test]
    fn total_running_jobs_never_exceed_the_core_count() {
        let cap = pool().cap;
        let (now, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let g = DispatchGroup::new();
        for i in 0..cap * 6 {
            let (now, peak) = (now.clone(), peak.clone());
            let qos = [Qos::UserInteractive, Qos::UserInitiated, Qos::Utility, Qos::Background][i % 4];
            DispatchQueue::global(qos).async_group(&g, move || {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(5));
                now.fetch_sub(1, Ordering::SeqCst);
            });
        }
        assert!(g.wait(T));
        assert!(peak.load(Ordering::SeqCst) <= cap, "peak {} > cap {}", peak.load(Ordering::SeqCst), cap);
    }

    #[test]
    fn background_still_runs_under_a_higher_qos_flood() {
        let cap = pool().cap;
        let g = DispatchGroup::new();
        for _ in 0..cap * 8 {
            DispatchQueue::global(Qos::UserInteractive).async_group(&g, || std::thread::sleep(Duration::from_millis(5)));
        }
        let bg = DispatchGroup::new();
        DispatchQueue::global(Qos::Background).async_group(&bg, || ());
        assert!(bg.wait(T), "background starved");
        assert!(g.wait(T));
    }

    #[test]
    fn group_notify_fires_after_all_leave() {
        let g = DispatchGroup::new();
        let n = Arc::new(AtomicUsize::new(0));
        g.enter();
        g.enter();
        let (tx, rx) = mpsc::channel();
        let n2 = n.clone();
        g.notify(&DispatchQueue::global(Qos::Utility), move || {
            tx.send(n2.load(Ordering::SeqCst)).unwrap();
        });
        n.fetch_add(1, Ordering::SeqCst);
        g.leave();
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "fired with one task outstanding");
        n.fetch_add(1, Ordering::SeqCst);
        g.leave();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        assert!(g.wait(T));
    }

    #[test]
    fn async_after_waits_for_the_deadline() {
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        DispatchQueue::global(Qos::Background).async_after(Duration::from_millis(80), move || {
            tx.send(Instant::now()).unwrap();
        });
        let at = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(at.duration_since(start) >= Duration::from_millis(80));
    }

    #[test]
    fn cancelled_work_item_does_not_run() {
        let ran = Arc::new(AtomicBool::new(false));
        let r = ran.clone();
        let item = DispatchWorkItem::new(move || r.store(true, Ordering::SeqCst));
        item.cancel();
        assert!(item.is_cancelled());
        let q = DispatchQueue::serial("t.cancel");
        q.async_item(&item);
        q.sync(|| ());
        assert!(item.wait(T));
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn work_item_runs_once_and_can_be_waited_on() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let item = DispatchWorkItem::new(move || {
            n2.fetch_add(1, Ordering::SeqCst);
        });
        DispatchQueue::global(Qos::UserInteractive).async_item(&item);
        assert!(item.wait(T));
        item.perform();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn sync_returns_the_value() {
        assert_eq!(DispatchQueue::serial("t.sync").sync(|| 7), 7);
        assert_eq!(DispatchQueue::global(Qos::Utility).sync(|| 8), 8);
    }

    #[test]
    fn main_queue_work_runs_only_when_drained() {
        let ran = Arc::new(AtomicBool::new(false));
        let r = ran.clone();
        DispatchQueue::main().async_(move || r.store(true, Ordering::SeqCst));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!ran.load(Ordering::SeqCst), "main work ran without a drain");
        assert!(drain_main());
        assert!(ran.load(Ordering::SeqCst));
        assert!(!drain_main());
    }
}

/// `DispatchSource.makeMemoryPressureSource`: `handler` runs on `queue` when the kernel reports memory pressure
/// (normal, warning, critical), built on the same PSI watcher the caches use (`bifrost_render::purgeable`).
pub struct DispatchSource;

pub use bifrost_render::purgeable::MemoryPressure;

impl DispatchSource {
    /// The handler lives for the process (there is no `cancel` yet: gap). Without PSI it is never called.
    pub fn memory_pressure<F: Fn(MemoryPressure) + Send + Sync + 'static>(queue: &DispatchQueue, handler: F) {
        let q = queue.clone();
        let handler = Arc::new(handler);
        bifrost_render::purgeable::on_memory_pressure(move |level| {
            let h = handler.clone();
            q.async_(move || h(level));
        });
    }
}

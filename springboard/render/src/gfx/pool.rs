//! A small long-lived worker pool for the per-frame band jobs (row bands of a draw, interleaved
//! rows of the scanout copy). Workers are spawned once, pinned to the big cluster once, and
//! sleep on a condvar between frames, so a frame no longer pays for thread spawns.
//! `run` returns only after every index ran, so the closure may borrow the caller's stack.

use std::sync::{Condvar, Mutex, OnceLock};

type Job = &'static (dyn Fn(usize) + Sync);

#[derive(Default)]
struct State {
    job: Option<Job>,
    next: usize,
    n: usize,
    running: usize,
    panicked: bool,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    done: Condvar,
    /// one dispatcher at a time
    gate: Mutex<()>,
}

/// Takes and runs indices of the current job until none is left; the caller of `run` does it too.
fn drain<'a>(sh: &'a Shared, mut st: std::sync::MutexGuard<'a, State>) {
    while let Some(job) = st.job.filter(|_| st.next < st.n) {
        let k = st.next;
        st.next += 1;
        st.running += 1;
        drop(st);
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(k))).is_ok();
        st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        st.running -= 1;
        st.panicked |= !ok;
    }
    if st.next >= st.n && st.running == 0 {
        sh.done.notify_all();
    }
}

fn shared(workers: usize, pin: bool) -> &'static Shared {
    let sh: &'static Shared = Box::leak(Box::new(Shared { state: Mutex::new(State::default()), work: Condvar::new(), done: Condvar::new(), gate: Mutex::new(()) }));
    for i in 0..workers {
        let spawned = std::thread::Builder::new().name(format!("render-{i}")).spawn(move || {
            if pin {
                super::affinity::pin_big();
            }
            let mut st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                while !(st.job.is_some() && st.next < st.n) {
                    st = sh.work.wait(st).unwrap_or_else(|e| e.into_inner());
                }
                drain(sh, st);
                st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            }
        });
        if spawned.is_err() {
            break; // the caller still drains every index alone
        }
    }
    sh
}

fn run_on(sh: &'static Shared, n: usize, f: &(dyn Fn(usize) + Sync)) {
    if n == 0 {
        return;
    }
    let _gate = sh.gate.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the reference is cleared again before this function returns, and it returns only
    // after `running == 0 && next >= n`, so no worker can still hold or start `f`.
    let job: Job = unsafe { std::mem::transmute::<&(dyn Fn(usize) + Sync), Job>(f) };
    {
        let mut st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        *st = State { job: Some(job), next: 0, n, running: 0, panicked: false };
    }
    sh.work.notify_all();
    drain(sh, sh.state.lock().unwrap_or_else(|e| e.into_inner()));
    let mut st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
    while st.next < st.n || st.running > 0 {
        st = sh.done.wait(st).unwrap_or_else(|e| e.into_inner());
    }
    st.job = None;
    let panicked = st.panicked;
    drop(st);
    if panicked {
        panic!("render pool job panicked");
    }
}

/// Runs `f(0)..f(n-1)` on the pool and the calling thread, returning when all are done.
pub fn run(n: usize, f: &(dyn Fn(usize) + Sync)) {
    static POOL: OnceLock<&'static Shared> = OnceLock::new();
    if n <= 1 {
        return (0..n).for_each(f);
    }
    run_on(POOL.get_or_init(|| shared(3, true)), n, f);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runs_every_band_and_returns() {
        let sh = shared(2, false);
        for n in [1, 2, 5, 16] {
            let hits: Vec<AtomicUsize> = (0..n).map(|_| AtomicUsize::new(0)).collect();
            run_on(sh, n, &|k| {
                hits[k].fetch_add(1, Ordering::SeqCst);
            });
            assert!(hits.iter().all(|h| h.load(Ordering::SeqCst) == 1), "n={n}");
        }
    }

    #[test]
    fn works_without_workers_and_borrows_the_stack() {
        let sh = shared(0, false);
        let sum = AtomicUsize::new(0);
        let local = [1usize, 2, 3, 4];
        run_on(sh, 4, &|k| {
            sum.fetch_add(local[k], Ordering::SeqCst);
        });
        assert_eq!(sum.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn a_panicking_band_is_reported_after_all_finish() {
        let sh = shared(2, false);
        let hits = AtomicUsize::new(0);
        let r = std::panic::catch_unwind(|| {
            run_on(sh, 4, &|k| {
                hits.fetch_add(1, Ordering::SeqCst);
                if k == 1 {
                    panic!("boom");
                }
            })
        });
        assert!(r.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 4);
        run_on(sh, 2, &|_| {});
    }
}

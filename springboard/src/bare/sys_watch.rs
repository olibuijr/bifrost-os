//! One thread for the watchers that only sleep on kernel fds or on a fixed deadline.
//!
//! Each watcher registers with `register` before `start` runs: the fds it blocks on, an optional
//! absolute deadline (`sys::now_ms()` clock) and a handler. `start` spawns the single "sys-watch"
//! thread, which blocks in one `poll(2)` whose timeout is the nearest deadline (no periodic wakeup
//! beyond the deadlines the watchers asked for). A handler runs when one of its fds is readable or
//! its deadline has passed, and returns its next deadline.

use super::sys;
use std::os::raw::c_int;
use std::sync::Mutex;

/// Handler: `ready[i]` says whether `fds[i]` is readable, has priority data (PSI triggers) or hung up; `now` is
/// `sys::now_ms()`. A watcher whose fd reports an error (POLLERR, POLLNVAL) is dropped, so a dead fd never spins.
/// Returns the next deadline, or `None` to wait for fds only.
pub type Run = Box<dyn FnMut(&[bool], f64) -> Option<f64> + Send>;

pub struct Watch {
    pub fds: Vec<c_int>,
    /// The poll events wanted on every fd: `sys::POLLIN` for pipes and sockets, `sys::POLLPRI` for PSI triggers
    /// (a proc file always polls readable, so POLLIN there would never sleep).
    pub events: i16,
    pub due: Option<f64>,
    pub run: Run,
}

static PENDING: Mutex<Vec<Watch>> = Mutex::new(Vec::new());

/// Adds a watcher; it is polled once `start` has run.
pub fn register(w: Watch) {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(w);
}

/// Spawns the "sys-watch" thread over everything registered so far. Call once, after registering.
pub fn start() {
    let watches = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    if watches.is_empty() {
        return;
    }
    sys::spawn_named("sys-watch", move || run(watches));
}

fn run(mut watches: Vec<Watch>) {
    let (mut fds, mut events, mut ready, mut revents) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    loop {
        fds.clear();
        events.clear();
        let spans: Vec<std::ops::Range<usize>> = watches
            .iter()
            .map(|w| {
                let start = fds.len();
                fds.extend_from_slice(&w.fds);
                events.extend(w.fds.iter().map(|_| w.events));
                start..fds.len()
            })
            .collect();
        let now = sys::now_ms();
        // the nearest deadline, rounded up to whole ms; -1 (no timeout) when no watcher has one
        let timeout = watches
            .iter()
            .filter_map(|w| w.due)
            .reduce(f64::min)
            .map_or(-1, |due| (due - now).ceil().max(0.0) as i32);
        sys::poll_revents(&fds, &events, timeout, &mut revents);
        let now = sys::now_ms();
        let mut dead = Vec::new();
        for (i, (w, span)) in watches.iter_mut().zip(spans).enumerate() {
            if revents[span.clone()].iter().any(|&r| r & (sys::POLLERR | sys::POLLNVAL) != 0) {
                dead.push(i);
                continue;
            }
            ready.clear();
            ready.extend(revents[span.clone()].iter().zip(&events[span]).map(|(&r, &e)| r & (e | sys::POLLHUP) != 0));
            let due = w.due.is_some_and(|d| now >= d);
            if due || ready.iter().any(|&r| r) {
                w.due = (w.run)(&ready, now);
            }
        }
        for i in dead.into_iter().rev() {
            eprintln!("springboard: sys-watch: dropping a watcher whose fd failed");
            watches.swap_remove(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn a_proc_file_polls_readable_but_not_priority() {
        // the 0.7.17 spin: PSI triggers are proc files, which always poll readable
        let f = std::fs::File::open("/proc/self/status").unwrap();
        let mut out = Vec::new();
        sys::poll_revents(&[f.as_raw_fd()], &[sys::POLLIN], 0, &mut out);
        assert_ne!(out[0] & sys::POLLIN, 0);
        sys::poll_revents(&[f.as_raw_fd()], &[sys::POLLPRI], 0, &mut out);
        assert_eq!(out[0] & sys::POLLPRI, 0);
    }
}

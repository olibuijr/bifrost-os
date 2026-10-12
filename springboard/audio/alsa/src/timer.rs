//! Timer scheduling: a `timerfd` on `CLOCK_MONOTONIC` armed with an absolute deadline. Deadlines come from the DLL
//! in host time (`CLOCK_MONOTONIC_RAW`); timerfd has no raw clock, so a deadline is moved to the monotonic clock
//! by the current offset between the two (they differ by the NTP slew, which changes slowly).

use crate::sys::{now_mono_ns, now_raw_ns};
use std::io;
use std::os::fd::RawFd;

/// Moves a raw-clock deadline to the monotonic clock given a simultaneous reading of both.
pub fn raw_to_mono(deadline_raw: u64, raw_now: u64, mono_now: u64) -> u64 {
    (deadline_raw as i128 + mono_now as i128 - raw_now as i128).max(1) as u64
}

/// The predicted host time to wake for a cycle: the DLL time of the cycle's first frame, minus the safety margin.
pub fn wake_time(cycle_start_ns: u64, safety_ns: u64) -> u64 {
    cycle_start_ns.saturating_sub(safety_ns)
}

pub struct CycleTimer {
    fd: RawFd,
}

impl CycleTimer {
    pub fn new() -> io::Result<CycleTimer> {
        // SAFETY: plain syscall; the returned fd is owned by the timer.
        let fd = unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(CycleTimer { fd })
    }

    /// The fd, for epoll.
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Arms a one-shot timer at an absolute `CLOCK_MONOTONIC` time. A time in the past fires at once.
    pub fn arm_mono(&self, deadline_mono_ns: u64) -> io::Result<()> {
        let spec = libc::itimerspec {
            it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 },
            it_value: libc::timespec { tv_sec: (deadline_mono_ns / 1_000_000_000) as libc::time_t, tv_nsec: (deadline_mono_ns % 1_000_000_000) as libc::c_long },
        };
        // SAFETY: `spec` is a valid itimerspec and the old-value pointer may be null.
        let r = unsafe { libc::timerfd_settime(self.fd, libc::TFD_TIMER_ABSTIME, &spec, std::ptr::null_mut()) };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Arms the timer at a host-time (`CLOCK_MONOTONIC_RAW`) deadline.
    pub fn arm_raw(&self, deadline_raw_ns: u64) -> io::Result<()> {
        let (raw, mono) = (now_raw_ns(), now_mono_ns());
        self.arm_mono(raw_to_mono(deadline_raw_ns, raw, mono))
    }

    /// Waits at most `timeout_ns` for the timer; true when it fired. Lets a caller check a stop flag between waits.
    pub fn wait_timeout(&self, timeout_ns: u64) -> io::Result<bool> {
        let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        let ms = timeout_ns.div_ceil(1_000_000).min(i32::MAX as u64) as i32;
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut p, 1, ms) };
        if r < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted { Ok(false) } else { Err(e) };
        }
        if r == 0 {
            return Ok(false);
        }
        self.wait().map(|_| true)
    }

    /// Blocks until the timer fires and returns the number of expirations (1 unless a cycle was missed).
    pub fn wait(&self) -> io::Result<u64> {
        let mut n = 0u64;
        loop {
            // SAFETY: reads exactly 8 bytes into `n`.
            let r = unsafe { libc::read(self.fd, &mut n as *mut u64 as *mut libc::c_void, 8) };
            if r == 8 {
                return Ok(n);
            }
            let e = io::Error::last_os_error();
            if r < 0 && e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
    }
}

impl Drop for CycleTimer {
    fn drop(&mut self) {
        crate::sys::close_fd(self.fd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_deadlines_move_by_the_clock_offset() {
        // mono reads 1000 ahead of raw: a raw deadline of 5000 is mono 6000
        assert_eq!(raw_to_mono(5_000, 10_000, 11_000), 6_000);
        // mono behind raw
        assert_eq!(raw_to_mono(5_000, 10_000, 9_000), 4_000);
        // never zero (zero disarms a timerfd)
        assert_eq!(raw_to_mono(0, 10_000, 1_000), 1);
    }

    #[test]
    fn wake_is_the_cycle_start_minus_the_margin() {
        assert_eq!(wake_time(1_000_000, 200_000), 800_000);
        assert_eq!(wake_time(100, 200), 0);
    }

    #[test]
    fn fires_at_an_absolute_deadline() {
        let t = CycleTimer::new().unwrap();
        let start = now_raw_ns();
        t.arm_raw(start + 5_000_000).unwrap();
        assert_eq!(t.wait().unwrap(), 1);
        let waited = now_raw_ns() - start;
        assert!(waited >= 4_900_000, "{waited}");
        assert!(waited < 500_000_000, "{waited}");
    }

    #[test]
    fn a_bounded_wait_returns_when_the_timer_is_far_away() {
        let t = CycleTimer::new().unwrap();
        t.arm_mono(now_mono_ns() + 10_000_000_000).unwrap();
        let t0 = std::time::Instant::now();
        assert!(!t.wait_timeout(5_000_000).unwrap());
        assert!(t0.elapsed() < std::time::Duration::from_millis(500));
    }

    #[test]
    fn a_deadline_in_the_past_fires_at_once() {
        let t = CycleTimer::new().unwrap();
        t.arm_mono(1).unwrap();
        assert_eq!(t.wait().unwrap(), 1);
    }
}

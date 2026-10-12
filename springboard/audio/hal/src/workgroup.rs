//! The device workgroup (owner: task 06): joining sets SCHED_FIFO through the server (`Request::PromoteThread`),
//! `sched_util_min` while the device runs, and CPU affinity hints; leaving restores the thread.
//!
//! Clients never need CAP_SYS_NICE: the server grants SCHED_FIFO 80 after checking the thread with a pidfd. The
//! uclamp and affinity calls are on the thread's own attributes and need no privilege (raising util_min is allowed
//! within the process's `RLIMIT`-free default; if the kernel refuses, the join still succeeds).

use bifrost_audio_types::AudioError;
use std::io;

/// The frequency hint while the device runs (0..=1024, 1024 = full capacity).
pub const DEFAULT_UTIL_MIN: u32 = 512;

/// Who can ask the server to promote a thread (implemented by the control connection).
pub trait Promote {
    /// Asks for real-time priority for thread `tid` of this process.
    fn promote_thread(&self, tid: i32) -> Result<(), AudioError>;
}

/// `struct sched_attr` (linux/sched/types.h), SCHED_ATTR_SIZE_VER1.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;

/// The calling thread's kernel id.
pub fn current_tid() -> i32 {
    // SAFETY: gettid takes no arguments and cannot fail.
    unsafe { libc::syscall(libc::SYS_gettid) as i32 }
}

/// Sets the uclamp minimum of thread `tid` (0 clears the hint).
pub fn set_util_min(tid: i32, util_min: u32) -> io::Result<()> {
    let attr = SchedAttr {
        size: std::mem::size_of::<SchedAttr>() as u32,
        sched_flags: SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_UTIL_CLAMP_MIN,
        sched_util_min: util_min.min(1024),
        sched_util_max: 1024,
        ..SchedAttr::default()
    };
    // SAFETY: `attr` is a valid sched_attr of the size it declares, alive for the call.
    let r = unsafe { libc::syscall(libc::SYS_sched_setattr, tid, &attr as *const SchedAttr, 0u32) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Pins thread `tid` to `cpus` (an affinity hint; an empty list is refused).
pub fn set_affinity(tid: i32, cpus: &[usize]) -> io::Result<()> {
    if cpus.is_empty() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    // SAFETY: cpu_set_t is plain data; CPU_SET stays in bounds because indexes >= CPU_SETSIZE are skipped.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        let mut any = false;
        for &c in cpus {
            if c < libc::CPU_SETSIZE as usize {
                libc::CPU_SET(c, &mut set);
                any = true;
            }
        }
        if !any {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        if libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &set) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A thread's membership of the device workgroup; leaving (or dropping) clears the frequency hint.
#[derive(Debug)]
pub struct Membership {
    tid: i32,
    uclamp: bool,
    promoted: bool,
}

impl Membership {
    pub fn tid(&self) -> i32 {
        self.tid
    }

    /// Whether the server granted real-time priority.
    pub fn promoted(&self) -> bool {
        self.promoted
    }

    /// Whether the frequency hint is in effect.
    pub fn has_util_hint(&self) -> bool {
        self.uclamp
    }

    pub fn leave(self) {}
}

impl Drop for Membership {
    fn drop(&mut self) {
        if self.uclamp {
            let _ = set_util_min(self.tid, 0);
        }
    }
}

/// Joins the calling thread to the device workgroup: asks the server for SCHED_FIFO, then sets `util_min`.
/// A refused promotion is the error (the caller decides whether to run unpromoted); a refused uclamp is not.
pub fn join(server: &dyn Promote, util_min: u32) -> Result<Membership, AudioError> {
    let tid = current_tid();
    server.promote_thread(tid)?;
    let uclamp = set_util_min(tid, util_min).is_ok();
    Ok(Membership { tid, uclamp, promoted: true })
}

/// Joins without a promotion (the server denied it or is a test double): only the frequency hint.
pub fn join_unpromoted(util_min: u32) -> Membership {
    let tid = current_tid();
    Membership { tid, uclamp: set_util_min(tid, util_min).is_ok(), promoted: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fake {
        asked: Cell<i32>,
        deny: bool,
    }
    impl Promote for Fake {
        fn promote_thread(&self, tid: i32) -> Result<(), AudioError> {
            self.asked.set(tid);
            if self.deny {
                Err(AudioError::PermissionDenied)
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn sched_attr_matches_the_kernel_size() {
        // SCHED_ATTR_SIZE_VER1 = 56: 4+4+8+4+4+8+8+8+4+4
        assert_eq!(std::mem::size_of::<SchedAttr>(), 56);
        assert_eq!(std::mem::offset_of!(SchedAttr, sched_util_min), 48);
    }

    #[test]
    fn join_asks_the_server_for_this_thread() {
        let f = Fake { asked: Cell::new(0), deny: false };
        let m = join(&f, DEFAULT_UTIL_MIN).unwrap();
        assert_eq!(f.asked.get(), current_tid());
        assert!(m.promoted());
        assert_eq!(m.tid(), current_tid());
    }

    #[test]
    fn a_denied_promotion_is_the_error() {
        let f = Fake { asked: Cell::new(0), deny: true };
        assert_eq!(join(&f, 100).unwrap_err(), AudioError::PermissionDenied);
    }

    #[test]
    fn affinity_refuses_empty_and_out_of_range_lists() {
        assert!(set_affinity(current_tid(), &[]).is_err());
        assert!(set_affinity(current_tid(), &[100_000]).is_err());
    }

    #[test]
    fn util_min_on_a_missing_thread_fails() {
        assert!(set_util_min(i32::MAX, 10).is_err());
    }
}

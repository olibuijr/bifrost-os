//! Hog mode (docs/audio-architecture.md, "Hog mode, aggregates and taps"): `HOG_PID` per device, granted by the
//! policy process, released when the owner exits (watched through a pidfd). Owner: task 10.
//! Log category: `policy` (subsystem is.olibuijr.audio).

use bifrost_audio_types::{AudioError, ObjectId};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// The value of `HOG_PID` when the device is free.
pub const NOT_HOGGED: i32 = -1;

/// A pidfd for one process: becomes readable when the process exits.
pub struct PidFd(OwnedFd);

impl PidFd {
    pub fn open(pid: i32) -> Result<PidFd, AudioError> {
        // SAFETY: pidfd_open takes a pid and flags and returns a new fd or -1; no memory is passed.
        let r = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::c_long, 0 as libc::c_long) };
        if r < 0 {
            return Err(AudioError::Io);
        }
        // SAFETY: r is a fresh fd owned by nobody else.
        Ok(PidFd(unsafe { OwnedFd::from_raw_fd(r as RawFd) }))
    }

    /// The fd to add to the server's poll set (readable on exit).
    pub fn raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// True once the process has exited.
    pub fn exited(&self) -> bool {
        let mut p = libc::pollfd { fd: self.raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: p is a valid pollfd for the duration of the call; timeout 0 never blocks.
        let r = unsafe { libc::poll(&mut p, 1, 0) };
        r > 0 && p.revents & libc::POLLIN != 0
    }
}

struct Hog {
    pid: i32,
    pidfd: Option<PidFd>,
}

/// Who hogs which device.
#[derive(Default)]
pub struct HogTable {
    hogs: HashMap<ObjectId, Hog>,
}

impl HogTable {
    pub fn new() -> HogTable {
        HogTable::default()
    }

    /// `get(device, HOG_PID)`.
    pub fn get(&self, device: ObjectId) -> i32 {
        self.hogs.get(&device).map_or(NOT_HOGGED, |h| h.pid)
    }

    /// `set(device, HOG_PID, value)` by `caller_pid`. `granted` is the policy decision (the `audio-exclusive`
    /// permission or the user's confirmation); it is only consulted when taking the device.
    /// Taking requires `value == caller_pid` and a free device; releasing requires `value == -1` and ownership.
    /// `pidfd` is the owner's pidfd (None in host tests).
    pub fn set(&mut self, device: ObjectId, caller_pid: i32, value: i32, granted: bool, pidfd: Option<PidFd>) -> Result<(), AudioError> {
        if value == NOT_HOGGED {
            return match self.hogs.get(&device) {
                None => Ok(()),
                Some(h) if h.pid == caller_pid => {
                    self.hogs.remove(&device);
                    Ok(())
                }
                Some(_) => Err(AudioError::Hogged),
            };
        }
        if value != caller_pid {
            return Err(AudioError::BadValue);
        }
        match self.hogs.get(&device) {
            Some(h) if h.pid == caller_pid => return Ok(()),
            Some(_) => return Err(AudioError::Hogged),
            None => {}
        }
        if !granted {
            return Err(AudioError::PermissionDenied);
        }
        self.hogs.insert(device, Hog { pid: caller_pid, pidfd });
        Ok(())
    }

    /// May `pid` open or run on `device`? Other clients get `Hogged` and move to the next default device.
    pub fn check_access(&self, device: ObjectId, pid: i32) -> Result<(), AudioError> {
        match self.hogs.get(&device) {
            Some(h) if h.pid != pid => Err(AudioError::Hogged),
            _ => Ok(()),
        }
    }

    /// Releases every hog of `pid` (it exited); returns the freed devices so listeners can be notified.
    pub fn release_pid(&mut self, pid: i32) -> Vec<ObjectId> {
        let mut freed: Vec<ObjectId> = self.hogs.iter().filter(|(_, h)| h.pid == pid).map(|(d, _)| *d).collect();
        freed.sort_by_key(|d| d.0);
        for d in &freed {
            self.hogs.remove(d);
        }
        freed
    }

    /// Releases the hogs whose pidfd says the owner exited (call when the poll set reports it).
    pub fn reap(&mut self) -> Vec<ObjectId> {
        let mut freed: Vec<ObjectId> =
            self.hogs.iter().filter(|(_, h)| h.pidfd.as_ref().is_some_and(|f| f.exited())).map(|(d, _)| *d).collect();
        freed.sort_by_key(|d| d.0);
        for d in &freed {
            self.hogs.remove(d);
        }
        freed
    }

    /// Pidfds to poll.
    pub fn pidfds(&self) -> Vec<RawFd> {
        self.hogs.values().filter_map(|h| h.pidfd.as_ref().map(|f| f.raw_fd())).collect()
    }

    /// The device a client falls back to when its device is hogged: the first candidate not hogged by another pid.
    pub fn fallback(&self, candidates: &[ObjectId], pid: i32) -> Option<ObjectId> {
        candidates.iter().copied().find(|d| self.check_access(*d, pid).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const D1: ObjectId = ObjectId(10);
    const D2: ObjectId = ObjectId(11);

    #[test]
    fn take_and_release() {
        let mut t = HogTable::new();
        assert_eq!(t.get(D1), -1);
        t.set(D1, 100, 100, true, None).unwrap();
        assert_eq!(t.get(D1), 100);
        assert_eq!(t.set(D1, 100, -1, false, None), Ok(()));
        assert_eq!(t.get(D1), -1);
    }

    #[test]
    fn denied_without_grant_and_bad_value() {
        let mut t = HogTable::new();
        assert_eq!(t.set(D1, 100, 100, false, None), Err(AudioError::PermissionDenied));
        assert_eq!(t.set(D1, 100, 7, true, None), Err(AudioError::BadValue));
        assert_eq!(t.get(D1), -1);
    }

    #[test]
    fn hogged_device_refuses_others() {
        let mut t = HogTable::new();
        t.set(D1, 100, 100, true, None).unwrap();
        assert_eq!(t.check_access(D1, 200), Err(AudioError::Hogged));
        assert_eq!(t.check_access(D1, 100), Ok(()));
        assert_eq!(t.check_access(D2, 200), Ok(()));
        assert_eq!(t.set(D1, 200, 200, true, None), Err(AudioError::Hogged));
        assert_eq!(t.set(D1, 200, -1, true, None), Err(AudioError::Hogged));
        assert_eq!(t.fallback(&[D1, D2], 200), Some(D2));
        assert_eq!(t.fallback(&[D1, D2], 100), Some(D1));
    }

    #[test]
    fn exit_releases() {
        let mut t = HogTable::new();
        t.set(D1, 100, 100, true, None).unwrap();
        t.set(D2, 100, 100, true, None).unwrap();
        assert_eq!(t.release_pid(100), vec![D1, D2]);
        assert_eq!(t.get(D2), -1);
    }

    #[test]
    fn pidfd_reaps_exited_child() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        let fd = PidFd::open(pid).unwrap();
        let mut t = HogTable::new();
        t.set(D1, pid, pid, true, Some(fd)).unwrap();
        child.wait().unwrap();
        assert_eq!(t.reap(), vec![D1]);
        assert_eq!(t.get(D1), -1);
    }
}

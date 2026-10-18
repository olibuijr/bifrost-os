//! Byte-range locking: shared or exclusive locks on part of a file, for apps that share one file between processes
//! (a database, a document package) and need to guard a record rather than the whole file. The locks are Linux open
//! file description locks (`fcntl` `F_OFD_SETLK`): they belong to the open file, not the process, so two opens of the
//! same file contend even inside one process, a thread cannot lose another thread's lock by closing a copy of the
//! descriptor, and a lock goes away when the last descriptor of that open closes. They are advisory, as on the reference
//! platform: they bind the processes that ask.
//!
//! - [`try_lock`] takes a range or says it is busy; [`lock_timeout`] waits up to a time for it.
//! - A [`ByteRangeLock`] unlocks its range when dropped.
//! - [`conflict`] asks whether a range could be locked, without taking it.
//! - [`ByteRange::overlaps`] and [`LockKind::conflicts_with`] are the pure rules.

use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

extern "C" {
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}

const F_OFD_GETLK: i32 = 36;
const F_OFD_SETLK: i32 = 37;
const F_RDLCK: i16 = 0;
const F_WRLCK: i16 = 1;
const F_UNLCK: i16 = 2;
const SEEK_SET: i16 = 0;
const EAGAIN: i32 = 11;
const EACCES: i32 = 13;

/// How long [`lock_timeout`] sleeps between two tries.
const RETRY_EVERY: Duration = Duration::from_millis(5);

/// The C `struct flock` of 64-bit Linux (x86_64 and aarch64 share it).
#[repr(C)]
struct Flock {
    l_type: i16,
    l_whence: i16,
    l_start: i64,
    l_len: i64,
    l_pid: i32,
}

/// Shared locks coexist; an exclusive lock excludes every other lock on an overlapping range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockKind {
    Shared,
    Exclusive,
}

impl LockKind {
    /// Whether two locks of these kinds cannot both hold the same bytes.
    pub fn conflicts_with(self, other: LockKind) -> bool {
        self == LockKind::Exclusive || other == LockKind::Exclusive
    }

    fn raw(self) -> i16 {
        match self {
            LockKind::Shared => F_RDLCK,
            LockKind::Exclusive => F_WRLCK,
        }
    }

    fn from_raw(t: i16) -> Option<LockKind> {
        match t {
            F_RDLCK => Some(LockKind::Shared),
            F_WRLCK => Some(LockKind::Exclusive),
            _ => None,
        }
    }
}

/// A span of a file. A `len` of 0 means from `start` to the end of the file, however far it grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub len: u64,
}

impl ByteRange {
    pub fn new(start: u64, len: u64) -> ByteRange {
        ByteRange { start, len }
    }

    /// The whole file.
    pub fn whole() -> ByteRange {
        ByteRange { start: 0, len: 0 }
    }

    fn end(self) -> u128 {
        if self.len == 0 {
            u128::MAX
        } else {
            self.start as u128 + self.len as u128
        }
    }

    /// Whether the two spans share a byte.
    pub fn overlaps(self, other: ByteRange) -> bool {
        (self.start as u128) < other.end() && (other.start as u128) < self.end()
    }

    /// Whether the span can be passed to the kernel (its offsets fit a signed 64-bit offset).
    fn valid(self) -> bool {
        self.start <= i64::MAX as u64 && self.len <= i64::MAX as u64 && (self.len == 0 || self.end() <= i64::MAX as u128)
    }
}

#[derive(Debug)]
pub enum LockError {
    /// Another open file holds a conflicting lock.
    Busy,
    /// The wait ran out.
    Timeout,
    /// The range does not fit a file offset.
    BadRange,
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy => write!(f, "the range is locked by another opener"),
            LockError::Timeout => write!(f, "timed out waiting for the range lock"),
            LockError::BadRange => write!(f, "the byte range does not fit a file offset"),
            LockError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// A lock held on a byte range; unlocks the range when dropped.
#[derive(Debug)]
pub struct ByteRangeLock<'a> {
    file: &'a File,
    range: ByteRange,
    kind: LockKind,
}

impl ByteRangeLock<'_> {
    pub fn range(&self) -> ByteRange {
        self.range
    }

    pub fn kind(&self) -> LockKind {
        self.kind
    }
}

impl Drop for ByteRangeLock<'_> {
    fn drop(&mut self) {
        let mut fl = request(F_UNLCK, self.range);
        // SAFETY: fcntl on a descriptor the borrowed File owns, with a properly laid out flock.
        unsafe { fcntl(self.file.as_raw_fd(), F_OFD_SETLK, &mut fl as *mut Flock) };
    }
}

fn request(kind: i16, range: ByteRange) -> Flock {
    // l_pid must be 0 for open file description locks
    Flock { l_type: kind, l_whence: SEEK_SET, l_start: range.start as i64, l_len: range.len as i64, l_pid: 0 }
}

/// Takes `kind` on `range` of `file` if no other open file holds a conflicting lock, else [`LockError::Busy`].
/// Locking a range this open file already locks changes the lock to `kind`.
pub fn try_lock(file: &File, range: ByteRange, kind: LockKind) -> Result<ByteRangeLock<'_>, LockError> {
    if !range.valid() {
        return Err(LockError::BadRange);
    }
    let mut fl = request(kind.raw(), range);
    // SAFETY: fcntl on a descriptor owned by `file`, with a properly laid out flock.
    let rc = unsafe { fcntl(file.as_raw_fd(), F_OFD_SETLK, &mut fl as *mut Flock) };
    if rc == 0 {
        return Ok(ByteRangeLock { file, range, kind });
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(EAGAIN) | Some(EACCES) => Err(LockError::Busy),
        _ => Err(LockError::Io(err)),
    }
}

/// Like [`try_lock`], but waits up to `timeout` for the range to come free.
pub fn lock_timeout(file: &File, range: ByteRange, kind: LockKind, timeout: Duration) -> Result<ByteRangeLock<'_>, LockError> {
    let deadline = Instant::now() + timeout;
    loop {
        match try_lock(file, range, kind) {
            Err(LockError::Busy) => {
                if Instant::now() >= deadline {
                    return Err(LockError::Timeout);
                }
                std::thread::sleep(RETRY_EVERY);
            }
            other => return other,
        }
    }
}

/// What blocks a lock: the kind and the span of a lock held by another open file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub kind: LockKind,
    pub range: ByteRange,
}

/// Asks whether `kind` on `range` could be taken now. None when it could; otherwise the lock in the way.
pub fn conflict(file: &File, range: ByteRange, kind: LockKind) -> Result<Option<Conflict>, LockError> {
    if !range.valid() {
        return Err(LockError::BadRange);
    }
    let mut fl = request(kind.raw(), range);
    // SAFETY: fcntl on a descriptor owned by `file`, with a properly laid out flock.
    let rc = unsafe { fcntl(file.as_raw_fd(), F_OFD_GETLK, &mut fl as *mut Flock) };
    if rc != 0 {
        return Err(LockError::Io(std::io::Error::last_os_error()));
    }
    if fl.l_type == F_UNLCK {
        return Ok(None);
    }
    Ok(LockKind::from_raw(fl.l_type).map(|kind| Conflict { kind, range: ByteRange { start: fl.l_start as u64, len: fl.l_len as u64 } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (tempfile::TempDir, File, File) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let a = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        let b = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        (dir, a, b)
    }

    #[test]
    fn the_pure_rules_for_spans_and_kinds() {
        assert!(ByteRange::new(0, 10).overlaps(ByteRange::new(9, 5)));
        assert!(!ByteRange::new(0, 10).overlaps(ByteRange::new(10, 5)));
        assert!(ByteRange::new(100, 0).overlaps(ByteRange::new(1_000_000, 1)));
        assert!(!ByteRange::new(100, 0).overlaps(ByteRange::new(0, 100)));
        assert!(ByteRange::whole().overlaps(ByteRange::new(7, 1)));
        assert!(!LockKind::Shared.conflicts_with(LockKind::Shared));
        assert!(LockKind::Shared.conflicts_with(LockKind::Exclusive));
        assert!(LockKind::Exclusive.conflicts_with(LockKind::Exclusive));
    }

    #[test]
    fn an_exclusive_range_blocks_another_open_of_the_same_file() {
        let (_d, a, b) = pair();
        let held = try_lock(&a, ByteRange::new(0, 100), LockKind::Exclusive).unwrap();
        assert!(matches!(try_lock(&b, ByteRange::new(50, 10), LockKind::Shared), Err(LockError::Busy)));
        assert!(matches!(try_lock(&b, ByteRange::new(0, 1), LockKind::Exclusive), Err(LockError::Busy)));
        assert_eq!(held.kind(), LockKind::Exclusive);
        assert_eq!(held.range(), ByteRange::new(0, 100));
    }

    #[test]
    fn ranges_that_do_not_overlap_and_shared_locks_coexist() {
        let (_d, a, b) = pair();
        let _x = try_lock(&a, ByteRange::new(0, 100), LockKind::Exclusive).unwrap();
        let _y = try_lock(&b, ByteRange::new(100, 100), LockKind::Exclusive).unwrap();
        let (_d2, c, d) = pair();
        let _s1 = try_lock(&c, ByteRange::whole(), LockKind::Shared).unwrap();
        let _s2 = try_lock(&d, ByteRange::whole(), LockKind::Shared).unwrap();
    }

    #[test]
    fn dropping_the_lock_frees_the_range() {
        let (_d, a, b) = pair();
        let held = try_lock(&a, ByteRange::new(0, 10), LockKind::Exclusive).unwrap();
        assert!(try_lock(&b, ByteRange::new(0, 10), LockKind::Exclusive).is_err());
        drop(held);
        assert!(try_lock(&b, ByteRange::new(0, 10), LockKind::Exclusive).is_ok());
    }

    #[test]
    fn the_conflict_query_names_the_lock_in_the_way() {
        let (_d, a, b) = pair();
        assert_eq!(conflict(&b, ByteRange::new(0, 10), LockKind::Exclusive).unwrap(), None);
        let _held = try_lock(&a, ByteRange::new(20, 30), LockKind::Shared).unwrap();
        assert_eq!(conflict(&b, ByteRange::new(0, 100), LockKind::Exclusive).unwrap(), Some(Conflict { kind: LockKind::Shared, range: ByteRange::new(20, 30) }));
        assert_eq!(conflict(&b, ByteRange::new(0, 100), LockKind::Shared).unwrap(), None);
    }

    #[test]
    fn waiting_gets_the_range_when_it_frees_and_times_out_when_it_does_not() {
        let (_d, a, b) = pair();
        let held = try_lock(&a, ByteRange::new(0, 10), LockKind::Exclusive).unwrap();
        assert!(matches!(lock_timeout(&b, ByteRange::new(0, 10), LockKind::Exclusive, Duration::from_millis(30)), Err(LockError::Timeout)));
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(40));
                drop(held);
            });
            assert!(lock_timeout(&b, ByteRange::new(0, 10), LockKind::Exclusive, Duration::from_secs(5)).is_ok());
        });
    }

    #[test]
    fn a_range_past_the_offset_limit_is_refused() {
        let (_d, a, _b) = pair();
        assert!(matches!(try_lock(&a, ByteRange::new(u64::MAX, 1), LockKind::Shared), Err(LockError::BadRange)));
        assert!(matches!(conflict(&a, ByteRange::new(0, u64::MAX), LockKind::Shared), Err(LockError::BadRange)));
    }
}

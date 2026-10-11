//! `NativeFileCoordinator` / `NativeFilePresenter` parity (Foundation).
//!
//! `NativeFileCoordinator::coordinate_reading(path, f)` and `coordinate_writing(path, f)` run `f` while holding an
//! advisory `flock` on the item's parent directory (shared for reads, exclusive for writes), so two processes using
//! the coordinator never read a half-written item. The directory is locked, not the file, because writers replace
//! items by rename (a new inode would escape a file lock). Waits are bounded: after `LOCK_TIMEOUT` the call returns
//! `Err(CoordinationError::Timeout)` without running `f`.
//!
//! Presenters are registered per process (`add_presenter`) and are told about changes other processes make. The
//! change feed is the app's own `fswatch::WatchStream`: feed each `poll()` batch to `deliver_events`. Events for
//! items this process just wrote through `coordinate_writing` are dropped once, so a presenter does not hear its own
//! write (Apricot: the writer is excluded). Presenters run on the calling (main) thread.
//!
//! Gaps: no `relinquish`/`savePresentedItemChanges` negotiation, no `itemAtURL:willMoveToURL:` callbacks (a rename is
//! reported as `presented_item_did_change` on the old and new path), no ubiquitous-item or version coordination,
//! and locking is advisory (a process that does not coordinate is not blocked). The suppression of own writes is
//! per path and consumed by the first event, so a write that raises several coalesced events is heard once.

use std::cell::RefCell;
use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::fswatch::{WatchEvent, WatchEventFlags};

extern "C" {
    fn flock(fd: i32, op: i32) -> i32;
}
const LOCK_SH: i32 = 1;
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;

/// How long a coordination call waits for another process before giving up.
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum CoordinationError {
    Timeout,
    Io(std::io::Error),
}

impl From<std::io::Error> for CoordinationError {
    fn from(e: std::io::Error) -> Self {
        CoordinationError::Io(e)
    }
}

/// `NativeFilePresenter`: an object interested in one file or folder.
pub trait FilePresenter {
    /// `presentedItemURL`: the item (a file, or a folder whose contents count) being presented.
    fn presented_item_url(&self) -> PathBuf;
    /// `presentedItemDidChange`: another process changed the item (or, for a folder, something inside it).
    fn presented_item_did_change(&mut self, _path: &Path) {}
    /// `accommodatePresentedItemDeletion`: the item was removed or moved away.
    fn accommodate_deletion(&mut self, _path: &Path) {}
}

struct Entry {
    id: u64,
    presenter: Box<dyn FilePresenter>,
}

#[derive(Default)]
struct State {
    next: u64,
    presenters: Vec<Entry>,
    own_writes: HashSet<PathBuf>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Removes its presenter when dropped.
#[must_use = "dropping the token removes the presenter"]
pub struct PresenterToken {
    id: u64,
}

impl Drop for PresenterToken {
    fn drop(&mut self) {
        let id = self.id;
        STATE.with(|s| s.borrow_mut().presenters.retain(|e| e.id != id));
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeFileCoordinator;

impl NativeFileCoordinator {
    /// `NativeFileCoordinator.addFilePresenter`.
    pub fn add_presenter(presenter: Box<dyn FilePresenter>) -> PresenterToken {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.next += 1;
            let id = s.next;
            s.presenters.push(Entry { id, presenter });
            PresenterToken { id }
        })
    }

    /// `coordinate(readingItemAt:)`: runs `f` under a shared lock.
    pub fn coordinate_reading<R>(&self, path: &Path, f: impl FnOnce(&Path) -> R) -> Result<R, CoordinationError> {
        let _guard = lock(path, LOCK_SH)?;
        Ok(f(path))
    }

    /// `coordinate(writingItemAt:)`: runs `f` under an exclusive lock; presenters in this process are not told about
    /// the resulting change.
    pub fn coordinate_writing<R>(&self, path: &Path, f: impl FnOnce(&Path) -> R) -> Result<R, CoordinationError> {
        let _guard = lock(path, LOCK_EX)?;
        STATE.with(|s| s.borrow_mut().own_writes.insert(path.to_path_buf()));
        Ok(f(path))
    }
}

struct Guard(std::fs::File);

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: unlocking a descriptor we own; closing the file would release it anyway.
        unsafe { flock(self.0.as_raw_fd(), LOCK_UN) };
    }
}

fn lock(path: &Path, op: i32) -> Result<Guard, CoordinationError> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file = std::fs::File::open(dir)?;
    let start = Instant::now();
    loop {
        // SAFETY: flock on a descriptor we own, non-blocking so the wait is bounded.
        if unsafe { flock(file.as_raw_fd(), op | LOCK_NB) } == 0 {
            return Ok(Guard(file));
        }
        if start.elapsed() >= LOCK_TIMEOUT {
            return Err(CoordinationError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Hands a batch from `WatchStream::poll` to the registered presenters.
pub fn deliver_events(events: &[WatchEvent]) {
    for ev in events {
        let own = STATE.with(|s| s.borrow_mut().own_writes.remove(&ev.path));
        if own {
            continue;
        }
        // take the presenters out so a callback may add or drop presenters
        let mut taken: Vec<Entry> = STATE.with(|s| std::mem::take(&mut s.borrow_mut().presenters));
        for e in taken.iter_mut() {
            let url = e.presenter.presented_item_url();
            if ev.path != url && !ev.path.starts_with(&url) {
                continue;
            }
            if ev.flags.contains(WatchEventFlags::REMOVED) && ev.path == url {
                e.presenter.accommodate_deletion(&ev.path);
            } else {
                e.presenter.presented_item_did_change(&ev.path);
            }
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            let added = std::mem::take(&mut s.presenters);
            s.presenters = taken;
            s.presenters.extend(added);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    struct P {
        url: PathBuf,
        log: Rc<RefCell<Vec<String>>>,
    }
    impl FilePresenter for P {
        fn presented_item_url(&self) -> PathBuf {
            self.url.clone()
        }
        fn presented_item_did_change(&mut self, path: &Path) {
            self.log.borrow_mut().push(format!("change {}", path.file_name().unwrap().to_string_lossy()));
        }
        fn accommodate_deletion(&mut self, _: &Path) {
            self.log.borrow_mut().push("deleted".into());
        }
    }

    fn ev(path: &Path, flags: WatchEventFlags) -> WatchEvent {
        WatchEvent { path: path.to_path_buf(), flags }
    }

    #[test]
    fn presenters_hear_other_processes_not_own_writes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        let log = Rc::new(RefCell::new(Vec::new()));
        let _t = NativeFileCoordinator::add_presenter(Box::new(P { url: file.clone(), log: log.clone() }));
        NativeFileCoordinator.coordinate_writing(&file, |p| std::fs::write(p, b"x").unwrap()).unwrap();
        deliver_events(&[ev(&file, WatchEventFlags::MODIFIED)]);
        assert!(log.borrow().is_empty(), "own write is not reported");
        deliver_events(&[ev(&file, WatchEventFlags::MODIFIED), ev(&dir.path().join("other"), WatchEventFlags::MODIFIED)]);
        deliver_events(&[ev(&file, WatchEventFlags::REMOVED)]);
        assert_eq!(*log.borrow(), ["change a.txt", "deleted"]);
    }

    #[test]
    fn dropping_the_token_removes_the_presenter() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b");
        let log = Rc::new(RefCell::new(Vec::new()));
        drop(NativeFileCoordinator::add_presenter(Box::new(P { url: file.clone(), log: log.clone() })));
        deliver_events(&[ev(&file, WatchEventFlags::MODIFIED)]);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn writers_exclude_each_other_through_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c");
        // a second descriptor stands in for another process: flock locks belong to the open file description
        let held = lock(&file, LOCK_EX).unwrap();
        let other = std::fs::File::open(dir.path()).unwrap();
        assert_ne!(unsafe { flock(other.as_raw_fd(), LOCK_SH | LOCK_NB) }, 0, "reader is blocked by a writer");
        drop(held);
        assert_eq!(unsafe { flock(other.as_raw_fd(), LOCK_SH | LOCK_NB) }, 0, "free after release");
        assert!(NativeFileCoordinator.coordinate_reading(&file, |_| 7).is_ok_and(|v| v == 7), "shared locks coexist");
    }
}

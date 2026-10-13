//! Volume capacity: the total, free and available bytes of the filesystem that holds a path (`statvfs(3)`).
//!
//! `free` counts every unused block; `available` counts the blocks an unprivileged process may still write, which
//! is what a save or a download can use. Both are reported in bytes, from the fragment size when the system gives one
//! and the block size otherwise. The call reads filesystem metadata only: it opens nothing and changes nothing.
//! This is the one place that declares `statvfs`; the shell's disk space readings and the SDK both use it.

use std::ffi::{c_char, c_int, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `struct statvfs` for 64-bit glibc and bionic: eleven `unsigned long` fields, then spare space. The spare
/// tail is larger than glibc's own, so the kernel's write stays inside the buffer on either C library.
#[repr(C)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    _spare: [u64; 8],
}

extern "C" {
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
}

/// The space on the filesystem that holds a path, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    /// The size of the filesystem.
    pub total: u64,
    /// Every unused byte, including the reserve kept for privileged writes.
    pub free: u64,
    /// The bytes an unprivileged process can write now.
    pub available: u64,
}

impl Capacity {
    /// The bytes in use: `total - free`.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// The fraction of the filesystem in use, from 0.0 to 1.0. An empty filesystem reads as 0.0.
    pub fn used_fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.used() as f64 / self.total as f64
    }
}

/// Reads the capacity of the filesystem that holds `path`. A missing path, or a path with a NUL byte, is an error.
pub fn capacity(path: &Path) -> io::Result<Capacity> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    // SAFETY: `c` is NUL-terminated and lives across the call; `st` is a zeroed struct at least as large as the
    // platform's `struct statvfs`, so the kernel's write stays inside it.
    let mut st: StatVfs = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(from_fields(st.f_frsize, st.f_bsize, st.f_blocks, st.f_bfree, st.f_bavail))
}

/// Turns the block counts of a `statvfs` answer into bytes. The fragment size is the unit when it is set.
fn from_fields(frsize: u64, bsize: u64, blocks: u64, bfree: u64, bavail: u64) -> Capacity {
    let unit = if frsize > 0 { frsize } else { bsize };
    Capacity {
        total: blocks.saturating_mul(unit),
        free: bfree.saturating_mul(unit),
        available: bavail.saturating_mul(unit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fragment_size_is_the_unit_when_it_is_set() {
        let c = from_fields(4096, 1024, 100, 40, 30);
        assert_eq!(c, Capacity { total: 409_600, free: 163_840, available: 122_880 });
    }

    #[test]
    fn the_block_size_is_the_unit_when_there_is_no_fragment_size() {
        let c = from_fields(0, 512, 10, 4, 2);
        assert_eq!(c, Capacity { total: 5120, free: 2048, available: 1024 });
    }

    #[test]
    fn used_and_fraction_follow_the_free_count() {
        let c = Capacity { total: 200, free: 50, available: 40 };
        assert_eq!(c.used(), 150);
        assert!((c.used_fraction() - 0.75).abs() < 1e-9);
        assert_eq!(Capacity { total: 0, free: 0, available: 0 }.used_fraction(), 0.0);
    }

    #[test]
    fn the_root_filesystem_reports_a_consistent_capacity() {
        let c = capacity(Path::new("/")).unwrap();
        assert!(c.total > 0);
        assert!(c.available <= c.free);
        assert!(c.free <= c.total);
    }

    #[test]
    fn a_missing_path_and_a_path_with_nul_are_errors() {
        assert!(capacity(Path::new("/nonexistent/bifrost/volume")).is_err());
        assert_eq!(capacity(Path::new("a\0b")).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}

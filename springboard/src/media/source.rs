//! Byte sources for the demuxer: a file that may still be growing (the
//! torrent streamer writes pieces into it) or an in-memory buffer.
//!
//! A range that is not available yet is reported as `Error::NeedData`
//! rather than read as zeros, so playback can wait for it and tell the
//! streamer which bytes to fetch first. `FileSource` detects two kinds of
//! missing data: bytes past the current end of the file, and holes in a
//! sparse file (pieces written out of order), found with `SEEK_DATA` /
//! `SEEK_HOLE`. A streamer that preallocates with zeros instead of holes
//! should implement `Source` itself over its piece map.

use super::Error;
use std::io;

/// Random-access bytes that may become available over time.
pub trait Source: Send {
    /// Fills `buf` from `offset`; `NeedData` if any byte of the range is
    /// not available yet (nothing is read in that case), `Eof` if the
    /// range lies past the end of a complete source.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error>;
    /// The current length in bytes; may grow between calls.
    fn size(&mut self) -> u64;
}

/// A regular file, re-measured on every call so growth is seen.
pub struct FileSource {
    file: std::fs::File,
}

impl FileSource {
    pub fn open(path: &std::path::Path) -> io::Result<Self> {
        Ok(FileSource { file: std::fs::File::open(path)? })
    }

    /// The first unavailable byte in `offset..offset + len` on a sparse
    /// file, or None when the whole range holds data.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn hole_in(&self, offset: u64, len: u64) -> Option<u64> {
        use std::os::fd::AsRawFd;
        const SEEK_DATA: std::ffi::c_int = 3;
        const SEEK_HOLE: std::ffi::c_int = 4;
        extern "C" {
            fn lseek(fd: std::ffi::c_int, off: i64, whence: std::ffi::c_int) -> i64;
        }
        let fd = self.file.as_raw_fd();
        // SAFETY: plain lseek on a file descriptor we own; it touches no memory.
        let data = unsafe { lseek(fd, offset as i64, SEEK_DATA) };
        if data < 0 || data as u64 > offset {
            // starts inside a hole (or there is no data after it at all)
            return Some(offset);
        }
        // SAFETY: as above.
        let hole = unsafe { lseek(fd, offset as i64, SEEK_HOLE) };
        (hole >= 0 && (hole as u64) < offset + len).then_some(hole as u64)
    }

    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    fn hole_in(&self, _offset: u64, _len: u64) -> Option<u64> {
        None
    }
}

impl Source for FileSource {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        let len = buf.len() as u64;
        let end = self.size();
        if offset + len > end {
            let from = offset.max(end);
            return Err(Error::NeedData { offset: from, len: offset + len - from });
        }
        if let Some(hole) = self.hole_in(offset, len) {
            return Err(Error::NeedData { offset: hole, len: offset + len - hole });
        }
        use std::os::unix::fs::FileExt;
        self.file.read_exact_at(buf, offset).map_err(Error::Io)
    }

    fn size(&mut self) -> u64 {
        self.file.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

/// An in-memory source with an availability mask, so tests can play a file
/// that arrives piece by piece.
pub struct MemSource {
    pub data: Vec<u8>,
    /// bytes at or past this offset are "not downloaded yet"
    pub available: u64,
    /// true once the data can no longer grow
    pub complete: bool,
}

impl MemSource {
    pub fn new(data: Vec<u8>) -> Self {
        let available = data.len() as u64;
        MemSource { data, available, complete: true }
    }
}

impl Source for MemSource {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        let (len, total) = (buf.len() as u64, self.data.len() as u64);
        if offset + len > total && self.complete {
            return Err(Error::Eof);
        }
        if offset + len > self.available {
            let from = offset.max(self.available);
            return Err(Error::NeedData { offset: from, len: offset + len - from });
        }
        buf.copy_from_slice(&self.data[offset as usize..(offset + len) as usize]);
        Ok(())
    }

    fn size(&mut self) -> u64 {
        self.available.min(self.data.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_source_reports_the_missing_tail() {
        let mut s = MemSource { data: (0..100).collect(), available: 50, complete: false };
        let mut buf = [0u8; 10];
        s.read_at(40, &mut buf).unwrap();
        assert_eq!(buf[9], 49);
        assert!(matches!(s.read_at(45, &mut buf), Err(Error::NeedData { offset: 50, len: 5 })));
        assert!(matches!(s.read_at(70, &mut buf), Err(Error::NeedData { offset: 70, len: 10 })));
        s.available = 100;
        s.complete = true;
        assert!(matches!(s.read_at(95, &mut buf), Err(Error::Eof)));
    }

    #[test]
    fn file_source_sees_growth_and_holes() {
        use std::io::{Seek, Write};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("grow.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[1u8; 4096]).unwrap();
        let mut s = FileSource::open(&path).unwrap();
        let mut buf = [0u8; 8];
        s.read_at(4088, &mut buf).unwrap();
        assert_eq!(buf, [1; 8]);
        assert!(matches!(s.read_at(4092, &mut buf), Err(Error::NeedData { offset: 4096, len: 4 })));
        // a piece written far ahead leaves a hole in between
        f.seek(std::io::SeekFrom::Start(1 << 20)).unwrap();
        f.write_all(&[2u8; 4096]).unwrap();
        f.sync_all().unwrap();
        assert_eq!(s.size(), (1 << 20) + 4096);
        s.read_at(1 << 20, &mut buf).unwrap();
        assert_eq!(buf, [2; 8]);
        match s.read_at(4000, &mut [0u8; 200]) {
            // sparse-aware filesystems report the hole; others read zeros
            Err(Error::NeedData { offset, len }) => assert_eq!((offset, len), (4096, 104)),
            Ok(()) => {}
            Err(e) => panic!("{e:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

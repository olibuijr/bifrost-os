//! Dispatch I/O channels (`DispatchIO`): asynchronous reads and writes on a file or a pipe, delivered to handlers
//! on a queue in chunks.
//!
//! A stream channel (`DispatchIO.StreamType.stream`) reads and writes sequentially from the descriptor's current
//! position. A random-access channel (`.randomAccess`) takes an offset on each call and uses positioned I/O.
//! Handlers get `(done, data, error)`: a read calls the handler once per chunk with `done == false`, then once with
//! `done == true` (data empty at end of file, error 0 on success, an errno otherwise). A write hands back the data
//! not yet written (`None` when everything is written) with `done == true` at the end. `set_limit(high)` bounds the
//! chunk size (`setLimit(highWater:)`), `set_low_water` delays a handler until that many bytes are ready (or the
//! operation ends), and `close` makes later operations fail with `ECANCELED`.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::dispatch::DispatchQueue;
use crate::dispatch_data::DispatchData;

pub const ECANCELED: i32 = 125;

/// `DispatchIO.StreamType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamType {
    Stream,
    RandomAccess,
}

struct Inner {
    file: Mutex<File>,
    kind: StreamType,
    closed: AtomicBool,
    high: AtomicUsize,
    low: AtomicUsize,
}

#[derive(Clone)]
pub struct DispatchIO(Arc<Inner>);

impl DispatchIO {
    /// A channel over an open file or pipe end.
    pub fn new(kind: StreamType, file: File) -> DispatchIO {
        DispatchIO(Arc::new(Inner {
            file: Mutex::new(file),
            kind,
            closed: AtomicBool::new(false),
            high: AtomicUsize::new(64 * 1024),
            low: AtomicUsize::new(1),
        }))
    }

    pub fn stream_type(&self) -> StreamType {
        self.0.kind
    }

    /// `setLimit(highWater:)`: the largest chunk handed to a handler. At least 1.
    pub fn set_limit(&self, high_water: usize) {
        self.0.high.store(high_water.max(1), Ordering::SeqCst);
    }

    /// `setLimit(lowWater:)`: handlers wait until this many bytes are ready, except for the last chunk.
    pub fn set_low_water(&self, low_water: usize) {
        self.0.low.store(low_water.max(1), Ordering::SeqCst);
    }

    /// `close`: stops pending and future operations.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
    }

    /// `read(offset:length:queue:ioHandler:)`. `offset` applies to random-access channels only. `length` may be
    /// `usize::MAX` to read to the end.
    pub fn read<H>(&self, offset: u64, length: usize, queue: &DispatchQueue, mut handler: H)
    where
        H: FnMut(bool, DispatchData, i32) + Send + 'static,
    {
        let io = self.0.clone();
        queue.async_(move || {
            let mut pos = offset;
            let mut left = length;
            let mut pending: Vec<u8> = Vec::new();
            loop {
                if io.closed.load(Ordering::SeqCst) {
                    handler(true, DispatchData::from_vec(std::mem::take(&mut pending)), ECANCELED);
                    return;
                }
                let high = io.high.load(Ordering::SeqCst);
                let low = io.low.load(Ordering::SeqCst).min(high);
                let want = left.min(high - pending.len().min(high - 1));
                let mut buf = vec![0u8; want];
                let got = if want == 0 {
                    Ok(0)
                } else {
                    match io.kind {
                        StreamType::Stream => io.file.lock().unwrap().read(&mut buf),
                        StreamType::RandomAccess => io.file.lock().unwrap().read_at(&mut buf, pos),
                    }
                };
                match got {
                    Ok(0) => {
                        handler(true, DispatchData::from_vec(std::mem::take(&mut pending)), 0);
                        return;
                    }
                    Ok(n) => {
                        pos += n as u64;
                        left = left.saturating_sub(n);
                        pending.extend_from_slice(&buf[..n]);
                        if left == 0 {
                            handler(true, DispatchData::from_vec(std::mem::take(&mut pending)), 0);
                            return;
                        }
                        if pending.len() >= low {
                            handler(false, DispatchData::from_vec(std::mem::take(&mut pending)), 0);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        handler(true, DispatchData::from_vec(std::mem::take(&mut pending)), e.raw_os_error().unwrap_or(5));
                        return;
                    }
                }
            }
        });
    }

    /// `write(offset:data:queue:ioHandler:)`. Writes in chunks of at most the high-water mark; the handler gets the
    /// unwritten remainder each time (`None` once everything is out).
    pub fn write<H>(&self, offset: u64, data: DispatchData, queue: &DispatchQueue, mut handler: H)
    where
        H: FnMut(bool, Option<DispatchData>, i32) + Send + 'static,
    {
        let io = self.0.clone();
        queue.async_(move || {
            let bytes = data.to_vec();
            let mut done = 0usize;
            loop {
                let remaining = || {
                    if done < bytes.len() {
                        Some(DispatchData::from_slice(&bytes[done..]))
                    } else {
                        None
                    }
                };
                if io.closed.load(Ordering::SeqCst) {
                    handler(true, remaining(), ECANCELED);
                    return;
                }
                if done >= bytes.len() {
                    handler(true, None, 0);
                    return;
                }
                let end = bytes.len().min(done + io.high.load(Ordering::SeqCst));
                let chunk = &bytes[done..end];
                let res = match io.kind {
                    StreamType::Stream => io.file.lock().unwrap().write(chunk),
                    StreamType::RandomAccess => io.file.lock().unwrap().write_at(chunk, offset + done as u64),
                };
                match res {
                    Ok(n) => {
                        done += n;
                        if done < bytes.len() {
                            handler(false, remaining(), 0);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        handler(true, remaining(), e.raw_os_error().unwrap_or(5));
                        return;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::os::fd::FromRawFd;
    use std::sync::mpsc;
    use std::time::Duration;

    type Ev = (bool, Vec<u8>, i32);

    fn collect_read(io: &DispatchIO, offset: u64, len: usize) -> Vec<Ev> {
        let q = DispatchQueue::serial("io.test");
        let (tx, rx) = mpsc::channel();
        io.read(offset, len, &q, move |done, data, err| {
            let _ = tx.send((done, data.to_vec(), err));
        });
        let mut out = Vec::new();
        loop {
            let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let done = ev.0;
            out.push(ev);
            if done {
                return out;
            }
        }
    }

    fn file_with(bytes: &[u8]) -> (tempfile::TempDir, File) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, bytes).unwrap();
        (d, File::open(p).unwrap())
    }

    #[test]
    fn stream_read_arrives_in_chunks_then_done() {
        let (_d, f) = file_with(b"abcdefghij");
        let io = DispatchIO::new(StreamType::Stream, f);
        io.set_limit(4);
        let evs = collect_read(&io, 0, usize::MAX);
        let data: Vec<u8> = evs.iter().flat_map(|e| e.1.clone()).collect();
        assert_eq!(data, b"abcdefghij");
        assert!(evs.len() >= 3);
        assert!(evs.iter().all(|e| e.1.len() <= 4 && e.2 == 0));
        assert!(evs.last().unwrap().0 && evs[..evs.len() - 1].iter().all(|e| !e.0));
    }

    #[test]
    fn random_access_read_uses_offset_and_length() {
        let (_d, f) = file_with(b"0123456789");
        let io = DispatchIO::new(StreamType::RandomAccess, f);
        let evs = collect_read(&io, 3, 4);
        let data: Vec<u8> = evs.iter().flat_map(|e| e.1.clone()).collect();
        assert_eq!(data, b"3456");
        assert!(evs.last().unwrap().0);
    }

    #[test]
    fn low_water_batches_chunks() {
        let (_d, f) = file_with(&[7u8; 10]);
        let io = DispatchIO::new(StreamType::Stream, f);
        io.set_limit(10);
        io.set_low_water(10);
        let evs = collect_read(&io, 0, usize::MAX);
        let total: usize = evs.iter().map(|e| e.1.len()).sum();
        assert_eq!(total, 10);
        assert!(evs.iter().filter(|e| !e.1.is_empty()).count() <= 2);
    }

    #[test]
    fn closed_channel_reports_canceled() {
        let (_d, f) = file_with(b"abc");
        let io = DispatchIO::new(StreamType::Stream, f);
        io.close();
        let evs = collect_read(&io, 0, usize::MAX);
        assert_eq!(evs.last().unwrap().2, ECANCELED);
    }

    #[test]
    fn pipe_read_sees_writer_data_until_close() {
        let mut fds = [0i32; 2];
        extern "C" {
            fn pipe(fds: *mut i32) -> i32;
        }
        assert_eq!(unsafe { pipe(fds.as_mut_ptr()) }, 0);
        let rd = unsafe { File::from_raw_fd(fds[0]) };
        let mut wr = unsafe { File::from_raw_fd(fds[1]) };
        wr.write_all(b"hello pipe").unwrap();
        drop(wr);
        let io = DispatchIO::new(StreamType::Stream, rd);
        let evs = collect_read(&io, 0, usize::MAX);
        let data: Vec<u8> = evs.iter().flat_map(|e| e.1.clone()).collect();
        assert_eq!(data, b"hello pipe");
    }

    #[test]
    fn write_then_read_back_with_chunk_handlers() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("w");
        let f = std::fs::OpenOptions::new().read(true).write(true).create(true).open(&p).unwrap();
        let io = DispatchIO::new(StreamType::RandomAccess, f);
        io.set_limit(3);
        let q = DispatchQueue::serial("io.write");
        let (tx, rx) = mpsc::channel();
        io.write(2, DispatchData::from_slice(b"abcdefg"), &q, move |done, rest, err| {
            let _ = tx.send((done, rest.map(|r| r.len()), err));
        });
        let mut last = (false, None, 0);
        while !last.0 {
            last = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(last, (true, None, 0));
        assert_eq!(std::fs::read(&p).unwrap(), b"\0\0abcdefg");
    }
}

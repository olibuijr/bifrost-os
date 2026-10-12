//! `InputStream`, `OutputStream` and `Stream.Status` parity for in-memory byte streams: `InputStream::with_data` reads a
//! byte buffer, `OutputStream::to_memory` and `OutputStream::to_buffer` collect the bytes written to them.
//!
//! Every operation is synchronous and never blocks. There is no run-loop scheduling, no delegate and no file or socket
//! stream; those need the system layer. A read or write before `open()` fails with `StreamError::NotOpen`, and after
//! `close()` with `StreamError::Closed`.

/// `Stream.Status` in this in-memory model. `AtEnd` is reached by an input stream once every byte was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamStatus {
    NotOpen,
    Open,
    AtEnd,
    Closed,
}

/// The reason a read or write did not transfer anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    NotOpen,
    Closed,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::NotOpen => write!(f, "The stream is not open"),
            StreamError::Closed => write!(f, "The stream is closed"),
        }
    }
}

/// `InputStream` over a byte buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputStream {
    data: Vec<u8>,
    pos: usize,
    status: StreamStatus,
}

impl InputStream {
    /// `InputStream(data:)`.
    pub fn with_data(data: Vec<u8>) -> Self {
        InputStream { data, pos: 0, status: StreamStatus::NotOpen }
    }

    /// `open()`. Opening an open stream does nothing.
    pub fn open(&mut self) {
        if self.status == StreamStatus::NotOpen {
            self.status = StreamStatus::Open;
        }
    }

    /// `close()`.
    pub fn close(&mut self) {
        self.status = StreamStatus::Closed;
    }

    /// `status`.
    pub fn status(&self) -> StreamStatus {
        self.status
    }

    /// `hasBytesAvailable`: the stream is open and has unread bytes.
    pub fn has_bytes_available(&self) -> bool {
        self.status == StreamStatus::Open && self.pos < self.data.len()
    }

    /// Adds bytes that arrived after the stream was created, such as data from a device. A stream that had reached
    /// its end reads them on the next `read`.
    pub fn append(&mut self, more: &[u8]) {
        self.data.extend_from_slice(more);
        if self.status == StreamStatus::AtEnd && self.pos < self.data.len() {
            self.status = StreamStatus::Open;
        }
    }

    /// `read(_:maxLength:)`: copies up to `buf.len()` unread bytes into `buf` and returns how many. 0 means the end
    /// was reached. The status becomes `AtEnd` with the last byte.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        match self.status {
            StreamStatus::NotOpen => return Err(StreamError::NotOpen),
            StreamStatus::Closed => return Err(StreamError::Closed),
            StreamStatus::AtEnd => return Ok(0),
            StreamStatus::Open => {}
        }
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        if self.pos == self.data.len() {
            self.status = StreamStatus::AtEnd;
        }
        Ok(n)
    }
}

/// `OutputStream` that collects written bytes, unbounded (`toMemory`) or up to a capacity (`toBuffer`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputStream {
    buf: Vec<u8>,
    capacity: Option<usize>,
    status: StreamStatus,
}

impl OutputStream {
    /// `OutputStream(toMemory:)`: accepts every byte.
    pub fn to_memory() -> Self {
        OutputStream { buf: Vec::new(), capacity: None, status: StreamStatus::NotOpen }
    }

    /// `OutputStream(toBuffer:capacity:)`: accepts bytes until `capacity` are stored, then `write` returns 0.
    pub fn to_buffer(capacity: usize) -> Self {
        OutputStream { buf: Vec::new(), capacity: Some(capacity), status: StreamStatus::NotOpen }
    }

    /// `open()`. Opening an open stream does nothing.
    pub fn open(&mut self) {
        if self.status == StreamStatus::NotOpen {
            self.status = StreamStatus::Open;
        }
    }

    /// `close()`.
    pub fn close(&mut self) {
        self.status = StreamStatus::Closed;
    }

    /// `status`.
    pub fn status(&self) -> StreamStatus {
        self.status
    }

    /// `hasSpaceAvailable`: the stream is open and below its capacity.
    pub fn has_space_available(&self) -> bool {
        self.status == StreamStatus::Open && self.capacity.map_or(true, |c| self.buf.len() < c)
    }

    /// `write(_:maxLength:)`: stores as many leading bytes of `bytes` as fit and returns how many were stored.
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, StreamError> {
        match self.status {
            StreamStatus::NotOpen => return Err(StreamError::NotOpen),
            StreamStatus::Closed => return Err(StreamError::Closed),
            StreamStatus::Open | StreamStatus::AtEnd => {}
        }
        let room = match self.capacity {
            Some(c) => c.saturating_sub(self.buf.len()),
            None => bytes.len(),
        };
        let n = bytes.len().min(room);
        self.buf.extend_from_slice(&bytes[..n]);
        Ok(n)
    }

    /// `property(forKey: .dataWrittenToMemoryStreamKey)`: every byte stored so far.
    pub fn written(&self) -> &[u8] {
        &self.buf
    }

    /// Removes and returns the stored bytes, for a transport that sends them. Taking the bytes frees the capacity
    /// they used, as a sent buffer would.
    pub fn take_written(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_reads_in_chunks_then_reaches_the_end() {
        let mut s = InputStream::with_data(b"abcdef".to_vec());
        s.open();
        let mut first = [0u8; 4];
        assert_eq!(s.read(&mut first), Ok(4));
        assert_eq!(&first, b"abcd");
        assert_eq!(s.status(), StreamStatus::Open);
        let mut rest = [0u8; 4];
        assert_eq!(s.read(&mut rest), Ok(2));
        assert_eq!(&rest[..2], b"ef");
        assert_eq!(s.status(), StreamStatus::AtEnd);
        assert_eq!(s.read(&mut rest), Ok(0));
    }

    #[test]
    fn read_before_open_and_after_close_fail() {
        let mut s = InputStream::with_data(vec![1, 2]);
        let mut b = [0u8; 2];
        assert_eq!(s.read(&mut b), Err(StreamError::NotOpen));
        s.open();
        s.close();
        assert_eq!(s.read(&mut b), Err(StreamError::Closed));
        assert_eq!(s.status(), StreamStatus::Closed);
    }

    #[test]
    fn has_bytes_available_only_while_open_with_unread_bytes() {
        let mut s = InputStream::with_data(vec![7]);
        assert!(!s.has_bytes_available());
        s.open();
        assert!(s.has_bytes_available());
        let mut b = [0u8; 1];
        assert_eq!(s.read(&mut b), Ok(1));
        assert!(!s.has_bytes_available());
    }

    #[test]
    fn empty_input_is_at_end_on_the_first_read() {
        let mut s = InputStream::with_data(Vec::new());
        s.open();
        assert!(!s.has_bytes_available());
        let mut b = [0u8; 8];
        assert_eq!(s.read(&mut b), Ok(0));
        assert_eq!(s.status(), StreamStatus::AtEnd);
    }

    #[test]
    fn to_memory_collects_every_write_in_order() {
        let mut o = OutputStream::to_memory();
        assert_eq!(o.write(b"x"), Err(StreamError::NotOpen));
        o.open();
        assert_eq!(o.write(b"hello "), Ok(6));
        assert_eq!(o.write(b"world"), Ok(5));
        assert_eq!(o.written(), b"hello world");
    }

    #[test]
    fn to_buffer_stops_accepting_bytes_at_its_capacity() {
        let mut o = OutputStream::to_buffer(4);
        o.open();
        assert_eq!(o.write(b"abcdef"), Ok(4));
        assert!(!o.has_space_available());
        assert_eq!(o.write(b"g"), Ok(0));
        assert_eq!(o.written(), b"abcd");
    }

    #[test]
    fn closed_output_refuses_writes_and_reports_no_space() {
        let mut o = OutputStream::to_memory();
        o.open();
        assert!(o.has_space_available());
        o.close();
        assert_eq!(o.write(b"x"), Err(StreamError::Closed));
        assert!(!o.has_space_available());
        assert_eq!(o.status(), StreamStatus::Closed);
    }

    #[test]
    fn append_after_end_reopens_reading() {
        let mut s = InputStream::with_data(b"a".to_vec());
        s.open();
        let mut b = [0u8; 4];
        assert_eq!(s.read(&mut b), Ok(1));
        assert_eq!(s.status(), StreamStatus::AtEnd);
        s.append(b"bc");
        assert_eq!(s.status(), StreamStatus::Open);
        assert_eq!(s.read(&mut b), Ok(2));
        assert_eq!(&b[..2], b"bc");
    }
}

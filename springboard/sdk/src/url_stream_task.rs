//! `URLSessionStreamTask` parity: a TCP connection with buffered reads and writes (blocking; call from a background
//! thread). Plain TCP only; `startSecureConnection()` is not provided here, use `url::URLSession` for TLS.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Errors of a stream task.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamError {
    CannotConnect(String),
    TimedOut,
    Io(String),
    Closed,
}

/// `URLSessionStreamTask`.
pub struct URLSessionStreamTask {
    stream: TcpStream,
    buffer: Vec<u8>,
    at_eof: bool,
}

impl URLSessionStreamTask {
    /// `streamTask(withHostName:port:)` plus `resume()`: connects within `timeout`.
    pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<Self, StreamError> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| StreamError::CannotConnect(e.to_string()))?
            .next()
            .ok_or_else(|| StreamError::CannotConnect("no address".into()))?;
        let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| StreamError::CannotConnect(e.to_string()))?;
        Ok(URLSessionStreamTask { stream, buffer: Vec::new(), at_eof: false })
    }

    /// `write(_:timeout:)`.
    pub fn write(&mut self, data: &[u8], timeout: Duration) -> Result<(), StreamError> {
        self.stream.set_write_timeout(Some(timeout.max(Duration::from_millis(1)))).map_err(io)?;
        self.stream.write_all(data).and_then(|_| self.stream.flush()).map_err(io)
    }

    /// Writes `line` followed by `\r\n`.
    pub fn write_line(&mut self, line: &str, timeout: Duration) -> Result<(), StreamError> {
        let mut data = line.as_bytes().to_vec();
        data.extend_from_slice(b"\r\n");
        self.write(&data, timeout)
    }

    fn fill(&mut self, timeout: Duration) -> Result<usize, StreamError> {
        self.stream.set_read_timeout(Some(timeout.max(Duration::from_millis(1)))).map_err(io)?;
        let mut chunk = [0u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) => {
                self.at_eof = true;
                Ok(0)
            }
            Ok(n) => {
                self.buffer.extend_from_slice(&chunk[..n]);
                Ok(n)
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(StreamError::TimedOut),
            Err(e) => Err(io(e)),
        }
    }

    /// `readData(ofMinLength:maxLength:timeout:)`: returns at least `min` bytes (fewer only at end of stream) and at
    /// most `max`, with a flag that is true once the stream has ended and the buffer is empty.
    pub fn read_data(&mut self, min: usize, max: usize, timeout: Duration) -> Result<(Vec<u8>, bool), StreamError> {
        while self.buffer.len() < min.max(1) && !self.at_eof {
            self.fill(timeout)?;
        }
        let n = self.buffer.len().min(max);
        let out: Vec<u8> = self.buffer.drain(..n).collect();
        Ok((out, self.at_eof && self.buffer.is_empty()))
    }

    /// Reads one line without its `\n` or `\r\n`. `Ok(None)` at the end of the stream with nothing buffered.
    pub fn read_line(&mut self, timeout: Duration) -> Result<Option<String>, StreamError> {
        loop {
            if let Some(i) = self.buffer.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=i).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            if self.at_eof {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into_owned()));
            }
            self.fill(timeout)?;
        }
    }

    /// `closeWrite()`.
    pub fn close_write(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Write);
    }

    /// `closeRead()`.
    pub fn close_read(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Read);
    }

    /// `cancel()`.
    pub fn cancel(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

fn io(e: std::io::Error) -> StreamError {
    StreamError::Io(e.to_string())
}

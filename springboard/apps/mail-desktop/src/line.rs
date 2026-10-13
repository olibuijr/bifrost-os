//! Line and byte reading over a connection, shared by the IMAP and SMTP dialogues. A reader keeps the bytes it has
//! read ahead of the line it returns, so a line followed by a literal (IMAP) or by a reply continuation is never lost.

use std::io::{self, Read, Write};

/// Longest line a server may send before the dialogue is treated as broken (headers and bodies are read as literals).
pub const MAX_LINE: usize = 64 * 1024;

pub struct Lines<S: Read + Write> {
    io: S,
    buf: Vec<u8>,
}

impl<S: Read + Write> Lines<S> {
    pub fn new(io: S) -> Self {
        Lines { io, buf: Vec::new() }
    }

    /// The next line without its CRLF (a bare LF also ends a line). Lossy UTF-8.
    pub fn read_line(&mut self) -> io::Result<String> {
        loop {
            if let Some(at) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=at).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            if self.buf.len() > MAX_LINE {
                return Err(io::Error::other("server line too long"));
            }
            self.fill()?;
        }
    }

    /// Exactly `n` bytes, from the read-ahead first.
    pub fn read_bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        while self.buf.len() < n {
            self.fill()?;
        }
        Ok(self.buf.drain(..n).collect())
    }

    /// Writes all of `text` and flushes.
    pub fn send(&mut self, text: &str) -> io::Result<()> {
        self.io.write_all(text.as_bytes())?;
        self.io.flush()
    }

    /// The connection, for a STARTTLS upgrade. Bytes read ahead of the upgrade are dropped; a server must not send
    /// anything between its 220 and the TLS handshake, so there are none.
    pub fn into_inner(self) -> S {
        self.io
    }

    fn fill(&mut self) -> io::Result<()> {
        let mut chunk = [0u8; 8192];
        let n = self.io.read(&mut chunk)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the server closed the connection"));
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }
}

/// A scripted connection for the protocol tests: reads come from `input`, writes are kept in `output`.
#[cfg(test)]
pub mod script {
    use std::io::{self, Cursor, Read, Write};

    pub struct Script {
        pub input: Cursor<Vec<u8>>,
        pub output: Vec<u8>,
    }

    impl Script {
        pub fn new(input: &str) -> Self {
            Script { input: Cursor::new(input.as_bytes().to_vec()), output: Vec::new() }
        }

        pub fn sent(&self) -> String {
            String::from_utf8_lossy(&self.output).into_owned()
        }
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::script::Script;

    #[test]
    fn reads_crlf_and_bare_lf_lines() {
        let mut lines = Lines::new(Script::new("one\r\ntwo\nthree\r\n"));
        assert_eq!(lines.read_line().unwrap(), "one");
        assert_eq!(lines.read_line().unwrap(), "two");
        assert_eq!(lines.read_line().unwrap(), "three");
        assert!(lines.read_line().is_err(), "end of input is an error, not an empty line");
    }

    #[test]
    fn literal_bytes_come_after_the_line_they_belong_to() {
        let mut lines = Lines::new(Script::new("* 1 FETCH {5}\r\nhello) done\r\n"));
        assert_eq!(lines.read_line().unwrap(), "* 1 FETCH {5}");
        assert_eq!(lines.read_bytes(5).unwrap(), b"hello");
        assert_eq!(lines.read_line().unwrap(), ") done");
    }

    #[test]
    fn send_writes_the_text() {
        let mut lines = Lines::new(Script::new(""));
        lines.send("A001 NOOP\r\n").unwrap();
        assert_eq!(lines.into_inner().output, b"A001 NOOP\r\n");
    }
}

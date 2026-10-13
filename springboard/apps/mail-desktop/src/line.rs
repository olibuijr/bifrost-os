//! Line and byte reading over a connection, shared by the IMAP and SMTP dialogues. A reader keeps the bytes it has
//! read ahead of the line it returns, so a line followed by a literal (IMAP) or by a reply continuation is never lost.

use std::io::{self, Read, Write};

/// Longest line a server may send before the dialogue is treated as broken (headers and bodies are read as literals).
pub const MAX_LINE: usize = 64 * 1024;

/// Largest literal a server may send (a message body or header block read as `{n}` bytes). Anything bigger is refused
/// before a byte of it is buffered, so a broken or hostile server cannot make the app grow its buffer without bound.
pub const MAX_LITERAL: usize = 64 * 1024 * 1024;

/// Largest total of literal bytes one response line may carry. Each literal is capped by [`MAX_LITERAL`], but a response
/// can hold several, so without this cap a server could still make the app buffer far more than one literal's worth.
pub const MAX_RESPONSE: usize = 128 * 1024 * 1024;

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

    /// Exactly `n` bytes, from the read-ahead first. A count over [`MAX_LITERAL`] is an error and nothing is read.
    pub fn read_bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        check_literal(n)?;
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

/// Refuses a literal count over [`MAX_LITERAL`].
fn check_literal(n: usize) -> io::Result<()> {
    if n > MAX_LITERAL {
        return Err(io::Error::other("server literal too large"));
    }
    Ok(())
}

/// The literal bytes a response line holds once one more literal of `n` bytes is folded in; refused past [`MAX_RESPONSE`].
pub fn response_after(folded: usize, n: usize) -> io::Result<usize> {
    // one literal over its own cap is reported as that, before the response total
    check_literal(n)?;
    let total = folded.saturating_add(n);
    if total > MAX_RESPONSE {
        return Err(io::Error::other("server response too large"));
    }
    Ok(total)
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

    /// A connection that fails the test if anything reads from it.
    struct NoReads;

    impl Read for NoReads {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("an oversized literal must be refused before any read");
        }
    }

    impl Write for NoReads {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_literal_over_the_limit_is_refused_without_reading() {
        let mut lines = Lines::new(NoReads);
        let err = lines.read_bytes(MAX_LITERAL + 1).unwrap_err();
        assert_eq!(err.to_string(), "server literal too large");
    }

    #[test]
    fn a_literal_at_the_limit_is_accepted() {
        assert!(check_literal(MAX_LITERAL).is_ok());
        assert!(check_literal(MAX_LITERAL + 1).is_err());
        assert!(check_literal(usize::MAX).is_err());
    }

    #[test]
    fn the_literals_of_one_response_are_capped_in_total() {
        assert_eq!(response_after(0, MAX_LITERAL).unwrap(), MAX_LITERAL);
        assert_eq!(response_after(MAX_RESPONSE - 1, 1).unwrap(), MAX_RESPONSE);
        assert_eq!(response_after(MAX_RESPONSE, 1).unwrap_err().to_string(), "server response too large");
        assert!(response_after(usize::MAX, usize::MAX).is_err());
    }

    #[test]
    fn send_writes_the_text() {
        let mut lines = Lines::new(Script::new(""));
        lines.send("A001 NOOP\r\n").unwrap();
        assert_eq!(lines.into_inner().output, b"A001 NOOP\r\n");
    }
}

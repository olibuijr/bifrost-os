//! App-facing archives and compression: DEFLATE for byte buffers and strings,
//! and folder or single-file archives on the file system.
//!
//! The file archives are ZIP (the Files app format, `springboard::files::zip`), not
//! the archive format of the system reference SDK, so they open in the Files app and in other ZIP tools.
//! Byte and string compression is raw DEFLATE (RFC 1951) with no header, so the
//! caller stores the expected length or passes a `max_len` on the way back.
//! Decoding goes through the shell's inflate, which is bounded by `max_len`.
//! `ByteStream` is the in-memory buffer with a cursor that archive data is read from and
//! written to; `compress_to` and `decompress_from` work on it.
//!
//! ```ignore
//! let packed = archive::compress_text("hello");
//! let text = archive::decompress_text(&packed, 1 << 20)?;
//! let mut stream = archive::ByteStream::new();
//! archive::compress_to(&mut stream, b"hello")?;
//! stream.seek(std::io::SeekFrom::Start(0))?;
//! let bytes = archive::decompress_from(&mut stream, 1 << 20)?;
//! let zip = archive::compress_directory(&folder, &dest_dir, &mut |done, total| true)?;
//! let out = archive::expand(&zip, &dest_dir, &mut |done, total| true)?;
//! ```

use springboard::files::zip;
use springboard::shell::books::engine::inflate;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub use springboard::files::zip::Extracted;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Raw DEFLATE of `data`. An empty input gives the two-byte empty block `03 00`.
pub fn compress(data: &[u8]) -> Vec<u8> {
    zip::deflate(data)
}

/// Inflates raw DEFLATE data. Fails with `InvalidData` when the stream is damaged
/// or when the output would pass `max_len` bytes.
pub fn decompress(data: &[u8], max_len: usize) -> io::Result<Vec<u8>> {
    match inflate::inflate_limited(data, max_len) {
        Ok(out) => Ok(out),
        Err(inflate::Error::TooBig) => Err(invalid("Decompressed data is larger than the limit")),
        Err(_) => Err(invalid("Damaged compressed data")),
    }
}

/// Compresses the UTF-8 bytes of `text`.
pub fn compress_text(text: &str) -> Vec<u8> {
    compress(text.as_bytes())
}

/// Inflates data made by `compress_text` back into a string. Fails with
/// `InvalidData` when the decoded bytes are not UTF-8.
pub fn decompress_text(data: &[u8], max_len: usize) -> io::Result<String> {
    let bytes = decompress(data, max_len)?;
    String::from_utf8(bytes).map_err(|_| invalid("Decompressed data is not UTF-8 text"))
}

/// An in-memory byte buffer with a cursor, the stream that archive data is read from
/// and written to. It implements `Read`, `Write` and `Seek`; `cancel` makes every later
/// read, write and flush fail with `Interrupted`. Writing past the end fills the gap with zeros.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ByteStream {
    buf: Vec<u8>,
    pos: usize,
    cancelled: bool,
}

impl ByteStream {
    /// An empty stream with the cursor at 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// A stream holding `bytes`, with the cursor at 0.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        ByteStream { buf: bytes, pos: 0, cancelled: false }
    }

    /// Length of the buffer in bytes.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True when the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The cursor position.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The whole buffer, whatever the cursor.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Gives back the buffer.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Stops the stream. Later reads, writes and flushes fail with `Interrupted`.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Copies bytes from `offset` into `out` without moving the cursor. Returns the count
    /// copied, which is 0 at or past the end.
    pub fn read_at(&self, offset: usize, out: &mut [u8]) -> io::Result<usize> {
        self.check_open()?;
        if offset >= self.buf.len() {
            return Ok(0);
        }
        let n = out.len().min(self.buf.len() - offset);
        out[..n].copy_from_slice(&self.buf[offset..offset + n]);
        Ok(n)
    }

    /// Writes `data` at `offset` without moving the cursor, growing the buffer as needed.
    /// Returns `data.len()`.
    pub fn write_at(&mut self, offset: usize, data: &[u8]) -> io::Result<usize> {
        self.check_open()?;
        let end = offset.checked_add(data.len()).ok_or_else(|| invalid("Write past the largest stream"))?;
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.buf[offset..end].copy_from_slice(data);
        Ok(data.len())
    }

    fn check_open(&self) -> io::Result<()> {
        if self.cancelled {
            Err(io::Error::new(io::ErrorKind::Interrupted, "Stream cancelled"))
        } else {
            Ok(())
        }
    }
}

impl Read for ByteStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let at = self.pos;
        let n = self.read_at(at, out)?;
        self.pos += n;
        Ok(n)
    }
}

impl Write for ByteStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let at = self.pos;
        let n = self.write_at(at, data)?;
        self.pos += n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check_open()
    }
}

impl Seek for ByteStream {
    /// Moves the cursor. A target before the start is `InvalidInput`; a target past the end is
    /// allowed, and a later write fills the gap with zeros.
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.check_open()?;
        let target: Option<i64> = match to {
            SeekFrom::Start(n) => i64::try_from(n).ok(),
            SeekFrom::End(off) => (self.buf.len() as i64).checked_add(off),
            SeekFrom::Current(off) => (self.pos as i64).checked_add(off),
        };
        let target = target
            .filter(|t| *t >= 0)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Seek before the start of the stream"))?;
        self.pos = target as usize;
        Ok(target as u64)
    }
}

/// Writes the raw DEFLATE of `data` at the stream's cursor and moves the cursor past it.
/// Returns the number of bytes written.
pub fn compress_to(stream: &mut ByteStream, data: &[u8]) -> io::Result<usize> {
    let packed = compress(data);
    stream.write_all(&packed)?;
    Ok(packed.len())
}

/// Inflates everything from the cursor to the end of `stream` and moves the cursor to the end.
/// Fails like `decompress`.
pub fn decompress_from(stream: &mut ByteStream, max_len: usize) -> io::Result<Vec<u8>> {
    let mut packed = Vec::new();
    stream.read_to_end(&mut packed)?;
    decompress(&packed, max_len)
}

/// Zips one regular file into a new "<name>.zip" inside `dest_dir`. `progress(done, total)`
/// returns false to cancel. A missing path or a folder is `InvalidInput`.
pub fn compress_file(src: &Path, dest_dir: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<PathBuf> {
    if !src.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a file"));
    }
    zip::create(&[src.to_path_buf()], dest_dir, progress)
}

/// Zips a folder tree into a new "<name>.zip" inside `dest_dir`. A missing path or a
/// file is `InvalidInput`.
pub fn compress_directory(src: &Path, dest_dir: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<PathBuf> {
    if !src.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a folder"));
    }
    zip::create(&[src.to_path_buf()], dest_dir, progress)
}

/// Extracts `archive` into a new folder named after it inside `dest`. Member paths that
/// would leave that folder are skipped and listed in `Extracted::skipped`.
pub fn expand(archive: &Path, dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<Extracted> {
    zip::extract(archive, dest, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_compresses_to_the_empty_fixed_block() {
        // Final bit 1, fixed Huffman type 01, then the 7-bit end-of-block code 0000000:
        // bits 1,1,0,0000000 packed LSB first give 0x03, then 0x00.
        assert_eq!(compress(&[]), vec![0x03, 0x00]);
    }

    #[test]
    fn repeated_text_round_trips_within_the_limit() {
        let packed = compress(b"hello hello hello");
        assert_eq!(decompress(&packed, 64).unwrap(), b"hello hello hello".to_vec());
        // The back-reference makes the packed form shorter than the 17-byte input.
        assert!(packed.len() < 17);
    }

    #[test]
    fn output_larger_than_max_len_is_rejected() {
        let packed = compress(b"abcdef");
        let err = decompress(&packed, 5).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(decompress(&packed, 6).unwrap(), b"abcdef".to_vec());
    }

    #[test]
    fn stored_block_yields_its_literal_bytes() {
        // Header byte 0x01: final block, stored. LEN 0x0003 and NLEN 0xFFFC (!3), then "abc".
        let stored = [0x01, 0x03, 0x00, 0xFC, 0xFF, b'a', b'b', b'c'];
        assert_eq!(decompress(&stored, 16).unwrap(), b"abc".to_vec());
    }

    #[test]
    fn stored_block_with_a_bad_complement_length_is_damaged() {
        // LEN 3 but NLEN 0 (not the complement of 3), so the block is refused.
        let bad = [0x01, 0x03, 0x00, 0x00, 0x00, b'a'];
        let err = decompress(&bad, 16).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn text_round_trips_and_invalid_utf8_is_refused() {
        let packed = compress_text("Ísafjörður");
        assert_eq!(decompress_text(&packed, 64).unwrap(), "Ísafjörður");
        // A stored block holding the single byte 0xFF, which is not UTF-8.
        let not_text = [0x01, 0x01, 0x00, 0xFE, 0xFF, 0xFF];
        assert_eq!(decompress_text(&not_text, 16).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn compress_file_refuses_a_missing_path() {
        let missing = Path::new("/nonexistent-bifrost-archive-check/none.txt");
        let dest = Path::new("/nonexistent-bifrost-archive-check");
        let err = compress_file(missing, dest, &mut |_, _| true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn stream_write_seek_and_read_back() {
        let mut s = ByteStream::new();
        assert_eq!(s.write(b"abc").unwrap(), 3);
        assert_eq!(s.position(), 3);
        assert_eq!(s.seek(SeekFrom::Start(0)).unwrap(), 0);
        let mut out = [0u8; 8];
        assert_eq!(s.read(&mut out).unwrap(), 3);
        assert_eq!(&out[..3], b"abc");
        // The cursor sits at the end, so the next read copies nothing.
        assert_eq!(s.read(&mut out).unwrap(), 0);
    }

    #[test]
    fn write_past_the_end_fills_the_gap_with_zeros() {
        let mut s = ByteStream::new();
        assert_eq!(s.write_at(4, b"x").unwrap(), 1);
        assert_eq!(s.len(), 5);
        assert_eq!(s.as_bytes(), &[0, 0, 0, 0, b'x']);
        assert_eq!(s.position(), 0);
    }

    #[test]
    fn read_at_clips_to_the_end_and_keeps_the_cursor() {
        let s = ByteStream::from_bytes(b"hello".to_vec());
        let mut out = [0u8; 3];
        assert_eq!(s.read_at(3, &mut out).unwrap(), 2);
        assert_eq!(&out[..2], b"lo");
        assert_eq!(s.read_at(5, &mut out).unwrap(), 0);
        assert_eq!(s.position(), 0);
    }

    #[test]
    fn seek_before_the_start_is_invalid_input() {
        let mut s = ByteStream::from_bytes(b"ab".to_vec());
        assert_eq!(s.seek(SeekFrom::Current(-1)).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(s.seek(SeekFrom::End(-3)).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(s.position(), 0);
        assert_eq!(s.seek(SeekFrom::End(-2)).unwrap(), 0);
    }

    #[test]
    fn cancelled_stream_refuses_reads_writes_and_flushes() {
        let mut s = ByteStream::from_bytes(b"ab".to_vec());
        s.cancel();
        let mut out = [0u8; 2];
        assert_eq!(s.read(&mut out).unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(s.write(b"c").unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(s.flush().unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(s.into_bytes(), b"ab".to_vec());
    }

    #[test]
    fn empty_input_is_the_two_byte_block_in_a_stream() {
        let mut s = ByteStream::new();
        assert_eq!(compress_to(&mut s, &[]).unwrap(), 2);
        assert_eq!(s.as_bytes(), &[0x03, 0x00]);
        s.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(decompress_from(&mut s, 16).unwrap(), Vec::<u8>::new());
        assert_eq!(s.position(), 2);
    }

    #[test]
    fn stream_compress_and_decompress_round_trip_from_the_cursor() {
        let mut s = ByteStream::new();
        compress_to(&mut s, b"hello hello hello").unwrap();
        assert_eq!(s.position(), s.len());
        s.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(decompress_from(&mut s, 64).unwrap(), b"hello hello hello".to_vec());
        // A stored block read from a stream gives its literal bytes.
        let mut stored = ByteStream::from_bytes(vec![0x01, 0x03, 0x00, 0xFC, 0xFF, b'a', b'b', b'c']);
        assert_eq!(decompress_from(&mut stored, 16).unwrap(), b"abc".to_vec());
    }
}

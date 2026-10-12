//! App-facing archives and compression: DEFLATE for byte buffers and strings,
//! and folder or single-file archives on the file system.
//!
//! The file archives are ZIP (the Files app format, `springboard::files::zip`), not
//! the archive format of the system reference SDK, so they open in the Files app and in other ZIP tools.
//! Byte and string compression is raw DEFLATE (RFC 1951) with no header, so the
//! caller stores the expected length or passes a `max_len` on the way back.
//! Decoding goes through the shell's inflate, which is bounded by `max_len`.
//!
//! ```ignore
//! let packed = archive::compress_text("hello");
//! let text = archive::decompress_text(&packed, 1 << 20)?;
//! let zip = archive::compress_directory(&folder, &dest_dir, &mut |done, total| true)?;
//! let out = archive::expand(&zip, &dest_dir, &mut |done, total| true)?;
//! ```

use springboard::files::zip;
use springboard::shell::books::engine::inflate;
use std::io;
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
}

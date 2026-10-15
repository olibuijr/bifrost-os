//! Compression for apps: single-step buffers, multiple-step streams and input and output filters.
//!
//! This is the app-facing counterpart of the reference compression API (`compression_encode_buffer`,
//! `compression_decode_buffer`, `compression_stream_*`, `InputFilter`, `OutputFilter`, `FilterError`).
//! The codecs are DEFLATE (RFC 1951) and the zlib wrapper (RFC 1950). The encoder is greedy LZ77 with
//! dynamic Huffman blocks, chosen by a [`Level`] (the chain depth of the match search). It can also
//! split large inputs over several threads (`encode_buffer_threaded`). Algorithm cases this build has
//! no codec for (Brotli, LZ4, LZBITMAP, LZFSE, LZMA) are named so that app code keeps the reference
//! shape, and they fail with `Error::Unsupported`. `Algorithm::Deflate` (raw DEFLATE, no wrapper) is a
//! BifrOSt addition.
//!
//! Streams and filters buffer their input and run the one-shot codec when they are finalized, so no
//! output appears before `finalize`. Output is still delivered in chunks of the requested size.

mod dynamic;

use springboard::pngenc::adler32;
use bifrost_render::cg::pdf::inflate;
use std::fmt;

pub use dynamic::Level;

/// The compression algorithms, named after the reference API's cases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Algorithm {
    Brotli,
    Lz4,
    LzBitmap,
    Lzfse,
    Lzma,
    /// zlib framing (RFC 1950) around DEFLATE.
    Zlib,
    /// Raw DEFLATE (RFC 1951) with no wrapper. A BifrOSt addition.
    Deflate,
}

impl Algorithm {
    /// True for the algorithms this build can encode and decode.
    pub fn is_supported(self) -> bool {
        matches!(self, Algorithm::Zlib | Algorithm::Deflate)
    }
}

/// Whether a stream or filter compresses or decompresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOperation {
    Compress,
    Decompress,
}

/// Errors from the buffer and stream APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The algorithm has no codec in this build.
    Unsupported(Algorithm),
    /// The input is not a valid stream of the algorithm (bad zlib header, bad block, bad code or distance).
    InvalidData,
    /// The input ends before the stream does.
    Truncated,
    /// The output would pass the caller's limit.
    TooBig,
    /// The stream was already finalized.
    Finished,
}

impl From<inflate::Error> for Error {
    fn from(e: inflate::Error) -> Error {
        match e {
            inflate::Error::Truncated => Error::Truncated,
            inflate::Error::TooBig => Error::TooBig,
            _ => Error::InvalidData,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(a) => write!(f, "compression algorithm {:?} is not available", a),
            Error::InvalidData => write!(f, "invalid compressed data"),
            Error::Truncated => write!(f, "compressed data ends early"),
            Error::TooBig => write!(f, "decompressed data is larger than the limit"),
            Error::Finished => write!(f, "compression stream already finalized"),
        }
    }
}

/// Errors from the input and output filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterError {
    /// The data given to the filter is not valid for its operation.
    InvalidData,
    /// The filter could not start (unsupported algorithm), or it was used after it finished.
    InvalidState,
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterError::InvalidData => write!(f, "invalid data for the filter"),
            FilterError::InvalidState => write!(f, "filter is in an invalid state"),
        }
    }
}

/// Compresses `src` in one call at the default level. `Zlib` output is a zlib stream with an Adler-32 trailer.
pub fn encode_buffer(algorithm: Algorithm, src: &[u8]) -> Result<Vec<u8>, Error> {
    encode_buffer_threaded(algorithm, src, Level::Default, 1)
}

/// Compresses `src` in one call at `level`, using up to `threads` threads for large inputs. The
/// result decodes to `src` with `decode_buffer`; threads change the bytes, not the decoded data.
pub fn encode_buffer_threaded(algorithm: Algorithm, src: &[u8], level: Level, threads: usize) -> Result<Vec<u8>, Error> {
    match algorithm {
        Algorithm::Zlib => {
            let mut out = vec![0x78, zlib_level_flags(level)]; // 32K window; FLEVEL from the level
            out.extend(dynamic::encode_threaded(src, level, threads));
            out.extend(adler32(src).to_be_bytes());
            Ok(out)
        }
        Algorithm::Deflate => Ok(dynamic::encode_threaded(src, level, threads)),
        other => Err(Error::Unsupported(other)),
    }
}

/// The second zlib header byte for `level`: FLEVEL in the top two bits, then the check bits that make
/// the header a multiple of 31 (0x5e, 0x9c and 0xda for the fast, default and best levels).
fn zlib_level_flags(level: Level) -> u8 {
    match level {
        Level::Fast => 0x5e,
        Level::Default => 0x9c,
        Level::Best => 0xda,
    }
}

/// Decompresses `src` in one call, refusing output larger than `inflate::MAX_OUT`.
pub fn decode_buffer(algorithm: Algorithm, src: &[u8]) -> Result<Vec<u8>, Error> {
    decode_buffer_limited(algorithm, src, inflate::MAX_OUT)
}

/// Decompresses `src`, failing with `Error::TooBig` once the output would pass `limit` bytes.
pub fn decode_buffer_limited(algorithm: Algorithm, src: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    match algorithm {
        Algorithm::Zlib => {
            if src.len() < 2 {
                return Err(Error::Truncated);
            }
            let (cmf, flg) = (src[0], src[1]);
            let check = ((cmf as u16) << 8) | flg as u16;
            // method must be DEFLATE (8), the header checksum must divide by 31, and no preset dictionary
            if (cmf & 0x0f) != 8 || check % 31 != 0 || (flg & 0x20) != 0 {
                return Err(Error::InvalidData);
            }
            inflate::inflate_limited(&src[2..], limit).map_err(Error::from)
        }
        Algorithm::Deflate => inflate::inflate_limited(src, limit).map_err(Error::from),
        other => Err(Error::Unsupported(other)),
    }
}

fn run(op: FilterOperation, algorithm: Algorithm, input: &[u8]) -> Result<Vec<u8>, Error> {
    match op {
        FilterOperation::Compress => encode_buffer(algorithm, input),
        FilterOperation::Decompress => decode_buffer(algorithm, input),
    }
}

/// A multiple-step compression stream. Input is collected by `process` and the codec runs on
/// the whole input when `finalize` is true, so the output comes from the final call.
#[derive(Debug)]
pub struct Stream {
    op: FilterOperation,
    algorithm: Algorithm,
    pending: Vec<u8>,
    finished: bool,
}

impl Stream {
    /// Starts a stream. Fails with `Unsupported` when the algorithm has no codec here.
    pub fn init(op: FilterOperation, algorithm: Algorithm) -> Result<Stream, Error> {
        if !algorithm.is_supported() {
            return Err(Error::Unsupported(algorithm));
        }
        Ok(Stream { op, algorithm, pending: Vec::new(), finished: false })
    }

    /// Adds `src` to the stream. With `finalize` set, returns the whole result and closes the stream;
    /// otherwise returns an empty buffer.
    pub fn process(&mut self, src: &[u8], finalize: bool) -> Result<Vec<u8>, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        self.pending.extend_from_slice(src);
        if !finalize {
            return Ok(Vec::new());
        }
        self.finished = true;
        let input = std::mem::take(&mut self.pending);
        run(self.op, self.algorithm, &input)
    }
}

/// Reads processed data from a source. The source is called with the buffer capacity and returns
/// `None` at the end of its input. The first `read_data` drains the source and runs the codec.
pub struct InputFilter<F> {
    op: FilterOperation,
    algorithm: Algorithm,
    capacity: usize,
    source: F,
    loaded: bool,
    failure: Option<FilterError>,
    data: Vec<u8>,
    pos: usize,
}

impl<F: FnMut(usize) -> Option<Vec<u8>>> InputFilter<F> {
    /// Creates a filter reading from `reading_from`. Fails with `InvalidState` for an unsupported algorithm.
    pub fn new(op: FilterOperation, algorithm: Algorithm, buffer_capacity: usize, reading_from: F) -> Result<Self, FilterError> {
        if !algorithm.is_supported() {
            return Err(FilterError::InvalidState);
        }
        Ok(InputFilter {
            op,
            algorithm,
            capacity: buffer_capacity.max(1),
            source: reading_from,
            loaded: false,
            failure: None,
            data: Vec::new(),
            pos: 0,
        })
    }

    /// Returns up to `length` bytes of processed data. An empty result means the end of the output.
    pub fn read_data(&mut self, length: usize) -> Result<Vec<u8>, FilterError> {
        if !self.loaded {
            self.loaded = true;
            let mut input = Vec::new();
            while let Some(chunk) = (self.source)(self.capacity) {
                input.extend(chunk);
            }
            match run(self.op, self.algorithm, &input) {
                Ok(data) => self.data = data,
                Err(_) => self.failure = Some(FilterError::InvalidData),
            }
        }
        if let Some(e) = self.failure {
            return Err(e);
        }
        let end = self.pos.saturating_add(length).min(self.data.len());
        let out = self.data[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }
}

/// Writes data into a filter and passes the processed result to a sink. `write` collects the data,
/// and `finalize` runs the codec and delivers the output in chunks of the buffer capacity.
pub struct OutputFilter<F> {
    op: FilterOperation,
    algorithm: Algorithm,
    capacity: usize,
    sink: F,
    pending: Vec<u8>,
    finished: bool,
}

impl<F: FnMut(&[u8])> OutputFilter<F> {
    /// Creates a filter writing to `writing_to`. Fails with `InvalidState` for an unsupported algorithm.
    pub fn new(op: FilterOperation, algorithm: Algorithm, buffer_capacity: usize, writing_to: F) -> Result<Self, FilterError> {
        if !algorithm.is_supported() {
            return Err(FilterError::InvalidState);
        }
        Ok(OutputFilter {
            op,
            algorithm,
            capacity: buffer_capacity.max(1),
            sink: writing_to,
            pending: Vec::new(),
            finished: false,
        })
    }

    /// Adds data to the filter. Fails with `InvalidState` after `finalize`.
    pub fn write(&mut self, data: &[u8]) -> Result<(), FilterError> {
        if self.finished {
            return Err(FilterError::InvalidState);
        }
        self.pending.extend_from_slice(data);
        Ok(())
    }

    /// Runs the codec over everything written and sends the output to the sink in chunks.
    pub fn finalize(&mut self) -> Result<(), FilterError> {
        if self.finished {
            return Err(FilterError::InvalidState);
        }
        self.finished = true;
        let input = std::mem::take(&mut self.pending);
        let out = run(self.op, self.algorithm, &input).map_err(|_| FilterError::InvalidData)?;
        for chunk in out.chunks(self.capacity) {
            (self.sink)(chunk);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_zlib_stream_has_the_default_header_and_the_empty_adler32() {
        // The block is a dynamic one holding only end-of-block, so its bytes are not fixed;
        // the header and the trailer are.
        let enc = encode_buffer(Algorithm::Zlib, b"").unwrap();
        assert_eq!(&enc[..2], &[0x78, 0x9c]);
        assert_eq!(&enc[enc.len() - 4..], &[0x00, 0x00, 0x00, 0x01]);
        assert_eq!(decode_buffer(Algorithm::Zlib, &enc).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn zlib_header_check_bits_are_a_multiple_of_31_for_every_level() {
        for level in [Level::Fast, Level::Default, Level::Best] {
            let enc = encode_buffer_threaded(Algorithm::Zlib, b"header", level, 1).unwrap();
            let check = ((enc[0] as u16) << 8) | enc[1] as u16;
            assert_eq!(check % 31, 0);
            assert_eq!(decode_buffer(Algorithm::Zlib, &enc).unwrap(), b"header".to_vec());
        }
    }

    #[test]
    fn every_level_round_trips_through_the_codec() {
        let text: Vec<u8> = (0..4000u32).map(|i| (i * i % 251) as u8).collect();
        for level in [Level::Fast, Level::Default, Level::Best] {
            let enc = encode_buffer_threaded(Algorithm::Deflate, &text, level, 1).unwrap();
            assert_eq!(decode_buffer(Algorithm::Deflate, &enc).unwrap(), text);
        }
    }

    #[test]
    fn zlib_trailer_is_the_adler32_of_the_input() {
        // a = 1+97+98+99 = 295 = 0x127; b = 98+196+295 = 589 = 0x24d; adler = b<<16 | a
        let enc = encode_buffer(Algorithm::Zlib, b"abc").unwrap();
        assert_eq!(&enc[enc.len() - 4..], &[0x02, 0x4d, 0x01, 0x27]);
        assert_eq!(decode_buffer(Algorithm::Zlib, &enc).unwrap(), b"abc".to_vec());
    }

    #[test]
    fn round_trip_repeats_and_raw_deflate() {
        let text = b"hello hello hello hello hello";
        for alg in [Algorithm::Zlib, Algorithm::Deflate] {
            let enc = encode_buffer(alg, text).unwrap();
            assert_eq!(decode_buffer(alg, &enc).unwrap(), text.to_vec());
        }
        // a repeated run of 100 bytes shrinks to a literal plus one long match; the dynamic
        // block header is most of the output at this size
        let run = vec![7u8; 100];
        assert!(encode_buffer(Algorithm::Deflate, &run).unwrap().len() < 40);
    }

    #[test]
    fn bad_zlib_header_and_short_input_are_reported() {
        // 0x7800 % 31 == 30, so the header checksum is wrong
        assert_eq!(decode_buffer(Algorithm::Zlib, &[0x78, 0x00, 0x03, 0x00]), Err(Error::InvalidData));
        assert_eq!(decode_buffer(Algorithm::Zlib, &[0x78]), Err(Error::Truncated));
        // a stream with a preset dictionary (FDICT bit 0x20) is refused
        assert_eq!(decode_buffer(Algorithm::Zlib, &[0x78, 0xbb, 0x03, 0x00]), Err(Error::InvalidData));
    }

    #[test]
    fn output_limit_and_unsupported_algorithms() {
        let enc = encode_buffer(Algorithm::Deflate, &[0u8; 100]).unwrap();
        assert_eq!(decode_buffer_limited(Algorithm::Deflate, &enc, 10), Err(Error::TooBig));
        assert_eq!(decode_buffer_limited(Algorithm::Deflate, &enc, 100).unwrap(), vec![0u8; 100]);
        assert_eq!(encode_buffer(Algorithm::Lzfse, b"x"), Err(Error::Unsupported(Algorithm::Lzfse)));
        assert!(!Algorithm::Lz4.is_supported());
    }

    #[test]
    fn stream_returns_output_only_when_finalized() {
        let mut s = Stream::init(FilterOperation::Compress, Algorithm::Zlib).unwrap();
        assert_eq!(s.process(b"ab", false).unwrap(), Vec::<u8>::new());
        let out = s.process(b"c", true).unwrap();
        assert_eq!(out, encode_buffer(Algorithm::Zlib, b"abc").unwrap());
        assert_eq!(s.process(b"x", false), Err(Error::Finished));
        assert_eq!(Stream::init(FilterOperation::Decompress, Algorithm::Lzma).unwrap_err(), Error::Unsupported(Algorithm::Lzma));
    }

    #[test]
    fn input_filter_serves_the_output_in_requested_sizes() {
        let enc = encode_buffer(Algorithm::Zlib, b"hello").unwrap();
        let mut chunks = vec![enc[..4].to_vec(), enc[4..].to_vec()].into_iter();
        let mut f = InputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 4, move |_: usize| chunks.next()).unwrap();
        assert_eq!(f.read_data(3).unwrap(), b"hel".to_vec());
        assert_eq!(f.read_data(10).unwrap(), b"lo".to_vec());
        assert_eq!(f.read_data(1).unwrap(), Vec::<u8>::new());
        assert!(InputFilter::new(FilterOperation::Decompress, Algorithm::Brotli, 4, |_: usize| None).is_err());
    }

    #[test]
    fn output_filter_sends_chunks_of_the_buffer_capacity() {
        let mut got: Vec<Vec<u8>> = Vec::new();
        {
            let mut f = OutputFilter::new(FilterOperation::Compress, Algorithm::Zlib, 2, |c: &[u8]| got.push(c.to_vec())).unwrap();
            f.write(b"ab").unwrap();
            f.write(b"c").unwrap();
            f.finalize().unwrap();
            assert_eq!(f.write(b"z"), Err(FilterError::InvalidState));
        }
        let expected = encode_buffer(Algorithm::Zlib, b"abc").unwrap();
        assert!(got.iter().all(|c| c.len() <= 2));
        assert_eq!(got.concat(), expected);
    }
}

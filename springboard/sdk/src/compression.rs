//! Compression for apps: single-step buffers, multiple-step streams and input and output filters.
//!
//! This is the app-facing counterpart of the reference compression API (`compression_encode_buffer`,
//! `compression_decode_buffer`, `compression_stream_*`, `InputFilter`, `OutputFilter`, `FilterError`).
//! The encoder is greedy or lazy LZ77 with dynamic Huffman blocks, chosen by a [`Level`] (the chain depth
//! of the match search; Default and Best are lazy). A block is stored when that is smaller. Large inputs
//! can be split over several threads (`encode_buffer_threaded`). Algorithm cases this build has no codec
//! for (Brotli, LZ4, LZBITMAP, LZFSE, LZMA) are named so that app code keeps the reference shape, and they
//! fail with `Error::Unsupported`. `Algorithm::Deflate` (raw DEFLATE, no wrapper) is a BifrOSt addition.
//!
//! Streams and filters are incremental. The encoder emits each block as soon as it is full (65535 bytes of input), and
//! the decoder returns output as soon as each symbol completes (`inflate_stream`), so output appears
//! before `finalize`. Output is delivered in chunks of the requested size.

mod dynamic;
mod inflate_stream;

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

use dynamic::Encoder as DeflateEncoder;
use inflate_stream::Decoder;

/// ADLER-32 modulus (RFC 1950 section 8).
const ADLER_MOD: u64 = 65_521;

/// zlib framing around the incremental DEFLATE encoder: the two-byte header goes out with the first
/// output, and the Adler-32 trailer with `finish`. Raw DEFLATE has neither.
#[derive(Debug)]
struct Compressor {
    zlib: bool,
    header: Vec<u8>,
    deflate: DeflateEncoder,
    a: u64,
    b: u64,
}

impl Compressor {
    fn new(zlib: bool) -> Compressor {
        let header = if zlib { vec![0x78, 0x9c] } else { Vec::new() };
        Compressor { zlib, header, deflate: DeflateEncoder::new(Level::Default), a: 1, b: 0 }
    }

    /// Adds `src` and returns the output it completes, header first.
    fn write(&mut self, src: &[u8]) -> Vec<u8> {
        if self.zlib {
            for &x in src {
                self.a = (self.a + x as u64) % ADLER_MOD;
                self.b = (self.b + self.a) % ADLER_MOD;
            }
        }
        let mut out = std::mem::take(&mut self.header);
        out.extend(self.deflate.write(src));
        out
    }

    /// Closes the stream and returns the rest of the output, trailer last.
    fn finish(self) -> Vec<u8> {
        let mut out = self.header;
        out.extend(self.deflate.finish());
        if self.zlib {
            out.extend((((self.b << 16) | self.a) as u32).to_be_bytes());
        }
        out
    }
}

/// The codec under a stream or filter: a compressor, or a decoder for the same algorithm.
#[derive(Debug)]
enum Codec {
    Encode(Compressor),
    Decode(Decoder),
}

impl Codec {
    fn new(op: FilterOperation, algorithm: Algorithm) -> Codec {
        let zlib = algorithm == Algorithm::Zlib;
        match op {
            FilterOperation::Compress => Codec::Encode(Compressor::new(zlib)),
            FilterOperation::Decompress => Codec::Decode(Decoder::new(zlib, inflate::MAX_OUT)),
        }
    }

    /// Feeds `src` and returns the output it completes.
    fn write(&mut self, src: &[u8]) -> Result<Vec<u8>, Error> {
        match self {
            Codec::Encode(c) => Ok(c.write(src)),
            Codec::Decode(d) => d.feed(src).map_err(Error::from),
        }
    }

    /// Ends the input and returns the rest of the output. Fails when the input was not a whole stream.
    fn finish(self) -> Result<Vec<u8>, Error> {
        match self {
            Codec::Encode(c) => Ok(c.finish()),
            Codec::Decode(d) => d.finish().map_err(Error::from),
        }
    }
}

/// Sends `out` to `sink` in pieces of at most `capacity` bytes.
fn deliver<F: FnMut(&[u8])>(sink: &mut F, capacity: usize, out: &[u8]) {
    for chunk in out.chunks(capacity) {
        sink(chunk);
    }
}

/// A multiple-step compression stream. `process` takes input as it comes and returns the output that is
/// ready: compressed blocks as they fill, or decoded bytes as they complete. `finalize` ends the stream
/// and returns the rest. Calls after the stream is finalized fail with `Error::Finished`.
#[derive(Debug)]
pub struct Stream {
    codec: Option<Codec>,
}

impl Stream {
    /// Starts a stream. Fails with `Unsupported` when the algorithm has no codec here.
    pub fn init(op: FilterOperation, algorithm: Algorithm) -> Result<Stream, Error> {
        if !algorithm.is_supported() {
            return Err(Error::Unsupported(algorithm));
        }
        Ok(Stream { codec: Some(Codec::new(op, algorithm)) })
    }

    /// Adds `src` and returns the output it completes. With `finalize` set, also ends the stream and
    /// returns the rest of its output.
    pub fn process(&mut self, src: &[u8], finalize: bool) -> Result<Vec<u8>, Error> {
        let codec = self.codec.as_mut().ok_or(Error::Finished)?;
        let mut out = codec.write(src)?;
        if finalize {
            if let Some(codec) = self.codec.take() {
                out.extend(codec.finish()?);
            }
        }
        Ok(out)
    }
}

/// Reads processed data from a source. The source is called with the buffer capacity and returns `None`
/// at the end of its input. `read_data` asks for input one capacity-sized piece at a time, and only
/// until it has the bytes it was asked for, so the source is never read further ahead than the reader.
pub struct InputFilter<F> {
    codec: Option<Codec>,
    capacity: usize,
    source: F,
    /// Processed output not read yet: `pending[pos..]`.
    pending: Vec<u8>,
    pos: usize,
    failure: Option<FilterError>,
}

impl<F: FnMut(usize) -> Option<Vec<u8>>> InputFilter<F> {
    /// Creates a filter reading from `reading_from`. Fails with `InvalidState` for an unsupported algorithm.
    pub fn new(op: FilterOperation, algorithm: Algorithm, buffer_capacity: usize, reading_from: F) -> Result<Self, FilterError> {
        if !algorithm.is_supported() {
            return Err(FilterError::InvalidState);
        }
        Ok(InputFilter {
            codec: Some(Codec::new(op, algorithm)),
            capacity: buffer_capacity.max(1),
            source: reading_from,
            pending: Vec::new(),
            pos: 0,
            failure: None,
        })
    }

    /// Returns up to `length` bytes of processed data. An empty result means the end of the output.
    /// Bad input is reported once, on the read that reaches it, and every later read fails the same way.
    pub fn read_data(&mut self, length: usize) -> Result<Vec<u8>, FilterError> {
        if let Some(e) = self.failure {
            return Err(e);
        }
        while self.pending.len() - self.pos < length && self.codec.is_some() {
            match (self.source)(self.capacity) {
                Some(chunk) => {
                    let Some(codec) = self.codec.as_mut() else { break };
                    match codec.write(&chunk) {
                        Ok(out) => self.pending.extend(out),
                        Err(_) => return Err(self.fail()),
                    }
                }
                None => {
                    if let Some(codec) = self.codec.take() {
                        match codec.finish() {
                            Ok(out) => self.pending.extend(out),
                            Err(_) => return Err(self.fail()),
                        }
                    }
                }
            }
        }
        let end = self.pos.saturating_add(length).min(self.pending.len());
        let out = self.pending[self.pos..end].to_vec();
        self.pos = end;
        if self.pos == self.pending.len() {
            self.pending.clear();
            self.pos = 0;
        }
        Ok(out)
    }

    fn fail(&mut self) -> FilterError {
        self.failure = Some(FilterError::InvalidData);
        FilterError::InvalidData
    }
}

/// Writes data into a filter and passes the processed result to a sink. `write` sends the output that is
/// ready to the sink in chunks of the buffer capacity, and `finalize` sends the rest.
pub struct OutputFilter<F> {
    codec: Option<Codec>,
    capacity: usize,
    sink: F,
    failure: Option<FilterError>,
}

impl<F: FnMut(&[u8])> OutputFilter<F> {
    /// Creates a filter writing to `writing_to`. Fails with `InvalidState` for an unsupported algorithm.
    pub fn new(op: FilterOperation, algorithm: Algorithm, buffer_capacity: usize, writing_to: F) -> Result<Self, FilterError> {
        if !algorithm.is_supported() {
            return Err(FilterError::InvalidState);
        }
        Ok(OutputFilter { codec: Some(Codec::new(op, algorithm)), capacity: buffer_capacity.max(1), sink: writing_to, failure: None })
    }

    /// Adds data and sends the output it completes to the sink. Fails with `InvalidData` as soon as a
    /// decompressing filter sees bad input, and with `InvalidState` after `finalize`.
    pub fn write(&mut self, data: &[u8]) -> Result<(), FilterError> {
        if let Some(e) = self.failure {
            return Err(e);
        }
        let codec = self.codec.as_mut().ok_or(FilterError::InvalidState)?;
        match codec.write(data) {
            Ok(out) => {
                deliver(&mut self.sink, self.capacity, &out);
                Ok(())
            }
            Err(_) => {
                self.failure = Some(FilterError::InvalidData);
                Err(FilterError::InvalidData)
            }
        }
    }

    /// Ends the input and sends the rest of the output to the sink. Fails with `InvalidState` if the
    /// filter was already finalized.
    pub fn finalize(&mut self) -> Result<(), FilterError> {
        if let Some(e) = self.failure {
            return Err(e);
        }
        let codec = self.codec.take().ok_or(FilterError::InvalidState)?;
        let out = codec.finish().map_err(|_| FilterError::InvalidData)?;
        deliver(&mut self.sink, self.capacity, &out);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(n: usize) -> Vec<u8> {
        let mut s = 1u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

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
    fn stored_fallback_keeps_incompressible_input_within_five_bytes_per_block() {
        let data = lcg(200_000);
        let packed = encode_buffer(Algorithm::Deflate, &data).unwrap();
        assert!(packed.len() <= data.len() + 5 * data.len().div_ceil(65_535), "{} bytes for {}", packed.len(), data.len());
        assert_eq!(decode_buffer(Algorithm::Deflate, &packed).unwrap(), data);
    }

    #[test]
    fn lazy_and_stored_output_decodes_for_a_mixed_corpus() {
        let mut mixed = lcg(3000);
        mixed.extend(std::iter::repeat(b'a').take(5000));
        mixed.extend(b"abcabcabcabcxabcabcabcy".repeat(40));
        let corpus = [b"".to_vec(), b"aaaa".to_vec(), mixed, lcg(70_000)];
        for data in &corpus {
            for level in [Level::Fast, Level::Default, Level::Best] {
                let enc = encode_buffer_threaded(Algorithm::Zlib, data, level, 1).unwrap();
                assert_eq!(decode_buffer(Algorithm::Zlib, &enc).unwrap(), *data);
                assert_eq!(encode_buffer_threaded(Algorithm::Zlib, data, level, 1).unwrap(), enc);
            }
        }
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
    fn a_compress_stream_emits_full_blocks_before_finalize() {
        let data = lcg(200_000);
        let mut stream = Stream::init(FilterOperation::Compress, Algorithm::Zlib).unwrap();
        let mut out = stream.process(&data, false).unwrap();
        assert!(out.len() > 60_000, "full blocks are returned without finalize, got {} bytes", out.len());
        out.extend(stream.process(&[], true).unwrap());
        assert_eq!(out, encode_buffer(Algorithm::Zlib, &data).unwrap());
        assert_eq!(stream.process(b"x", false), Err(Error::Finished));
    }

    #[test]
    fn a_compress_stream_gives_the_one_shot_bytes_for_any_split() {
        let mut data = lcg(150_000);
        data.extend_from_slice(&b"split me, split me, ".repeat(3000));
        let whole = encode_buffer(Algorithm::Zlib, &data).unwrap();
        for piece in [1usize, 7, 4096, 65_535, 100_000] {
            let mut stream = Stream::init(FilterOperation::Compress, Algorithm::Zlib).unwrap();
            let mut out = Vec::new();
            for chunk in data.chunks(piece) {
                out.extend(stream.process(chunk, false).unwrap());
            }
            out.extend(stream.process(&[], true).unwrap());
            assert_eq!(out, whole, "piece size {}", piece);
        }
    }

    #[test]
    fn a_decompress_stream_returns_output_before_finalize() {
        let mut data = lcg(70_000);
        data.extend_from_slice(&b"decoded early, decoded early, ".repeat(500));
        let packed = encode_buffer(Algorithm::Zlib, &data).unwrap();
        let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Zlib).unwrap();
        let early = stream.process(&packed[..packed.len() / 2], false).unwrap();
        assert!(!early.is_empty());
        assert_eq!(&data[..early.len()], &early[..]);
        let mut out = early;
        out.extend(stream.process(&packed[packed.len() / 2..], true).unwrap());
        assert_eq!(out, data);
    }

    #[test]
    fn a_decompress_stream_takes_one_byte_at_a_time() {
        let data = b"one byte at a time ".repeat(100);
        let packed = encode_buffer(Algorithm::Deflate, &data).unwrap();
        let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Deflate).unwrap();
        let mut out = Vec::new();
        for (i, b) in packed.iter().enumerate() {
            out.extend(stream.process(&[*b], i + 1 == packed.len()).unwrap());
        }
        assert_eq!(out, data);
    }

    #[test]
    fn a_decompress_stream_reports_bad_input_as_soon_as_it_arrives() {
        let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Zlib).unwrap();
        assert_eq!(stream.process(&[0x79, 0x9c], false), Err(Error::InvalidData));
    }

    #[test]
    fn a_truncated_decompress_stream_fails_at_finalize_and_then_is_finished() {
        let packed = encode_buffer(Algorithm::Zlib, b"truncate this stream please").unwrap();
        let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Zlib).unwrap();
        assert_eq!(stream.process(&packed[..packed.len() - 6], true), Err(Error::Truncated));
        assert_eq!(stream.process(&[], true), Err(Error::Finished));
    }

    #[test]
    fn stream_errors_name_the_finished_state_and_unsupported_algorithms() {
        let mut s = Stream::init(FilterOperation::Compress, Algorithm::Zlib).unwrap();
        s.process(b"ab", true).unwrap();
        assert_eq!(s.process(b"c", false), Err(Error::Finished));
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
    fn input_filter_pulls_input_only_as_far_as_the_reader_needs() {
        let data = lcg(100_000);
        let packed = encode_buffer(Algorithm::Deflate, &data).unwrap();
        let mut calls = 0usize;
        let mut offset = 0usize;
        let mut f = InputFilter::new(FilterOperation::Decompress, Algorithm::Deflate, 1024, |cap: usize| {
            calls += 1;
            if offset >= packed.len() {
                return None;
            }
            let end = (offset + cap).min(packed.len());
            let piece = packed[offset..end].to_vec();
            offset = end;
            Some(piece)
        })
        .unwrap();
        assert_eq!(f.read_data(10).unwrap(), data[..10].to_vec());
        drop(f);
        assert!(calls <= 2, "{} source reads for 10 bytes of output", calls);
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

    #[test]
    fn output_filter_reports_bad_input_on_the_write_that_has_it() {
        let mut calls = 0;
        {
            let mut f = OutputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 8, |_: &[u8]| calls += 1).unwrap();
            assert_eq!(f.write(&[0xff, 0xff, 0x00]), Err(FilterError::InvalidData));
            assert_eq!(f.write(&[0x00]), Err(FilterError::InvalidData));
            assert_eq!(f.finalize(), Err(FilterError::InvalidData));
        }
        assert_eq!(calls, 0);
    }
}

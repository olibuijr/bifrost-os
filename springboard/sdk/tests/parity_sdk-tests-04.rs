//! Host tests for the SDK compression and archive modules (parity rows for Compression, Apple Archive
//! and Accelerate's compression feature). Everything runs in memory or in a temporary folder: no
//! display, device or network is used, and no clock is read.

use bifrost_app::archive::{self, ByteStream};
use bifrost_app::compression::{self, Algorithm, Error, FilterError, FilterOperation, InputFilter, Level, OutputFilter, Stream};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// `zlib.compress(b"hello")` as produced by the reference zlib at its default level.
const ZLIB_HELLO: [u8; 13] = [0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x06, 0x2c, 0x02, 0x15];
/// The raw DEFLATE body of `ZLIB_HELLO`, with no zlib header or trailer.
const DEFLATE_HELLO: [u8; 7] = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];

const LEVELS: [Level; 3] = [Level::Fast, Level::Default, Level::Best];

fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn write_file(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

// ------------------------------------------------------------------ compression: buffers

#[test]
fn zlib_reference_bytes_decode_to_hello() {
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &ZLIB_HELLO).unwrap(), b"hello".to_vec());
}

#[test]
fn raw_deflate_reference_bytes_decode_to_hello() {
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &DEFLATE_HELLO).unwrap(), b"hello".to_vec());
}

#[test]
fn zlib_trailer_is_the_adler32_of_the_input() {
    // adler32("hello") is 0x062c0215, so the last four bytes of the stream are fixed.
    let enc = compression::encode_buffer(Algorithm::Zlib, b"hello").unwrap();
    assert_eq!(&enc[enc.len() - 4..], &[0x06, 0x2c, 0x02, 0x15]);
}

#[test]
fn zlib_header_flevel_follows_the_level() {
    for (level, flag) in [(Level::Fast, 0x5e_u8), (Level::Default, 0x9c), (Level::Best, 0xda)] {
        let enc = compression::encode_buffer_threaded(Algorithm::Zlib, b"x", level, 1).unwrap();
        assert_eq!(enc[0], 0x78, "CMF is deflate with a 32K window");
        assert_eq!(enc[1], flag, "FLG for {level:?}");
        assert_eq!(((enc[0] as u16) << 8 | enc[1] as u16) % 31, 0);
    }
}

#[test]
fn every_level_and_algorithm_round_trips() {
    let inputs: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0x41],
        b"the quick brown fox jumps over the lazy dog, the quick brown fox".to_vec(),
        vec![7u8; 5000],
        pseudo_random(20_000, 1),
    ];
    for input in &inputs {
        for alg in [Algorithm::Zlib, Algorithm::Deflate] {
            for level in LEVELS {
                let enc = compression::encode_buffer_threaded(alg, input, level, 1).unwrap();
                assert_eq!(compression::decode_buffer(alg, &enc).unwrap(), *input, "{alg:?} {level:?} len {}", input.len());
            }
        }
    }
}

#[test]
fn repetitive_input_compresses_and_random_input_expands_little() {
    let repeated = b"abc".repeat(1000);
    let packed = compression::encode_buffer(Algorithm::Deflate, &repeated).unwrap();
    assert!(packed.len() < 100, "3000 repeated bytes packed to {} bytes", packed.len());

    let noise = pseudo_random(4096, 5);
    let packed = compression::encode_buffer(Algorithm::Deflate, &noise).unwrap();
    assert!(packed.len() < noise.len() + 256, "random input packed to {} bytes", packed.len());
}

#[test]
fn one_thread_gives_the_same_bytes_as_the_single_step_call() {
    let text = b"abc abc abc abc";
    for alg in [Algorithm::Zlib, Algorithm::Deflate] {
        assert_eq!(
            compression::encode_buffer_threaded(alg, text, Level::Default, 1).unwrap(),
            compression::encode_buffer(alg, text).unwrap()
        );
    }
}

#[test]
fn large_input_split_over_two_threads_is_deterministic_and_round_trips() {
    // Two split sizes (2 MiB) are needed before the encoder uses more than one piece.
    let data = pseudo_random(2 * 1024 * 1024 + 5, 9);
    let first = compression::encode_buffer_threaded(Algorithm::Deflate, &data, Level::Default, 2).unwrap();
    let second = compression::encode_buffer_threaded(Algorithm::Deflate, &data, Level::Default, 2).unwrap();
    assert_eq!(first, second);
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &first).unwrap(), data);

    let zlib = compression::encode_buffer_threaded(Algorithm::Zlib, &data, Level::Fast, 2).unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &zlib).unwrap(), data);
}

#[test]
fn stored_zlib_from_the_png_writer_decodes_through_the_codec() {
    // The PNG writer emits stored blocks only; the codec must still read them back. 100000 bytes
    // span two stored blocks (the writer caps a block at 65535 bytes).
    let data = pseudo_random(100_000, 3);
    let packed = springboard::pngenc::zlib_stored(&data);
    assert!(packed.len() > data.len(), "stored blocks add framing and do not compress");
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &packed).unwrap(), data);
}

#[test]
fn decode_limit_is_inclusive_and_refuses_one_byte_more() {
    assert_eq!(compression::decode_buffer_limited(Algorithm::Zlib, &ZLIB_HELLO, 5).unwrap(), b"hello".to_vec());
    assert_eq!(compression::decode_buffer_limited(Algorithm::Zlib, &ZLIB_HELLO, 4), Err(Error::TooBig));
}

#[test]
fn decode_rejects_a_reserved_block_type() {
    // Bit 0 (final) is 1 and bits 1-2 (block type) are 11, which DEFLATE reserves.
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &[0x07]), Err(Error::InvalidData));
}

#[test]
fn decode_reports_truncated_input() {
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &[0xcb, 0x48]), Err(Error::Truncated));
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x78]), Err(Error::Truncated));
}

#[test]
fn zlib_header_errors_are_invalid_data() {
    // compression method 9 instead of deflate (8)
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x79, 0x9c, 0xcb, 0x48]), Err(Error::InvalidData));
    // FDICT set (preset dictionary), with a header that is otherwise a multiple of 31
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x78, 0xbb, 0xcb, 0x48]), Err(Error::InvalidData));
    // header check bits do not divide by 31
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x78, 0x00, 0xcb, 0x48]), Err(Error::InvalidData));
}

#[test]
fn unsupported_algorithms_fail_in_every_entry_point() {
    for alg in [Algorithm::Brotli, Algorithm::Lz4, Algorithm::LzBitmap, Algorithm::Lzfse, Algorithm::Lzma] {
        assert!(!alg.is_supported(), "{alg:?}");
        assert_eq!(compression::encode_buffer(alg, b"x"), Err(Error::Unsupported(alg)));
        assert_eq!(compression::encode_buffer_threaded(alg, b"x", Level::Fast, 2), Err(Error::Unsupported(alg)));
        assert_eq!(compression::decode_buffer(alg, b"x"), Err(Error::Unsupported(alg)));
        assert_eq!(compression::decode_buffer_limited(alg, b"x", 8), Err(Error::Unsupported(alg)));
        assert_eq!(Stream::init(FilterOperation::Compress, alg).unwrap_err(), Error::Unsupported(alg));
        assert_eq!(InputFilter::new(FilterOperation::Decompress, alg, 4, |_: usize| None).err(), Some(FilterError::InvalidState));
        assert_eq!(OutputFilter::new(FilterOperation::Compress, alg, 4, |_: &[u8]| {}).err(), Some(FilterError::InvalidState));
    }
    assert!(Algorithm::Zlib.is_supported());
    assert!(Algorithm::Deflate.is_supported());
}

#[test]
fn error_messages_name_the_failure() {
    assert_eq!(Error::Unsupported(Algorithm::Lzfse).to_string(), "compression algorithm Lzfse is not available");
    assert_eq!(Error::InvalidData.to_string(), "invalid compressed data");
    assert_eq!(Error::Truncated.to_string(), "compressed data ends early");
    assert_eq!(Error::TooBig.to_string(), "decompressed data is larger than the limit");
    assert_eq!(Error::Finished.to_string(), "compression stream already finalized");
    assert_eq!(FilterError::InvalidData.to_string(), "invalid data for the filter");
    assert_eq!(FilterError::InvalidState.to_string(), "filter is in an invalid state");
}

// ------------------------------------------------------------------ compression: streams and filters

#[test]
fn compress_stream_returns_everything_on_the_finalizing_call() {
    let mut s = Stream::init(FilterOperation::Compress, Algorithm::Zlib).unwrap();
    assert_eq!(s.process(b"hel", false).unwrap(), Vec::<u8>::new());
    let out = s.process(b"lo", true).unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &out).unwrap(), b"hello".to_vec());
    assert_eq!(s.process(b"!", true), Err(Error::Finished));
    assert_eq!(s.process(b"!", false), Err(Error::Finished));
}

#[test]
fn decompress_stream_accepts_input_in_pieces() {
    let mut d = Stream::init(FilterOperation::Decompress, Algorithm::Zlib).unwrap();
    assert_eq!(d.process(&ZLIB_HELLO[..5], false).unwrap(), Vec::<u8>::new());
    assert_eq!(d.process(&ZLIB_HELLO[5..], true).unwrap(), b"hello".to_vec());
}

#[test]
fn decompress_stream_reports_bad_input_on_finalize() {
    let mut bad = Stream::init(FilterOperation::Decompress, Algorithm::Deflate).unwrap();
    assert_eq!(bad.process(&[0x07], true), Err(Error::InvalidData));
}

#[test]
fn input_filter_serves_decoded_data_in_requested_sizes() {
    let mut chunks = vec![ZLIB_HELLO[..4].to_vec(), ZLIB_HELLO[4..9].to_vec(), ZLIB_HELLO[9..].to_vec()].into_iter();
    let mut f = InputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 4, move |_: usize| chunks.next()).unwrap();
    assert_eq!(f.read_data(2).unwrap(), b"he".to_vec());
    assert_eq!(f.read_data(2).unwrap(), b"ll".to_vec());
    assert_eq!(f.read_data(100).unwrap(), b"o".to_vec());
    assert_eq!(f.read_data(1).unwrap(), Vec::<u8>::new());
}

#[test]
fn input_filter_failure_is_sticky() {
    let mut src = vec![vec![0x07u8]].into_iter();
    let mut f = InputFilter::new(FilterOperation::Decompress, Algorithm::Deflate, 8, move |_: usize| src.next()).unwrap();
    assert_eq!(f.read_data(4), Err(FilterError::InvalidData));
    assert_eq!(f.read_data(4), Err(FilterError::InvalidData));
}

#[test]
fn input_filter_compresses_to_a_stream_that_decodes() {
    let mut src = vec![b"hello".to_vec()].into_iter();
    let mut c = InputFilter::new(FilterOperation::Compress, Algorithm::Zlib, 16, move |_: usize| src.next()).unwrap();
    let packed = c.read_data(usize::MAX).unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &packed).unwrap(), b"hello".to_vec());
}

#[test]
fn output_filter_decodes_and_sends_chunks_of_the_capacity() {
    let mut got: Vec<Vec<u8>> = Vec::new();
    {
        let mut f = OutputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 3, |c: &[u8]| got.push(c.to_vec())).unwrap();
        f.write(&ZLIB_HELLO[..4]).unwrap();
        f.write(&ZLIB_HELLO[4..]).unwrap();
        f.finalize().unwrap();
        assert_eq!(f.finalize(), Err(FilterError::InvalidState));
        assert_eq!(f.write(b"x"), Err(FilterError::InvalidState));
    }
    assert_eq!(got.len(), 2, "five bytes in chunks of three");
    assert!(got.iter().all(|c| c.len() <= 3));
    assert_eq!(got.concat(), b"hello".to_vec());
}

#[test]
fn output_filter_compress_output_decodes_back() {
    let mut got: Vec<u8> = Vec::new();
    {
        let mut f = OutputFilter::new(FilterOperation::Compress, Algorithm::Deflate, 64, |c: &[u8]| got.extend_from_slice(c)).unwrap();
        f.write(b"abc").unwrap();
        f.finalize().unwrap();
    }
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &got).unwrap(), b"abc".to_vec());
}

#[test]
fn output_filter_leaves_the_sink_untouched_when_the_data_is_invalid() {
    let mut sunk = 0;
    {
        let mut f = OutputFilter::new(FilterOperation::Decompress, Algorithm::Deflate, 4, |_: &[u8]| sunk += 1).unwrap();
        f.write(&[0x07]).unwrap();
        assert_eq!(f.finalize(), Err(FilterError::InvalidData));
    }
    assert_eq!(sunk, 0);
}

// ------------------------------------------------------------------ archive: text and raw DEFLATE

#[test]
fn text_helpers_round_trip_and_match_the_codec() {
    let packed = archive::compress_text("Ísafjörður, 2026");
    assert_eq!(archive::decompress_text(&packed, 64).unwrap(), "Ísafjörður, 2026");
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &packed).unwrap(), "Ísafjörður, 2026".as_bytes());

    assert_eq!(archive::compress_text(""), vec![0x03, 0x00]);
    assert_eq!(archive::decompress_text(&[0x03, 0x00], 16).unwrap(), "");
}

#[test]
fn raw_deflate_helpers_read_the_codec_output() {
    assert_eq!(archive::decompress(&DEFLATE_HELLO, 16).unwrap(), b"hello".to_vec());
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &archive::compress(b"hello")).unwrap(), b"hello".to_vec());
}

#[test]
fn decompress_reports_the_limit_and_damage_in_its_messages() {
    let too_big = archive::decompress(&DEFLATE_HELLO, 4).unwrap_err();
    assert_eq!(too_big.kind(), io::ErrorKind::InvalidData);
    assert_eq!(too_big.to_string(), "Decompressed data is larger than the limit");

    let damaged = archive::decompress(&[0x07], 16).unwrap_err();
    assert_eq!(damaged.to_string(), "Damaged compressed data");
    let truncated = archive::decompress(&[0xcb, 0x48], 16).unwrap_err();
    assert_eq!(truncated.to_string(), "Damaged compressed data");
}

#[test]
fn decompress_text_refuses_bytes_that_are_not_utf8() {
    let packed = archive::compress(&[0xff]);
    let err = archive::decompress_text(&packed, 16).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "Decompressed data is not UTF-8 text");
}

// ------------------------------------------------------------------ archive: byte streams

#[test]
fn byte_stream_starts_empty_and_converts_back_to_bytes() {
    let s = ByteStream::new();
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert_eq!(s.position(), 0);
    assert_eq!(s, ByteStream::default());

    let s = ByteStream::from_bytes(b"abc".to_vec());
    assert_eq!(s.len(), 3);
    assert_eq!(s.as_bytes(), &b"abc"[..]);
    assert_eq!(s.into_bytes(), b"abc".to_vec());
}

#[test]
fn byte_stream_reads_from_the_cursor_after_a_seek() {
    let mut s = ByteStream::from_bytes(b"abcdef".to_vec());
    assert_eq!(s.seek(SeekFrom::End(0)).unwrap(), 6);
    assert_eq!(s.seek(SeekFrom::Start(2)).unwrap(), 2);
    let mut buf = [0u8; 3];
    assert_eq!(s.read(&mut buf).unwrap(), 3);
    assert_eq!(&buf, b"cde");
    assert_eq!(s.position(), 5);
}

#[test]
fn byte_stream_writes_past_the_end_with_zero_gap() {
    let mut s = ByteStream::from_bytes(b"abcdef".to_vec());
    s.seek(SeekFrom::End(0)).unwrap();
    assert_eq!(s.seek(SeekFrom::Current(3)).unwrap(), 9);
    assert_eq!(s.write(b"Z").unwrap(), 1);
    assert_eq!(s.as_bytes(), &b"abcdef\0\0\0Z"[..]);
    assert_eq!(s.position(), 10);
    let mut none = [0u8; 1];
    assert_eq!(s.read(&mut none).unwrap(), 0);
}

#[test]
fn byte_stream_refuses_an_offset_that_overflows() {
    let mut s = ByteStream::new();
    let err = s.write_at(usize::MAX, b"x").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "Write past the largest stream");
    assert!(s.is_empty());
}

#[test]
fn compress_to_writes_at_the_cursor_and_decompress_from_reads_to_the_end() {
    let mut s = ByteStream::new();
    let n = archive::compress_to(&mut s, b"hello world").unwrap();
    assert_eq!(n, s.len());
    assert_eq!(s.position(), n);
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, s.as_bytes()).unwrap(), b"hello world".to_vec());

    s.seek(SeekFrom::Start(0)).unwrap();
    assert_eq!(archive::decompress_from(&mut s, 64).unwrap(), b"hello world".to_vec());
    assert_eq!(s.position(), n);
}

#[test]
fn compress_to_threaded_with_one_thread_matches_the_single_step_bytes() {
    let small = b"abc abc abc".to_vec();
    let mut t = ByteStream::new();
    archive::compress_to_threaded(&mut t, &small, 1).unwrap();
    assert_eq!(t.as_bytes(), &compression::encode_buffer(Algorithm::Deflate, &small).unwrap()[..]);
}

#[test]
fn compress_to_threaded_matches_the_codec_and_round_trips_large_input() {
    let data = pseudo_random(2 * 1024 * 1024 + 5, 9);
    let mut s = ByteStream::new();
    let n = archive::compress_to_threaded(&mut s, &data, 2).unwrap();
    let expected = compression::encode_buffer_threaded(Algorithm::Deflate, &data, Level::Default, 2).unwrap();
    assert_eq!(n, expected.len());
    assert_eq!(s.as_bytes(), &expected[..]);

    s.seek(SeekFrom::Start(0)).unwrap();
    assert_eq!(archive::decompress_from(&mut s, data.len()).unwrap(), data);
}

// ------------------------------------------------------------------ archive: files and folders

#[test]
fn compress_file_archives_one_file_and_expands_it_again() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("note.txt");
    write_file(&src, "hello file\n");
    let out = root.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut last = (0u64, 0u64);
    let zip = archive::compress_file(&src, &out, &mut |done, total| {
        last = (done, total);
        true
    })
    .unwrap();
    assert_eq!(zip, out.join("note.txt.zip"));
    assert!(zip.is_file());
    assert_eq!(last, (11, 11), "progress reports the 11 bytes of the file");

    let restore = root.path().join("restore");
    fs::create_dir(&restore).unwrap();
    let got = archive::expand(&zip, &restore, &mut |_, _| true).unwrap();
    assert_eq!(got.dir, restore.join("note.txt"));
    assert_eq!(got.files, 1);
    assert!(got.skipped.is_empty());
    assert_eq!(fs::read_to_string(got.dir.join("note.txt")).unwrap(), "hello file\n");
}

#[test]
fn compress_file_refuses_a_folder_and_stops_when_progress_cancels() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("note.txt");
    write_file(&src, "hello file\n");
    let out = root.path().join("out");
    fs::create_dir(&out).unwrap();

    let err = archive::compress_file(root.path(), &out, &mut |_, _| true).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(err.to_string(), "Not a file");

    let cancelled = archive::compress_file(&src, &out, &mut |_, _| false).unwrap_err();
    assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn compress_directory_archives_the_tree_and_expands_it_again() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    write_file(&tree.join("a.txt"), "alpha");
    write_file(&tree.join("sub").join("b.txt"), "beta");
    let out = root.path().join("out");
    fs::create_dir(&out).unwrap();

    let zip = archive::compress_directory(&tree, &out, &mut |_, _| true).unwrap();
    assert_eq!(zip, out.join("tree.zip"));

    let restore = root.path().join("restore");
    fs::create_dir(&restore).unwrap();
    let got = archive::expand(&zip, &restore, &mut |_, _| true).unwrap();
    assert_eq!(got.dir, restore.join("tree"));
    assert_eq!(got.files, 2);
    assert!(got.skipped.is_empty());
    // Members keep the folder name they were zipped under, so the tree sits one level down.
    assert_eq!(fs::read_to_string(got.dir.join("tree").join("a.txt")).unwrap(), "alpha");
    assert_eq!(fs::read_to_string(got.dir.join("tree").join("sub").join("b.txt")).unwrap(), "beta");
}

#[test]
fn compress_directory_refuses_a_plain_file() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("note.txt");
    write_file(&src, "hello file\n");
    let err = archive::compress_directory(&src, root.path(), &mut |_, _| true).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(err.to_string(), "Not a folder");
}

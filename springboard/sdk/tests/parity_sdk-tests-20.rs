//! Parity sdk-tests-20: host tests for the compression and archive rows. Everything runs in memory or in a
//! temporary folder. No display, device, network or clock is used. The cases check wire bytes, error
//! values, limits, cursor rules and file round trips through the public `bifrost_app` API.

use bifrost_app::archive::{self, ByteStream};
use bifrost_app::compression::{self, Algorithm, Error, FilterError, FilterOperation, InputFilter, Level, OutputFilter, Stream};
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Raw DEFLATE of `b"hello"` with no zlib wrapper.
const DEFLATE_HELLO: [u8; 7] = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];
/// `zlib.compress(b"hello")` at the default level.
const ZLIB_HELLO: [u8; 13] = [0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x06, 0x2c, 0x02, 0x15];

const LEVELS: [Level; 3] = [Level::Fast, Level::Default, Level::Best];
const CODECS: [Algorithm; 2] = [Algorithm::Zlib, Algorithm::Deflate];
const UNSUPPORTED: [Algorithm; 5] = [Algorithm::Brotli, Algorithm::Lz4, Algorithm::LzBitmap, Algorithm::Lzfse, Algorithm::Lzma];

fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn sample_inputs() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"a".to_vec(),
        b"The quick brown fox jumps over the lazy dog. ".repeat(40),
        pseudo_random(5000, 7),
    ]
}

fn write_file(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

/// Finds the first file called `name` anywhere under `root`.
fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

// ------------------------------------------------------------------ compression: buffer encode

#[test]
fn zlib_header_second_byte_follows_the_level() {
    let data = b"level header";
    let fast = compression::encode_buffer_threaded(Algorithm::Zlib, data, Level::Fast, 1).unwrap();
    let default = compression::encode_buffer(Algorithm::Zlib, data).unwrap();
    let best = compression::encode_buffer_threaded(Algorithm::Zlib, data, Level::Best, 1).unwrap();
    assert_eq!(&fast[..2], &[0x78, 0x5e]);
    assert_eq!(&default[..2], &[0x78, 0x9c]);
    assert_eq!(&best[..2], &[0x78, 0xda]);
    for header in [[0x78u8, 0x5e], [0x78, 0x9c], [0x78, 0xda]] {
        assert_eq!((u16::from(header[0]) << 8 | u16::from(header[1])) % 31, 0, "a zlib header is a multiple of 31");
    }
}

#[test]
fn zlib_trailer_is_the_big_endian_adler32_of_the_input() {
    // Adler-32 of "Wikipedia" is 0x11E60398, the worked example in RFC 1950 literature.
    let packed = compression::encode_buffer(Algorithm::Zlib, b"Wikipedia").unwrap();
    assert_eq!(&packed[packed.len() - 4..], &[0x11, 0xe6, 0x03, 0x98]);
}

#[test]
fn deflate_output_has_no_zlib_wrapper() {
    let packed = compression::encode_buffer(Algorithm::Deflate, b"hello").unwrap();
    assert_ne!(packed[0], 0x78, "raw DEFLATE does not start with a zlib header");
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &packed).unwrap(), b"hello".to_vec());
}

#[test]
fn every_codec_and_level_round_trips_every_sample() {
    for algorithm in CODECS {
        for level in LEVELS {
            for data in sample_inputs() {
                let packed = compression::encode_buffer_threaded(algorithm, &data, level, 1).unwrap();
                let back = compression::decode_buffer(algorithm, &packed).unwrap();
                assert_eq!(back, data, "{:?} at {:?} for {} bytes", algorithm, level, data.len());
            }
        }
    }
}

#[test]
fn repetitive_text_compresses_far_below_its_size_at_every_level() {
    let text = b"BifrOSt ".repeat(4000);
    for level in LEVELS {
        let packed = compression::encode_buffer_threaded(Algorithm::Zlib, &text, level, 1).unwrap();
        assert!(packed.len() * 50 < text.len(), "{:?} gave {} bytes for {}", level, packed.len(), text.len());
    }
}

#[test]
fn one_thread_gives_the_same_bytes_as_the_single_step_call() {
    let data = pseudo_random(10_000, 3);
    let single = compression::encode_buffer(Algorithm::Deflate, &data).unwrap();
    let threaded = compression::encode_buffer_threaded(Algorithm::Deflate, &data, Level::Default, 1).unwrap();
    assert_eq!(single, threaded);
}

#[test]
fn threads_change_the_bytes_but_never_the_decoded_data() {
    let data = b"0123456789abcdef".repeat(4096);
    let packed = compression::encode_buffer_threaded(Algorithm::Zlib, &data, Level::Default, 4).unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &packed).unwrap(), data);
}

// ------------------------------------------------------------------ compression: buffer decode

#[test]
fn the_reference_zlib_bytes_decode_to_hello() {
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &ZLIB_HELLO).unwrap(), b"hello".to_vec());
}

#[test]
fn the_reference_raw_deflate_bytes_decode_to_hello() {
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &DEFLATE_HELLO).unwrap(), b"hello".to_vec());
}

#[test]
fn zlib_header_with_a_method_other_than_deflate_is_invalid() {
    // Compression method 9 in the low nibble of the first byte.
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x79, 0x9c, 0x00]), Err(Error::InvalidData));
}

#[test]
fn zlib_header_whose_check_bits_do_not_divide_by_31_is_invalid() {
    // 0x789d is 30877, which leaves a remainder of 1 when divided by 31.
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x78, 0x9d, 0x00]), Err(Error::InvalidData));
}

#[test]
fn zlib_header_that_asks_for_a_preset_dictionary_is_invalid() {
    // 0x78bb divides by 31 but sets the FDICT bit (0x20) in the second byte.
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x78, 0xbb, 0x00]), Err(Error::InvalidData));
}

#[test]
fn a_zlib_stream_shorter_than_its_header_is_truncated() {
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[0x78]), Err(Error::Truncated));
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &[]), Err(Error::Truncated));
}

#[test]
fn a_deflate_stream_cut_short_is_truncated() {
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &DEFLATE_HELLO[..2]), Err(Error::Truncated));
}

#[test]
fn the_decode_limit_is_inclusive_and_one_byte_less_refuses() {
    let data = vec![7u8; 100];
    for algorithm in CODECS {
        let packed = compression::encode_buffer(algorithm, &data).unwrap();
        assert_eq!(compression::decode_buffer_limited(algorithm, &packed, 100).unwrap(), data);
        assert_eq!(compression::decode_buffer_limited(algorithm, &packed, 99), Err(Error::TooBig));
    }
}

#[test]
fn decode_buffer_passes_the_limit_to_the_decoder() {
    let packed = compression::encode_buffer(Algorithm::Deflate, b"tiny").unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &packed).unwrap(), b"tiny".to_vec());
    assert_eq!(compression::decode_buffer_limited(Algorithm::Deflate, &packed, 3), Err(Error::TooBig));
}

// ------------------------------------------------------------------ algorithm selection

#[test]
fn only_zlib_and_raw_deflate_are_supported() {
    assert!(Algorithm::Zlib.is_supported());
    assert!(Algorithm::Deflate.is_supported());
    for algorithm in UNSUPPORTED {
        assert!(!algorithm.is_supported(), "{:?} has no codec in this build", algorithm);
    }
}

#[test]
fn unsupported_algorithms_are_named_in_every_buffer_and_stream_entry_point() {
    for algorithm in UNSUPPORTED {
        assert_eq!(compression::encode_buffer(algorithm, b"x"), Err(Error::Unsupported(algorithm)));
        assert_eq!(compression::decode_buffer(algorithm, b"x"), Err(Error::Unsupported(algorithm)));
        assert_eq!(Stream::init(FilterOperation::Compress, algorithm).err(), Some(Error::Unsupported(algorithm)));
    }
}

// ------------------------------------------------------------------ multiple-step streams

#[test]
fn a_compress_stream_returns_nothing_until_it_is_finalized() {
    let mut stream = Stream::init(FilterOperation::Compress, Algorithm::Zlib).unwrap();
    assert!(stream.process(b"hello ", false).unwrap().is_empty());
    let packed = stream.process(b"world", true).unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &packed).unwrap(), b"hello world".to_vec());
}

#[test]
fn a_decompress_stream_accepts_the_input_in_pieces() {
    let data = b"stream piece ".repeat(50);
    let packed = compression::encode_buffer(Algorithm::Zlib, &data).unwrap();
    let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Zlib).unwrap();
    for piece in packed[..packed.len() - 1].chunks(7) {
        assert!(stream.process(piece, false).unwrap().is_empty());
    }
    let out = stream.process(&packed[packed.len() - 1..], true).unwrap();
    assert_eq!(out, data);
}

#[test]
fn a_finalized_stream_refuses_more_input() {
    let mut stream = Stream::init(FilterOperation::Compress, Algorithm::Deflate).unwrap();
    stream.process(b"abc", true).unwrap();
    assert_eq!(stream.process(b"def", false), Err(Error::Finished));
    assert_eq!(stream.process(b"", true), Err(Error::Finished));
}

#[test]
fn a_decompress_stream_reports_bad_input_when_finalized() {
    let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Zlib).unwrap();
    assert!(stream.process(&[0x79, 0x9c], false).unwrap().is_empty());
    assert_eq!(stream.process(&[], true), Err(Error::InvalidData));
}

// ------------------------------------------------------------------ input and output filters

#[test]
fn an_input_filter_serves_the_decoded_data_in_requested_sizes() {
    let packed = compression::encode_buffer(Algorithm::Deflate, b"0123456789").unwrap();
    let mut pieces = vec![packed.clone()].into_iter();
    let mut filter = InputFilter::new(FilterOperation::Decompress, Algorithm::Deflate, 4, move |_: usize| pieces.next()).unwrap();
    assert_eq!(filter.read_data(4).unwrap(), b"0123".to_vec());
    assert_eq!(filter.read_data(4).unwrap(), b"4567".to_vec());
    assert_eq!(filter.read_data(100).unwrap(), b"89".to_vec());
    assert!(filter.read_data(100).unwrap().is_empty(), "an empty read is the end of the output");
}

#[test]
fn an_input_filter_drains_its_source_in_buffer_sized_requests() {
    let mut requested = Vec::new();
    let mut pieces = vec![b"ab".to_vec(), b"cd".to_vec()].into_iter();
    {
        let mut filter = InputFilter::new(FilterOperation::Compress, Algorithm::Deflate, 16, |n: usize| {
            requested.push(n);
            pieces.next()
        })
        .unwrap();
        let packed = filter.read_data(1 << 20).unwrap();
        assert_eq!(compression::decode_buffer(Algorithm::Deflate, &packed).unwrap(), b"abcd".to_vec());
    }
    assert!(requested.iter().all(|n| *n == 16), "the source is asked for the buffer capacity");
    assert!(requested.len() >= 3, "two pieces and the end of the source are each requested");
}

#[test]
fn an_input_filter_failure_is_sticky() {
    let mut filter = InputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 8, |_: usize| None::<Vec<u8>>).unwrap();
    assert_eq!(filter.read_data(1), Err(FilterError::InvalidData));
    assert_eq!(filter.read_data(1), Err(FilterError::InvalidData));
}

#[test]
fn an_input_filter_compresses_to_a_stream_that_decodes() {
    let data = pseudo_random(3000, 11);
    let mut pieces = vec![data[..1000].to_vec(), data[1000..].to_vec()].into_iter();
    let mut filter = InputFilter::new(FilterOperation::Compress, Algorithm::Zlib, 64, move |_: usize| pieces.next()).unwrap();
    let packed = filter.read_data(usize::MAX).unwrap();
    assert_eq!(compression::decode_buffer(Algorithm::Zlib, &packed).unwrap(), data);
}

#[test]
fn an_input_filter_refuses_an_unsupported_algorithm() {
    let filter = InputFilter::new(FilterOperation::Decompress, Algorithm::Lzfse, 8, |_: usize| None::<Vec<u8>>);
    assert_eq!(filter.err(), Some(FilterError::InvalidState));
}

#[test]
fn an_output_filter_delivers_the_result_in_chunks_of_the_capacity() {
    let data = pseudo_random(100, 5);
    let packed = compression::encode_buffer(Algorithm::Zlib, &data).unwrap();
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    {
        let mut filter = OutputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 7, |c: &[u8]| chunks.push(c.to_vec())).unwrap();
        for piece in packed.chunks(5) {
            filter.write(piece).unwrap();
        }
        filter.finalize().unwrap();
    }
    assert!(chunks.iter().all(|c| !c.is_empty() && c.len() <= 7));
    assert_eq!(chunks.len(), (data.len() + 6) / 7);
    assert_eq!(chunks.concat(), data);
}

#[test]
fn an_output_filter_compresses_data_that_decodes_back() {
    let data = b"output filter ".repeat(30);
    let mut out = Vec::new();
    {
        let mut filter = OutputFilter::new(FilterOperation::Compress, Algorithm::Deflate, 32, |c: &[u8]| out.extend_from_slice(c)).unwrap();
        filter.write(&data[..100]).unwrap();
        filter.write(&data[100..]).unwrap();
        filter.finalize().unwrap();
    }
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &out).unwrap(), data);
}

#[test]
fn an_output_filter_refuses_writes_and_a_second_finalize_after_finishing() {
    let mut filter = OutputFilter::new(FilterOperation::Compress, Algorithm::Zlib, 8, |_: &[u8]| {}).unwrap();
    filter.write(b"x").unwrap();
    filter.finalize().unwrap();
    assert_eq!(filter.write(b"y"), Err(FilterError::InvalidState));
    assert_eq!(filter.finalize(), Err(FilterError::InvalidState));
}

#[test]
fn an_output_filter_leaves_the_sink_untouched_when_the_data_is_invalid() {
    let mut calls = 0;
    {
        let mut filter = OutputFilter::new(FilterOperation::Decompress, Algorithm::Zlib, 8, |_: &[u8]| calls += 1).unwrap();
        filter.write(&[0x79, 0x9c, 0x00]).unwrap();
        assert_eq!(filter.finalize(), Err(FilterError::InvalidData));
    }
    assert_eq!(calls, 0);
}

// ------------------------------------------------------------------ errors

#[test]
fn error_messages_name_each_failure() {
    assert_eq!(Error::Unsupported(Algorithm::Lzma).to_string(), "compression algorithm Lzma is not available");
    assert_eq!(Error::InvalidData.to_string(), "invalid compressed data");
    assert_eq!(Error::Truncated.to_string(), "compressed data ends early");
    assert_eq!(Error::TooBig.to_string(), "decompressed data is larger than the limit");
    assert_eq!(Error::Finished.to_string(), "compression stream already finalized");
    assert_eq!(FilterError::InvalidData.to_string(), "invalid data for the filter");
    assert_eq!(FilterError::InvalidState.to_string(), "filter is in an invalid state");
}

// ------------------------------------------------------------------ archive: DEFLATE helpers

#[test]
fn an_empty_archive_buffer_is_the_two_byte_empty_block() {
    assert_eq!(archive::compress(&[]), vec![0x03, 0x00]);
    assert!(archive::decompress(&[0x03, 0x00], 0).unwrap().is_empty());
}

#[test]
fn text_helpers_round_trip_unicode() {
    let text = "Héllo, wörld ✓";
    let packed = archive::compress_text(text);
    assert_eq!(archive::decompress_text(&packed, 1 << 20).unwrap(), text);
}

#[test]
fn text_helpers_write_raw_deflate_that_the_codec_decodes() {
    let packed = archive::compress_text("match me");
    assert_eq!(compression::decode_buffer(Algorithm::Deflate, &packed).unwrap(), b"match me".to_vec());
}

#[test]
fn decompress_text_refuses_bytes_that_are_not_utf8() {
    let packed = archive::compress(&[0xff, 0xfe, 0x00]);
    let err = archive::decompress_text(&packed, 16).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "Decompressed data is not UTF-8 text");
}

#[test]
fn decompress_refuses_output_past_the_limit_with_a_clear_message() {
    let packed = archive::compress(&[1u8; 64]);
    assert_eq!(archive::decompress(&packed, 64).unwrap(), vec![1u8; 64]);
    let err = archive::decompress(&packed, 63).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "Decompressed data is larger than the limit");
}

#[test]
fn decompress_reports_damaged_input() {
    let err = archive::decompress(&[0xcb, 0x48], 16).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "Damaged compressed data");
}

#[test]
fn the_reference_deflate_bytes_inflate_through_the_archive_helpers() {
    assert_eq!(archive::decompress(&DEFLATE_HELLO, 5).unwrap(), b"hello".to_vec());
    assert_eq!(archive::decompress_text(&DEFLATE_HELLO, 5).unwrap(), "hello");
}

// ------------------------------------------------------------------ archive: byte streams

#[test]
fn a_new_byte_stream_is_empty_with_the_cursor_at_zero() {
    let stream = ByteStream::new();
    assert!(stream.is_empty());
    assert_eq!(stream.len(), 0);
    assert_eq!(stream.position(), 0);
    assert_eq!(ByteStream::from_bytes(vec![1, 2, 3]).into_bytes(), vec![1, 2, 3]);
}

#[test]
fn byte_stream_reads_from_the_cursor_after_a_seek() {
    let mut stream = ByteStream::from_bytes(b"abcdef".to_vec());
    assert_eq!(stream.seek(SeekFrom::Start(2)).unwrap(), 2);
    let mut out = [0u8; 3];
    assert_eq!(stream.read(&mut out).unwrap(), 3);
    assert_eq!(&out, b"cde");
    assert_eq!(stream.position(), 5);
    assert_eq!(stream.seek(SeekFrom::End(-1)).unwrap(), 5);
    assert_eq!(stream.seek(SeekFrom::Current(-2)).unwrap(), 3);
}

#[test]
fn byte_stream_writes_past_the_end_with_a_zero_gap() {
    let mut stream = ByteStream::new();
    stream.seek(SeekFrom::Start(3)).unwrap();
    stream.write_all(b"x").unwrap();
    assert_eq!(stream.as_bytes(), &[0u8, 0, 0, b'x']);
    assert_eq!(stream.position(), 4);
}

#[test]
fn byte_stream_refuses_a_seek_before_the_start() {
    let mut stream = ByteStream::from_bytes(vec![0; 4]);
    assert_eq!(stream.seek(SeekFrom::Current(-1)).unwrap_err().kind(), ErrorKind::InvalidInput);
    assert_eq!(stream.seek(SeekFrom::End(-5)).unwrap_err().kind(), ErrorKind::InvalidInput);
    assert_eq!(stream.position(), 0, "a refused seek leaves the cursor where it was");
}

#[test]
fn byte_stream_positional_access_leaves_the_cursor_alone() {
    let mut stream = ByteStream::from_bytes(b"0123456789".to_vec());
    stream.seek(SeekFrom::Start(4)).unwrap();
    let mut out = [0u8; 3];
    assert_eq!(stream.read_at(7, &mut out).unwrap(), 3);
    assert_eq!(&out, b"789");
    assert_eq!(stream.read_at(10, &mut out).unwrap(), 0);
    assert_eq!(stream.write_at(0, b"AB").unwrap(), 2);
    assert_eq!(stream.position(), 4);
    assert_eq!(stream.as_bytes(), b"AB23456789");
}

#[test]
fn a_cancelled_byte_stream_refuses_every_operation() {
    let mut stream = ByteStream::from_bytes(b"data".to_vec());
    stream.cancel();
    let mut out = [0u8; 2];
    assert_eq!(stream.read(&mut out).unwrap_err().kind(), ErrorKind::Interrupted);
    assert_eq!(stream.write(b"x").unwrap_err().kind(), ErrorKind::Interrupted);
    assert_eq!(stream.flush().unwrap_err().kind(), ErrorKind::Interrupted);
    assert_eq!(stream.seek(SeekFrom::Start(0)).unwrap_err().kind(), ErrorKind::Interrupted);
    assert_eq!(stream.as_bytes(), b"data", "a cancelled write leaves the bytes unchanged");
}

#[test]
fn compress_to_writes_at_the_cursor_and_decompress_from_reads_to_the_end() {
    let mut stream = ByteStream::from_bytes(b"HDR".to_vec());
    stream.seek(SeekFrom::Start(3)).unwrap();
    let written = archive::compress_to(&mut stream, b"payload bytes").unwrap();
    assert_eq!(stream.len(), 3 + written);
    assert_eq!(&stream.as_bytes()[..3], b"HDR");
    stream.seek(SeekFrom::Start(3)).unwrap();
    assert_eq!(archive::decompress_from(&mut stream, 1 << 16).unwrap(), b"payload bytes".to_vec());
    assert_eq!(stream.position(), 3 + written);
}

#[test]
fn compress_to_threaded_round_trips_large_input_over_two_threads() {
    let data = pseudo_random(2 * 1024 * 1024 + 17, 9);
    let mut stream = ByteStream::new();
    let written = archive::compress_to_threaded(&mut stream, &data, 2).unwrap();
    assert_eq!(written, stream.len());
    stream.seek(SeekFrom::Start(0)).unwrap();
    assert_eq!(archive::decompress_from(&mut stream, data.len()).unwrap(), data);
}

#[test]
fn compress_to_threaded_with_one_thread_matches_the_single_step_bytes() {
    let data = b"thread count one ".repeat(100);
    let mut threaded = ByteStream::new();
    archive::compress_to_threaded(&mut threaded, &data, 1).unwrap();
    assert_eq!(threaded.as_bytes(), compression::encode_buffer(Algorithm::Deflate, &data).unwrap().as_slice());
}

#[test]
fn compress_to_threaded_is_deterministic_for_a_fixed_thread_count() {
    let data = pseudo_random(2 * 1024 * 1024 + 1, 21);
    let mut first = ByteStream::new();
    let mut second = ByteStream::new();
    archive::compress_to_threaded(&mut first, &data, 3).unwrap();
    archive::compress_to_threaded(&mut second, &data, 3).unwrap();
    assert_eq!(first.as_bytes(), second.as_bytes());
}

// ------------------------------------------------------------------ archive: files and folders

#[test]
fn compress_file_writes_a_zip_named_after_the_file_and_expands_it() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("notes.txt");
    write_file(&src, "one file");
    let zip = archive::compress_file(&src, root.path(), &mut |_, _| true).unwrap();
    assert_eq!(zip.file_name().unwrap(), "notes.zip");
    assert!(zip.is_file());
    let out_dir = root.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let extracted = archive::expand(&zip, &out_dir, &mut |_, _| true).unwrap();
    assert_eq!(extracted.files, 1);
    assert!(extracted.skipped.is_empty());
    assert_eq!(fs::read_to_string(find_file(&extracted.dir, "notes.txt").unwrap()).unwrap(), "one file");
}

#[test]
fn compress_file_refuses_a_folder_and_a_missing_path() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("folder")).unwrap();
    let folder_err = archive::compress_file(&root.path().join("folder"), root.path(), &mut |_, _| true).unwrap_err();
    assert_eq!(folder_err.kind(), ErrorKind::InvalidInput);
    assert_eq!(folder_err.to_string(), "Not a file");
    let missing = archive::compress_file(&root.path().join("gone.txt"), root.path(), &mut |_, _| true).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::InvalidInput);
}

#[test]
fn compress_file_stops_when_progress_asks_it_to() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("big.bin");
    fs::write(&src, pseudo_random(64 * 1024, 4)).unwrap();
    let result = archive::compress_file(&src, root.path(), &mut |_, _| false);
    assert!(result.is_err(), "a cancelled archive is an error");
}

#[test]
fn compress_directory_archives_the_whole_tree_and_expands_it_again() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("project");
    write_file(&folder.join("readme.txt"), "top level");
    write_file(&folder.join("src/lib.txt"), "nested content");
    let zip = archive::compress_directory(&folder, root.path(), &mut |_, _| true).unwrap();
    assert_eq!(zip.file_name().unwrap(), "project.zip");
    let out_dir = root.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let extracted = archive::expand(&zip, &out_dir, &mut |_, _| true).unwrap();
    assert_eq!(extracted.files, 2);
    assert_eq!(fs::read_to_string(find_file(&extracted.dir, "readme.txt").unwrap()).unwrap(), "top level");
    assert_eq!(fs::read_to_string(find_file(&extracted.dir, "lib.txt").unwrap()).unwrap(), "nested content");
}

#[test]
fn compress_directory_refuses_a_file_and_a_missing_path() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("plain.txt");
    write_file(&file, "not a folder");
    assert_eq!(archive::compress_directory(&file, root.path(), &mut |_, _| true).unwrap_err().kind(), ErrorKind::InvalidInput);
    let missing = archive::compress_directory(&root.path().join("absent"), root.path(), &mut |_, _| true).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::InvalidInput);
}

#[test]
fn expanding_the_same_archive_twice_takes_a_new_folder_each_time() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("twice.txt");
    write_file(&src, "again");
    let zip = archive::compress_file(&src, root.path(), &mut |_, _| true).unwrap();
    let first = archive::expand(&zip, root.path(), &mut |_, _| true).unwrap();
    let second = archive::expand(&zip, root.path(), &mut |_, _| true).unwrap();
    assert_ne!(first.dir, second.dir, "an existing folder is never reused");
    assert_eq!(fs::read_to_string(find_file(&second.dir, "twice.txt").unwrap()).unwrap(), "again");
}

#[test]
fn a_byte_stream_reads_to_the_end_and_then_returns_zero() {
    let mut stream = ByteStream::from_bytes(b"read me".to_vec());
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    assert_eq!(text, "read me");
    assert_eq!(stream.read(&mut [0u8; 4]).unwrap(), 0, "the cursor is at the end");
}

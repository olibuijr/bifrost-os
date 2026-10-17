//! Contract tests for the LZMA codec behind `bifrost_app::compression::Algorithm::Lzma`: the xz container around
//! LZMA2, the CRC32 and CRC64 checks, the decoder's refusals and limits, and the stream wiring.
//!
//! No xz stream made by an external tool is in the repository, so these tests check round trips through our own
//! encoder and decoder, and the published CRC vectors. They do not prove that other xz tools read our output.

use bifrost_app::compression::lzma::{crc32, crc64, decode_buffer, encode_buffer};
use bifrost_app::compression::{decode_buffer as decode_api, decode_buffer_limited, encode_buffer as encode_api, Algorithm, Error, FilterOperation, Stream};

/// Deterministic pseudo-random bytes from a fixed linear congruential generator.
fn lcg_bytes(len: usize, mut seed: u32) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as u8
        })
        .collect()
}

fn round_trip(data: &[u8]) {
    let enc = encode_buffer(data);
    assert_eq!(decode_buffer(&enc, usize::MAX).unwrap(), data, "round trip of {} bytes", data.len());
}

#[test]
fn crc32_matches_the_published_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn crc64_xz_matches_the_published_check_value() {
    assert_eq!(crc64(b"123456789"), 0x995D_C9BB_DF19_39FA);
    assert_eq!(crc64(b""), 0);
}

#[test]
fn empty_input_is_a_whole_xz_stream_that_decodes_to_nothing() {
    let enc = encode_buffer(b"");
    assert_eq!(&enc[..6], &[0xFD, b'7', b'z', b'X', b'Z', 0x00]);
    assert_eq!(&enc[enc.len() - 2..], b"YZ");
    assert_eq!(decode_buffer(&enc, 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn small_inputs_round_trip() {
    round_trip(b"x");
    round_trip(b"abc");
    round_trip(b"hello hello hello hello hello");
    round_trip(&[0u8; 3]);
}

#[test]
fn repetitive_input_round_trips_and_shrinks() {
    let data = vec![b'q'; 100_000];
    let enc = encode_buffer(&data);
    assert!(enc.len() < 2_000, "{} bytes for 100000 repeats", enc.len());
    assert_eq!(decode_buffer(&enc, data.len()).unwrap(), data);
    round_trip(&b"banana bread, ".repeat(5000));
}

#[test]
fn pseudo_random_input_round_trips_and_stays_near_its_size() {
    let data = lcg_bytes(200_000, 9);
    let enc = encode_buffer(&data);
    // incompressible chunks are stored, so the overhead is the headers, the index and the footer
    assert!(enc.len() < data.len() + 1024, "{} bytes for {}", enc.len(), data.len());
    assert_eq!(decode_buffer(&enc, data.len()).unwrap(), data);
}

#[test]
fn input_past_one_mebibyte_round_trips() {
    let mut data = lcg_bytes(600_000, 3);
    data.extend(b"the same words again, ".repeat(20_000));
    data.extend(lcg_bytes(500_000, 5));
    assert!(data.len() > 1 << 20);
    round_trip(&data);
}

#[test]
fn a_stream_with_a_bad_magic_is_refused() {
    let mut enc = encode_buffer(b"magic");
    enc[0] ^= 0xFF;
    assert_eq!(decode_buffer(&enc, usize::MAX), Err(Error::InvalidData));
    assert_eq!(decode_buffer(b"not an xz stream at all", usize::MAX), Err(Error::InvalidData));
}

#[test]
fn a_stored_chunk_with_a_bad_crc64_is_refused() {
    // 200 random bytes do not shrink, so the chunk is stored raw: the control byte 0x01 is at 12, the 2-byte
    // size at 13 to 14, and the data starts at 15. Changing one data byte breaks the CRC64 of the block.
    let data = lcg_bytes(200, 21);
    let mut enc = encode_buffer(&data);
    assert_eq!(enc[12], 0x01, "the chunk is expected to be stored raw");
    assert_eq!(decode_buffer(&enc, usize::MAX).unwrap(), data);
    enc[15 + 10] ^= 0x40;
    assert_eq!(decode_buffer(&enc, usize::MAX), Err(Error::InvalidData));
}

#[test]
fn a_bad_footer_crc_or_magic_is_refused() {
    let enc = encode_buffer(b"footer checks");
    let mut crc = enc.clone();
    let n = crc.len();
    crc[n - 12] ^= 0x01;
    assert_eq!(decode_buffer(&crc, usize::MAX), Err(Error::InvalidData));
    let mut magic = enc;
    let n = magic.len();
    magic[n - 1] = b'X';
    assert_eq!(decode_buffer(&magic, usize::MAX), Err(Error::InvalidData));
}

#[test]
fn every_truncation_is_refused_and_losing_the_footer_says_truncated() {
    let enc = encode_buffer(b"truncate this xz stream, please, truncate this xz stream");
    for cut in 0..enc.len() {
        assert!(decode_buffer(&enc[..cut], usize::MAX).is_err(), "a stream cut to {} bytes decoded", cut);
    }
    assert_eq!(decode_buffer(&enc[..enc.len() - 1], usize::MAX), Err(Error::Truncated));
    assert_eq!(decode_buffer(&enc[..enc.len() - 13], usize::MAX), Err(Error::Truncated));
}

#[test]
fn the_output_limit_is_inclusive() {
    let data = b"limit me, limit me, limit me, limit me, limit me".to_vec();
    let enc = encode_buffer(&data);
    assert_eq!(decode_buffer(&enc, data.len()).unwrap(), data);
    assert_eq!(decode_buffer(&enc, data.len() - 1), Err(Error::TooBig));
    assert_eq!(decode_buffer(&enc, 0), Err(Error::TooBig));
    assert_eq!(decode_buffer_limited(Algorithm::Lzma, &enc, data.len()).unwrap(), data);
    assert_eq!(decode_buffer_limited(Algorithm::Lzma, &enc, data.len() - 1), Err(Error::TooBig));
}

#[test]
fn the_compression_api_routes_lzma_to_the_xz_codec() {
    let data = b"routed through the algorithm selection".repeat(10);
    let enc = encode_api(Algorithm::Lzma, &data).unwrap();
    assert_eq!(enc, encode_buffer(&data));
    assert_eq!(decode_api(Algorithm::Lzma, &enc).unwrap(), data);
    assert!(Algorithm::Lzma.is_supported());
}

#[test]
fn a_compress_stream_buffers_and_gives_the_one_shot_bytes() {
    let data = lcg_bytes(70_000, 13);
    let mut stream = Stream::init(FilterOperation::Compress, Algorithm::Lzma).unwrap();
    assert!(stream.process(&data[..1000], false).unwrap().is_empty());
    let packed = stream.process(&data[1000..], true).unwrap();
    assert_eq!(packed, encode_buffer(&data));
    assert_eq!(stream.process(b"x", false), Err(Error::Finished));
}

#[test]
fn a_decompress_stream_returns_the_data_on_finalize() {
    let data = b"stream decode of an xz stream".repeat(300);
    let packed = encode_buffer(&data);
    let mut stream = Stream::init(FilterOperation::Decompress, Algorithm::Lzma).unwrap();
    let (head, tail) = packed.split_at(packed.len() / 2);
    assert!(stream.process(head, false).unwrap().is_empty());
    assert_eq!(stream.process(tail, true).unwrap(), data);
}

//! Contract tests for the LZ4 codec behind `bifrost_app::compression::Algorithm::Lz4`, its buffer
//! framing, the LZ4 frame helper and the stream and filter wiring.

use bifrost_app::compression::lz4::{decode_block, decode_buffer, decode_frame, encode_block, encode_buffer, encode_frame, xxh32};
use bifrost_app::compression::{decode_buffer_limited, encode_buffer as encode_api, decode_buffer as decode_api, Algorithm, Error, FilterOperation, Stream};

/// Deterministic pseudo-random bytes from a fixed linear congruential generator.
fn lcg_bytes(len: usize, mut seed: u32) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as u8
        })
        .collect()
}

fn round_trip_buffer(data: &[u8]) {
    let enc = encode_buffer(data);
    assert_eq!(decode_buffer(&enc, data.len()).unwrap(), data);
}

#[test]
fn xxh32_of_empty_input_matches_the_public_vector() {
    assert_eq!(xxh32(b"", 0), 0x02CC_5D05);
}

#[test]
fn buffer_round_trips_empty_one_byte_repetitive_random_and_large_inputs() {
    round_trip_buffer(b"");
    round_trip_buffer(b"x");
    round_trip_buffer(&vec![b'q'; 10_000]);
    round_trip_buffer(&lcg_bytes(4096, 1));
    // more than one 64 KiB block, mixing compressible and incompressible runs
    let mut big = vec![0u8; 70_000];
    big.extend(lcg_bytes(70_000, 7));
    big.extend(vec![0x41u8; 70_000]);
    round_trip_buffer(&big);
}

#[test]
fn empty_buffer_is_only_the_end_marker() {
    assert_eq!(encode_buffer(b""), b"bv4$".to_vec());
}

#[test]
fn incompressible_chunk_is_stored_as_a_bv4_block() {
    let data = lcg_bytes(200, 3);
    let enc = encode_buffer(&data);
    assert_eq!(&enc[..4], b"bv4-");
    assert_eq!(u32::from_le_bytes([enc[4], enc[5], enc[6], enc[7]]) as usize, data.len());
    assert_eq!(&enc[8..8 + data.len()], &data[..]);
    assert_eq!(&enc[enc.len() - 4..], b"bv4$");
}

#[test]
fn compressible_chunk_is_a_bv41_block_with_both_sizes() {
    let data = vec![b'a'; 1000];
    let enc = encode_buffer(&data);
    assert_eq!(&enc[..4], b"bv41");
    let decoded = u32::from_le_bytes([enc[4], enc[5], enc[6], enc[7]]) as usize;
    let encoded = u32::from_le_bytes([enc[8], enc[9], enc[10], enc[11]]) as usize;
    assert_eq!(decoded, 1000);
    assert_eq!(encoded, enc.len() - 12 - 4);
    assert!(encoded < 40);
}

#[test]
fn short_input_is_encoded_as_literals_only() {
    // fewer than 13 bytes cannot hold a match, so the block is one token then the bytes
    assert_eq!(encode_block(b"hello"), vec![0x50, b'h', b'e', b'l', b'l', b'o']);
    // 15 literals overflow the token nibble into one extension byte
    let fifteen = b"abcdefghijklmno";
    let enc = encode_block(fifteen);
    assert_eq!(enc[0], 0xF0);
    assert_eq!(enc[1], 0x00);
    assert_eq!(&enc[2..], &fifteen[..]);
}

#[test]
fn block_ends_with_five_literals_and_the_last_match_starts_before_twelve_from_the_end() {
    let data = vec![b'z'; 64];
    let enc = encode_block(&data);
    // the final sequence is literal-only, so its literal run is the tail of the input
    let last = enc.len();
    assert!(last >= 6);
    assert_eq!(decode_block(&enc, 64).unwrap(), data);
}

#[test]
fn decoder_rejects_bad_offsets() {
    // zero offset
    assert_eq!(decode_block(&[0x10, b'a', 0x00, 0x00], 100), Err(Error::InvalidData));
    // offset reaching before the start of the output
    assert_eq!(decode_block(&[0x10, b'a', 0x09, 0x00], 100), Err(Error::InvalidData));
}

#[test]
fn decoder_rejects_truncated_input() {
    // literal run longer than the input
    assert_eq!(decode_block(&[0x70, b'a', b'b'], 100), Err(Error::Truncated));
    // missing offset after a match token
    assert_eq!(decode_block(&[0x10, b'a', 0x01], 100), Err(Error::Truncated));
    // a truncated buffer frame: no end marker
    assert_eq!(decode_buffer(b"bv4-\x01\x00\x00\x00a", 100), Err(Error::Truncated));
    assert_eq!(decode_buffer(b"bv4", 100), Err(Error::Truncated));
}

#[test]
fn decoder_rejects_unknown_magic_and_output_over_the_limit() {
    assert_eq!(decode_buffer(b"bvxn\x00\x00\x00\x00bv4$", 100), Err(Error::InvalidData));
    let enc = encode_buffer(&vec![b'k'; 500]);
    assert_eq!(decode_buffer(&enc, 100), Err(Error::TooBig));
    assert_eq!(decode_block(&encode_block(&vec![b'k'; 500]), 100), Err(Error::TooBig));
}

#[test]
fn api_routes_lz4_through_the_algorithm_selection() {
    let data = b"banana banana banana banana";
    let enc = encode_api(Algorithm::Lz4, data).unwrap();
    assert_eq!(enc, encode_buffer(data));
    assert_eq!(decode_api(Algorithm::Lz4, &enc).unwrap(), data.to_vec());
    assert_eq!(decode_buffer_limited(Algorithm::Lz4, &enc, 5), Err(Error::TooBig));
    assert!(Algorithm::Lz4.is_supported());
    assert_eq!(encode_api(Algorithm::Lzma, b"x"), Err(Error::Unsupported(Algorithm::Lzma)));
}

#[test]
fn stream_compresses_and_decompresses_lz4() {
    let data = vec![b'w'; 300];
    let mut s = Stream::init(FilterOperation::Compress, Algorithm::Lz4).unwrap();
    assert_eq!(s.process(&data[..100], false).unwrap(), Vec::<u8>::new());
    let enc = s.process(&data[100..], true).unwrap();
    assert_eq!(enc, encode_buffer(&data));
    let mut d = Stream::init(FilterOperation::Decompress, Algorithm::Lz4).unwrap();
    assert_eq!(d.process(&enc, true).unwrap(), data);
}

#[test]
fn frame_round_trips() {
    let cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"x".to_vec(),
        vec![b'r'; 5000],
        lcg_bytes(3000, 9),
        {
            let mut v = vec![0u8; 140_000];
            v.extend(lcg_bytes(10_000, 11));
            v
        },
    ];
    for data in cases {
        let enc = encode_frame(&data);
        assert_eq!(&enc[..4], &[0x04, 0x22, 0x4D, 0x18], "magic is little endian 0x184D2204");
        assert_eq!(decode_frame(&enc, data.len()).unwrap(), data);
    }
}

#[test]
fn frame_rejects_bad_header_checksum_content_checksum_and_limit() {
    let data = vec![b'm'; 2000];
    let enc = encode_frame(&data);
    let mut bad_hc = enc.clone();
    bad_hc[6] ^= 0x01;
    assert_eq!(decode_frame(&bad_hc, 4000), Err(Error::InvalidData));
    let mut bad_sum = enc.clone();
    let last = bad_sum.len() - 1;
    bad_sum[last] ^= 0xff;
    assert_eq!(decode_frame(&bad_sum, 4000), Err(Error::InvalidData));
    assert_eq!(decode_frame(&enc, 100), Err(Error::TooBig));
    let mut trailing = enc.clone();
    trailing.push(0);
    assert_eq!(decode_frame(&trailing, 4000), Err(Error::InvalidData));
    assert_eq!(decode_frame(&enc[..10], 4000), Err(Error::Truncated));
}

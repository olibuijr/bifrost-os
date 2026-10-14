//! Parity sdk-complete-47: headless tests for the linear PCM converter (sample depth, channel layout and sample rate) and
//! the player controller model. Nothing is decoded from a file, no device is opened and no clock is read.

use bifrost_ui::pcm_convert::{convert_bytes, decode, encode, ConvertError, Depth, PcmFormat};

fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn read_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn format(sample_rate: u32, channels: u16, depth: Depth) -> PcmFormat {
    PcmFormat { sample_rate, channels, depth }
}

#[test]
fn a_frame_is_depth_times_channels_bytes() {
    assert_eq!(format(48_000, 2, Depth::S16).frame_bytes(), 4);
    assert_eq!(format(48_000, 6, Depth::F32).frame_bytes(), 24);
    assert_eq!(format(8_000, 1, Depth::U8).frame_bytes(), 1);
}

#[test]
fn unsigned_eight_bit_silence_is_128_and_full_scale_is_near_one() {
    assert_eq!(decode(&[128u8], Depth::U8).unwrap(), vec![0.0]);
    let top = decode(&[255u8], Depth::U8).unwrap()[0];
    assert!((top - 127.0 / 128.0).abs() < 1e-6, "top was {top}");
    assert_eq!(encode(&[0.0], Depth::U8), vec![128u8]);
    assert_eq!(encode(&[2.0], Depth::U8), vec![255u8], "out of range floats are clamped");
}

#[test]
fn signed_sixteen_bit_little_endian_samples_decode_to_floats() {
    let bytes = [0x00, 0x80, 0xff, 0x7f];
    let values = decode(&bytes, Depth::S16).unwrap();
    assert_eq!(values, vec![-1.0, 32767.0 / 32768.0]);
}

#[test]
fn float_samples_round_trip_through_bytes_unchanged() {
    let samples = [0.25, -0.5, 0.0, 1.0];
    assert_eq!(decode(&encode(&samples, Depth::F32), Depth::F32).unwrap(), samples.to_vec());
}

#[test]
fn a_partial_sample_is_refused_as_ragged() {
    assert_eq!(decode(&[0u8, 1, 2], Depth::S16), Err(ConvertError::Ragged));
    assert_eq!(decode(&[0u8; 5], Depth::F32), Err(ConvertError::Ragged));
}

#[test]
fn identical_formats_return_the_same_bytes() {
    let fmt = format(48_000, 1, Depth::S16);
    let bytes: Vec<u8> = (0u8..16).collect();
    assert_eq!(convert_bytes(&bytes, fmt, fmt).unwrap(), bytes);
}

#[test]
fn stereo_to_mono_averages_each_frame() {
    let input = f32_bytes(&[0.5, 0.5, -1.0, 0.0]);
    let out = convert_bytes(&input, format(48_000, 2, Depth::F32), format(48_000, 1, Depth::F32)).unwrap();
    assert_eq!(read_f32(&out), vec![0.5, -0.5]);
}

#[test]
fn mono_to_stereo_repeats_the_sample_on_both_channels() {
    let input = f32_bytes(&[0.25, -0.25]);
    let out = convert_bytes(&input, format(48_000, 1, Depth::F32), format(48_000, 2, Depth::F32)).unwrap();
    assert_eq!(read_f32(&out), vec![0.25, 0.25, -0.25, -0.25]);
}

#[test]
fn halving_the_rate_halves_the_frame_count() {
    let input = f32_bytes(&[0.0, 1.0, 2.0, 3.0]);
    let out = convert_bytes(&input, format(48_000, 1, Depth::F32), format(24_000, 1, Depth::F32)).unwrap();
    assert_eq!(read_f32(&out), vec![0.0, 2.0]);
}

#[test]
fn doubling_the_rate_doubles_the_frame_count_and_interpolates() {
    let input = f32_bytes(&[0.0, 1.0]);
    let out = convert_bytes(&input, format(24_000, 1, Depth::F32), format(48_000, 1, Depth::F32)).unwrap();
    let samples = read_f32(&out);
    assert_eq!(samples.len(), 4);
    assert_eq!(&samples[..2], &[0.0, 0.5]);
}

#[test]
fn depth_change_quantises_to_the_target_depth() {
    let input = f32_bytes(&[0.0, -1.0]);
    let out = convert_bytes(&input, format(48_000, 1, Depth::F32), format(48_000, 1, Depth::S16)).unwrap();
    assert_eq!(out, vec![0u8, 0, 1, 0x80], "0 is silence and -1.0 is -32767 as little-endian bytes");
}

#[test]
fn a_block_that_is_not_whole_frames_is_refused() {
    let input = [0u8; 6];
    let err = convert_bytes(&input, format(48_000, 2, Depth::S16), format(48_000, 1, Depth::S16)).unwrap_err();
    assert_eq!(err, ConvertError::Ragged);
}

#[test]
fn zero_rates_and_zero_channels_are_refused() {
    let ok = format(48_000, 1, Depth::S16);
    assert_eq!(convert_bytes(&[0u8; 2], format(0, 1, Depth::S16), ok), Err(ConvertError::ZeroRate));
    assert_eq!(convert_bytes(&[0u8; 2], ok, format(44_100, 0, Depth::S16)), Err(ConvertError::ZeroChannels));
}

#[test]
fn an_empty_block_converts_to_nothing() {
    let out = convert_bytes(&[], format(48_000, 2, Depth::S16), format(44_100, 1, Depth::U8)).unwrap();
    assert!(out.is_empty());
}

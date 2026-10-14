//! Parity sdk-complete-30: headless tests for the stream description consistency check and the format identifier
//! table. No device, no clock and no server is involved.

use bifrost_audio_types::codec_id::CodecId;
use bifrost_audio_types::format::{FormatFlags, StreamFormat};
use bifrost_audio_types::AudioError;

#[test]
fn a_float32_stream_description_is_consistent() {
    assert_eq!(StreamFormat::float32(48_000.0, 2).validate(), Ok(()));
}

#[test]
fn integer_descriptions_with_packed_and_high_aligned_samples_are_consistent() {
    assert_eq!(StreamFormat::integer(44_100.0, 2, 16, 2).validate(), Ok(()));
    assert_eq!(StreamFormat::integer(96_000.0, 6, 24, 4).validate(), Ok(()));
}

#[test]
fn a_zero_or_non_finite_sample_rate_is_refused() {
    assert_eq!(StreamFormat::float32(0.0, 2).validate(), Err(AudioError::BadValue));
    assert_eq!(StreamFormat::float32(-44_100.0, 2).validate(), Err(AudioError::BadValue));
    assert_eq!(StreamFormat::float32(f64::NAN, 2).validate(), Err(AudioError::BadValue));
}

#[test]
fn a_description_without_channels_is_refused() {
    assert_eq!(StreamFormat::float32(48_000.0, 0).validate(), Err(AudioError::BadValue));
}

#[test]
fn a_packet_size_that_disagrees_with_the_frame_size_is_refused() {
    let mut format = StreamFormat::float32(48_000.0, 2);
    format.bytes_per_packet = 8;
    assert_eq!(format.validate(), Err(AudioError::BadValue));
}

#[test]
fn a_sample_wider_than_its_container_is_refused() {
    let mut format = StreamFormat::integer(44_100.0, 2, 16, 2);
    format.bits_per_channel = 24;
    assert_eq!(format.validate(), Err(AudioError::BadValue));
}

#[test]
fn a_float_stream_must_be_32_or_64_bits_wide() {
    let mut format = StreamFormat::float32(48_000.0, 2);
    format.bits_per_channel = 16;
    assert_eq!(format.validate(), Err(AudioError::BadValue));
    format.bits_per_channel = 64;
    format.bytes_per_frame = 8;
    format.bytes_per_packet = 8;
    assert_eq!(format.validate(), Ok(()));
}

#[test]
fn a_format_that_is_both_float_and_signed_integer_is_refused() {
    let mut format = StreamFormat::float32(48_000.0, 2);
    format.flags = format.flags | FormatFlags::SIGNED_INTEGER;
    assert_eq!(format.validate(), Err(AudioError::BadValue));
}

#[test]
fn each_named_encoding_round_trips_through_its_four_character_code() {
    for codec in CodecId::ALL {
        assert_eq!(CodecId::from_fourcc(codec.fourcc()), Some(codec));
    }
    assert_eq!(CodecId::Aac.fourcc(), *b"aac ", "the AAC code keeps its trailing space");
    assert_eq!(CodecId::Mp3.fourcc(), *b".mp3");
}

#[test]
fn an_unnamed_code_is_not_an_encoding() {
    assert_eq!(CodecId::from_fourcc(*b"apac"), None, "APAC is not supported");
    assert_eq!(CodecId::from_fourcc(*b"mp4a"), None, "mp4a is a container sample entry, not an encoding");
}

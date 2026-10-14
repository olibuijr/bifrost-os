//! Parity sdk-complete-30: headless tests for the AudioSpecificConfig fields that describe HE-AAC output. The config
//! bytes are built by hand from their bit fields, so each expected value can be checked against the bit layout.

use springboard::media::mp4::{asc_params, parse_asc, sbr_output_rate};

/// An HE-AAC config: object 5 (SBR), core rate index 6 (24000 Hz), one channel, extension rate index 3 (48000 Hz),
/// core object 2 (AAC LC). Bits: 00101 0110 0001 0011 00010, padded with zeros to whole bytes.
const HE_AAC_EXPLICIT: [u8; 3] = [0x2B, 0x09, 0x88];

/// An AAC LC config at 48000 Hz with two channels: object 2, rate index 3, channels 2.
const AAC_LC_48K_STEREO: [u8; 2] = [0x11, 0x90];

/// An HE-AAC config with the escaped core rate: object 5, core rate index 15 followed by 24,000 in 24 bits, one
/// channel, then extension rate index 3 (48000 Hz) and core object 2. Bits are padded with two zeros at the end.
const HE_AAC_ESCAPED_CORE_RATE: [u8; 6] = [0x2F, 0x80, 0x2E, 0xE0, 0x09, 0x88];

#[test]
fn an_explicit_sbr_extension_rate_is_read_from_the_config() {
    assert_eq!(sbr_output_rate(&HE_AAC_EXPLICIT), Some(48_000));
}

#[test]
fn the_core_rate_stays_the_reported_rate_when_the_extension_is_read() {
    let config = parse_asc(&HE_AAC_EXPLICIT).expect("a complete HE-AAC config parses");
    assert_eq!(config.object_type, 5);
    assert_eq!(config.sample_rate, 24_000, "the sample rate field keeps the core rate");
    assert_eq!(config.channels, 1);
    assert_eq!(asc_params(&HE_AAC_EXPLICIT), Some((24_000, 1)));
}

#[test]
fn a_config_without_an_sbr_object_has_no_extension_rate() {
    assert_eq!(sbr_output_rate(&AAC_LC_48K_STEREO), None);
    assert_eq!(parse_asc(&AAC_LC_48K_STEREO).map(|c| c.sample_rate), Some(48_000));
}

#[test]
fn an_sbr_config_that_stops_before_the_extension_has_no_extension_rate() {
    // The core fields alone: object 5, rate index 6, channel 1. The extension bits are missing.
    let truncated = [0x2B, 0x08];
    assert_eq!(sbr_output_rate(&truncated), None);
    let config = parse_asc(&truncated).expect("the core fields still parse");
    assert_eq!((config.object_type, config.sample_rate, config.channels), (5, 24_000, 1));
}

#[test]
fn an_escaped_core_rate_and_an_indexed_extension_rate_are_both_read() {
    let config = parse_asc(&HE_AAC_ESCAPED_CORE_RATE).expect("the escaped config parses");
    assert_eq!((config.object_type, config.sample_rate, config.channels), (5, 24_000, 1));
    assert_eq!(sbr_output_rate(&HE_AAC_ESCAPED_CORE_RATE), Some(48_000));
}

#[test]
fn an_escaped_extension_rate_reads_its_24_bit_value() {
    // Object 5, core rate index 6 (24000 Hz), one channel, extension rate index 15 followed by 96,000 in 24 bits
    // (0x017700), then core object 2. Bits: 00101 0110 0001 1111 000000010111011100000000 00010 00, as bytes.
    let escaped_extension = [0x2B, 0x0F, 0x80, 0xBB, 0x80, 0x08];
    assert_eq!(sbr_output_rate(&escaped_extension), Some(96_000));
}

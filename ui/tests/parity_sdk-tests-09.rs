//! Parity tests (sdk-tests-09) for the recorder settings model (`audio_settings`): the settings dictionary, the format
//! rules and the nearest supported configuration. Headless host tests: the model is pure data, with no audio device,
//! no clock and no file. Each test drives the public API of the module.

use std::collections::BTreeMap;

use bifrost_ui::audio_settings::{Format, Quality, Settings};

fn dict(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn headless_the_default_settings_are_linear_pcm_mono_at_44_1_khz() {
    let s = Settings::default();
    assert_eq!(s.format, Format::LinearPcm);
    assert_eq!((s.sample_rate, s.channels, s.bit_depth), (44100, 1, 16));
    assert_eq!((s.quality, s.bit_rate), (Quality::High, 0));
}

#[test]
fn headless_the_dictionary_has_the_format_rate_channels_depth_and_quality_keys() {
    let m = Settings::default().to_map();
    assert_eq!(m.get("format").map(String::as_str), Some("lpcm"));
    assert_eq!(m.get("rate").map(String::as_str), Some("44100"));
    assert_eq!(m.get("channels").map(String::as_str), Some("1"));
    assert_eq!(m.get("depth").map(String::as_str), Some("16"));
    assert_eq!(m.get("quality").map(String::as_str), Some("3"), "High is level three");
    assert!(!m.contains_key("bitrate"), "a lossless setting with no bit rate has no bitrate key");
}

#[test]
fn headless_a_lossy_setting_writes_its_bit_rate_key() {
    let s = Settings { format: Format::Opus, sample_rate: 48000, channels: 2, bit_depth: 0, quality: Quality::Low, bit_rate: 24000 };
    assert_eq!(s.to_map().get("bitrate").map(String::as_str), Some("24000"));
}

#[test]
fn headless_a_flac_setting_survives_the_dictionary_round_trip() {
    let s = Settings { format: Format::Flac, sample_rate: 48000, channels: 2, bit_depth: 24, quality: Quality::Max, bit_rate: 0 };
    assert_eq!(Settings::from_map(&s.to_map()), s);
}

#[test]
fn headless_unreadable_dictionary_values_keep_the_defaults() {
    let s = Settings::from_map(&dict(&[("format", "mp3"), ("rate", "fast")]));
    assert_eq!(s, Settings::default(), "an unknown format and a non-number rate are both ignored");
}

#[test]
fn headless_quality_levels_read_from_the_dictionary_and_out_of_range_means_max() {
    assert_eq!(Settings::from_map(&dict(&[("quality", "0")])).quality, Quality::Min);
    assert_eq!(Settings::from_map(&dict(&[("quality", "1")])).quality, Quality::Low);
    assert_eq!(Settings::from_map(&dict(&[("quality", "9")])).quality, Quality::Max);
}

#[test]
fn headless_every_format_key_reads_back_to_its_format() {
    for f in [Format::LinearPcm, Format::Flac, Format::Opus, Format::Aac] {
        assert_eq!(Format::from_key(f.key()), Some(f));
    }
    assert_eq!(Format::from_key("wav"), None);
}

#[test]
fn headless_pcm_and_flac_are_lossless_and_the_others_are_not() {
    assert!(Format::LinearPcm.lossless() && Format::Flac.lossless());
    assert!(!Format::Opus.lossless() && !Format::Aac.lossless());
}

#[test]
fn headless_a_rate_snaps_to_the_nearest_rate_the_format_supports() {
    let opus = Settings { format: Format::Opus, sample_rate: 44100, ..Settings::default() }.normalized();
    assert_eq!(opus.sample_rate, 48000);
    let aac = Settings { format: Format::Aac, sample_rate: 8000, ..Settings::default() }.normalized();
    assert_eq!(aac.sample_rate, 16000);
}

#[test]
fn headless_a_bit_depth_snaps_for_flac_and_is_zero_for_lossy_formats() {
    let flac = Settings { format: Format::Flac, bit_depth: 22, ..Settings::default() }.normalized();
    assert_eq!(flac.bit_depth, 24, "22 is nearer 24 than 16");
    let opus = Settings { format: Format::Opus, bit_depth: 16, ..Settings::default() }.normalized();
    assert_eq!(opus.bit_depth, 0);
}

#[test]
fn headless_channels_are_limited_by_the_format() {
    let pcm = Settings { channels: 6, ..Settings::default() }.normalized();
    assert_eq!(pcm.channels, 6, "linear PCM takes up to eight channels");
    let opus = Settings { format: Format::Opus, channels: 6, ..Settings::default() }.normalized();
    assert_eq!(opus.channels, 2, "Opus takes up to two channels");
    let mono = Settings { format: Format::Aac, channels: 0, ..Settings::default() }.normalized();
    assert_eq!(mono.channels, 1, "at least one channel");
}

#[test]
fn headless_a_lossy_format_without_a_bit_rate_takes_the_quality_rate() {
    let s = Settings { format: Format::Aac, channels: 2, quality: Quality::High, bit_rate: 0, ..Settings::default() };
    assert_eq!(s.default_bit_rate(), 192_000, "96 kb/s per channel, two channels");
    assert_eq!(s.normalized().bit_rate, 192_000);
    let min = Settings { format: Format::Aac, channels: 1, quality: Quality::Min, ..Settings::default() };
    assert_eq!(min.default_bit_rate(), 16_000);
}

#[test]
fn headless_a_lossy_bit_rate_is_clamped_and_a_lossless_one_is_dropped() {
    let high = Settings { format: Format::Opus, bit_rate: 900_000, ..Settings::default() }.normalized();
    assert_eq!(high.bit_rate, 320_000);
    let low = Settings { format: Format::Opus, bit_rate: 1_000, ..Settings::default() }.normalized();
    assert_eq!(low.bit_rate, 6_000);
    let flac = Settings { format: Format::Flac, bit_rate: 128_000, ..Settings::default() }.normalized();
    assert_eq!(flac.bit_rate, 0);
}

#[test]
fn headless_pcm_bytes_per_second_multiply_rate_channels_and_sample_bytes() {
    let s = Settings { sample_rate: 44100, channels: 2, bit_depth: 16, ..Settings::default() };
    assert_eq!(s.pcm_bytes_per_second(), 176_400);
    let lossy = Settings { format: Format::Opus, ..s };
    assert_eq!(lossy.pcm_bytes_per_second(), 0, "a compressed format has no fixed data rate");
}

#[test]
fn headless_only_a_normalised_setting_is_supported_as_it_is() {
    assert!(Settings::default().is_supported());
    let off = Settings { sample_rate: 45000, ..Settings::default() };
    assert!(!off.is_supported(), "45 kHz is not a supported PCM rate");
    assert!(off.normalized().is_supported());
}

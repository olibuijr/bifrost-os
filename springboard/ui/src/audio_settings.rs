//! Audio recorder settings: the settings dictionary keys (format, sample rate, channels, bit depth, quality,
//! bit rate), validation per format, and the choice of the nearest supported configuration, so an app can ask for what
//! it wants and the recorder picks a working one. Pure std.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Format {
    LinearPcm,
    Flac,
    Opus,
    Aac,
}

impl Format {
    pub fn key(self) -> &'static str {
        match self {
            Format::LinearPcm => "lpcm",
            Format::Flac => "flac",
            Format::Opus => "opus",
            Format::Aac => "aac",
        }
    }

    pub fn from_key(k: &str) -> Option<Format> {
        [Format::LinearPcm, Format::Flac, Format::Opus, Format::Aac].into_iter().find(|f| f.key() == k)
    }

    pub fn lossless(self) -> bool {
        matches!(self, Format::LinearPcm | Format::Flac)
    }

    fn sample_rates(self) -> &'static [u32] {
        match self {
            Format::LinearPcm | Format::Flac => &[8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000],
            Format::Opus => &[8000, 12000, 16000, 24000, 48000],
            Format::Aac => &[16000, 22050, 24000, 32000, 44100, 48000],
        }
    }

    fn bit_depths(self) -> &'static [u32] {
        match self {
            Format::LinearPcm => &[8, 16, 24, 32],
            Format::Flac => &[16, 24],
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Min,
    Low,
    Medium,
    High,
    Max,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub format: Format,
    pub sample_rate: u32,
    pub channels: u8,
    pub bit_depth: u32,
    pub quality: Quality,
    /// Bits per second for lossy formats (0 = derive from quality).
    pub bit_rate: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { format: Format::LinearPcm, sample_rate: 44100, channels: 1, bit_depth: 16, quality: Quality::High, bit_rate: 0 }
    }
}

fn nearest(options: &[u32], want: u32) -> u32 {
    options.iter().copied().min_by_key(|o| o.abs_diff(want)).unwrap_or(want)
}

impl Settings {
    /// The closest configuration the format supports: rate and depth snapped to supported values, channels 1 or 2
    /// (Opus up to 2, PCM and FLAC up to 8), bit rate clamped.
    pub fn normalized(&self) -> Settings {
        let f = self.format;
        let max_channels = if matches!(f, Format::LinearPcm | Format::Flac) { 8 } else { 2 };
        let depths = f.bit_depths();
        Settings {
            format: f,
            sample_rate: nearest(f.sample_rates(), self.sample_rate),
            channels: self.channels.clamp(1, max_channels),
            bit_depth: if depths.is_empty() { 0 } else { nearest(depths, self.bit_depth) },
            quality: self.quality,
            bit_rate: if f.lossless() { 0 } else if self.bit_rate == 0 { self.default_bit_rate() } else { self.bit_rate.clamp(6_000, 320_000) },
        }
    }

    /// The bit rate a quality level means for the format and channel count.
    pub fn default_bit_rate(&self) -> u32 {
        let per_channel = match self.quality {
            Quality::Min => 16_000,
            Quality::Low => 32_000,
            Quality::Medium => 64_000,
            Quality::High => 96_000,
            Quality::Max => 128_000,
        };
        (per_channel * u32::from(self.channels.clamp(1, 2))).clamp(6_000, 320_000)
    }

    /// Raw data rate of lossless PCM in bytes per second (0 for other formats).
    pub fn pcm_bytes_per_second(&self) -> u64 {
        if self.format != Format::LinearPcm {
            return 0;
        }
        u64::from(self.sample_rate) * u64::from(self.channels) * u64::from(self.bit_depth / 8)
    }

    /// Whether the settings are already normalised (the recorder can use them as they are).
    pub fn is_supported(&self) -> bool {
        *self == self.normalized()
    }

    /// The dictionary form.
    pub fn to_map(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("format".into(), self.format.key().into());
        m.insert("rate".into(), self.sample_rate.to_string());
        m.insert("channels".into(), self.channels.to_string());
        m.insert("depth".into(), self.bit_depth.to_string());
        m.insert("quality".into(), (self.quality as u8).to_string());
        if self.bit_rate > 0 {
            m.insert("bitrate".into(), self.bit_rate.to_string());
        }
        m
    }

    /// Reads the dictionary form; missing or unreadable keys keep their defaults. The result is not normalised.
    pub fn from_map(m: &BTreeMap<String, String>) -> Settings {
        let mut s = Settings::default();
        if let Some(f) = m.get("format").and_then(|v| Format::from_key(v)) {
            s.format = f;
        }
        if let Some(v) = m.get("rate").and_then(|v| v.parse().ok()) {
            s.sample_rate = v;
        }
        if let Some(v) = m.get("channels").and_then(|v| v.parse().ok()) {
            s.channels = v;
        }
        if let Some(v) = m.get("depth").and_then(|v| v.parse().ok()) {
            s.bit_depth = v;
        }
        if let Some(v) = m.get("bitrate").and_then(|v| v.parse().ok()) {
            s.bit_rate = v;
        }
        if let Some(q) = m.get("quality").and_then(|v| v.parse::<u8>().ok()) {
            s.quality = match q {
                0 => Quality::Min,
                1 => Quality::Low,
                2 => Quality::Medium,
                3 => Quality::High,
                _ => Quality::Max,
            };
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_supported_values() {
        let s = Settings { format: Format::Opus, sample_rate: 44100, channels: 6, bit_depth: 16, ..Settings::default() }.normalized();
        assert_eq!(s.sample_rate, 48000);
        assert_eq!(s.channels, 2);
        assert_eq!(s.bit_depth, 0);
        assert!(s.bit_rate >= 6_000);
        let p = Settings { sample_rate: 45000, bit_depth: 21, ..Settings::default() }.normalized();
        assert_eq!((p.sample_rate, p.bit_depth), (44100, 24));
        assert_eq!(Settings { format: Format::Flac, bit_depth: 8, ..Settings::default() }.normalized().bit_depth, 16);
    }

    #[test]
    fn bit_rates() {
        let mono = Settings { format: Format::Aac, quality: Quality::Medium, channels: 1, ..Settings::default() };
        assert_eq!(mono.default_bit_rate(), 64_000);
        assert_eq!(Settings { channels: 2, ..mono.clone() }.default_bit_rate(), 128_000);
        assert_eq!(Settings { bit_rate: 900_000, ..mono.clone() }.normalized().bit_rate, 320_000);
        assert_eq!(Settings { format: Format::Flac, bit_rate: 5, ..Settings::default() }.normalized().bit_rate, 0);
    }

    #[test]
    fn data_rate_and_support() {
        let s = Settings { sample_rate: 48000, channels: 2, bit_depth: 16, ..Settings::default() };
        assert_eq!(s.pcm_bytes_per_second(), 192_000);
        assert!(s.is_supported());
        assert!(!Settings { sample_rate: 12345, ..Settings::default() }.is_supported());
        assert_eq!(Settings { format: Format::Aac, ..Settings::default() }.pcm_bytes_per_second(), 0);
    }

    #[test]
    fn dictionary_round_trip() {
        let s = Settings { format: Format::Aac, sample_rate: 48000, channels: 2, bit_depth: 0, quality: Quality::Low, bit_rate: 64_000 };
        assert_eq!(Settings::from_map(&s.to_map()), s);
        let mut m = BTreeMap::new();
        m.insert("rate".to_string(), "abc".to_string());
        m.insert("format".to_string(), "mp9".to_string());
        assert_eq!(Settings::from_map(&m), Settings::default());
        assert_eq!(Format::from_key("flac"), Some(Format::Flac));
    }
}

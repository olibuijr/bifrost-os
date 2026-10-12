//! Recording settings: the format an app asks for when it records, as a settings table holds it (format identifier,
//! sample rate, channel count, linear PCM sample layout, encoder quality). `Settings::resolve` checks the table
//! against the formats the recorder can deliver and returns the concrete format, or the first rule the request breaks.
//! The shell recorder captures signed 16-bit interleaved samples at the (rate, channels) pairs it tries, so the
//! caller passes that list in. Encoder quality applies only to compressed formats and is ignored for linear PCM.
//! Pure: no device is opened and no file is written.

/// The lowest sample rate a request may name, in hertz.
pub const MIN_RATE: f64 = 8_000.0;
/// The highest sample rate a request may name, in hertz.
pub const MAX_RATE: f64 = 192_000.0;

/// The format identifier of the recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatId {
    /// Uncompressed samples.
    LinearPcm,
    /// Compressed AAC. Named so a request can be refused clearly; the recorder does not encode it.
    Aac,
}

/// The encoder quality of a compressed recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Min,
    Low,
    Medium,
    High,
    Max,
}

/// A recording format request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub format: FormatId,
    /// Sample rate in hertz. Must name a rate in the supported list exactly.
    pub sample_rate: f64,
    pub channels: u32,
    /// Bits per sample. Linear PCM is recorded at 16 bits only.
    pub bit_depth: u32,
    /// Floating-point samples. Refused: the recorder writes integers.
    pub float: bool,
    /// Big-endian samples. Refused: the recorder writes little-endian.
    pub big_endian: bool,
    /// Channels stored one after another. Refused: the recorder interleaves channels.
    pub non_interleaved: bool,
    /// Encoder quality. Only compressed formats use it.
    pub quality: Option<Quality>,
}

/// The concrete format the recorder will use for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_depth: u32,
}

/// The first rule a request breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The format is not linear PCM, and the recorder cannot encode it.
    UnsupportedFormat,
    /// The sample rate is not finite or lies outside `MIN_RATE..=MAX_RATE`.
    InvalidRate,
    /// No supported format has this sample rate.
    RateNotSupported,
    /// The sample rate is supported, but not with this channel count.
    ChannelsNotSupported,
    /// The sample layout (bit depth, float, byte order or channel order) is not recordable.
    LayoutNotSupported,
}

impl Settings {
    /// A request for interleaved, 16-bit, little-endian, integer linear PCM.
    pub fn linear_pcm(sample_rate: f64, channels: u32) -> Self {
        Settings {
            format: FormatId::LinearPcm,
            sample_rate,
            channels,
            bit_depth: 16,
            float: false,
            big_endian: false,
            non_interleaved: false,
            quality: None,
        }
    }

    /// Checks the request against `supported`, a list of (sample rate, channels) pairs in the order the recorder
    /// tries them. The rules run in order: format, rate range, sample layout, then the exact rate and channels.
    pub fn resolve(&self, supported: &[(u32, u32)]) -> Result<Pcm, Error> {
        if self.format != FormatId::LinearPcm {
            return Err(Error::UnsupportedFormat);
        }
        if !self.sample_rate.is_finite() || self.sample_rate < MIN_RATE || self.sample_rate > MAX_RATE {
            return Err(Error::InvalidRate);
        }
        if self.float || self.big_endian || self.non_interleaved || self.bit_depth != 16 {
            return Err(Error::LayoutNotSupported);
        }
        let rate_known = supported.iter().any(|&(rate, _)| self.sample_rate == f64::from(rate));
        if !rate_known {
            return Err(Error::RateNotSupported);
        }
        let found = supported
            .iter()
            .any(|&(rate, channels)| self.sample_rate == f64::from(rate) && channels == self.channels);
        if !found {
            return Err(Error::ChannelsNotSupported);
        }
        Ok(Pcm { sample_rate: self.sample_rate as u32, channels: self.channels, bit_depth: 16 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell recorder's list: (48 kHz, stereo), (48 kHz, mono), (44.1 kHz, stereo), (16 kHz, mono), (8 kHz, mono).
    const RECORDER: [(u32, u32); 5] = [(48_000, 2), (48_000, 1), (44_100, 2), (16_000, 1), (8_000, 1)];

    #[test]
    fn a_supported_stereo_request_resolves_to_16_bit_pcm() {
        let pcm = Settings::linear_pcm(48_000.0, 2).resolve(&RECORDER);
        assert_eq!(pcm, Ok(Pcm { sample_rate: 48_000, channels: 2, bit_depth: 16 }));
    }

    #[test]
    fn a_rate_supported_only_in_stereo_refuses_mono() {
        assert_eq!(Settings::linear_pcm(44_100.0, 1).resolve(&RECORDER), Err(Error::ChannelsNotSupported));
    }

    #[test]
    fn a_rate_outside_the_list_is_not_supported() {
        assert_eq!(Settings::linear_pcm(22_050.0, 1).resolve(&RECORDER), Err(Error::RateNotSupported));
        // A fraction of a hertz is a different rate, so it does not match 48 kHz.
        assert_eq!(Settings::linear_pcm(48_000.5, 2).resolve(&RECORDER), Err(Error::RateNotSupported));
    }

    #[test]
    fn a_rate_outside_the_allowed_range_is_invalid() {
        assert_eq!(Settings::linear_pcm(200_000.0, 2).resolve(&RECORDER), Err(Error::InvalidRate));
        assert_eq!(Settings::linear_pcm(7_999.0, 1).resolve(&RECORDER), Err(Error::InvalidRate));
        assert_eq!(Settings::linear_pcm(f64::NAN, 2).resolve(&RECORDER), Err(Error::InvalidRate));
    }

    #[test]
    fn a_compressed_format_is_refused_before_the_rate_is_checked() {
        let mut s = Settings::linear_pcm(48_000.0, 2);
        s.format = FormatId::Aac;
        s.quality = Some(Quality::High);
        assert_eq!(s.resolve(&RECORDER), Err(Error::UnsupportedFormat));
    }

    #[test]
    fn a_sample_layout_the_recorder_does_not_write_is_refused() {
        let mut s = Settings::linear_pcm(48_000.0, 2);
        s.bit_depth = 24;
        assert_eq!(s.resolve(&RECORDER), Err(Error::LayoutNotSupported));

        let mut s = Settings::linear_pcm(48_000.0, 2);
        s.float = true;
        assert_eq!(s.resolve(&RECORDER), Err(Error::LayoutNotSupported));

        let mut s = Settings::linear_pcm(48_000.0, 2);
        s.non_interleaved = true;
        assert_eq!(s.resolve(&RECORDER), Err(Error::LayoutNotSupported));
    }

    #[test]
    fn encoder_quality_does_not_change_a_pcm_result() {
        let mut s = Settings::linear_pcm(16_000.0, 1);
        s.quality = Some(Quality::Max);
        assert_eq!(s.resolve(&RECORDER), Ok(Pcm { sample_rate: 16_000, channels: 1, bit_depth: 16 }));
    }
}

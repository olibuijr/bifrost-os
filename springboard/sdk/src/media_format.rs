//! Format descriptions for media samples (`CMFormatDescription`): the media type (video or sound), the codec as its
//! four-character code, and the properties that type needs (the picture size for video; the rate, channel count and
//! sample size for sound). A sample buffer carries one of these so a consumer knows how to decode it. Pure data: no
//! decoder is run here.

use std::fmt;

/// Why a description was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// The codec is not a four-character code of printable ASCII (a space pads a short one, as in `'avc1'`).
    BadCodec,
    /// A picture size is zero or larger than the limit.
    BadSize,
    /// A sound property is outside its range.
    BadSound,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::BadCodec => write!(f, "the codec must be four printable ASCII characters"),
            FormatError::BadSize => write!(f, "the picture size must be between 1 and {MAX_SIDE} pixels"),
            FormatError::BadSound => write!(f, "the sound format is outside its range"),
        }
    }
}

/// The largest picture side accepted, in pixels.
pub const MAX_SIDE: u32 = 16_384;

/// The sound properties of a description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundFormat {
    /// Samples per second per channel, 1 to 768 000.
    pub sample_rate: u32,
    /// Channels, 1 to 32.
    pub channels: u16,
    /// Bits per sample: 8, 16, 24 or 32.
    pub sample_bits: u16,
}

/// The media type and its properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Video { width: u32, height: u32 },
    Sound(SoundFormat),
}

/// A format description: the codec and what its media type needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatDescription {
    /// The codec's four-character code, such as `avc1` or `mp4a`, padded to four bytes with spaces when shorter.
    codec: [u8; 4],
    kind: Kind,
}

impl FormatDescription {
    /// A video description: the codec and the picture size in pixels.
    pub fn video(codec: &str, width: u32, height: u32) -> Result<FormatDescription, FormatError> {
        let codec = four_cc(codec)?;
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            return Err(FormatError::BadSize);
        }
        Ok(FormatDescription { codec, kind: Kind::Video { width, height } })
    }

    /// A sound description: the codec and the sound properties.
    pub fn sound(codec: &str, sound: SoundFormat) -> Result<FormatDescription, FormatError> {
        let codec = four_cc(codec)?;
        let rate_ok = (1..=768_000).contains(&sound.sample_rate);
        let channels_ok = (1..=32).contains(&sound.channels);
        let bits_ok = matches!(sound.sample_bits, 8 | 16 | 24 | 32);
        if !(rate_ok && channels_ok && bits_ok) {
            return Err(FormatError::BadSound);
        }
        Ok(FormatDescription { codec, kind: Kind::Sound(sound) })
    }

    /// The codec as text, without the padding.
    pub fn codec(&self) -> String {
        String::from_utf8_lossy(&self.codec).trim_end().to_string()
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    /// Whether the description is for a picture.
    pub fn is_video(&self) -> bool {
        matches!(self.kind, Kind::Video { .. })
    }

    /// Whether `other` describes the same codec and the same media properties, so one decoder serves both.
    pub fn matches(&self, other: &FormatDescription) -> bool {
        self == other
    }
}

/// Reads a four-character code: one to four printable ASCII characters, padded with spaces to four.
fn four_cc(text: &str) -> Result<[u8; 4], FormatError> {
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(FormatError::BadCodec);
    }
    let mut code = [b' '; 4];
    code[..text.len()].copy_from_slice(text.as_bytes());
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo() -> SoundFormat {
        SoundFormat { sample_rate: 48_000, channels: 2, sample_bits: 16 }
    }

    #[test]
    fn a_video_description_keeps_its_codec_and_size() {
        let d = FormatDescription::video("avc1", 1920, 1080).unwrap();
        assert!(d.is_video());
        assert_eq!(d.codec(), "avc1");
        assert_eq!(d.kind(), &Kind::Video { width: 1920, height: 1080 });
    }

    #[test]
    fn a_short_codec_is_padded_and_read_back_without_the_padding() {
        let d = FormatDescription::sound("mp4", stereo()).unwrap();
        assert_eq!(d.codec(), "mp4");
        assert_eq!(d.codec, *b"mp4 ");
    }

    #[test]
    fn a_codec_that_is_not_printable_ascii_is_refused() {
        assert_eq!(FormatDescription::video("", 1, 1), Err(FormatError::BadCodec));
        assert_eq!(FormatDescription::video("avc1x", 1, 1), Err(FormatError::BadCodec));
        assert_eq!(FormatDescription::video("a\u{7f}c", 1, 1), Err(FormatError::BadCodec));
    }

    #[test]
    fn picture_sizes_and_sound_properties_have_ranges() {
        assert_eq!(FormatDescription::video("avc1", 0, 1080), Err(FormatError::BadSize));
        assert_eq!(FormatDescription::video("avc1", 1, MAX_SIDE + 1), Err(FormatError::BadSize));
        assert!(FormatDescription::sound("lpcm", SoundFormat { sample_bits: 24, ..stereo() }).is_ok());
        assert_eq!(FormatDescription::sound("lpcm", SoundFormat { sample_bits: 12, ..stereo() }), Err(FormatError::BadSound));
        assert_eq!(FormatDescription::sound("lpcm", SoundFormat { channels: 0, ..stereo() }), Err(FormatError::BadSound));
        assert_eq!(FormatDescription::sound("lpcm", SoundFormat { sample_rate: 0, ..stereo() }), Err(FormatError::BadSound));
    }

    #[test]
    fn two_descriptions_match_only_when_codec_and_properties_agree() {
        let a = FormatDescription::sound("lpcm", stereo()).unwrap();
        let b = FormatDescription::sound("lpcm", stereo()).unwrap();
        let mono = FormatDescription::sound("lpcm", SoundFormat { channels: 1, ..stereo() }).unwrap();
        assert!(a.matches(&b));
        assert!(!a.matches(&mono));
    }
}

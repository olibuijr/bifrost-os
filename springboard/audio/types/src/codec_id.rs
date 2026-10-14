//! Format identifiers: the four-character codes that name a stream's encoding in a container or a file, mapped to
//! the encodings the audio service knows by name. The server decodes only linear PCM itself; the other encodings are
//! named here so that a container reader can report what it found.

/// A stream encoding, named by its four-character code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecId {
    /// Uncompressed linear PCM, code `lpcm`.
    LinearPcm,
    /// MPEG-4 AAC, code `aac `.
    Aac,
    /// MPEG-1 Layer 3, code `.mp3`.
    Mp3,
    /// Free Lossless Audio Codec, code `flac`.
    Flac,
    /// Lossless compressed PCM (ALAC), code `alac`.
    Alac,
}

impl CodecId {
    /// Every named encoding, in declaration order.
    pub const ALL: [CodecId; 5] = [CodecId::LinearPcm, CodecId::Aac, CodecId::Mp3, CodecId::Flac, CodecId::Alac];

    /// The four-character code of this encoding.
    pub const fn fourcc(self) -> [u8; 4] {
        match self {
            CodecId::LinearPcm => *b"lpcm",
            CodecId::Aac => *b"aac ",
            CodecId::Mp3 => *b".mp3",
            CodecId::Flac => *b"flac",
            CodecId::Alac => *b"alac",
        }
    }

    /// The encoding with this four-character code, or `None` when the code is not one this crate names.
    pub fn from_fourcc(code: [u8; 4]) -> Option<CodecId> {
        CodecId::ALL.into_iter().find(|codec| codec.fourcc() == code)
    }
}

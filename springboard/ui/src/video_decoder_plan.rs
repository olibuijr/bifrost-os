//! Video decoder choice: which decode path a video stream gets, from its codec, the hardware decode support the
//! GPU reports, the force-software setting and whether the decoder library is the expected one. The shell's media
//! module opens the decoder (libavcodec through VA-API, with a software fallback); this model is the rule it applies
//! and the apps can read.

/// The video codecs the shell decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
}

impl VideoCodec {
    /// The codec named by an MP4 sample entry four-character code: `avc1` or `avc3` for H.264, `hvc1` or `hev1`
    /// for HEVC. Other codes are not decoded and return `None`.
    pub fn from_sample_entry(code: &str) -> Option<Self> {
        match code {
            "avc1" | "avc3" => Some(VideoCodec::H264),
            "hvc1" | "hev1" => Some(VideoCodec::Hevc),
            _ => None,
        }
    }
}

/// The path a stream decodes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodePath {
    /// The GPU decodes the stream through its fixed-function unit.
    Hardware,
    /// The CPU decodes the stream.
    Software,
}

impl DecodePath {
    /// The path to reopen on when this one fails, or `None` when software is already the last resort.
    pub fn fallback(self) -> Option<DecodePath> {
        match self {
            DecodePath::Hardware => Some(DecodePath::Software),
            DecodePath::Software => None,
        }
    }
}

/// Which codecs the GPU reports a hardware decode entry point for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareSupport {
    pub h264: bool,
    pub hevc: bool,
}

impl HardwareSupport {
    pub fn supports(self, codec: VideoCodec) -> bool {
        match codec {
            VideoCodec::H264 => self.h264,
            VideoCodec::Hevc => self.hevc,
        }
    }
}

/// The inputs of the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecoderRequest {
    pub codec: VideoCodec,
    pub support: HardwareSupport,
    /// True when the software-only switch is set in the environment.
    pub software_forced: bool,
    /// True when the decoder library reports the major version the shell was built against.
    pub library_matches: bool,
}

/// Hardware when the switch allows it, the library matches and the GPU supports the codec; software otherwise.
pub fn choose(request: DecoderRequest) -> DecodePath {
    if !request.software_forced && request.library_matches && request.support.supports(request.codec) {
        DecodePath::Hardware
    } else {
        DecodePath::Software
    }
}

//! Video decoding for apps: which decoder a stream of a codec runs on. The desktop decodes in hardware when the GPU's
//! driver decodes the codec (the hardware path is preferred), and libavcodec decodes it in software otherwise. An app
//! gets the decision, not a decoder: no frame output is exposed to apps yet.
//!
//! Rules of this model:
//! - A codec the hardware reports is decoded in hardware. Any other codec is decoded in software.
//! - [`Preference::RequireHardware`] refuses a codec the hardware does not decode, instead of falling back.
//! - The codecs are the four the hardware path probes: H.264, HEVC, VP9 and AV1.

/// A video codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
    Vp9,
    Av1,
}

/// Where a codec is decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Hardware,
    Software,
}

/// Whether the caller accepts software decoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    /// Hardware when it decodes the codec, software otherwise.
    #[default]
    Automatic,
    /// Hardware only.
    RequireHardware,
}

/// Why a decoder cannot be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The caller required hardware and the hardware does not decode this codec.
    NoHardwarePath(Codec),
}

/// The codecs the hardware decodes. Built from the hardware probe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareSupport {
    pub h264: bool,
    pub hevc: bool,
    pub vp9: bool,
    pub av1: bool,
}

impl HardwareSupport {
    /// Whether the hardware decodes `codec`.
    pub fn supports(&self, codec: Codec) -> bool {
        match codec {
            Codec::H264 => self.h264,
            Codec::Hevc => self.hevc,
            Codec::Vp9 => self.vp9,
            Codec::Av1 => self.av1,
        }
    }
}

/// The decoder for `codec` under `preference`.
pub fn choose(codec: Codec, hardware: HardwareSupport, preference: Preference) -> Result<Backend, DecodeError> {
    match (hardware.supports(codec), preference) {
        (true, _) => Ok(Backend::Hardware),
        (false, Preference::Automatic) => Ok(Backend::Software),
        (false, Preference::RequireHardware) => Err(DecodeError::NoHardwarePath(codec)),
    }
}

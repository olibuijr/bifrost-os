//! Video decompression for apps: a decode session, the hardware decode query and the sample format check.
//!
//! A [`DecompressionSession`] takes a [`FormatDescription`] (codec, picture size and the parameter sets), accepts
//! length-prefixed samples through [`DecompressionSession::decode_frame`] and hands every decoded picture to the
//! output handler given at creation. The decoder is the one the shell's media stack runs: H.264 and HEVC through
//! libavcodec, on the render node's hardware decoder when it decodes the codec and in software otherwise.
//! [`hardware_decode_supported`] reports the render node's answer and [`can_accept`] checks a format without
//! opening a decoder.
//!
//! Limits of this build: only H.264 and HEVC decode (VP9 and AV1 are refused with [`SessionError::UnsupportedCodec`]),
//! no decoder can be added by a plug-in, and pictures are copied out unless zero-copy is requested, which leaves them
//! in the decoder's buffers as [`DecodedPixels::Buffer`].

use crate::video_decode::{choose, Codec, DecodeError, HardwareSupport, Preference};
use springboard::gfx::Image;
use springboard::media::hwdec::{nal, vaapi};
use springboard::media::{Codec as MediaCodec, DmaPicture, Frame, Pixels, VideoDecoder};
use std::io;

/// The largest picture side a session accepts, in samples.
pub const MAX_DIMENSION: u32 = 8192;

/// Why a session, format or sample is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// No decoder of this build handles the codec.
    UnsupportedCodec(Codec),
    /// The caller required hardware decode and the render node does not decode the codec.
    NoHardwarePath(Codec),
    /// The picture size or the parameter-set record is malformed.
    InvalidFormat(&'static str),
    /// The sample cannot be decoded.
    InvalidSample(&'static str),
    /// The decoder failed; the text is the decoder's own message.
    Decoder(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::UnsupportedCodec(c) => write!(f, "no decoder for {c:?} in this build"),
            SessionError::NoHardwarePath(c) => write!(f, "the hardware does not decode {c:?}"),
            SessionError::InvalidFormat(what) | SessionError::InvalidSample(what) => f.write_str(what),
            SessionError::Decoder(message) => f.write_str(message),
        }
    }
}

impl From<DecodeError> for SessionError {
    fn from(e: DecodeError) -> Self {
        match e {
            DecodeError::NoHardwarePath(codec) => SessionError::NoHardwarePath(codec),
        }
    }
}

fn decoder_error(e: io::Error) -> SessionError {
    SessionError::Decoder(e.to_string())
}

/// The codecs this build decodes: the set is fixed in the media stack.
pub fn is_decodable(codec: Codec) -> bool {
    matches!(codec, Codec::H264 | Codec::Hevc)
}

/// The codec a sample description format (a four-character code such as `avc1`) names, decodable or not.
/// None for a code no codec of this build is registered under.
pub fn codec_for_sample_entry(fourcc: &[u8; 4]) -> Option<Codec> {
    match fourcc {
        b"avc1" | b"avc3" => Some(Codec::H264),
        b"hvc1" | b"hev1" => Some(Codec::Hevc),
        b"vp09" => Some(Codec::Vp9),
        b"av01" => Some(Codec::Av1),
        _ => None,
    }
}

fn media_codec(codec: Codec) -> Result<MediaCodec, SessionError> {
    match codec {
        Codec::H264 => Ok(MediaCodec::H264),
        Codec::Hevc => Ok(MediaCodec::Hevc),
        Codec::Vp9 | Codec::Av1 => Err(SessionError::UnsupportedCodec(codec)),
    }
}

/// The shortest parameter-set record: the avcC header, or the 23-byte hvcC header with its array count.
fn min_parameter_record(codec: MediaCodec) -> usize {
    match codec {
        MediaCodec::H264 => 7,
        MediaCodec::Hevc => 23,
    }
}

/// The picture size and parameter sets of a stream. Built only through [`FormatDescription::new`], which checks them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatDescription {
    codec: Codec,
    width: u32,
    height: u32,
    extradata: Vec<u8>,
}

impl FormatDescription {
    /// Checks the size (1 to [`MAX_DIMENSION`] per side) and the parameter sets: an avcC or hvcC record (version 1
    /// and the fixed header present), or parameter sets already in Annex B form.
    pub fn new(codec: Codec, width: u32, height: u32, extradata: Vec<u8>) -> Result<Self, SessionError> {
        let media = media_codec(codec)?;
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(SessionError::InvalidFormat("the picture size must be 1 to 8192 samples a side"));
        }
        let annexb = nal::is_annexb(&extradata);
        if !annexb && (extradata.len() < min_parameter_record(media) || extradata[0] != 1) {
            return Err(SessionError::InvalidFormat("the parameter sets are not an avcC or hvcC record"));
        }
        Ok(FormatDescription { codec, width, height, extradata })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn extradata(&self) -> &[u8] {
        &self.extradata
    }
}

/// The codecs the render node decodes in hardware, from the libva probe (`None` when there is no usable node).
pub fn support_from_probe(probe: Option<&vaapi::Support>) -> HardwareSupport {
    probe.map_or_else(HardwareSupport::default, |s| HardwareSupport { h264: s.h264, hevc: s.hevc, vp9: s.vp9, av1: s.av1 })
}

/// The hardware decode support of this machine, probed once by the media stack.
pub fn hardware_support() -> HardwareSupport {
    support_from_probe(vaapi::probe())
}

/// Whether the hardware decodes `codec` (the hardware decode query of the reference).
pub fn hardware_decode_supported(codec: Codec) -> bool {
    hardware_support().supports(codec)
}

/// Whether a session can be created for `format` under `preference`, given the hardware's support. Opens no decoder.
pub fn can_accept(format: &FormatDescription, preference: Preference, hardware: HardwareSupport) -> Result<(), SessionError> {
    media_codec(format.codec)?;
    choose(format.codec, hardware, preference)?;
    Ok(())
}

/// One decoded picture in display order.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    /// Presentation time in microseconds, as the sample was submitted.
    pub presentation_us: i64,
    pub pixels: DecodedPixels,
}

/// The pixels of a decoded picture.
pub enum DecodedPixels {
    /// Luma and interleaved chroma copied out of the decoder, both `stride` bytes per row.
    Nv12 { luma: Vec<u8>, chroma: Vec<u8>, stride: u32 },
    /// Converted to premultiplied RGBA by the media stack.
    Rgba(Image),
    /// Left in the decoder's buffer for zero-copy import; the handle is the picture's dma-buf.
    Buffer(DmaPicture),
}

impl From<Frame> for DecodedFrame {
    fn from(frame: Frame) -> Self {
        let pixels = match frame.pixels {
            Pixels::Nv12 { y, uv, stride } => DecodedPixels::Nv12 { luma: y, chroma: uv, stride },
            Pixels::Rgba(image) => DecodedPixels::Rgba(image),
            Pixels::Dma(picture) => DecodedPixels::Buffer(picture),
        };
        DecodedFrame { width: frame.width, height: frame.height, presentation_us: frame.pts_us, pixels }
    }
}

/// A decompression session. Samples go in with [`decode_frame`](Self::decode_frame); pictures come out through the
/// output handler, in the order the decoder emits them.
pub struct DecompressionSession {
    decoder: Box<dyn VideoDecoder>,
    format: FormatDescription,
    length_size: usize,
    /// The parameter sets go in front of the next sample: at the start and after every flush.
    needs_parameter_sets: bool,
    output: Box<dyn FnMut(DecodedFrame)>,
}

impl DecompressionSession {
    /// Opens a session on the decoder the media stack chooses for `format`, refused when `preference` requires
    /// hardware the render node lacks.
    pub fn create(format: FormatDescription, preference: Preference, output: Box<dyn FnMut(DecodedFrame)>) -> Result<Self, SessionError> {
        can_accept(&format, preference, hardware_support())?;
        let decoder = springboard::media::hwdec::open(media_codec(format.codec)?).map_err(decoder_error)?;
        Self::with_decoder(decoder, format, output)
    }

    /// A session over a decoder the caller opened. The decoder is configured for the format at once.
    pub fn with_decoder(mut decoder: Box<dyn VideoDecoder>, format: FormatDescription, output: Box<dyn FnMut(DecodedFrame)>) -> Result<Self, SessionError> {
        let codec = media_codec(format.codec)?;
        decoder.configure(codec, &format.extradata, format.width, format.height).map_err(decoder_error)?;
        let length_size = nal::length_size(codec, &format.extradata);
        Ok(DecompressionSession { decoder, format, length_size, needs_parameter_sets: true, output })
    }

    pub fn format(&self) -> &FormatDescription {
        &self.format
    }

    /// Decodes one sample of length-prefixed NAL units with its presentation time in microseconds. Every picture the
    /// sample completes reaches the output handler before this returns.
    pub fn decode_frame(&mut self, sample: &[u8], presentation_us: i64) -> Result<(), SessionError> {
        if sample.is_empty() {
            return Err(SessionError::InvalidSample("the sample is empty"));
        }
        let codec = media_codec(self.format.codec)?;
        let mut access_unit = if self.needs_parameter_sets { nal::extradata_to_annexb(codec, &self.format.extradata) } else { Vec::new() };
        access_unit.extend(nal::to_annexb(sample, self.length_size));
        self.decoder.push(&access_unit, presentation_us).map_err(decoder_error)?;
        self.needs_parameter_sets = false;
        self.drain()
    }

    /// Drops every picture still in flight, as a seek does. The next sample carries the parameter sets again.
    pub fn flush(&mut self) -> Result<(), SessionError> {
        self.decoder.flush().map_err(decoder_error)?;
        self.needs_parameter_sets = true;
        Ok(())
    }

    /// Asks for zero-copy output (pictures stay in the decoder's buffers). Returns whether the decoder honours it.
    pub fn set_zero_copy(&mut self, on: bool) -> bool {
        self.decoder.set_zero_copy(on)
    }

    /// What is doing the decoding, for diagnostics (hardware or software, and the decoder's name).
    pub fn decoder_description(&self) -> String {
        self.decoder.describe()
    }

    fn drain(&mut self) -> Result<(), SessionError> {
        while let Some(frame) = self.decoder.pull().map_err(decoder_error)? {
            (self.output)(DecodedFrame::from(frame));
        }
        Ok(())
    }
}

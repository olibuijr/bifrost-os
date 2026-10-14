//! Decoded video frames for apps: the picture hand-off from a decoder to the app. A [`VideoFormat`] names the codec, the
//! picture size and the pixel format a stream delivers. A [`VideoFrame`] pairs one [`PixelBuffer`] with its
//! presentation time and is accepted only when the picture has the format's size and pixel format. A [`FrameStream`]
//! holds frames in presentation order and refuses a frame that is not later than the one before it.
//!
//! The decoder stays in the system layer. This model checks what the decoder hands over and does not decode anything.

use crate::pixel_buffer::PixelBuffer;
use crate::video_decode::Codec;
use bifrost_render::pixel_format::{self, FourCc};
use std::collections::VecDeque;

/// Why a frame or a format was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The picture width or height is zero.
    EmptySize,
    /// The pixel format code is not in the system's table.
    UnknownPixelFormat,
    /// The picture size or pixel format differs from the stream's format.
    FormatMismatch,
    /// The presentation time is not later than the previous frame's.
    NotInPresentationOrder,
}

/// The codec, picture size and pixel format of the frames a stream delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoFormat {
    codec: Codec,
    code: FourCc,
    width: usize,
    height: usize,
}

impl VideoFormat {
    /// A checked format. The size must be non-zero and the pixel format must be one the system knows.
    pub fn new(codec: Codec, code: FourCc, width: usize, height: usize) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::EmptySize);
        }
        if pixel_format::find(code).is_none() {
            return Err(FrameError::UnknownPixelFormat);
        }
        Ok(VideoFormat { codec, code, width, height })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    pub fn code(&self) -> FourCc {
        self.code
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Whether a picture has this format's pixel format and size.
    pub fn matches(&self, picture: &PixelBuffer) -> bool {
        picture.code() == self.code && picture.width() == self.width && picture.height() == self.height
    }
}

/// One decoded picture with its presentation time in microseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    presentation_us: i64,
    picture: PixelBuffer,
}

impl VideoFrame {
    /// A frame whose picture matches `format`.
    pub fn new(format: &VideoFormat, presentation_us: i64, picture: PixelBuffer) -> Result<Self, FrameError> {
        if !format.matches(&picture) {
            return Err(FrameError::FormatMismatch);
        }
        Ok(VideoFrame { presentation_us, picture })
    }

    pub fn presentation_us(&self) -> i64 {
        self.presentation_us
    }

    pub fn picture(&self) -> &PixelBuffer {
        &self.picture
    }

    /// Gives the picture to the caller, for example to lock it and read its planes.
    pub fn into_picture(self) -> PixelBuffer {
        self.picture
    }
}

/// Frames of one format, held in presentation order until the consumer takes them.
#[derive(Debug)]
pub struct FrameStream {
    format: VideoFormat,
    last_us: Option<i64>,
    frames: VecDeque<VideoFrame>,
}

impl FrameStream {
    pub fn new(format: VideoFormat) -> Self {
        FrameStream { format, last_us: None, frames: VecDeque::new() }
    }

    pub fn format(&self) -> &VideoFormat {
        &self.format
    }

    /// Queues a picture. It must match the format and its time must be later than every earlier frame's.
    pub fn push(&mut self, presentation_us: i64, picture: PixelBuffer) -> Result<(), FrameError> {
        if self.last_us.is_some_and(|last| presentation_us <= last) {
            return Err(FrameError::NotInPresentationOrder);
        }
        let frame = VideoFrame::new(&self.format, presentation_us, picture)?;
        self.last_us = Some(presentation_us);
        self.frames.push_back(frame);
        Ok(())
    }

    /// The oldest queued frame, or None when the queue is empty.
    pub fn next_frame(&mut self) -> Option<VideoFrame> {
        self.frames.pop_front()
    }

    /// How many frames wait to be taken.
    pub fn pending(&self) -> usize {
        self.frames.len()
    }
}

//! Photo capture settings: the file format of a still photo, its quality, the self-timer delay and the square crop.
//! This is the settings model of a photo output. `PhotoSettings::plan` checks the settings and returns the part of
//! a frame the photo keeps. Pure: no frame is captured or encoded here. The Camera app's JPEG writer takes the same
//! format and quality values.

use crate::capture_shot::ImageFormat;

/// The self-timer delays a photo may start after, in seconds. Zero takes the photo at once.
pub const SELF_TIMER_SECONDS: [u32; 3] = [0, 3, 10];

/// Why a photo request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhotoError {
    /// The quality of a lossy format is not a finite number from 0 to 1.
    InvalidQuality,
    /// The self-timer delay is not one of `SELF_TIMER_SECONDS`.
    UnsupportedTimer { seconds: u32 },
    /// The frame has no width or no height.
    EmptyFrame,
}

/// The settings of one photo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoSettings {
    pub format: ImageFormat,
    /// Quality of a lossy format, from 0 to 1. PNG is lossless and ignores it.
    pub quality: f32,
    /// Seconds to wait before the shutter fires. One of `SELF_TIMER_SECONDS`.
    pub self_timer_seconds: u32,
    /// Keep only the largest centred square of the frame.
    pub square: bool,
}

impl Default for PhotoSettings {
    fn default() -> Self {
        Self { format: ImageFormat::Jpeg, quality: 0.9, self_timer_seconds: 0, square: false }
    }
}

/// A rectangle inside a frame, in frame pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PhotoSettings {
    /// Checks the settings: a lossy format needs a quality from 0 to 1, and the timer must be a supported delay.
    pub fn validate(&self) -> Result<(), PhotoError> {
        if self.format.has_quality() && !(self.quality.is_finite() && (0.0..=1.0).contains(&self.quality)) {
            return Err(PhotoError::InvalidQuality);
        }
        if !SELF_TIMER_SECONDS.contains(&self.self_timer_seconds) {
            return Err(PhotoError::UnsupportedTimer { seconds: self.self_timer_seconds });
        }
        Ok(())
    }

    /// Checks the settings against a frame of `width` by `height`, and returns the part of the frame the photo
    /// keeps. A square photo keeps the largest square at the centre; any other photo keeps the whole frame.
    pub fn plan(&self, width: u32, height: u32) -> Result<Crop, PhotoError> {
        self.validate()?;
        if width == 0 || height == 0 {
            return Err(PhotoError::EmptyFrame);
        }
        if !self.square {
            return Ok(Crop { x: 0, y: 0, width, height });
        }
        let side = width.min(height);
        Ok(Crop { x: (width - side) / 2, y: (height - side) / 2, width: side, height: side })
    }
}

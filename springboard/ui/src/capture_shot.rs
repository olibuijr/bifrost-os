//! Single-frame screenshot settings: the output size, the source area, the pointer and shadow options, and the file
//! format of one still image. `ShotConfig::plan` checks the settings against the display being captured and returns
//! the source area and the output size the capture will use. Pure: nothing is captured or encoded here.

use crate::Rect;

/// The largest output side this model accepts, in pixels. This is a BifrOSt bound, not a value from the reference.
pub const MAX_SIDE: u32 = 8192;

/// The file formats a still image can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// Lossless. The quality setting does not apply.
    Png,
    /// Lossy. The quality setting applies.
    Jpeg,
}

impl ImageFormat {
    /// The file extension for the format, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }

    /// Whether the format has a quality setting.
    pub fn has_quality(self) -> bool {
        matches!(self, ImageFormat::Jpeg)
    }
}

/// Why a screenshot request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotError {
    /// The output width or height is zero or above `MAX_SIDE`.
    InvalidSize,
    /// The quality of a lossy format is not a finite number from 0 to 1.
    InvalidQuality,
    /// The display being captured has no area, or the source area is empty.
    EmptySource,
    /// The source area reaches outside the display.
    SourceOutside,
}

/// The settings of one screenshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ShotConfig {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    pub format: ImageFormat,
    /// Encoding quality from 0 to 1, used only by a lossy format.
    pub quality: f32,
    pub shows_cursor: bool,
    pub ignores_shadows: bool,
    /// The part of the display to capture, in display pixels. `None` means the whole display.
    pub source: Option<Rect>,
}

/// What a checked request will capture and produce.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotPlan {
    /// The area of the display that is captured, in display pixels.
    pub source: Rect,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
}

impl Default for ShotConfig {
    fn default() -> Self {
        ShotConfig {
            width: 1920,
            height: 1080,
            format: ImageFormat::Png,
            quality: 0.9,
            shows_cursor: false,
            ignores_shadows: false,
            source: None,
        }
    }
}

impl ShotConfig {
    /// Checks the output size and, for a lossy format, the quality.
    pub fn validate(&self) -> Result<(), ShotError> {
        if self.width == 0 || self.height == 0 || self.width > MAX_SIDE || self.height > MAX_SIDE {
            return Err(ShotError::InvalidSize);
        }
        if self.format.has_quality() && !(self.quality.is_finite() && (0.0..=1.0).contains(&self.quality)) {
            return Err(ShotError::InvalidQuality);
        }
        Ok(())
    }

    /// Checks the settings against a display of `display_w` by `display_h` pixels and returns what the capture will
    /// use. A source area must lie inside the display and have a positive size.
    pub fn plan(&self, display_w: u32, display_h: u32) -> Result<ShotPlan, ShotError> {
        self.validate()?;
        if display_w == 0 || display_h == 0 {
            return Err(ShotError::EmptySource);
        }
        let source = match self.source {
            None => Rect { x: 0.0, y: 0.0, w: display_w as f32, h: display_h as f32 },
            Some(r) => {
                if !(r.w > 0.0 && r.h > 0.0) {
                    return Err(ShotError::EmptySource);
                }
                let inside = r.x >= 0.0
                    && r.y >= 0.0
                    && r.x + r.w <= display_w as f32
                    && r.y + r.h <= display_h as f32;
                if !inside {
                    return Err(ShotError::SourceOutside);
                }
                r
            }
        };
        Ok(ShotPlan { source, width: self.width, height: self.height })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_plan_captures_the_whole_display_at_the_output_size() {
        let plan = ShotConfig::default().plan(2560, 1600).unwrap();
        assert_eq!(plan.source, Rect { x: 0.0, y: 0.0, w: 2560.0, h: 1600.0 });
        assert_eq!((plan.width, plan.height), (1920, 1080));
    }

    #[test]
    fn a_zero_or_oversized_output_side_is_refused() {
        let mut c = ShotConfig::default();
        c.width = 0;
        assert_eq!(c.validate(), Err(ShotError::InvalidSize));
        c.width = MAX_SIDE + 1;
        assert_eq!(c.validate(), Err(ShotError::InvalidSize));
        c.width = MAX_SIDE;
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn quality_is_checked_only_for_a_lossy_format() {
        let mut c = ShotConfig::default();
        c.quality = 1.5;
        assert_eq!(c.validate(), Ok(()));
        c.format = ImageFormat::Jpeg;
        assert_eq!(c.validate(), Err(ShotError::InvalidQuality));
        c.quality = f32::NAN;
        assert_eq!(c.validate(), Err(ShotError::InvalidQuality));
        c.quality = 0.0;
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn a_source_area_inside_the_display_is_kept_as_given() {
        let mut c = ShotConfig::default();
        c.source = Some(Rect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 });
        let plan = c.plan(1920, 1080).unwrap();
        assert_eq!(plan.source, Rect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 });
    }

    #[test]
    fn a_source_area_outside_the_display_or_empty_is_refused() {
        let mut c = ShotConfig::default();
        c.source = Some(Rect { x: 1800.0, y: 0.0, w: 200.0, h: 100.0 });
        assert_eq!(c.plan(1920, 1080), Err(ShotError::SourceOutside));
        c.source = Some(Rect { x: -1.0, y: 0.0, w: 100.0, h: 100.0 });
        assert_eq!(c.plan(1920, 1080), Err(ShotError::SourceOutside));
        c.source = Some(Rect { x: 0.0, y: 0.0, w: 0.0, h: 100.0 });
        assert_eq!(c.plan(1920, 1080), Err(ShotError::EmptySource));
        assert_eq!(ShotConfig::default().plan(0, 1080), Err(ShotError::EmptySource));
    }

    #[test]
    fn each_format_has_its_extension_and_only_jpeg_has_quality() {
        assert_eq!(ImageFormat::Png.extension(), "png");
        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
        assert!(!ImageFormat::Png.has_quality());
        assert!(ImageFormat::Jpeg.has_quality());
    }
}

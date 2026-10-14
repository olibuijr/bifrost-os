//! Drawable sizing for GPU-backed layers.
//!
//! An app that presents frames on the GPU asks for a drawable: a pixel buffer sized from the layer's size in points,
//! the display scale and a limit on the longer edge. This module validates the pixel format and the number of drawables
//! in the swap set (two or three, the range the presenter supports), computes the pixel size and the memory the set
//! takes. The GPU presenter in the system layer owns the buffers and the frame pacing.

/// Pixel formats a drawable may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Four bytes per pixel, blue, green, red, alpha.
    Bgra8,
    /// Eight bytes per pixel, four half-precision float channels.
    Rgba16Float,
}

impl PixelFormat {
    /// Bytes one pixel occupies.
    pub fn bytes_per_pixel(self) -> u64 {
        match self {
            Self::Bgra8 => 4,
            Self::Rgba16Float => 8,
        }
    }
}

/// Fewest drawables in a swap set: one being shown, one being drawn.
pub const MIN_DRAWABLE_COUNT: u8 = 2;
/// Most drawables in a swap set.
pub const MAX_DRAWABLE_COUNT: u8 = 3;
/// Default limit on the longer edge of a drawable, in pixels.
pub const DEFAULT_MAX_EDGE: u32 = 8192;

/// Why a drawable request is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawableError {
    /// The count is outside `MIN_DRAWABLE_COUNT..=MAX_DRAWABLE_COUNT`.
    DrawableCount(u8),
    /// The edge limit is zero.
    MaxEdge,
    /// The scale is not a positive finite number.
    Scale,
    /// A size is negative or not finite.
    Size,
}

/// The validated settings of a drawable swap set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawableConfig {
    pub format: PixelFormat,
    pub count: u8,
    pub max_edge: u32,
}

impl DrawableConfig {
    /// Settings from a format, a drawable count and an edge limit.
    pub fn new(format: PixelFormat, count: u8, max_edge: u32) -> Result<Self, DrawableError> {
        if !(MIN_DRAWABLE_COUNT..=MAX_DRAWABLE_COUNT).contains(&count) {
            return Err(DrawableError::DrawableCount(count));
        }
        if max_edge == 0 {
            return Err(DrawableError::MaxEdge);
        }
        Ok(Self { format, count, max_edge })
    }

    /// The pixel size of one drawable for a layer of `points` (width, height) at `scale` pixels per point.
    /// The aspect ratio is kept and the longer edge is held to `max_edge`; sides round to the nearest pixel.
    pub fn drawable_size(&self, points: (f32, f32), scale: f32) -> Result<(u32, u32), DrawableError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(DrawableError::Scale);
        }
        let (w, h) = points;
        if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
            return Err(DrawableError::Size);
        }
        let (mut pw, mut ph) = ((w * scale).round() as f64, (h * scale).round() as f64);
        let longest = pw.max(ph);
        if longest > self.max_edge as f64 {
            let k = self.max_edge as f64 / longest;
            pw = (pw * k).round();
            ph = (ph * k).round();
        }
        Ok((pw as u32, ph as u32))
    }

    /// Memory the whole swap set takes at a pixel size.
    pub fn memory_bytes(&self, pixels: (u32, u32)) -> u64 {
        pixels.0 as u64 * pixels.1 as u64 * self.format.bytes_per_pixel() * self.count as u64
    }
}

impl Default for DrawableConfig {
    fn default() -> Self {
        Self { format: PixelFormat::Bgra8, count: MAX_DRAWABLE_COUNT, max_edge: DEFAULT_MAX_EDGE }
    }
}

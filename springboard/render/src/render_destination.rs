//! Render destinations and accumulators (`CIRenderDestination`, `CIImageAccumulator`).
//!
//! A `Destination` is a caller-owned buffer that a filtered image is rendered into. It has a pixel
//! format (8-bit premultiplied, 8-bit straight, or 32-bit float premultiplied), a colour space tag
//! (sRGB only for now) and a placement: `render_into` copies a source rectangle to a destination
//! origin, clipping both to their bounds, and returns how many pixels were written.
//!
//! An `Accumulator` is a float image that keeps state across frames. Each `feed` composites a new
//! frame over the retained previous state (source-over with a retain factor), which gives painting
//! and feedback effects: retain 0 shows only the latest frame, retain 1 never fades.
//!
//! Pure CPU model with host unit tests. The GPU path (GL dispatch) is not wired.
use crate::gfx::Image;

/// The layout of a destination buffer's pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 8 bits per channel, premultiplied alpha (RGBA_8888 from the software rasteriser).
    Rgba8Premultiplied,
    /// 8 bits per channel, straight (unpremultiplied) alpha.
    Rgba8Straight,
    /// 32-bit float per channel, premultiplied alpha, little-endian.
    RgbaF32Premultiplied,
}

impl PixelFormat {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Rgba8Premultiplied | PixelFormat::Rgba8Straight => 4,
            PixelFormat::RgbaF32Premultiplied => 16,
        }
    }
}

/// The colour space of a destination. Only sRGB is produced; the tag records what the bytes mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
}

/// A rectangle in pixels. `x` and `y` may be negative (the rectangle is then clipped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: i64,
    pub y: i64,
    pub w: usize,
    pub h: usize,
}

/// A caller-owned pixel buffer that images are rendered into. Rows are tightly packed.
#[derive(Clone, Debug, PartialEq)]
pub struct Destination {
    pub w: usize,
    pub h: usize,
    pub format: PixelFormat,
    pub colour_space: ColorSpace,
    pub bytes: Vec<u8>,
}

impl Destination {
    /// A zeroed destination. `None` for an empty size or a size that overflows.
    pub fn new(w: usize, h: usize, format: PixelFormat) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        let len = w.checked_mul(h)?.checked_mul(format.bytes_per_pixel())?;
        Some(Destination { w, h, format, colour_space: ColorSpace::Srgb, bytes: vec![0; len] })
    }

    /// The pixel at (x, y) as straight-or-premultiplied floats in the format's own alpha mode, 0..1.
    /// `None` outside the buffer.
    pub fn pixel(&self, x: usize, y: usize) -> Option<[f32; 4]> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let bpp = self.format.bytes_per_pixel();
        let at = (y * self.w + x) * bpp;
        let b = &self.bytes[at..at + bpp];
        Some(match self.format {
            PixelFormat::RgbaF32Premultiplied => {
                let mut v = [0.0f32; 4];
                for (i, c) in v.iter_mut().enumerate() {
                    let mut q = [0u8; 4];
                    q.copy_from_slice(&b[i * 4..i * 4 + 4]);
                    *c = f32::from_le_bytes(q);
                }
                v
            }
            _ => [b[0] as f32 / 255.0, b[1] as f32 / 255.0, b[2] as f32 / 255.0, b[3] as f32 / 255.0],
        })
    }
}

/// Renders the part of `src` covered by `src_rect` into `dest` with its top-left at `dest_origin`.
/// Both rectangles are clipped to their images. Returns the number of pixels written, or `None`
/// when the source pixel count does not match its size.
pub fn render_into(src: &Image, src_rect: Region, dest: &mut Destination, dest_origin: (i64, i64)) -> Option<usize> {
    if src.w.checked_mul(src.h)? != src.px.len() {
        return None;
    }
    // Clip the source rectangle to the image, then move the destination origin by the same amount.
    let sx0 = src_rect.x.max(0);
    let sy0 = src_rect.y.max(0);
    let sx1 = (src_rect.x + src_rect.w as i64).min(src.w as i64);
    let sy1 = (src_rect.y + src_rect.h as i64).min(src.h as i64);
    if sx1 <= sx0 || sy1 <= sy0 {
        return Some(0);
    }
    let dx0 = dest_origin.0 + (sx0 - src_rect.x);
    let dy0 = dest_origin.1 + (sy0 - src_rect.y);
    // Clip to the destination.
    let cx0 = dx0.max(0);
    let cy0 = dy0.max(0);
    let cx1 = (dx0 + (sx1 - sx0)).min(dest.w as i64);
    let cy1 = (dy0 + (sy1 - sy0)).min(dest.h as i64);
    if cx1 <= cx0 || cy1 <= cy0 {
        return Some(0);
    }
    let mut written = 0usize;
    for dy in cy0..cy1 {
        let sy = (sy0 + (dy - dy0)) as usize;
        for dx in cx0..cx1 {
            let sx = (sx0 + (dx - dx0)) as usize;
            let p = src.px[sy * src.w + sx];
            write_pixel(dest, dx as usize, dy as usize, p);
            written += 1;
        }
    }
    Some(written)
}

/// Writes one premultiplied 8-bit pixel (R in the low byte) in the destination's format.
fn write_pixel(dest: &mut Destination, x: usize, y: usize, px: u32) {
    let bpp = dest.format.bytes_per_pixel();
    let at = (y * dest.w + x) * bpp;
    let c = [px as u8, (px >> 8) as u8, (px >> 16) as u8, (px >> 24) as u8];
    match dest.format {
        PixelFormat::Rgba8Premultiplied => dest.bytes[at..at + 4].copy_from_slice(&c),
        PixelFormat::Rgba8Straight => {
            let a = c[3] as u32;
            let mut out = [0u8; 4];
            for i in 0..3 {
                out[i] = if a == 0 { 0 } else { ((c[i] as u32 * 255 + a / 2) / a).min(255) as u8 };
            }
            out[3] = c[3];
            dest.bytes[at..at + 4].copy_from_slice(&out);
        }
        PixelFormat::RgbaF32Premultiplied => {
            for i in 0..4 {
                let v = c[i] as f32 / 255.0;
                dest.bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

/// A float image that keeps state across frames. Premultiplied float RGBA, one value per channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Accumulator {
    w: usize,
    h: usize,
    state: Vec<[f32; 4]>,
}

impl Accumulator {
    /// An empty (transparent) accumulator. `None` for an empty or overflowing size.
    pub fn new(w: usize, h: usize) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        Some(Accumulator { w, h, state: vec![[0.0; 4]; w.checked_mul(h)?] })
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    /// Clears the state to transparent.
    pub fn clear(&mut self) {
        for p in &mut self.state {
            *p = [0.0; 4];
        }
    }

    /// Composites `frame` over the retained state: `out = frame + retain * state * (1 - frame.a)`.
    /// `retain` is clamped to 0..1. Returns `None` when the frame size differs from the accumulator.
    pub fn feed(&mut self, frame: &Image, retain: f32) -> Option<()> {
        if frame.w != self.w || frame.h != self.h || frame.px.len() != self.state.len() {
            return None;
        }
        let retain = if retain.is_nan() { 0.0 } else { retain.clamp(0.0, 1.0) };
        for (s, &px) in self.state.iter_mut().zip(&frame.px) {
            let f = unpack_f32(px);
            let keep = retain * (1.0 - f[3]);
            *s = [f[0] + s[0] * keep, f[1] + s[1] * keep, f[2] + s[2] * keep, f[3] + s[3] * keep];
        }
        Some(())
    }

    /// The current state as a premultiplied 8-bit image. Channels are clamped to 0..1 and colour
    /// never exceeds alpha.
    pub fn snapshot(&self) -> Image {
        let px = self
            .state
            .iter()
            .map(|s| {
                let a = s[3].clamp(0.0, 1.0);
                let r = s[0].clamp(0.0, a);
                let g = s[1].clamp(0.0, a);
                let b = s[2].clamp(0.0, a);
                pack8([r, g, b, a])
            })
            .collect();
        Image { w: self.w, h: self.h, px }
    }
}

fn unpack_f32(px: u32) -> [f32; 4] {
    [
        (px & 0xff) as f32 / 255.0,
        ((px >> 8) & 0xff) as f32 / 255.0,
        ((px >> 16) & 0xff) as f32 / 255.0,
        ((px >> 24) & 0xff) as f32 / 255.0,
    ]
}

fn pack8(v: [f32; 4]) -> u32 {
    let q = |c: f32| (c * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
    q(v[0]) | (q(v[1]) << 8) | (q(v[2]) << 16) | (q(v[3]) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    const RED: u32 = 0xff00_00ff; // opaque red, R in the low byte
    const CLEAR: u32 = 0;

    #[test]
    fn destination_rejects_empty_and_sets_length() {
        assert!(Destination::new(0, 4, PixelFormat::Rgba8Premultiplied).is_none());
        let d = Destination::new(3, 2, PixelFormat::RgbaF32Premultiplied).unwrap();
        assert_eq!(d.bytes.len(), 3 * 2 * 16);
        assert_eq!(d.colour_space, ColorSpace::Srgb);
    }

    #[test]
    fn render_copies_region_to_origin() {
        // 2x2 image: red top-left, transparent elsewhere.
        let src = Image { w: 2, h: 2, px: vec![RED, CLEAR, CLEAR, CLEAR] };
        let mut d = Destination::new(3, 3, PixelFormat::Rgba8Premultiplied).unwrap();
        let n = render_into(&src, Region { x: 0, y: 0, w: 2, h: 2 }, &mut d, (1, 1)).unwrap();
        assert_eq!(n, 4);
        assert_eq!(d.pixel(1, 1), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(d.pixel(0, 0), Some([0.0; 4]));
    }

    #[test]
    fn render_clips_negative_origin_and_overflow() {
        let src = solid(4, 4, RED);
        let mut d = Destination::new(2, 2, PixelFormat::Rgba8Premultiplied).unwrap();
        // Origin (-1, -1): only the 2x2 bottom-right of the source lands in the destination.
        let n = render_into(&src, Region { x: 0, y: 0, w: 4, h: 4 }, &mut d, (-1, -1)).unwrap();
        assert_eq!(n, 4);
        // Source crop starting at (3, 3) with a 2x2 size keeps one pixel of the 4x4 image.
        let mut d2 = Destination::new(4, 4, PixelFormat::Rgba8Premultiplied).unwrap();
        let n2 = render_into(&src, Region { x: 3, y: 3, w: 2, h: 2 }, &mut d2, (0, 0)).unwrap();
        assert_eq!(n2, 1);
        // A crop entirely outside the image writes nothing.
        let n3 = render_into(&src, Region { x: 10, y: 0, w: 2, h: 2 }, &mut d2, (0, 0)).unwrap();
        assert_eq!(n3, 0);
    }

    #[test]
    fn straight_alpha_unpremultiplies() {
        // Half-transparent red, premultiplied: R = 128, A = 128.
        let src = Image { w: 1, h: 1, px: vec![128 | (128 << 24)] };
        let mut d = Destination::new(1, 1, PixelFormat::Rgba8Straight).unwrap();
        render_into(&src, Region { x: 0, y: 0, w: 1, h: 1 }, &mut d, (0, 0)).unwrap();
        assert_eq!(&d.bytes[..], &[255, 0, 0, 128]);
    }

    #[test]
    fn float_format_keeps_premultiplied_values() {
        let src = Image { w: 1, h: 1, px: vec![255 | (255 << 24)] };
        let mut d = Destination::new(1, 1, PixelFormat::RgbaF32Premultiplied).unwrap();
        render_into(&src, Region { x: 0, y: 0, w: 1, h: 1 }, &mut d, (0, 0)).unwrap();
        assert_eq!(d.pixel(0, 0), Some([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn render_rejects_bad_pixel_count() {
        let bad = Image { w: 2, h: 2, px: vec![0; 3] };
        let mut d = Destination::new(2, 2, PixelFormat::Rgba8Premultiplied).unwrap();
        assert!(render_into(&bad, Region { x: 0, y: 0, w: 2, h: 2 }, &mut d, (0, 0)).is_none());
    }

    #[test]
    fn accumulator_opaque_frame_replaces_state() {
        let mut acc = Accumulator::new(1, 1).unwrap();
        acc.feed(&solid(1, 1, RED), 1.0).unwrap();
        acc.feed(&solid(1, 1, 0xff00_ff00), 1.0).unwrap(); // opaque green
        assert_eq!(acc.snapshot().px, vec![0xff00_ff00]);
    }

    #[test]
    fn accumulator_transparent_frames_fade_by_retain() {
        let mut acc = Accumulator::new(1, 1).unwrap();
        acc.feed(&solid(1, 1, RED), 0.0).unwrap();
        // Transparent frame with retain 0.5 keeps half of the previous state.
        acc.feed(&solid(1, 1, CLEAR), 0.5).unwrap();
        assert_eq!(acc.snapshot().px, vec![128 | (128 << 24)]);
        acc.feed(&solid(1, 1, CLEAR), 0.5).unwrap();
        assert_eq!(acc.snapshot().px, vec![64 | (64 << 24)]);
    }

    #[test]
    fn accumulator_retain_zero_keeps_latest_frame_only() {
        let mut acc = Accumulator::new(1, 1).unwrap();
        acc.feed(&solid(1, 1, RED), 0.0).unwrap();
        acc.feed(&solid(1, 1, CLEAR), 0.0).unwrap();
        assert_eq!(acc.snapshot().px, vec![0]);
    }

    #[test]
    fn accumulator_rejects_size_change_and_clears() {
        let mut acc = Accumulator::new(2, 2).unwrap();
        assert!(acc.feed(&solid(1, 1, RED), 1.0).is_none());
        acc.feed(&solid(2, 2, RED), 1.0).unwrap();
        acc.clear();
        assert_eq!(acc.snapshot().px, vec![0; 4]);
    }
}

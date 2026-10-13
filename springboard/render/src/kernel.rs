//! Per-pixel kernels over the renderer's packed pixels: a sampler that reads an image at continuous
//! positions, a colour kernel that maps each pixel's colour, a warp kernel that moves each destination
//! pixel to a source position and samples it, and a two-image blend kernel with the separable blend
//! modes. Pixels are the premultiplied `u32` values of `gfx` (red in the low byte). Kernels work on
//! straight (unpremultiplied) colour in 0..=1, except that sampling interpolates premultiplied values so
//! that transparent pixels do not bleed their colour into their neighbours. A kernel is a Rust closure:
//! there is no kernel language and no GPU path yet, and nothing here is wired into an app.

use crate::gfx::rgba;

/// How a sampler treats positions outside its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    /// Positions outside the image take the nearest edge pixel.
    Edge,
    /// Positions outside the image are transparent.
    Clear,
}

/// How a sampler reconstructs a colour at a continuous position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// The pixel that contains the position.
    Nearest,
    /// Bilinear interpolation of the four nearest pixel centres.
    Linear,
}

/// A source image that kernels read through. Pixel (i, j) covers the area [i, i+1) x [j, j+1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sampler {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
    pub wrap: Wrap,
}

impl Sampler {
    /// Wraps `pixels` as a sampler. Returns `None` for an empty size or when the pixel count does not match.
    pub fn new(width: usize, height: usize, pixels: Vec<u32>, wrap: Wrap) -> Option<Self> {
        (width > 0 && height > 0 && pixels.len() == width.checked_mul(height)?)
            .then_some(Sampler { width, height, pixels, wrap })
    }

    /// The premultiplied colour of pixel (x, y), with the wrap rule applied to positions outside the image.
    pub fn fetch(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (self.width as i64, self.height as i64);
        let (x, y) = match self.wrap {
            Wrap::Edge => (x.clamp(0, w - 1), y.clamp(0, h - 1)),
            Wrap::Clear if x < 0 || y < 0 || x >= w || y >= h => return [0.0; 4],
            Wrap::Clear => (x, y),
        };
        premul(self.pixels[y as usize * self.width + x as usize])
    }

    /// The premultiplied colour at a continuous position. Non-finite positions read as transparent.
    pub fn sample(&self, x: f32, y: f32, filter: Filter) -> [f32; 4] {
        if !x.is_finite() || !y.is_finite() {
            return [0.0; 4];
        }
        match filter {
            Filter::Nearest => self.fetch(x.floor() as i64, y.floor() as i64),
            Filter::Linear => {
                // Pixel centres sit at i + 0.5, so the four neighbours of (x, y) start at floor(x - 0.5).
                let (fx, fy) = (x - 0.5, y - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let top = lerp4(self.fetch(x0, y0), self.fetch(x0 + 1, y0), tx);
                let bottom = lerp4(self.fetch(x0, y0 + 1), self.fetch(x0 + 1, y0 + 1), tx);
                lerp4(top, bottom, ty)
            }
        }
    }
}

/// A colour kernel: maps every pixel's straight colour through `f`. The output has the source's size.
pub fn color_kernel(src: &Sampler, f: impl Fn([f32; 4]) -> [f32; 4]) -> Vec<u32> {
    src.pixels.iter().map(|&px| pack(f(straight(premul(px))))).collect()
}

/// A warp kernel: the centre of each destination pixel, (x + 0.5, y + 0.5), is moved to the source
/// position `map` returns, and that position is sampled. Returns `None` for an empty output size.
pub fn warp_kernel(
    src: &Sampler,
    width: usize,
    height: usize,
    filter: Filter,
    map: impl Fn(f32, f32) -> (f32, f32),
) -> Option<Vec<u32>> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(width.checked_mul(height)?);
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = map(x as f32 + 0.5, y as f32 + 0.5);
            out.push(pack(straight(src.sample(sx, sy, filter))));
        }
    }
    Some(out)
}

/// A separable blend mode. `Normal` is plain source-over compositing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    HardLight,
    Darken,
    Lighten,
    Difference,
    Exclusion,
}

/// The blend function B(cb, cs) for one channel, with `cb` the background and `cs` the foreground.
fn blend_channel(mode: Blend, cb: f32, cs: f32) -> f32 {
    match mode {
        Blend::Normal => cs,
        Blend::Multiply => cb * cs,
        Blend::Screen => cb + cs - cb * cs,
        Blend::Overlay => blend_channel(Blend::HardLight, cs, cb),
        Blend::HardLight => {
            if cs <= 0.5 {
                cb * 2.0 * cs
            } else {
                let s = 2.0 * cs - 1.0;
                cb + s - cb * s
            }
        }
        Blend::Darken => cb.min(cs),
        Blend::Lighten => cb.max(cs),
        Blend::Difference => (cb - cs).abs(),
        Blend::Exclusion => cb + cs - 2.0 * cb * cs,
    }
}

/// A blend kernel: composites `fg` over `bg` pixel by pixel with `mode`. Both buffers must hold
/// `width * height` pixels; returns `None` otherwise. Alpha follows source-over, and colour is blended
/// where both pixels are opaque (the standard separable compositing rule).
pub fn blend_kernel(fg: &[u32], bg: &[u32], width: usize, height: usize, mode: Blend) -> Option<Vec<u32>> {
    let n = width.checked_mul(height)?;
    if fg.len() != n || bg.len() != n {
        return None;
    }
    let out = fg
        .iter()
        .zip(bg)
        .map(|(&f, &b)| {
            let s = straight(premul(f));
            let d = straight(premul(b));
            let (a_s, a_b) = (s[3], d[3]);
            let a_o = a_s + a_b - a_s * a_b;
            if a_o <= 0.0 {
                return 0;
            }
            let channel = |i: usize| {
                let (cs, cb) = (s[i], d[i]);
                let premultiplied = (1.0 - a_b) * a_s * cs
                    + a_b * a_s * blend_channel(mode, cb, cs)
                    + (1.0 - a_s) * a_b * cb;
                premultiplied / a_o
            };
            pack([channel(0), channel(1), channel(2), a_o])
        })
        .collect();
    Some(out)
}

/// A packed premultiplied pixel as premultiplied floats in 0..=1.
fn premul(px: u32) -> [f32; 4] {
    let ch = |shift: u32| ((px >> shift) & 0xff) as f32 / 255.0;
    [ch(0), ch(8), ch(16), ch(24)]
}

/// Premultiplied floats to straight colour. Fully transparent input gives zero colour.
fn straight(p: [f32; 4]) -> [f32; 4] {
    let a = p[3];
    if a <= 0.0 {
        return [0.0; 4];
    }
    [
        (p[0] / a).clamp(0.0, 1.0),
        (p[1] / a).clamp(0.0, 1.0),
        (p[2] / a).clamp(0.0, 1.0),
        a.clamp(0.0, 1.0),
    ]
}

/// Straight colour in 0..=1 to a packed premultiplied pixel, rounding each channel to 8 bits.
fn pack(c: [f32; 4]) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if q(c[3]) == 0 {
        return 0;
    }
    rgba(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// The base of a unary image kernel, the counterpart of MPSUnaryImageKernel. A kernel reads one image and writes an
/// image of the same size. `encode` does the work, and `label` names the kernel in logs.
pub trait UnaryImageKernel {
    /// The name of the kernel, for logs.
    fn label(&self) -> &str;

    /// The output pixels for `src`: one packed pixel per source pixel, in the source's row order.
    fn encode(&self, src: &Sampler) -> Vec<u32>;
}

/// A colour kernel as a `UnaryImageKernel`: `map` gives the new straight colour of each pixel.
pub struct ColorMap<F: Fn([f32; 4]) -> [f32; 4]> {
    pub name: &'static str,
    pub map: F,
}

impl<F: Fn([f32; 4]) -> [f32; 4]> UnaryImageKernel for ColorMap<F> {
    fn label(&self) -> &str {
        self.name
    }

    fn encode(&self, src: &Sampler) -> Vec<u32> {
        color_kernel(src, &self.map)
    }
}

/// Runs `kernels` in order, each on the output of the one before it, and returns the last output. An empty list
/// returns a copy of `src`. Returns None when a kernel does not return one pixel per source pixel.
pub fn run_chain(src: &Sampler, kernels: &[&dyn UnaryImageKernel]) -> Option<Sampler> {
    let mut current = src.clone();
    for kernel in kernels {
        let pixels = kernel.encode(&current);
        current = Sampler::new(current.width, current.height, pixels, current.wrap)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: u32 = 0xff00_0000;
    const WHITE: u32 = 0xffff_ffff;

    struct Truncated;

    impl UnaryImageKernel for Truncated {
        fn label(&self) -> &str {
            "truncated"
        }

        fn encode(&self, _src: &Sampler) -> Vec<u32> {
            Vec::new()
        }
    }

    fn invert(c: [f32; 4]) -> [f32; 4] {
        [1.0 - c[0], 1.0 - c[1], 1.0 - c[2], c[3]]
    }

    #[test]
    fn color_map_labels_and_applies_its_function() {
        let src = Sampler::new(1, 1, vec![BLACK], Wrap::Edge).unwrap();
        let k = ColorMap { name: "invert", map: invert };
        assert_eq!(k.label(), "invert");
        assert_eq!(k.encode(&src), vec![WHITE]);
    }

    #[test]
    fn chain_applies_kernels_in_order() {
        let src = Sampler::new(2, 1, vec![BLACK, WHITE], Wrap::Clear).unwrap();
        let inv = ColorMap { name: "invert", map: invert };
        let twice = run_chain(&src, &[&inv, &inv]).unwrap();
        assert_eq!(twice.pixels, src.pixels);
        let once = run_chain(&src, &[&inv]).unwrap();
        assert_eq!(once.pixels, vec![WHITE, BLACK]);
        assert_eq!(once.width, 2);
        assert_eq!(once.wrap, Wrap::Clear);
    }

    fn whiten(c: [f32; 4]) -> [f32; 4] {
        [1.0, 1.0, 1.0, c[3]]
    }

    #[test]
    fn chain_order_matters() {
        let src = Sampler::new(1, 1, vec![BLACK], Wrap::Edge).unwrap();
        let white = ColorMap { name: "whiten", map: whiten };
        let inv = ColorMap { name: "invert", map: invert };
        assert_eq!(run_chain(&src, &[&white, &inv]).unwrap().pixels, vec![BLACK]);
        assert_eq!(run_chain(&src, &[&inv, &white]).unwrap().pixels, vec![WHITE]);
    }

    #[test]
    fn empty_chain_copies_the_source() {
        let src = Sampler::new(1, 2, vec![1, 2], Wrap::Edge).unwrap();
        assert_eq!(run_chain(&src, &[]).unwrap(), src);
    }

    #[test]
    fn kernel_with_wrong_output_size_fails_the_chain() {
        let src = Sampler::new(1, 1, vec![BLACK], Wrap::Edge).unwrap();
        assert!(run_chain(&src, &[&Truncated]).is_none());
    }

    #[test]
    fn sampler_rejects_empty_or_mismatched_sizes() {
        assert!(Sampler::new(2, 2, vec![0; 3], Wrap::Edge).is_none());
        assert!(Sampler::new(0, 2, vec![], Wrap::Edge).is_none());
        assert!(Sampler::new(2, 2, vec![0; 4], Wrap::Clear).is_some());
    }

    #[test]
    fn edge_wrap_repeats_border_and_clear_wrap_is_transparent() {
        let edge = Sampler::new(2, 1, vec![BLACK, WHITE], Wrap::Edge).unwrap();
        assert_eq!(edge.fetch(-5, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(edge.fetch(9, 0), [1.0, 1.0, 1.0, 1.0]);
        let clear = Sampler::new(2, 1, vec![BLACK, WHITE], Wrap::Clear).unwrap();
        assert_eq!(clear.fetch(-1, 0), [0.0; 4]);
        assert_eq!(clear.fetch(1, 0), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn nearest_picks_the_containing_pixel_and_linear_interpolates_centres() {
        let s = Sampler::new(2, 1, vec![BLACK, WHITE], Wrap::Edge).unwrap();
        // x = 1.0 is the edge between the two pixels: nearest takes pixel 1 (white).
        assert_eq!(s.sample(1.0, 0.5, Filter::Nearest)[0], 1.0);
        // x = 1.0 is halfway between the centres 0.5 and 1.5: linear gives 0.5.
        assert!((s.sample(1.0, 0.5, Filter::Linear)[0] - 0.5).abs() < 1e-6);
        // Clear wrap at x = 0.0 mixes transparency with black: alpha 0.5, colour 0.
        let c = Sampler::new(2, 1, vec![BLACK, WHITE], Wrap::Clear).unwrap();
        let edge = c.sample(0.0, 0.5, Filter::Linear);
        assert!((edge[3] - 0.5).abs() < 1e-6 && edge[0] == 0.0);
    }

    #[test]
    fn colour_kernel_inverts_an_opaque_red_to_cyan() {
        let s = Sampler::new(1, 1, vec![0xff00_00ff], Wrap::Edge).unwrap();
        let out = color_kernel(&s, |c| [1.0 - c[0], 1.0 - c[1], 1.0 - c[2], c[3]]);
        assert_eq!(out, vec![0xffff_ff00]);
    }

    #[test]
    fn warp_kernel_mirrors_a_row_and_clear_wrap_leaves_gaps() {
        // Four opaque pixels with red 10, 20, 30, 40.
        let px = vec![0xff00_000a, 0xff00_0014, 0xff00_001e, 0xff00_0028];
        let s = Sampler::new(4, 1, px.clone(), Wrap::Edge).unwrap();
        let mirror = warp_kernel(&s, 4, 1, Filter::Nearest, |x, y| (4.0 - x, y)).unwrap();
        // Destination 0 centres at 0.5, maps to 3.5 and reads pixel 3 (red 40); the reverse holds for 3.
        assert_eq!(mirror[0] & 0xff, 40);
        assert_eq!(mirror[3] & 0xff, 10);
        let c = Sampler::new(4, 1, px, Wrap::Clear).unwrap();
        // Shift right by one: destination 0 reads pixel 1 (red 20), destination 3 falls outside and is clear.
        let shift = warp_kernel(&c, 4, 1, Filter::Nearest, |x, y| (x + 1.0, y)).unwrap();
        assert_eq!(shift[0] & 0xff, 20);
        assert_eq!(shift[3], 0);
        assert!(warp_kernel(&c, 0, 1, Filter::Nearest, |x, y| (x, y)).is_none());
    }

    #[test]
    fn blend_multiply_and_difference_work_on_opaque_pixels() {
        // Foreground red 128, background white: multiply keeps 128 (1 * 128/255).
        let fg = [0xff00_0080u32];
        let white = [WHITE];
        let m = blend_kernel(&fg, &white, 1, 1, Blend::Multiply).unwrap();
        assert_eq!(m[0], 0xff00_0080);
        // Red 255 against red 128: |1 - 128/255| * 255 = 127.
        let d = blend_kernel(&[0xff00_00ff], &[0xff00_0080], 1, 1, Blend::Difference).unwrap();
        assert_eq!(d[0] & 0xff, 127);
    }

    #[test]
    fn normal_blend_of_half_alpha_white_over_black_is_mid_grey() {
        // Premultiplied white at alpha 128: straight colour 1.0, coverage 128/255.
        let half_white = [0x8080_8080u32];
        let out = blend_kernel(&half_white, &[BLACK], 1, 1, Blend::Normal).unwrap();
        assert_eq!(out[0], 0xff80_8080);
    }

    #[test]
    fn blend_rejects_buffers_that_do_not_match_the_size() {
        assert!(blend_kernel(&[0; 4], &[0; 2], 2, 2, Blend::Normal).is_none());
        assert!(blend_kernel(&[0; 4], &[0; 4], 2, 2, Blend::Screen).is_some());
    }
}

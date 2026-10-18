//! CPU reference kernels for the image filter path: the per-pixel and neighbourhood programs that Core Image filters
//! (`CIGaussianBlur`, `CIColorControls`, `CIColorMatrix`, `CISepiaTone`, `CIVignette`, `CIAffineTransform`,
//! `CILanczosScaleTransform`, `CISourceOverCompositing`) and the shader-library image kernels
//! (`MPSImageConvolution`, `MPSImageHistogram`) run. A GPU back end checks its output against these functions; with no
//! adapter they are the fallback and the only path.
//!
//! Pixels are the `Image` format: premultiplied, `r | g << 8 | b << 16 | a << 24`. Colour kernels unpremultiply, work in
//! 0..1 floats and premultiply again, so a half-transparent pixel keeps its hue. Output is rounded to nearest. Rows run
//! top to bottom and the affine matrix follows `CGAffineTransform` (`x' = a x + c y + tx`, `y' = b x + d y + ty`) in that
//! pixel space. Pixel kernels walk a grid with `compute_dispatch::run_threads`, so a dispatch and its reference agree.
use crate::compute_dispatch::{run_threads, Size};
use bifrost_render::blur_filters::{gaussian_blur, Edge};
use bifrost_render::gfx::Image;

const GROUP: Size = Size::new(8, 8, 1);
const MAX_THREADS: usize = 256;

/// Which Core Image kernel class a kernel belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelKind {
    /// `CIKernel`: reads arbitrary pixels of its input.
    General,
    /// `CIColorKernel`: one output pixel from the same input pixel.
    Color,
    /// `CIWarpKernel`: moves coordinates, never changes colour.
    Warp,
}

/// An image kernel with its arguments.
#[derive(Clone, Debug, PartialEq)]
pub enum Kernel {
    /// `CIGaussianBlur`, `radius` is the standard deviation; edges clamp.
    GaussianBlur { radius: f32 },
    /// `CIColorControls`: saturation (1 keeps), brightness (0 keeps, added), contrast (1 keeps, about 0.5).
    ColorControls { saturation: f32, brightness: f32, contrast: f32 },
    /// `CIColorMatrix`: `out = rows * [r g b a] + bias`; `rows` are the R, G, B and A weight vectors.
    ColorMatrix { rows: [[f32; 4]; 4], bias: [f32; 4] },
    /// `CISepiaTone`: mixes the input with its sepia tone by `intensity` (0..1).
    Sepia { intensity: f32 },
    /// `CIVignette`: darkens by `intensity` towards the corners; `radius` scales the clear area (1 reaches the corners).
    Vignette { intensity: f32, radius: f32 },
    /// `CIAffineTransform` as `[a, b, c, d, tx, ty]`, nearest sampling, transparent outside.
    AffineTransform { m: [f32; 6] },
    /// `CILanczosScaleTransform`: `scale` on both axes and `aspect_ratio` on the width.
    LanczosScale { scale: f32, aspect_ratio: f32 },
    /// `MPSImageConvolution`: odd `width` by `height` weights, row-major correlation, plus `bias`; edges clamp.
    Convolution { width: usize, height: usize, weights: Vec<f32>, bias: f32 },
}

/// A kernel an app can define for itself, the base class of `CIKernel` and its subclasses. It states its class and
/// runs over a whole image. The built-in [`Kernel`] values implement it, and [`apply_chain`] runs several in order.
pub trait ImageKernel {
    /// The class of the kernel.
    fn kind(&self) -> KernelKind;
    /// Runs the kernel over `img` and returns the result.
    fn run(&self, img: &Image) -> Image;
}

impl ImageKernel for Kernel {
    fn kind(&self) -> KernelKind {
        Kernel::kind(self)
    }

    fn run(&self, img: &Image) -> Image {
        Kernel::run(self, img)
    }
}

/// Runs each kernel in turn, feeding every result into the next. An empty chain returns a copy of `img`.
pub fn apply_chain(kernels: &[&dyn ImageKernel], img: &Image) -> Image {
    kernels.iter().fold(img.clone(), |current, kernel| kernel.run(&current))
}

impl Kernel {
    /// The Core Image class of this kernel.
    pub fn kind(&self) -> KernelKind {
        match self {
            Kernel::ColorControls { .. } | Kernel::ColorMatrix { .. } | Kernel::Sepia { .. } => KernelKind::Color,
            Kernel::AffineTransform { .. } => KernelKind::Warp,
            _ => KernelKind::General,
        }
    }

    /// Runs the kernel over `img`. A convolution with a bad shape returns the input unchanged.
    pub fn run(&self, img: &Image) -> Image {
        match self {
            Kernel::GaussianBlur { radius } => gaussian_blur(img, *radius, Edge::Clamp),
            Kernel::ColorControls { saturation, brightness, contrast } => {
                map_color(img, |c| color_controls(c, *saturation, *brightness, *contrast))
            }
            Kernel::ColorMatrix { rows, bias } => map_color(img, |c| color_matrix(c, rows, bias)),
            Kernel::Sepia { intensity } => map_color(img, |c| sepia(c, *intensity)),
            Kernel::Vignette { intensity, radius } => vignette(img, *intensity, *radius),
            Kernel::AffineTransform { m } => affine(img, m),
            Kernel::LanczosScale { scale, aspect_ratio } => lanczos_scale(img, *scale, *aspect_ratio),
            Kernel::Convolution { width, height, weights, bias } => {
                convolve(img, *width, *height, weights, *bias).unwrap_or_else(|| img.clone())
            }
        }
    }
}

fn unpack(p: u32) -> [f32; 4] {
    [p & 255, p >> 8 & 255, p >> 16 & 255, p >> 24].map(|c| c as f32 / 255.0)
}

fn pack(v: [f32; 4]) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let a = q(v[3]);
    q(v[0]).min(a) | q(v[1]).min(a) << 8 | q(v[2]).min(a) << 16 | a << 24
}

fn unpremultiply(v: [f32; 4]) -> [f32; 4] {
    if v[3] <= 0.0 {
        [0.0; 4]
    } else {
        [v[0] / v[3], v[1] / v[3], v[2] / v[3], v[3]]
    }
}

fn premultiply(v: [f32; 4]) -> [f32; 4] {
    let a = v[3].clamp(0.0, 1.0);
    [v[0].clamp(0.0, 1.0) * a, v[1].clamp(0.0, 1.0) * a, v[2].clamp(0.0, 1.0) * a, a]
}

/// Runs `f` on every pixel position through the dispatch walk and collects the results.
fn for_each_pixel<F: FnMut(usize, usize) -> u32>(w: usize, h: usize, mut f: F) -> Image {
    let mut out = Image::new(w, h);
    if w == 0 || h == 0 {
        return out;
    }
    let _ = run_threads(Size::new(w, h, 1), GROUP, MAX_THREADS, |t| out.px[t.height * w + t.width] = f(t.width, t.height));
    out
}

fn map_color<F: Fn([f32; 4]) -> [f32; 4]>(img: &Image, f: F) -> Image {
    for_each_pixel(img.w, img.h, |x, y| pack(premultiply(f(unpremultiply(unpack(img.px[y * img.w + x]))))))
}

fn luma(c: [f32; 4]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

fn color_controls(c: [f32; 4], saturation: f32, brightness: f32, contrast: f32) -> [f32; 4] {
    let l = luma(c);
    let f = |v: f32| ((l + (v - l) * saturation + brightness).clamp(0.0, 1.0) - 0.5) * contrast + 0.5;
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

fn color_matrix(c: [f32; 4], rows: &[[f32; 4]; 4], bias: &[f32; 4]) -> [f32; 4] {
    let mut o = [0.0; 4];
    for i in 0..4 {
        o[i] = rows[i].iter().zip(c).map(|(w, v)| w * v).sum::<f32>() + bias[i];
    }
    o
}

fn sepia(c: [f32; 4], intensity: f32) -> [f32; 4] {
    let t = intensity.clamp(0.0, 1.0);
    let s = [
        0.393 * c[0] + 0.769 * c[1] + 0.189 * c[2],
        0.349 * c[0] + 0.686 * c[1] + 0.168 * c[2],
        0.272 * c[0] + 0.534 * c[1] + 0.131 * c[2],
    ];
    let m = |i: usize| (c[i] + (s[i].min(1.0) - c[i]) * t).clamp(0.0, 1.0);
    [m(0), m(1), m(2), c[3]]
}

fn vignette(img: &Image, intensity: f32, radius: f32) -> Image {
    let (cx, cy) = (img.w as f32 / 2.0, img.h as f32 / 2.0);
    let reach = (0.5 * ((img.w * img.w + img.h * img.h) as f32).sqrt() * radius).max(1e-6);
    for_each_pixel(img.w, img.h, |x, y| {
        let d = ((x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) / reach).clamp(0.0, 1.0);
        let smooth = d * d * (3.0 - 2.0 * d);
        let k = (1.0 - intensity * smooth).clamp(0.0, 1.0);
        let p = unpack(img.px[y * img.w + x]);
        pack([p[0] * k, p[1] * k, p[2] * k, p[3]])
    })
}

fn affine(img: &Image, m: &[f32; 6]) -> Image {
    let [a, b, c, d, tx, ty] = *m;
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return Image::new(img.w, img.h);
    }
    for_each_pixel(img.w, img.h, |x, y| {
        let (px, py) = (x as f32 + 0.5 - tx, y as f32 + 0.5 - ty);
        let sx = ((d * px - c * py) / det).floor();
        let sy = ((-b * px + a * py) / det).floor();
        if sx < 0.0 || sy < 0.0 || sx >= img.w as f32 || sy >= img.h as f32 {
            0
        } else {
            img.px[sy as usize * img.w + sx as usize]
        }
    })
}

fn lanczos(x: f32) -> f32 {
    const A: f32 = 3.0;
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() >= A {
        return 0.0;
    }
    let p = std::f32::consts::PI * x;
    A * p.sin() * (p / A).sin() / (p * p)
}

/// Normalised taps `(source index, weight)` for output index `i` when resizing `n_in` to `n_out`.
fn taps(i: usize, n_in: usize, n_out: usize) -> Vec<(usize, f32)> {
    let s = n_out as f32 / n_in as f32;
    let centre = (i as f32 + 0.5) / s - 0.5;
    let widen = s.min(1.0);
    let reach = (3.0 / widen).ceil() as i64;
    let first = centre.floor() as i64 - reach + 1;
    let mut t: Vec<(usize, f32)> = (first..first + 2 * reach)
        .map(|k| (k.clamp(0, n_in as i64 - 1) as usize, lanczos((k as f32 - centre) * widen)))
        .collect();
    let sum: f32 = t.iter().map(|x| x.1).sum();
    if sum.abs() > 1e-9 {
        t.iter_mut().for_each(|x| x.1 /= sum);
    }
    t
}

fn lanczos_scale(img: &Image, scale: f32, aspect: f32) -> Image {
    let size = |n: usize, s: f32| ((n as f32 * s).round() as usize).max(1);
    if img.w == 0 || img.h == 0 || scale <= 0.0 || aspect <= 0.0 {
        return Image::new(0, 0);
    }
    let (ow, oh) = (size(img.w, scale * aspect), size(img.h, scale));
    // Horizontal pass into floats, then vertical.
    let mut mid = vec![[0.0f32; 4]; ow * img.h];
    for y in 0..img.h {
        for x in 0..ow {
            for (sx, w) in taps(x, img.w, ow) {
                let p = unpack(img.px[y * img.w + sx]);
                (0..4).for_each(|c| mid[y * ow + x][c] += p[c] * w);
            }
        }
    }
    for_each_pixel(ow, oh, |x, y| {
        let mut o = [0.0f32; 4];
        for (sy, w) in taps(y, img.h, oh) {
            (0..4).for_each(|c| o[c] += mid[sy * ow + x][c] * w);
        }
        pack(o)
    })
}

/// Correlates `img` with an odd-sized `width` by `height` kernel. `None` when a size is even, zero or the weights do not fit.
pub fn convolve(img: &Image, width: usize, height: usize, weights: &[f32], bias: f32) -> Option<Image> {
    if width % 2 == 0 || height % 2 == 0 || weights.len() != width * height {
        return None;
    }
    let (rx, ry) = ((width / 2) as i64, (height / 2) as i64);
    Some(for_each_pixel(img.w, img.h, |x, y| {
        let mut o = [bias; 4];
        for ky in 0..height {
            for kx in 0..width {
                let sx = (x as i64 + kx as i64 - rx).clamp(0, img.w as i64 - 1) as usize;
                let sy = (y as i64 + ky as i64 - ry).clamp(0, img.h as i64 - 1) as usize;
                let p = unpack(img.px[sy * img.w + sx]);
                (0..4).for_each(|c| o[c] += p[c] * weights[ky * width + kx]);
            }
        }
        pack(o)
    }))
}

/// `CISourceOverCompositing`: `foreground` over `background`, both premultiplied. `None` when the sizes differ.
pub fn source_over(foreground: &Image, background: &Image) -> Option<Image> {
    if foreground.w != background.w || foreground.h != background.h {
        return None;
    }
    Some(for_each_pixel(foreground.w, foreground.h, |x, y| {
        let i = y * foreground.w + x;
        let (f, b) = (unpack(foreground.px[i]), unpack(background.px[i]));
        let k = 1.0 - f[3];
        pack([f[0] + b[0] * k, f[1] + b[1] * k, f[2] + b[2] * k, f[3] + b[3] * k])
    }))
}

/// `MPSImageHistogram`: counts of each 8-bit value per channel (red, green, blue, alpha) over the stored pixels.
pub fn histogram(img: &Image) -> [[u32; 256]; 4] {
    let mut bins = [[0u32; 256]; 4];
    for &p in &img.px {
        for (c, bin) in bins.iter_mut().enumerate() {
            bin[(p >> (8 * c) & 255) as usize] += 1;
        }
    }
    bins
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize, px: &[u32]) -> Image {
        let mut img = Image::new(w, h);
        img.px.copy_from_slice(px);
        img
    }

    #[test]
    fn centre_one_kernel_returns_the_image_unchanged() {
        let src = image(2, 2, &[0xff10_2030, 0xff40_5060, 0x8000_0000, 0xffff_ffff]);
        let weights = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let out = convolve(&src, 3, 3, &weights, 0.0).unwrap();
        assert_eq!(out.px, src.px);
    }

    #[test]
    fn even_or_mismatched_kernels_are_refused() {
        let src = image(2, 2, &[0; 4]);
        assert!(convolve(&src, 2, 2, &[0.0; 4], 0.0).is_none());
        assert!(convolve(&src, 3, 3, &[0.0; 4], 0.0).is_none());
        assert!(convolve(&src, 3, 1, &[1.0 / 3.0; 3], 0.0).is_some());
    }

    #[test]
    fn three_tap_average_spreads_a_single_bright_pixel() {
        // A row of red 0, 255, 0. A 3 by 1 box averages each pixel with its neighbours: the middle
        // one becomes red 255 / 3, about 85.
        let src = image(3, 1, &[0xff00_0000, 0xff00_00ff, 0xff00_0000]);
        let out = convolve(&src, 3, 1, &[1.0 / 3.0; 3], 0.0).unwrap();
        let red = out.px[1] & 0xff;
        assert!((red as i32 - 85).abs() <= 1, "red {red}");
    }

    #[test]
    fn opaque_foreground_hides_the_background() {
        let fg = image(1, 1, &[0xff00_00ff]);
        let bg = image(1, 1, &[0xffff_ffff]);
        assert_eq!(source_over(&fg, &bg).unwrap().px, vec![0xff00_00ff]);
    }

    #[test]
    fn half_transparent_foreground_mixes_with_the_background() {
        // Premultiplied red 0x80 at alpha 0x80 over opaque white: green is the white showing through,
        // about 127 of 255, and the result stays opaque.
        let fg = image(1, 1, &[0x80_00_00_80]);
        let bg = image(1, 1, &[0xffff_ffff]);
        let px = source_over(&fg, &bg).unwrap().px[0];
        let green = ((px >> 8) & 0xff) as i32;
        assert!((green - 127).abs() <= 1, "green {green}");
        assert_eq!(px >> 24, 0xff);
    }

    #[test]
    fn compositing_refuses_different_sizes() {
        let a = image(2, 1, &[0; 2]);
        let b = image(1, 2, &[0; 2]);
        assert!(source_over(&a, &b).is_none());
    }

    #[test]
    fn histogram_counts_each_channel_value() {
        let img = image(2, 1, &[0xff00_0000, 0xff00_0000]);
        let bins = histogram(&img);
        assert_eq!(bins[0][0], 2);
        assert_eq!(bins[1][0], 2);
        assert_eq!(bins[2][0], 2);
        assert_eq!(bins[3][255], 2);
        assert_eq!(bins.iter().map(|b| b.iter().sum::<u32>()).sum::<u32>(), 8);
    }
}

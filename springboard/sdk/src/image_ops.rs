//! Image kernels for apps over `Image` (premultiplied RGBA, see `bifrost_render::gfx`): colour convolution, scaling
//! (nearest, bilinear, Lanczos), channel statistics, integral images, row and column reductions, pixel arithmetic and
//! transfer conversion, Gaussian and Laplacian pyramids, the Euclidean distance transform, the guided filter and keypoint
//! search. They are the app-facing entry points for the plane kernels in `bifrost_render` (`lanczos`, `plane_stats`,
//! `plane_arith`, `laplacian_pyramid`, `distance`, `guided`, `filter`, `gaussian_filter`); the luma-only filters are in
//! `pixel_filter` and the blurs in `image_blur`.
//!
//! [`KernelSpec`] names a kernel and its arguments as a value. It implements `bifrost_ui::image_kernels::ImageKernel`, so
//! `apply_chain` runs it with the built-in kernels, and [`encode_kernels`] and [`decode_kernels`] store and restore a chain
//! as text, the counterpart of archiving a kernel. The reader treats its input as hostile: it caps the line length, the
//! kernel count and the sizes it will allocate.
//!
//! Everything runs on the CPU and is the reference a GPU back end is checked against. There is no GPU dispatch.
//! See docs/bifrost-ui.md "GPU kits" for the decisions and the gaps.
use bifrost_render::blur_filters::{gaussian_blur as blur_gaussian, Edge};
use bifrost_render::filter::{self as f, Plane};
use bifrost_render::gaussian_filter as gf;
use bifrost_render::gfx::{rgba, Image};
use bifrost_render::pixel_layout::unpremultiply;
use bifrost_render::{distance, guided, lanczos, laplacian_pyramid as lp, plane_arith as pa, plane_stats as ps};
use bifrost_ui::image_kernels::{ImageKernel, KernelKind};

use crate::pixel_filter::{grey_image, luma_plane};

pub use bifrost_render::filter::Threshold;
pub use bifrost_render::laplacian_pyramid::LaplacianLevel;
pub use bifrost_render::plane_arith::{Op as ArithmeticOp, Transfer};
pub use bifrost_render::plane_stats::{Reduce, Statistics};

/// The largest side a scaled image may have.
pub const MAX_SIDE: usize = 16384;
/// The most pixels a scaled image may have.
pub const MAX_PIXELS: usize = 1 << 26;
/// The most levels a pyramid may have.
pub const MAX_LEVELS: usize = 32;
/// The largest tent radius, in pixels; a larger one is taken as this.
pub const MAX_TENT_RADIUS: usize = 1024;
/// The largest side of an erosion, dilation or median window; a kernel with a larger one returns its input.
pub const MAX_WINDOW: usize = 255;
/// The largest Gaussian sigma `KernelSpec::Gaussian` uses; a larger one is taken as this.
pub const MAX_SIGMA: f32 = 64.0;

/// One channel of the stored (premultiplied) pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Red,
    Green,
    Blue,
    Alpha,
}

impl Channel {
    const ALL: [Channel; 4] = [Channel::Red, Channel::Green, Channel::Blue, Channel::Alpha];

    fn shift(self) -> u32 {
        match self {
            Channel::Red => 0,
            Channel::Green => 8,
            Channel::Blue => 16,
            Channel::Alpha => 24,
        }
    }
}

fn well_formed(img: &Image) -> bool {
    img.w > 0 && img.h > 0 && img.w.checked_mul(img.h) == Some(img.px.len())
}

/// The plane of one channel of `img`.
pub fn channel_plane(img: &Image, ch: Channel) -> Plane {
    let shift = ch.shift();
    Plane { width: img.w, height: img.h, pixels: img.px.iter().map(|&p| (p >> shift) as u8).collect() }
}

/// Joins four planes of one size into an image. Colour is held at or below alpha so the pixels stay premultiplied.
fn merge(r: Plane, g: Plane, b: Plane, a: Plane) -> Image {
    let n = a.pixels.len();
    let px = (0..n)
        .map(|i| {
            let al = a.pixels[i];
            r.pixels[i].min(al) as u32 | (g.pixels[i].min(al) as u32) << 8 | (b.pixels[i].min(al) as u32) << 16 | (al as u32) << 24
        })
        .collect();
    Image { w: a.width, h: a.height, px }
}

/// Joins four planes of one size into an image as they are, without holding colour under alpha.
fn merge_raw(r: Plane, g: Plane, b: Plane, a: Plane) -> Image {
    let px = (0..a.pixels.len())
        .map(|i| r.pixels[i] as u32 | (g.pixels[i] as u32) << 8 | (b.pixels[i] as u32) << 16 | (a.pixels[i] as u32) << 24)
        .collect();
    Image { w: a.width, h: a.height, px }
}

/// How `scale` samples the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    /// The nearest source pixel.
    Nearest,
    /// Bilinear (`Image::resized`).
    Bilinear,
    /// Lanczos-3, widened when shrinking so each output pixel averages its footprint.
    Lanczos,
}

/// Resamples `img` to `w` by `h`. `None` for a zero side, a side above `MAX_SIDE`, more than `MAX_PIXELS` output
/// pixels, or an image whose pixel count does not match its size.
pub fn scale(img: &Image, w: usize, h: usize, mode: ScaleMode) -> Option<Image> {
    if !well_formed(img) || w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w.checked_mul(h)? > MAX_PIXELS {
        return None;
    }
    match mode {
        ScaleMode::Nearest => {
            let mut out = Image::new(w, h);
            for y in 0..h {
                let sy = y * img.h / h;
                for x in 0..w {
                    out.px[y * w + x] = img.px[sy * img.w + x * img.w / w];
                }
            }
            Some(out)
        }
        ScaleMode::Bilinear => Some(img.resized(w, h)),
        ScaleMode::Lanczos => {
            let [r, g, b, a] = Channel::ALL.map(|c| lanczos::lanczos_scale(&channel_plane(img, c), w, h));
            Some(merge(r?, g?, b?, a?))
        }
    }
}

/// Mean, population variance, minimum and maximum of one channel over the rectangle (`x`, `y`, `w`, `h`), clipped to the
/// image. `None` when the clipped region is empty.
pub fn statistics(img: &Image, ch: Channel, x: usize, y: usize, w: usize, h: usize) -> Option<Statistics> {
    if !well_formed(img) {
        return None;
    }
    ps::statistics(&channel_plane(img, ch), x, y, w, h)
}

/// Like [`statistics`], on the luma of the image.
pub fn luma_statistics(img: &Image, x: usize, y: usize, w: usize, h: usize) -> Option<Statistics> {
    if !well_formed(img) {
        return None;
    }
    ps::statistics(&luma_plane(img), x, y, w, h)
}

/// The integral image of one channel, or of its squares, for fast region sums. An image that is not well formed gives
/// the integral of an empty plane.
pub fn integral(img: &Image, ch: Channel, squares: bool) -> ps::Integral {
    let plane = if well_formed(img) { channel_plane(img, ch) } else { Plane { width: 0, height: 0, pixels: Vec::new() } };
    if squares {
        ps::Integral::of_squares(&plane)
    } else {
        ps::Integral::new(&plane)
    }
}

/// One value per row of one channel. Sums are not clamped to a byte.
pub fn reduce_rows(img: &Image, ch: Channel, op: Reduce) -> Vec<f32> {
    if !well_formed(img) {
        return Vec::new();
    }
    ps::reduce_rows(&channel_plane(img, ch), op)
}

/// One value per column of one channel. Sums are not clamped to a byte.
pub fn reduce_columns(img: &Image, ch: Channel, op: Reduce) -> Vec<f32> {
    if !well_formed(img) {
        return Vec::new();
    }
    ps::reduce_columns(&channel_plane(img, ch), op)
}

/// Combines two images channel by channel (add, subtract, multiply, divide), alpha included, as the stored values. The
/// result is not renormalised, so colour can end above alpha; combine opaque images, or run `invert` or a scale after.
/// `None` when the sizes differ.
pub fn combine(a: &Image, b: &Image, op: ArithmeticOp) -> Option<Image> {
    if (a.w, a.h) != (b.w, b.h) || !well_formed(a) || !well_formed(b) {
        return None;
    }
    let [r, g, bl, al] = Channel::ALL.map(|c| pa::combine(&channel_plane(a, c), &channel_plane(b, c), op));
    Some(merge_raw(r?, g?, bl?, al?))
}

/// The negative: each colour channel `c` becomes `alpha - c`, so the pixel stays premultiplied. Alpha is kept.
pub fn invert(img: &Image) -> Image {
    let px = img
        .px
        .iter()
        .map(|&p| {
            let a = (p >> 24) & 255;
            let c = |shift: u32| (a - ((p >> shift) & 255).min(a)) << shift;
            c(0) | c(8) | c(16) | a << 24
        })
        .collect();
    Image { w: img.w, h: img.h, px }
}

/// Converts the colour between the sRGB and linear encodings. Alpha is kept. The linear side is 8-bit, so a round trip
/// moves the dark codes most: the worst case is six steps (codes 1 to 7), and codes from 60 up stay within one.
pub fn convert_transfer(img: &Image, from: Transfer, to: Transfer) -> Image {
    if !well_formed(img) {
        return img.clone();
    }
    let straight: Vec<[u8; 4]> = img.px.iter().map(|&p| unpremultiply(p)).collect();
    let make = |i: usize| Plane { width: img.w, height: img.h, pixels: straight.iter().map(|s| s[i]).collect() };
    let (r, g, b) = (pa::convert_transfer(&make(0), from, to), pa::convert_transfer(&make(1), from, to), pa::convert_transfer(&make(2), from, to));
    let px = (0..straight.len()).map(|i| rgba(r.pixels[i], g.pixels[i], b.pixels[i], straight[i][3])).collect();
    Image { w: img.w, h: img.h, px }
}

/// The transpose: pixel (x, y) moves to (y, x), and the result is `h` wide and `w` tall.
pub fn transpose(img: &Image) -> Image {
    if !well_formed(img) {
        return img.clone();
    }
    let mut out = Image::new(img.h, img.w);
    for y in 0..img.h {
        for x in 0..img.w {
            out.px[x * img.h + y] = img.px[y * img.w + x];
        }
    }
    out
}

/// Convolves every channel with an odd `kw` by `kh` kernel (sides 3 to 9, row-major weights, no flip, edges clamp). `None`
/// for any other side or a weight count that is not `kw * kh`.
pub fn convolve(img: &Image, kw: usize, kh: usize, weights: &[f32], bias: f32) -> Option<Image> {
    if !well_formed(img) || !f::is_convolution_side(kw) || !f::is_convolution_side(kh) || weights.len() != kw * kh {
        return None;
    }
    let [r, g, b, a] = Channel::ALL.map(|c| f::convolve(&channel_plane(img, c), kw, kh, weights, bias));
    Some(merge(r?, g?, b?, a?))
}

/// A tent blur of radius `radius` pixels (`2 * radius + 1` taps per side) on every channel.
pub fn tent_blur(img: &Image, radius: usize) -> Image {
    if !well_formed(img) {
        return img.clone();
    }
    let radius = radius.min(MAX_TENT_RADIUS);
    let [r, g, b, a] = Channel::ALL.map(|c| gf::tent_blur(&channel_plane(img, c), radius));
    merge(r, g, b, a)
}

/// The Gaussian pyramid: `levels` images (at most `MAX_LEVELS`), the first being `img`, each half the size of the one
/// before, rounded up. Empty for zero levels or an image that is not well formed.
pub fn gaussian_pyramid(img: &Image, levels: usize) -> Vec<Image> {
    let levels = levels.min(MAX_LEVELS);
    if levels == 0 || !well_formed(img) {
        return Vec::new();
    }
    let chans = Channel::ALL.map(|c| lp::gaussian_pyramid(&channel_plane(img, c), levels));
    (0..levels)
        .map(|i| merge(chans[0][i].clone(), chans[1][i].clone(), chans[2][i].clone(), chans[3][i].clone()))
        .collect()
}

/// The Laplacian pyramid of the luma of `img`. Empty for zero levels or an image that is not well formed.
pub fn laplacian_pyramid(img: &Image, levels: usize) -> Vec<LaplacianLevel> {
    if levels == 0 || !well_formed(img) {
        return Vec::new();
    }
    lp::laplacian_pyramid(&luma_plane(img), levels.min(MAX_LEVELS))
}

/// Rebuilds the grey image a Laplacian pyramid was made from. `None` for an empty or inconsistent pyramid.
pub fn collapse_laplacian(levels: &[LaplacianLevel]) -> Option<Image> {
    lp::collapse(levels).map(|p| grey_image(&p))
}

/// Euclidean distance from every pixel to the nearest pixel whose luma is not zero, row-major. With `limit`, farther
/// pixels get infinity. Empty for an image that is not well formed.
pub fn distance_transform(img: &Image, limit: Option<f32>) -> Vec<f32> {
    if !well_formed(img) {
        return Vec::new();
    }
    distance::euclidean_distance(&luma_plane(img), limit)
}

/// The guided filter: smooths the luma of `src` along the edges of the luma of `guide`. `diameter` is the odd window
/// width and `epsilon` the regularisation, in the 0..1 range of the normalised luma. `None` when the sizes differ or the
/// diameter is zero or even. The result is a grey image.
pub fn guided_filter(guide: &Image, src: &Image, diameter: usize, epsilon: f32) -> Option<Image> {
    if (guide.w, guide.h) != (src.w, src.h) || !well_formed(guide) || !well_formed(src) {
        return None;
    }
    let norm = |img: &Image| -> Vec<f32> { luma_plane(img).pixels.iter().map(|&v| v as f32 / 255.0).collect() };
    let out = guided::guided_filter(&norm(guide), &norm(src), src.w, src.h, diameter, epsilon)?;
    let pixels = out.iter().map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    Some(grey_image(&Plane { width: src.w, height: src.h, pixels }))
}

/// A pixel found by [`find_keypoints`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keypoint {
    pub x: usize,
    pub y: usize,
    /// The luma at the pixel.
    pub value: u8,
}

/// The pixels of the rectangle (`x`, `y`, `w`, `h`), clipped to the image, whose luma is at least `threshold`, in scan
/// order (rows top to bottom, left to right), at most `max_count` of them.
pub fn find_keypoints(img: &Image, region: (usize, usize, usize, usize), threshold: u8, max_count: usize) -> Vec<Keypoint> {
    if !well_formed(img) {
        return Vec::new();
    }
    let plane = luma_plane(img);
    let (rx, ry, rw, rh) = region;
    let x1 = rx.saturating_add(rw).min(img.w);
    let y1 = ry.saturating_add(rh).min(img.h);
    let mut out = Vec::new();
    for y in ry.min(img.h)..y1 {
        for x in rx.min(img.w)..x1 {
            let value = plane.pixels[y * img.w + x];
            if value >= threshold {
                if out.len() >= max_count {
                    return out;
                }
                out.push(Keypoint { x, y, value });
            }
        }
    }
    out
}

/// A kernel with its arguments, as a value an app can run, chain and store. Invalid arguments make `run` return the
/// input unchanged, as the built-in Core Image kernels do.
#[derive(Clone, Debug, PartialEq)]
pub enum KernelSpec {
    /// Gaussian blur, `sigma` in pixels, edges clamp.
    Gaussian { sigma: f32 },
    /// Tent blur of `radius` pixels.
    Tent { radius: usize },
    /// A threshold on the luma, giving a grey image.
    Threshold { mode: Threshold, t: u8, max: u8 },
    /// Erosion of the luma over a `w` by `h` window.
    Erode { w: usize, h: usize },
    /// Dilation of the luma over a `w` by `h` window.
    Dilate { w: usize, h: usize },
    /// The median of the luma over a square of side `diameter`.
    Median { diameter: usize },
    /// Sobel gradient magnitude of the luma.
    Sobel,
    /// Four-neighbour Laplacian of the luma with `bias`.
    Laplacian { bias: f32 },
    /// Histogram equalisation of the luma.
    Equalize,
    /// The negative, see [`invert`].
    Invert,
    /// Colour convolution, see [`convolve`].
    Convolution { w: usize, h: usize, weights: Vec<f32>, bias: f32 },
    /// Resampling to `w` by `h`.
    Scale { w: usize, h: usize, mode: ScaleMode },
}

impl ImageKernel for KernelSpec {
    fn kind(&self) -> KernelKind {
        KernelKind::General
    }

    fn run(&self, img: &Image) -> Image {
        let same = || img.clone();
        if !well_formed(img) {
            return same();
        }
        match self {
            KernelSpec::Gaussian { sigma } => blur_gaussian(img, sigma.clamp(0.0, MAX_SIGMA), Edge::Clamp),
            KernelSpec::Tent { radius } => tent_blur(img, *radius),
            KernelSpec::Threshold { mode, t, max } => grey_image(&f::threshold(&luma_plane(img), *mode, *t, *max)),
            KernelSpec::Erode { w, h } if *w > MAX_WINDOW || *h > MAX_WINDOW => same(),
            KernelSpec::Dilate { w, h } if *w > MAX_WINDOW || *h > MAX_WINDOW => same(),
            KernelSpec::Median { diameter } if *diameter > MAX_WINDOW => same(),
            KernelSpec::Erode { w, h } => f::erode(&luma_plane(img), *w, *h).map(|p| grey_image(&p)).unwrap_or_else(same),
            KernelSpec::Dilate { w, h } => f::dilate(&luma_plane(img), *w, *h).map(|p| grey_image(&p)).unwrap_or_else(same),
            KernelSpec::Median { diameter } => f::median(&luma_plane(img), *diameter).map(|p| grey_image(&p)).unwrap_or_else(same),
            KernelSpec::Sobel => grey_image(&f::sobel(&luma_plane(img))),
            KernelSpec::Laplacian { bias } => grey_image(&f::laplacian(&luma_plane(img), *bias)),
            KernelSpec::Equalize => grey_image(&f::equalize(&luma_plane(img))),
            KernelSpec::Invert => invert(img),
            KernelSpec::Convolution { w, h, weights, bias } => convolve(img, *w, *h, weights, *bias).unwrap_or_else(same),
            KernelSpec::Scale { w, h, mode } => scale(img, *w, *h, *mode).unwrap_or_else(same),
        }
    }
}

/// The first line of an archive.
pub const ARCHIVE_HEADER: &str = "bifrost-kernels 1";
/// The most kernels an archive may hold.
pub const MAX_KERNELS: usize = 256;
/// The longest line an archive may have, in bytes.
pub const MAX_LINE_BYTES: usize = 4096;

/// Why an archive was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveError {
    /// The first line is not `ARCHIVE_HEADER`.
    BadHeader,
    /// More than `MAX_KERNELS` kernels.
    TooManyKernels,
    /// A line could not be read. `line` counts from 1, the header being line 1.
    Line { line: usize, reason: &'static str },
}

impl core::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ArchiveError::BadHeader => write!(f, "kernel archive: the header is missing or not version 1"),
            ArchiveError::TooManyKernels => write!(f, "kernel archive: more than {MAX_KERNELS} kernels"),
            ArchiveError::Line { line, reason } => write!(f, "kernel archive: line {line}: {reason}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

fn threshold_name(mode: Threshold) -> &'static str {
    match mode {
        Threshold::Binary => "binary",
        Threshold::BinaryInverse => "binary-inverse",
        Threshold::ToZero => "to-zero",
        Threshold::ToZeroInverse => "to-zero-inverse",
        Threshold::Truncate => "truncate",
    }
}

fn threshold_from(name: &str) -> Option<Threshold> {
    Some(match name {
        "binary" => Threshold::Binary,
        "binary-inverse" => Threshold::BinaryInverse,
        "to-zero" => Threshold::ToZero,
        "to-zero-inverse" => Threshold::ToZeroInverse,
        "truncate" => Threshold::Truncate,
        _ => return None,
    })
}

fn scale_name(mode: ScaleMode) -> &'static str {
    match mode {
        ScaleMode::Nearest => "nearest",
        ScaleMode::Bilinear => "bilinear",
        ScaleMode::Lanczos => "lanczos",
    }
}

fn scale_from(name: &str) -> Option<ScaleMode> {
    Some(match name {
        "nearest" => ScaleMode::Nearest,
        "bilinear" => ScaleMode::Bilinear,
        "lanczos" => ScaleMode::Lanczos,
        _ => return None,
    })
}

/// Writes a chain of kernels as text, one kernel per line after the header.
pub fn encode_kernels(kernels: &[KernelSpec]) -> String {
    let mut out = String::from(ARCHIVE_HEADER);
    out.push('\n');
    for k in kernels {
        let line = match k {
            KernelSpec::Gaussian { sigma } => format!("gaussian {sigma}"),
            KernelSpec::Tent { radius } => format!("tent {radius}"),
            KernelSpec::Threshold { mode, t, max } => format!("threshold {} {t} {max}", threshold_name(*mode)),
            KernelSpec::Erode { w, h } => format!("erode {w} {h}"),
            KernelSpec::Dilate { w, h } => format!("dilate {w} {h}"),
            KernelSpec::Median { diameter } => format!("median {diameter}"),
            KernelSpec::Sobel => "sobel".to_string(),
            KernelSpec::Laplacian { bias } => format!("laplacian {bias}"),
            KernelSpec::Equalize => "equalize".to_string(),
            KernelSpec::Invert => "invert".to_string(),
            KernelSpec::Convolution { w, h, weights, bias } => {
                let mut s = format!("convolution {w} {h} {bias}");
                for v in weights {
                    s.push(' ');
                    s.push_str(&v.to_string());
                }
                s
            }
            KernelSpec::Scale { w, h, mode } => format!("scale {w} {h} {}", scale_name(*mode)),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn float(s: &str) -> Result<f32, &'static str> {
    let v: f32 = s.parse().map_err(|_| "not a number")?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err("not a finite number")
    }
}

fn uint(s: &str) -> Result<usize, &'static str> {
    s.parse().map_err(|_| "not a whole number")
}

fn window(s: &str) -> Result<usize, &'static str> {
    let n = uint(s)?;
    if n > MAX_WINDOW {
        return Err("a window side is at most 255");
    }
    Ok(n)
}

fn byte(s: &str) -> Result<u8, &'static str> {
    s.parse().map_err(|_| "not a number from 0 to 255")
}

fn parse_kernel(line: &str) -> Result<KernelSpec, &'static str> {
    let mut tokens = line.split_whitespace();
    let name = tokens.next().ok_or("empty line")?;
    let rest: Vec<&str> = tokens.collect();
    let args = rest.as_slice();
    match (name, args) {
        ("gaussian", [s]) => Ok(KernelSpec::Gaussian { sigma: float(s)? }),
        ("tent", [r]) => Ok(KernelSpec::Tent { radius: uint(r)? }),
        ("threshold", [m, t, max]) => {
            Ok(KernelSpec::Threshold { mode: threshold_from(m).ok_or("unknown threshold mode")?, t: byte(t)?, max: byte(max)? })
        }
        ("erode", [w, h]) => Ok(KernelSpec::Erode { w: window(w)?, h: window(h)? }),
        ("dilate", [w, h]) => Ok(KernelSpec::Dilate { w: window(w)?, h: window(h)? }),
        ("median", [d]) => Ok(KernelSpec::Median { diameter: window(d)? }),
        ("sobel", []) => Ok(KernelSpec::Sobel),
        ("laplacian", [b]) => Ok(KernelSpec::Laplacian { bias: float(b)? }),
        ("equalize", []) => Ok(KernelSpec::Equalize),
        ("invert", []) => Ok(KernelSpec::Invert),
        ("convolution", [w, h, bias, weights @ ..]) => {
            let (w, h) = (uint(w)?, uint(h)?);
            if !f::is_convolution_side(w) || !f::is_convolution_side(h) {
                return Err("a convolution side is odd and from 3 to 9");
            }
            if weights.len() != w * h {
                return Err("the weight count is not width times height");
            }
            let weights = weights.iter().map(|v| float(v)).collect::<Result<Vec<f32>, _>>()?;
            Ok(KernelSpec::Convolution { w, h, weights, bias: float(bias)? })
        }
        ("scale", [w, h, m]) => {
            let (w, h) = (uint(w)?, uint(h)?);
            if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
                return Err("a scale side is from 1 to 16384");
            }
            Ok(KernelSpec::Scale { w, h, mode: scale_from(m).ok_or("unknown scale mode")? })
        }
        ("gaussian" | "tent" | "threshold" | "erode" | "dilate" | "median" | "sobel" | "laplacian" | "equalize" | "invert" | "convolution" | "scale", _) => {
            Err("wrong number of arguments")
        }
        _ => Err("unknown kernel"),
    }
}

/// Reads a chain written by [`encode_kernels`]. Blank lines are skipped.
pub fn decode_kernels(text: &str) -> Result<Vec<KernelSpec>, ArchiveError> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, first)) if first.trim() == ARCHIVE_HEADER => {}
        _ => return Err(ArchiveError::BadHeader),
    }
    let mut out = Vec::new();
    for (i, line) in lines {
        let number = i + 1;
        if line.len() > MAX_LINE_BYTES {
            return Err(ArchiveError::Line { line: number, reason: "line too long" });
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if out.len() >= MAX_KERNELS {
            return Err(ArchiveError::TooManyKernels);
        }
        out.push(parse_kernel(line).map_err(|reason| ArchiveError::Line { line: number, reason })?);
    }
    Ok(out)
}

/// Runs a chain of kernels in order. An empty chain returns a copy of `img`.
pub fn run_chain(kernels: &[KernelSpec], img: &Image) -> Image {
    kernels.iter().fold(img.clone(), |current, k| k.run(&current))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::gfx::pack;
    use bifrost_ui::image_kernels::apply_chain;

    fn red_image(w: usize, h: usize, values: &[u8]) -> Image {
        Image { w, h, px: values.iter().map(|&v| pack(v, 0, 0, 255)).collect() }
    }

    fn grey(w: usize, h: usize, values: &[u8]) -> Image {
        Image { w, h, px: values.iter().map(|&v| pack(v, v, v, 255)).collect() }
    }

    #[test]
    fn statistics_of_a_channel() {
        let img = red_image(2, 2, &[0, 100, 200, 100]);
        let s = statistics(&img, Channel::Red, 0, 0, 2, 2).unwrap();
        assert_eq!((s.mean, s.variance, s.min, s.max), (100.0, 5000.0, 0, 200));
        let a = statistics(&img, Channel::Alpha, 0, 0, 2, 2).unwrap();
        assert_eq!((a.min, a.max), (255, 255));
        assert!(statistics(&img, Channel::Red, 5, 5, 2, 2).is_none());
        assert!(statistics(&Image { w: 2, h: 2, px: vec![0; 3] }, Channel::Red, 0, 0, 2, 2).is_none());
    }

    #[test]
    fn the_integral_gives_region_sums() {
        let img = red_image(3, 2, &[1, 2, 3, 4, 5, 6]);
        let i = integral(&img, Channel::Red, false);
        assert_eq!(i.rect_sum(0, 0, 3, 2), 21);
        assert_eq!(i.rect_sum(1, 1, 2, 1), 11);
        assert_eq!(integral(&img, Channel::Red, true).rect_sum(0, 0, 3, 2), 91);
        assert_eq!(integral(&Image { w: 0, h: 0, px: vec![] }, Channel::Red, false).rect_sum(0, 0, 3, 2), 0);
    }

    #[test]
    fn reductions_are_not_clamped_to_a_byte() {
        let img = red_image(3, 2, &[200, 200, 200, 1, 2, 3]);
        assert_eq!(reduce_rows(&img, Channel::Red, Reduce::Sum), vec![600.0, 6.0]);
        assert_eq!(reduce_columns(&img, Channel::Red, Reduce::Max), vec![200.0, 200.0, 200.0]);
        assert!(reduce_rows(&Image { w: 1, h: 1, px: vec![] }, Channel::Red, Reduce::Sum).is_empty());
    }

    #[test]
    fn scaling_keeps_flat_images_flat_and_refuses_bad_sizes() {
        let flat = grey(3, 3, &[90; 9]);
        for mode in [ScaleMode::Nearest, ScaleMode::Bilinear, ScaleMode::Lanczos] {
            let out = scale(&flat, 7, 5, mode).unwrap();
            assert_eq!((out.w, out.h), (7, 5));
            assert!(out.px.iter().all(|&p| (p & 255) >= 89 && (p & 255) <= 91 && p >> 24 == 255), "{mode:?}");
        }
        assert!(scale(&flat, 0, 5, ScaleMode::Lanczos).is_none());
        assert!(scale(&flat, MAX_SIDE + 1, 1, ScaleMode::Nearest).is_none());
        assert!(scale(&flat, MAX_SIDE, MAX_SIDE, ScaleMode::Nearest).is_none());
    }

    #[test]
    fn nearest_scaling_repeats_pixels() {
        let img = red_image(2, 1, &[10, 20]);
        let out = scale(&img, 4, 2, ScaleMode::Nearest).unwrap();
        let reds: Vec<u32> = out.px.iter().map(|&p| p & 255).collect();
        assert_eq!(reds, vec![10, 10, 20, 20, 10, 10, 20, 20]);
    }

    #[test]
    fn arithmetic_saturates_and_combines_alpha() {
        let a = red_image(1, 1, &[200]);
        let b = red_image(1, 1, &[100]);
        let sum = combine(&a, &b, ArithmeticOp::Add).unwrap();
        assert_eq!(sum.px[0] & 255, 255);
        assert_eq!(sum.px[0] >> 24, 255);
        assert_eq!(combine(&a, &b, ArithmeticOp::Subtract).unwrap().px[0] & 255, 100);
        assert!(combine(&a, &red_image(2, 1, &[1, 2]), ArithmeticOp::Add).is_none());
        // Alpha is combined too, and a sum past 255 saturates.
        let half = Image { w: 1, h: 1, px: vec![pack(100, 100, 100, 128)] };
        let doubled = combine(&half, &half, ArithmeticOp::Add).unwrap().px[0];
        assert_eq!((doubled & 255, doubled >> 24), (200, 255));
    }

    #[test]
    fn invert_twice_restores_a_premultiplied_image() {
        let img = Image { w: 3, h: 1, px: vec![pack(0, 0, 0, 255), pack(10, 20, 30, 100), pack(0, 0, 0, 0)] };
        assert_eq!(invert(&invert(&img)), img);
        assert_eq!(invert(&img).px[0], pack(255, 255, 255, 255));
        assert_eq!(invert(&img).px[2], 0);
    }

    #[test]
    fn a_transfer_round_trip_stays_within_the_eight_bit_bound() {
        let img = Image { w: 4, h: 1, px: vec![pack(0, 0, 0, 255), pack(255, 255, 255, 255), pack(128, 64, 32, 255), pack(200, 10, 90, 255)] };
        let back = convert_transfer(&convert_transfer(&img, Transfer::Srgb, Transfer::Linear), Transfer::Linear, Transfer::Srgb);
        for (a, b) in img.px.iter().zip(&back.px) {
            for shift in [0u32, 8, 16] {
                let d = ((a >> shift) & 255) as i32 - ((b >> shift) & 255) as i32;
                // 8-bit linear light cannot hold the darkest sRGB codes exactly: code 10 comes back three steps off
                // and the worst code (1 to 7) six. Bound the darks by what the 8-bit intermediate allows.
                assert!(d.abs() <= 6, "{d}");
            }
            assert_eq!(a >> 24, b >> 24);
        }
        let mid = convert_transfer(&grey(1, 1, &[188]), Transfer::Srgb, Transfer::Linear);
        assert!(((mid.px[0] & 255) as i32 - 128).abs() <= 2, "sRGB 188 is about half of linear light");
    }

    #[test]
    fn transposing_twice_is_the_identity() {
        let img = red_image(3, 2, &[1, 2, 3, 4, 5, 6]);
        let t = transpose(&img);
        assert_eq!((t.w, t.h), (2, 3));
        assert_eq!(t.px.iter().map(|&p| p & 255).collect::<Vec<_>>(), vec![1, 4, 2, 5, 3, 6]);
        assert_eq!(transpose(&t), img);
    }

    #[test]
    fn convolution_with_an_identity_kernel_changes_nothing() {
        let img = grey(4, 3, &[0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110]);
        let mut id = [0.0f32; 9];
        id[4] = 1.0;
        assert_eq!(convolve(&img, 3, 3, &id, 0.0).unwrap(), img);
        assert!(convolve(&img, 2, 3, &[0.0; 6], 0.0).is_none());
        assert!(convolve(&img, 3, 3, &[0.0; 8], 0.0).is_none());
        assert!(convolve(&img, 11, 3, &[0.0; 33], 0.0).is_none());
    }

    #[test]
    fn tent_blur_of_a_flat_image_is_flat() {
        let flat = grey(5, 5, &[77; 25]);
        assert_eq!(tent_blur(&flat, 2), flat);
    }

    #[test]
    fn oversized_arguments_are_capped_instead_of_allocated() {
        let img = grey(4, 4, &[50; 16]);
        assert_eq!(tent_blur(&img, usize::MAX), img);
        assert_eq!(KernelSpec::Median { diameter: 100_001 }.run(&img), img);
        assert_eq!(KernelSpec::Dilate { w: usize::MAX, h: 3 }.run(&img), img);
        assert_eq!(KernelSpec::Gaussian { sigma: 1e9 }.run(&img).px.len(), 16);
    }

    #[test]
    fn a_gaussian_pyramid_halves_each_level() {
        let img = grey(9, 5, &[100; 45]);
        let p = gaussian_pyramid(&img, 3);
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].w, p[0].h), (9, 5));
        assert_eq!((p[1].w, p[1].h), (5, 3));
        assert_eq!((p[2].w, p[2].h), (3, 2));
        assert_eq!(p[0], img);
        assert!(gaussian_pyramid(&img, 0).is_empty());
        assert_eq!(gaussian_pyramid(&img, 1000).len(), MAX_LEVELS);
    }

    #[test]
    fn collapsing_a_laplacian_pyramid_restores_the_luma() {
        let values: Vec<u8> = (0..48u32).map(|i| ((i * 37) % 251) as u8).collect();
        let img = grey(8, 6, &values);
        let levels = laplacian_pyramid(&img, 3);
        assert_eq!(levels.len(), 3);
        assert_eq!(collapse_laplacian(&levels).unwrap(), grey_image(&luma_plane(&img)));
        assert!(collapse_laplacian(&[]).is_none());
        assert!(laplacian_pyramid(&img, 0).is_empty());
    }

    #[test]
    fn the_distance_transform_counts_pixels_to_the_nearest_lit_one() {
        let img = grey(4, 1, &[255, 0, 0, 0]);
        assert_eq!(distance_transform(&img, None), vec![0.0, 1.0, 2.0, 3.0]);
        let limited = distance_transform(&img, Some(1.5));
        assert_eq!(limited[1], 1.0);
        assert!(limited[3].is_infinite());
        assert!(distance_transform(&Image { w: 0, h: 0, px: vec![] }, None).is_empty());
    }

    #[test]
    fn the_guided_filter_keeps_a_flat_image_and_refuses_bad_input() {
        let flat = grey(5, 5, &[128; 25]);
        let out = guided_filter(&flat, &flat, 3, 0.01).unwrap();
        assert!(out.px.iter().all(|&p| ((p & 255) as i32 - 128).abs() <= 1));
        assert!(guided_filter(&flat, &flat, 4, 0.01).is_none());
        assert!(guided_filter(&flat, &grey(4, 5, &[0; 20]), 3, 0.01).is_none());
    }

    #[test]
    fn keypoints_come_in_scan_order_and_respect_the_limit() {
        let img = grey(3, 2, &[0, 200, 0, 250, 0, 220]);
        let all = find_keypoints(&img, (0, 0, 3, 2), 200, 10);
        assert_eq!(all, vec![Keypoint { x: 1, y: 0, value: 200 }, Keypoint { x: 0, y: 1, value: 250 }, Keypoint { x: 2, y: 1, value: 220 }]);
        assert_eq!(find_keypoints(&img, (0, 0, 3, 2), 200, 2).len(), 2);
        assert_eq!(find_keypoints(&img, (1, 1, 5, 5), 200, 10), vec![Keypoint { x: 2, y: 1, value: 220 }]);
        assert!(find_keypoints(&img, (9, 9, 1, 1), 0, 10).is_empty());
        assert!(find_keypoints(&img, (0, 0, 3, 2), 0, 0).is_empty());
    }

    #[test]
    fn mps_kernels_run_as_image_kernels() {
        let img = grey(3, 3, &[10; 9]);
        assert_eq!(KernelSpec::Invert.kind(), KernelKind::General);
        let chained = apply_chain(&[&KernelSpec::Invert, &KernelSpec::Invert], &img);
        assert_eq!(chained, img);
        // A bad argument returns the input.
        assert_eq!(KernelSpec::Erode { w: 2, h: 2 }.run(&img), img);
        let bin = KernelSpec::Threshold { mode: Threshold::Binary, t: 5, max: 255 }.run(&img);
        assert!(bin.px.iter().all(|&p| p & 255 == 255));
        assert_eq!(run_chain(&[], &img), img);
    }

    fn sample_chain() -> Vec<KernelSpec> {
        vec![
            KernelSpec::Gaussian { sigma: 1.5 },
            KernelSpec::Tent { radius: 2 },
            KernelSpec::Threshold { mode: Threshold::ToZeroInverse, t: 100, max: 200 },
            KernelSpec::Erode { w: 3, h: 5 },
            KernelSpec::Dilate { w: 3, h: 3 },
            KernelSpec::Median { diameter: 3 },
            KernelSpec::Sobel,
            KernelSpec::Laplacian { bias: 0.25 },
            KernelSpec::Equalize,
            KernelSpec::Invert,
            KernelSpec::Convolution { w: 3, h: 3, weights: vec![0.0, -1.0, 0.0, -1.0, 4.5, -1.0, 0.0, -1.0, 0.0], bias: 0.1 },
            KernelSpec::Scale { w: 64, h: 32, mode: ScaleMode::Lanczos },
        ]
    }

    #[test]
    fn a_kernel_chain_round_trips_through_the_archive() {
        let chain = sample_chain();
        let text = encode_kernels(&chain);
        assert!(text.starts_with("bifrost-kernels 1\n"));
        assert_eq!(decode_kernels(&text).unwrap(), chain);
        assert_eq!(decode_kernels(&encode_kernels(&[])).unwrap(), Vec::new());
    }

    #[test]
    fn a_restored_chain_gives_the_same_pixels() {
        let values: Vec<u8> = (0..64u32).map(|i| ((i * 29) % 256) as u8).collect();
        let img = grey(8, 8, &values);
        let chain = vec![KernelSpec::Gaussian { sigma: 1.0 }, KernelSpec::Sobel, KernelSpec::Invert];
        let restored = decode_kernels(&encode_kernels(&chain)).unwrap();
        assert_eq!(run_chain(&chain, &img), run_chain(&restored, &img));
    }

    #[test]
    fn hostile_archives_are_refused() {
        assert_eq!(decode_kernels(""), Err(ArchiveError::BadHeader));
        assert_eq!(decode_kernels("bifrost-kernels 2\n"), Err(ArchiveError::BadHeader));
        let h = "bifrost-kernels 1\n";
        assert_eq!(decode_kernels(&format!("{h}mystery 1\n")), Err(ArchiveError::Line { line: 2, reason: "unknown kernel" }));
        assert_eq!(decode_kernels(&format!("{h}sobel\ngaussian\n")), Err(ArchiveError::Line { line: 3, reason: "wrong number of arguments" }));
        assert_eq!(decode_kernels(&format!("{h}gaussian NaN\n")), Err(ArchiveError::Line { line: 2, reason: "not a finite number" }));
        assert_eq!(decode_kernels(&format!("{h}gaussian inf\n")), Err(ArchiveError::Line { line: 2, reason: "not a finite number" }));
        assert_eq!(decode_kernels(&format!("{h}tent -1\n")), Err(ArchiveError::Line { line: 2, reason: "not a whole number" }));
        assert_eq!(decode_kernels(&format!("{h}threshold binary 256 0\n")), Err(ArchiveError::Line { line: 2, reason: "not a number from 0 to 255" }));
        assert_eq!(decode_kernels(&format!("{h}erode 1000 3\n")), Err(ArchiveError::Line { line: 2, reason: "a window side is at most 255" }));
        assert_eq!(decode_kernels(&format!("{h}scale 99999999 99999999 nearest\n")), Err(ArchiveError::Line { line: 2, reason: "a scale side is from 1 to 16384" }));
        assert_eq!(decode_kernels(&format!("{h}convolution 3 3 0 1 2\n")), Err(ArchiveError::Line { line: 2, reason: "the weight count is not width times height" }));
        assert_eq!(decode_kernels(&format!("{h}convolution 100000 100000 0\n")), Err(ArchiveError::Line { line: 2, reason: "a convolution side is odd and from 3 to 9" }));
        let long = format!("{h}sobel {}\n", "x".repeat(MAX_LINE_BYTES));
        assert_eq!(decode_kernels(&long), Err(ArchiveError::Line { line: 2, reason: "line too long" }));
        let many = format!("{h}{}", "sobel\n".repeat(MAX_KERNELS + 1));
        assert_eq!(decode_kernels(&many), Err(ArchiveError::TooManyKernels));
        assert_eq!(decode_kernels(&format!("{h}\n\nsobel\n")).unwrap(), vec![KernelSpec::Sobel]);
    }
}

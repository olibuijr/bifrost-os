//! CPU image filters on 8-bit single-channel planes: the five threshold modes,
//! histograms with histogram equalization, rectangular minimum and maximum
//! (erosion and dilation), convolution with a caller kernel, the median, Sobel
//! and Laplacian filters, and element-wise arithmetic and transposition. The
//! modes follow the reference image-filter classes of the same names. Everything
//! here is plain std code over a `Plane`, so it runs without a GPU, and it has
//! no app-facing entry point yet.

/// An 8-bit single-channel image, row-major, `width * height` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Plane {
    /// Wraps `pixels` as a plane. Returns `None` when the byte count does not
    /// match the size.
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Option<Self> {
        (pixels.len() == width.checked_mul(height)?).then_some(Plane { width, height, pixels })
    }

    /// A plane where every pixel holds `value`.
    pub fn filled(width: usize, height: usize, value: u8) -> Self {
        Plane { width, height, pixels: vec![value; width * height] }
    }
}

/// A threshold mode. `t` is the threshold and `max` the output maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threshold {
    /// `v > t` gives `max`, otherwise 0.
    Binary,
    /// `v > t` gives 0, otherwise `max`.
    BinaryInverse,
    /// `v > t` keeps `v`, otherwise 0.
    ToZero,
    /// `v > t` gives 0, otherwise `v` is kept.
    ToZeroInverse,
    /// `v` is clamped to at most `t`. `max` is not used.
    Truncate,
}

/// Applies a threshold mode to every pixel.
pub fn threshold(src: &Plane, mode: Threshold, t: u8, max: u8) -> Plane {
    let pixels = src
        .pixels
        .iter()
        .map(|&v| match mode {
            Threshold::Binary => if v > t { max } else { 0 },
            Threshold::BinaryInverse => if v > t { 0 } else { max },
            Threshold::ToZero => if v > t { v } else { 0 },
            Threshold::ToZeroInverse => if v > t { 0 } else { v },
            Threshold::Truncate => v.min(t),
        })
        .collect();
    Plane { width: src.width, height: src.height, pixels }
}

/// How a histogram is binned: `entries` bins over the value range `min..=max`.
/// Values outside the range count in the nearest end bin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistogramInfo {
    pub entries: usize,
    pub min: u8,
    pub max: u8,
}

impl HistogramInfo {
    /// Returns `None` unless `1 <= entries <= 256` and `min < max`.
    pub fn new(entries: usize, min: u8, max: u8) -> Option<Self> {
        (1..=256).contains(&entries).then_some(())?;
        (min < max).then_some(HistogramInfo { entries, min, max })
    }

    /// The bin index for pixel value `v`.
    pub fn bin(&self, v: u8) -> usize {
        let v = v.clamp(self.min, self.max);
        let range = (self.max - self.min) as u64 + 1;
        let idx = (v - self.min) as u64 * self.entries as u64 / range;
        (idx as usize).min(self.entries - 1)
    }
}

/// Counts the pixels of `src` in each bin of `info`. The counts sum to the pixel count.
pub fn histogram(src: &Plane, info: HistogramInfo) -> Vec<u32> {
    let mut bins = vec![0u32; info.entries];
    for &v in &src.pixels {
        bins[info.bin(v)] += 1;
    }
    bins
}

/// Builds the equalization lookup table for a histogram: bin `i` maps to the
/// output value spread over 0..=255 by the cumulative count. A histogram with
/// one occupied bin maps every bin to itself.
pub fn equalization_table(bins: &[u32]) -> Vec<u8> {
    let total: u64 = bins.iter().map(|&c| c as u64).sum();
    let first = bins.iter().copied().find(|&c| c > 0).unwrap_or(0) as u64;
    if total <= first {
        return (0..bins.len()).map(|i| i.min(255) as u8).collect();
    }
    let mut cumulative = 0u64;
    bins.iter()
        .map(|&c| {
            cumulative += c as u64;
            (cumulative.saturating_sub(first) * 255 / (total - first)) as u8
        })
        .collect()
}

/// Histogram equalization of an 8-bit plane: the 256-bin histogram of the
/// pixels is mapped through its cumulative distribution.
pub fn equalize(src: &Plane) -> Plane {
    let info = HistogramInfo { entries: 256, min: 0, max: 255 };
    let lut = equalization_table(&histogram(src, info));
    let pixels = src.pixels.iter().map(|&v| lut[v as usize]).collect();
    Plane { width: src.width, height: src.height, pixels }
}

/// Whether a rectangular extreme keeps the smallest or the largest value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extreme {
    Min,
    Max,
}

/// The minimum (`Extreme::Min`) or maximum over a `kw` by `kh` rectangle
/// centred on each pixel. Pixels outside the plane repeat the nearest edge
/// pixel. Min is erosion and max is dilation. Returns `None` unless both sides
/// are odd and at least 1.
pub fn area_filter(src: &Plane, kw: usize, kh: usize, extreme: Extreme) -> Option<Plane> {
    if kw == 0 || kh == 0 || kw % 2 == 0 || kh % 2 == 0 {
        return None;
    }
    let (w, h) = (src.width, src.height);
    let (rw, rh) = (kw / 2, kh / 2);
    let pick = |a: u8, b: u8| match extreme {
        Extreme::Min => a.min(b),
        Extreme::Max => a.max(b),
    };
    // Horizontal pass, then vertical pass. A rectangle extreme is separable,
    // so this costs w * h * (kw + kh) rather than w * h * kw * kh.
    let mut tmp = vec![0u8; w * h];
    for y in 0..h {
        let row = &src.pixels[y * w..(y + 1) * w];
        for x in 0..w {
            let mut acc = row[x];
            for dx in 1..=rw {
                acc = pick(acc, row[x.saturating_sub(dx)]);
                acc = pick(acc, row[(x + dx).min(w - 1)]);
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = tmp[y * w + x];
            for dy in 1..=rh {
                acc = pick(acc, tmp[y.saturating_sub(dy) * w + x]);
                acc = pick(acc, tmp[(y + dy).min(h - 1) * w + x]);
            }
            pixels[y * w + x] = acc;
        }
    }
    Some(Plane { width: w, height: h, pixels })
}

/// Erosion: the rectangular minimum. See `area_filter`.
pub fn erode(src: &Plane, kw: usize, kh: usize) -> Option<Plane> {
    area_filter(src, kw, kh, Extreme::Min)
}

/// Dilation: the rectangular maximum. See `area_filter`.
pub fn dilate(src: &Plane, kw: usize, kh: usize) -> Option<Plane> {
    area_filter(src, kw, kh, Extreme::Max)
}

/// Whether `n` is an odd side length (1, 3, 5, ...).
fn odd_side(n: usize) -> bool {
    n % 2 == 1
}

/// The pixel at (`x`, `y`), with coordinates outside the plane clamped to the nearest edge pixel.
/// Callers only ask on a non-empty plane.
fn edge_pixel(src: &Plane, x: isize, y: isize) -> u8 {
    let cx = x.clamp(0, src.width as isize - 1) as usize;
    let cy = y.clamp(0, src.height as isize - 1) as usize;
    src.pixels[cy * src.width + cx]
}

/// Rounds a filter sum to a byte, clamping to 0..=255.
fn to_byte(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Convolves `src` with a `kw` by `kh` kernel. `weights` is row-major and is applied as it is laid out, with
/// no flip. `bias` is added to each sum before it is rounded and clamped to 0..=255. Pixels outside the plane
/// repeat the nearest edge pixel. Returns `None` unless both sides are odd and `weights` has `kw * kh` entries.
pub fn convolve(src: &Plane, kw: usize, kh: usize, weights: &[f32], bias: f32) -> Option<Plane> {
    if !odd_side(kw) || !odd_side(kh) || weights.len() != kw * kh {
        return None;
    }
    let (rw, rh) = ((kw / 2) as isize, (kh / 2) as isize);
    let (w, h) = (src.width, src.height);
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = bias;
            for ky in 0..kh {
                for kx in 0..kw {
                    let sx = x as isize + kx as isize - rw;
                    let sy = y as isize + ky as isize - rh;
                    acc += weights[ky * kw + kx] * (edge_pixel(src, sx, sy) as f32);
                }
            }
            pixels[y * w + x] = to_byte(acc);
        }
    }
    Some(Plane { width: w, height: h, pixels })
}

/// The four-neighbour Laplacian: the 3 by 3 kernel `0 1 0 / 1 -4 1 / 0 1 0` with `bias` added before
/// clamping. A flat area gives `bias`, so a bias of 128 keeps negative responses visible.
pub fn laplacian(src: &Plane, bias: f32) -> Plane {
    const KERNEL: [f32; 9] = [0.0, 1.0, 0.0, 1.0, -4.0, 1.0, 0.0, 1.0, 0.0];
    convolve(src, 3, 3, &KERNEL, bias).expect("a 3 by 3 kernel with nine weights is valid")
}

/// The Sobel gradient magnitude, `sqrt(gx^2 + gy^2)`, from the 3 by 3 horizontal and vertical Sobel
/// kernels, clamped to 0..=255. Pixels outside the plane repeat the nearest edge pixel.
pub fn sobel(src: &Plane) -> Plane {
    let (w, h) = (src.width, src.height);
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let p = |dx: isize, dy: isize| -> f32 { edge_pixel(src, xi + dx, yi + dy) as f32 };
            let gx = (p(1, -1) + 2.0 * p(1, 0) + p(1, 1)) - (p(-1, -1) + 2.0 * p(-1, 0) + p(-1, 1));
            let gy = (p(-1, 1) + 2.0 * p(0, 1) + p(1, 1)) - (p(-1, -1) + 2.0 * p(0, -1) + p(1, -1));
            pixels[y * w + x] = to_byte((gx * gx + gy * gy).sqrt());
        }
    }
    Plane { width: w, height: h, pixels }
}

/// The median of the `diameter` by `diameter` square centred on each pixel. Pixels outside the plane repeat
/// the nearest edge pixel. Returns `None` unless `diameter` is odd and at least 1.
pub fn median(src: &Plane, diameter: usize) -> Option<Plane> {
    if !odd_side(diameter) {
        return None;
    }
    let r = (diameter / 2) as isize;
    let (w, h) = (src.width, src.height);
    let mut window: Vec<u8> = Vec::with_capacity(diameter * diameter);
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            window.clear();
            for dy in -r..=r {
                for dx in -r..=r {
                    window.push(edge_pixel(src, x as isize + dx, y as isize + dy));
                }
            }
            let mid = window.len() / 2;
            pixels[y * w + x] = *window.select_nth_unstable(mid).1;
        }
    }
    Some(Plane { width: w, height: h, pixels })
}

/// An element-wise operation on two planes. Values are read as fractions of 255, so `Multiply` is
/// `a * b / 255` and `Divide` is `a * 255 / b`, which saturates to 255 when `b` is 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// Applies `op` to each pair of pixels from `a` and `b`. Add and divide saturate at 255, subtract at 0.
/// Returns `None` when the two planes differ in size.
pub fn arithmetic(a: &Plane, b: &Plane, op: Arithmetic) -> Option<Plane> {
    if a.width != b.width || a.height != b.height {
        return None;
    }
    let pixels = a
        .pixels
        .iter()
        .zip(&b.pixels)
        .map(|(&x, &y)| {
            let (x, y) = (x as u32, y as u32);
            let v = match op {
                Arithmetic::Add => (x + y).min(255),
                Arithmetic::Subtract => x.saturating_sub(y),
                Arithmetic::Multiply => x * y / 255,
                Arithmetic::Divide => {
                    if y == 0 {
                        255
                    } else {
                        (x * 255 / y).min(255)
                    }
                }
            };
            v as u8
        })
        .collect();
    Some(Plane { width: a.width, height: a.height, pixels })
}

/// Transposes `src`: the result has `src.height` columns and `src.width` rows, and pixel (x, y) moves to (y, x).
pub fn transpose(src: &Plane) -> Plane {
    let (w, h) = (src.width, src.height);
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            pixels[x * h + y] = src.pixels[y * w + x];
        }
    }
    Plane { width: h, height: w, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[u8]) -> Plane {
        Plane::new(values.len(), 1, values.to_vec()).unwrap()
    }

    #[test]
    fn plane_rejects_a_wrong_byte_count() {
        assert!(Plane::new(3, 2, vec![0; 5]).is_none());
        assert_eq!(Plane::new(3, 2, vec![7; 6]).unwrap().pixels.len(), 6);
    }

    #[test]
    fn threshold_modes_follow_their_definitions() {
        let src = row(&[0, 100, 101, 255]);
        let out = |m| threshold(&src, m, 100, 200).pixels;
        assert_eq!(out(Threshold::Binary), vec![0, 0, 200, 200]);
        assert_eq!(out(Threshold::BinaryInverse), vec![200, 200, 0, 0]);
        assert_eq!(out(Threshold::ToZero), vec![0, 0, 101, 255]);
        assert_eq!(out(Threshold::ToZeroInverse), vec![0, 100, 0, 0]);
        assert_eq!(out(Threshold::Truncate), vec![0, 100, 100, 100]);
    }

    #[test]
    fn histogram_bins_cover_the_full_range() {
        let src = row(&[0, 1, 1, 255]);
        let bins = histogram(&src, HistogramInfo::new(256, 0, 255).unwrap());
        assert_eq!(bins.len(), 256);
        assert_eq!((bins[0], bins[1], bins[255]), (1, 2, 1));
        assert_eq!(bins.iter().sum::<u32>(), 4);
    }

    #[test]
    fn histogram_clips_values_outside_the_range_into_end_bins() {
        let src = row(&[0, 150, 199, 255]);
        let info = HistogramInfo::new(4, 100, 199).unwrap();
        assert_eq!(histogram(&src, info), vec![1, 0, 1, 2]);
        assert!(HistogramInfo::new(0, 0, 255).is_none());
        assert!(HistogramInfo::new(4, 9, 9).is_none());
    }

    #[test]
    fn equalization_spreads_a_two_level_image_to_both_ends() {
        let out = equalize(&row(&[10, 10, 10, 200]));
        assert_eq!(out.pixels, vec![0, 0, 0, 255]);
    }

    #[test]
    fn equalization_leaves_a_flat_image_unchanged() {
        let src = Plane::filled(4, 2, 77);
        assert_eq!(equalize(&src), src);
    }

    #[test]
    fn dilate_grows_a_bright_pixel_across_the_kernel() {
        let src = row(&[0, 0, 255, 0, 0]);
        assert_eq!(dilate(&src, 3, 1).unwrap().pixels, vec![0, 255, 255, 255, 0]);
    }

    #[test]
    fn erode_shrinks_a_dark_gap_and_clamps_at_the_edge() {
        let src = row(&[255, 255, 0, 255, 255]);
        assert_eq!(erode(&src, 3, 1).unwrap().pixels, vec![255, 0, 0, 0, 255]);
    }

    #[test]
    fn two_dimensional_dilation_fills_a_three_by_three_block() {
        let mut src = Plane::filled(5, 5, 0);
        src.pixels[2 * 5 + 2] = 255;
        let out = dilate(&src, 3, 3).unwrap();
        for y in 0..5 {
            for x in 0..5 {
                let inside = (1..=3).contains(&x) && (1..=3).contains(&y);
                assert_eq!(out.pixels[y * 5 + x], if inside { 255 } else { 0 }, "({x}, {y})");
            }
        }
    }

    #[test]
    fn even_or_zero_kernels_are_rejected() {
        let src = Plane::filled(3, 3, 1);
        assert!(erode(&src, 2, 3).is_none());
        assert!(dilate(&src, 3, 0).is_none());
    }

    #[test]
    fn convolution_with_a_box_kernel_keeps_a_flat_plane() {
        let src = Plane::filled(3, 3, 90);
        let weights = [1.0f32 / 9.0; 9];
        assert_eq!(convolve(&src, 3, 3, &weights, 0.0).unwrap().pixels, vec![90u8; 9]);
    }

    #[test]
    fn convolution_refuses_even_sides_and_a_wrong_weight_count() {
        let src = Plane::filled(3, 3, 1);
        assert!(convolve(&src, 2, 3, &[1.0; 6], 0.0).is_none());
        assert!(convolve(&src, 3, 3, &[1.0; 8], 0.0).is_none());
    }

    #[test]
    fn convolution_adds_the_bias_and_clamps_both_ends() {
        let src = row(&[10, 200]);
        assert_eq!(convolve(&src, 1, 1, &[1.0], 100.0).unwrap().pixels, vec![110, 255]);
        assert_eq!(convolve(&src, 1, 1, &[1.0], -50.0).unwrap().pixels, vec![0, 150]);
    }

    #[test]
    fn laplacian_marks_a_bright_dot_on_its_four_neighbours() {
        let mut src = Plane::filled(5, 5, 0);
        src.pixels[2 * 5 + 2] = 255;
        let out = laplacian(&src, 0.0);
        // The centre is 0 - 4 * 255, clamped to 0. Its four neighbours see the dot once, so they read 255.
        assert_eq!(out.pixels[2 * 5 + 2], 0);
        assert_eq!(out.pixels[2 * 5 + 1], 255);
        assert_eq!(out.pixels[5 + 2], 255);
        assert_eq!(out.pixels[2 * 5 + 3], 255);
        assert_eq!(out.pixels[3 * 5 + 2], 255);
        assert_eq!(out.pixels[1 * 5 + 1], 0);
    }

    #[test]
    fn laplacian_of_a_flat_plane_is_the_bias() {
        let src = Plane::filled(4, 4, 77);
        assert_eq!(laplacian(&src, 128.0).pixels, vec![128u8; 16]);
        assert_eq!(laplacian(&src, 0.0).pixels, vec![0u8; 16]);
    }

    #[test]
    fn sobel_responds_to_a_step_edge_and_is_flat_elsewhere() {
        // Columns hold 0, 0, 16, 16 in every row. A step of 16 over a 4-weight column gives 64.
        let src = Plane::new(4, 3, vec![0, 0, 16, 16, 0, 0, 16, 16, 0, 0, 16, 16]).unwrap();
        let out = sobel(&src);
        assert_eq!(out.pixels[0..4].to_vec(), vec![0, 64, 64, 0]);
        assert_eq!(out.pixels[4..8].to_vec(), vec![0, 64, 64, 0]);
    }

    #[test]
    fn sobel_of_a_flat_plane_is_zero() {
        assert_eq!(sobel(&Plane::filled(3, 2, 200)).pixels, vec![0u8; 6]);
    }

    #[test]
    fn median_removes_a_single_outlier() {
        let mut src = Plane::filled(3, 3, 10);
        src.pixels[4] = 255;
        assert_eq!(median(&src, 3).unwrap().pixels, vec![10u8; 9]);
    }

    #[test]
    fn median_of_diameter_one_is_the_identity_and_even_diameters_are_refused() {
        let src = row(&[3, 9, 1]);
        assert_eq!(median(&src, 1).unwrap(), src);
        assert!(median(&src, 2).is_none());
    }

    #[test]
    fn arithmetic_saturates_on_add_and_subtract() {
        let a = row(&[200, 50, 128, 64]);
        let b = row(&[100, 80, 128, 0]);
        assert_eq!(arithmetic(&a, &b, Arithmetic::Add).unwrap().pixels, vec![255, 130, 255, 64]);
        assert_eq!(arithmetic(&a, &b, Arithmetic::Subtract).unwrap().pixels, vec![100, 0, 0, 64]);
    }

    #[test]
    fn arithmetic_multiplies_and_divides_as_fractions_of_255() {
        let a = row(&[200, 50, 128, 64]);
        let b = row(&[100, 80, 128, 0]);
        // 200 * 100 / 255 = 78.4, 50 * 80 / 255 = 15.6, 128 * 128 / 255 = 64.2, 64 * 0 / 255 = 0.
        assert_eq!(arithmetic(&a, &b, Arithmetic::Multiply).unwrap().pixels, vec![78, 15, 64, 0]);
        // 200 * 255 / 100 saturates, 50 * 255 / 80 = 159.4, 128 * 255 / 128 = 255, divide by 0 gives 255.
        assert_eq!(arithmetic(&a, &b, Arithmetic::Divide).unwrap().pixels, vec![255, 159, 255, 255]);
    }

    #[test]
    fn arithmetic_refuses_planes_of_different_sizes() {
        let a = Plane::filled(2, 2, 1);
        let b = Plane::filled(4, 1, 1);
        assert!(arithmetic(&a, &b, Arithmetic::Add).is_none());
    }

    #[test]
    fn transpose_swaps_rows_and_columns() {
        let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
        let out = transpose(&src);
        assert_eq!((out.width, out.height), (2, 3));
        assert_eq!(out.pixels, vec![1, 4, 2, 5, 3, 6]);
        assert_eq!(transpose(&out), src);
    }
}

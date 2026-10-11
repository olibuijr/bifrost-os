//! CPU image filters on 8-bit single-channel planes: the five threshold modes,
//! histograms with histogram equalization, rectangular minimum and maximum
//! (erosion and dilation), convolution with an arbitrary odd kernel, median,
//! the Sobel gradients and the Laplacian. The modes follow the reference
//! image-filter classes of the same names. Everything here is plain std code
//! over a `Plane`, so it runs without a GPU, and it has no app-facing entry
//! point yet. Reductions, integral images and arithmetic live in
//! `plane_stats` and `plane_arith`.

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

/// Whether `n` is an odd side length of at least 1.
fn odd_side(n: usize) -> bool {
    n >= 1 && n % 2 == 1
}

/// The source index for window offset `step` around `at`, clamped to the
/// plane. `len` must be at least 1.
fn window_index(at: usize, step: usize, radius: usize, len: usize) -> usize {
    (at as isize + step as isize - radius as isize).clamp(0, len as isize - 1) as usize
}

/// Correlates `src` with a `kw` by `kh` kernel. The caller has checked the sizes.
fn correlate(src: &Plane, kw: usize, kh: usize, weights: &[f32], bias: f32) -> Plane {
    let (w, h) = (src.width, src.height);
    let (rw, rh) = (kw / 2, kh / 2);
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = bias;
            for j in 0..kh {
                let sy = window_index(y, j, rh, h);
                for i in 0..kw {
                    let sx = window_index(x, i, rw, w);
                    acc += weights[j * kw + i] * src.pixels[sy * w + sx] as f32;
                }
            }
            pixels[y * w + x] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
        }
    }
    Plane { width: w, height: h, pixels }
}

/// Convolution with a `kw` by `kh` kernel, stored row by row with the top-left
/// weight first. Output pixel (x, y) is the weighted sum of the window centred
/// on it (the kernel is applied without flipping, as a correlation), plus
/// `bias`, rounded and clamped to 0..=255. Pixels outside the plane repeat the
/// nearest edge pixel. Returns `None` unless both sides are odd and at least 1
/// and `weights` holds exactly `kw * kh` values.
pub fn convolve(src: &Plane, kw: usize, kh: usize, weights: &[f32], bias: f32) -> Option<Plane> {
    if !odd_side(kw) || !odd_side(kh) || weights.len() != kw * kh {
        return None;
    }
    Some(correlate(src, kw, kh, weights, bias))
}

/// The median of each `diameter` by `diameter` window, centred on each pixel,
/// with edge pixels repeated outside the plane. Returns `None` unless the
/// diameter is odd and at least 1.
pub fn median(src: &Plane, diameter: usize) -> Option<Plane> {
    if !odd_side(diameter) {
        return None;
    }
    let (w, h) = (src.width, src.height);
    let radius = diameter / 2;
    let mut pixels = vec![0u8; w * h];
    let mut window = Vec::with_capacity(diameter * diameter);
    for y in 0..h {
        for x in 0..w {
            window.clear();
            for j in 0..diameter {
                let sy = window_index(y, j, radius, h);
                for i in 0..diameter {
                    let sx = window_index(x, i, radius, w);
                    window.push(src.pixels[sy * w + sx]);
                }
            }
            let mid = window.len() / 2;
            window.select_nth_unstable(mid);
            pixels[y * w + x] = window[mid];
        }
    }
    Some(Plane { width: w, height: h, pixels })
}

/// The direction of a Sobel gradient. `Horizontal` responds to brightness
/// that rises from left to right, `Vertical` to brightness that rises from top
/// to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gradient {
    Horizontal,
    Vertical,
}

const SOBEL_HORIZONTAL: [f32; 9] = [-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0];
const SOBEL_VERTICAL: [f32; 9] = [-1.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0];
const LAPLACIAN: [f32; 9] = [0.0, -1.0, 0.0, -1.0, 4.0, -1.0, 0.0, -1.0, 0.0];

/// The 3 by 3 Sobel gradient in one direction. Only the positive part is kept:
/// a falling edge gives 0, so an 8-bit output cannot hold a negative value.
pub fn sobel(src: &Plane, direction: Gradient) -> Plane {
    let weights = match direction {
        Gradient::Horizontal => &SOBEL_HORIZONTAL,
        Gradient::Vertical => &SOBEL_VERTICAL,
    };
    correlate(src, 3, 3, weights, 0.0)
}

/// The 3 by 3 Laplacian (centre 4, four edge neighbours -1, corners 0), plus
/// `bias`, clamped to 0..=255. A flat plane gives `bias` everywhere.
pub fn laplacian(src: &Plane, bias: f32) -> Plane {
    correlate(src, 3, 3, &LAPLACIAN, bias)
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
    fn convolution_with_a_centre_one_kernel_is_the_identity() {
        let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
        let mut weights = [0.0f32; 9];
        weights[4] = 1.0;
        assert_eq!(convolve(&src, 3, 3, &weights, 0.0).unwrap(), src);
    }

    #[test]
    fn convolution_averages_a_three_tap_window_with_edge_repeat() {
        let src = row(&[0, 0, 255, 0, 0]);
        let third = [1.0f32 / 3.0; 3];
        // Window sums: x=0 sees 0,0,0; x=1 sees 0,0,255; x=2 sees 0,255,0; x=3 sees 255,0,0; x=4 sees 0,0,0.
        assert_eq!(convolve(&src, 3, 1, &third, 0.0).unwrap().pixels, vec![0, 85, 85, 85, 0]);
    }

    #[test]
    fn convolution_adds_bias_then_clamps_both_ends() {
        let src = row(&[0, 100, 200]);
        // 2 * v - 100: -100 clamps to 0, 100 stays, 300 clamps to 255.
        assert_eq!(convolve(&src, 1, 1, &[2.0], -100.0).unwrap().pixels, vec![0, 100, 255]);
    }

    #[test]
    fn convolution_rejects_even_sides_and_wrong_weight_counts() {
        let src = Plane::filled(3, 3, 1);
        assert!(convolve(&src, 2, 3, &[0.0; 6], 0.0).is_none());
        assert!(convolve(&src, 3, 3, &[0.0; 8], 0.0).is_none());
    }

    #[test]
    fn median_removes_a_single_bright_spike() {
        let mut src = Plane::filled(3, 3, 10);
        src.pixels[4] = 255;
        assert_eq!(median(&src, 3).unwrap().pixels, vec![10; 9]);
        assert!(median(&src, 2).is_none());
    }

    #[test]
    fn sobel_responds_only_to_a_rising_vertical_edge() {
        // Columns 0 and 1 are dark, columns 2 and 3 are bright, in every row.
        let src = Plane::new(4, 3, [0u8, 0, 255, 255].repeat(3)).unwrap();
        assert_eq!(sobel(&src, Gradient::Horizontal).pixels, [0u8, 255, 255, 0].repeat(3));
        assert_eq!(sobel(&src, Gradient::Vertical).pixels, vec![0; 12]);
    }

    #[test]
    fn laplacian_of_a_flat_plane_is_the_bias() {
        let src = Plane::filled(4, 2, 50);
        assert_eq!(laplacian(&src, 128.0).pixels, vec![128; 8]);
    }

    #[test]
    fn laplacian_isolates_a_spike_at_its_centre() {
        let mut src = Plane::filled(3, 3, 0);
        src.pixels[4] = 100;
        // The centre gives 4 * 100 = 400, clamped to 255. Each edge neighbour
        // sees the spike with weight -1, which clamps to 0.
        let mut expected = vec![0u8; 9];
        expected[4] = 255;
        assert_eq!(laplacian(&src, 0.0).pixels, expected);
    }
}

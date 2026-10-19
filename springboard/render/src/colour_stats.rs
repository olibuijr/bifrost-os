//! Colour reductions over a region of an image: the average, minimum and maximum colour, per-channel
//! histograms, row and column averages, and k-means clustering of the colours. These are the colour
//! counterparts of `CIAreaAverage`, `CIAreaMinimum`, `CIAreaMaximum`, `CIAreaMinMax`, `CIAreaHistogram`,
//! `CIRowAverage`, `CIColumnAverage` and `CIKMeans`; `plane_stats` covers single grey planes.
//! Pixels are premultiplied and packed like `gfx::Image` (red in the low byte), and every channel is
//! reduced as stored, which is what the reference area filters do. A region is clipped to the image,
//! and a region with no pixels left gives `None`. Pure std code, no GPU.
use crate::gfx::Image;

/// A region in pixels: `x`, `y`, width and height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Region {
    /// The whole of `img`.
    pub fn whole(img: &Image) -> Region {
        Region { x: 0, y: 0, w: img.w, h: img.h }
    }

    /// The part inside a `w` by `h` image as `(x0, x1, y0, y1)`, `None` when empty.
    fn clip(self, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
        let x0 = self.x.min(w);
        let y0 = self.y.min(h);
        let x1 = self.x.saturating_add(self.w).min(w);
        let y1 = self.y.saturating_add(self.h).min(h);
        (x1 > x0 && y1 > y0).then_some((x0, x1, y0, y1))
    }
}

/// A colour with alpha, each channel 0..=1, as stored (premultiplied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// The packed pixel for this colour, each channel rounded and clamped.
    pub fn pack(self) -> u32 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        q(self.r) | q(self.g) << 8 | q(self.b) << 16 | q(self.a) << 24
    }

    fn from_sums(sum: [f64; 4], n: f64) -> Rgba {
        let f = |v: f64| (v / n / 255.0) as f32;
        Rgba { r: f(sum[0]), g: f(sum[1]), b: f(sum[2]), a: f(sum[3]) }
    }
}

fn channels(p: u32) -> [u8; 4] {
    [(p & 255) as u8, (p >> 8 & 255) as u8, (p >> 16 & 255) as u8, (p >> 24) as u8]
}

/// `CIAreaAverage`: the mean of every channel over the region.
pub fn area_average(img: &Image, region: Region) -> Option<Rgba> {
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let mut sum = [0f64; 4];
    for y in y0..y1 {
        for &p in &img.px[y * img.w + x0..y * img.w + x1] {
            for (s, c) in sum.iter_mut().zip(channels(p)) {
                *s += c as f64;
            }
        }
    }
    Some(Rgba::from_sums(sum, ((x1 - x0) * (y1 - y0)) as f64))
}

/// `CIAreaMinimum` and `CIAreaMaximum` in one pass: the smallest and the largest value of every
/// channel over the region, as `(minimum, maximum)` (`CIAreaMinMax`).
pub fn area_min_max(img: &Image, region: Region) -> Option<(Rgba, Rgba)> {
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let mut lo = [255u8; 4];
    let mut hi = [0u8; 4];
    for y in y0..y1 {
        for &p in &img.px[y * img.w + x0..y * img.w + x1] {
            for (i, c) in channels(p).into_iter().enumerate() {
                lo[i] = lo[i].min(c);
                hi[i] = hi[i].max(c);
            }
        }
    }
    let f = |v: [u8; 4]| Rgba { r: v[0] as f32 / 255.0, g: v[1] as f32 / 255.0, b: v[2] as f32 / 255.0, a: v[3] as f32 / 255.0 };
    Some((f(lo), f(hi)))
}

/// `CIAreaHistogram`: for each channel, how many pixels of the region fall into each of `bins` equal
/// slices of 0..=255. Returns `[red, green, blue, alpha]`, each with `bins` counts. `None` when `bins` is
/// not 1 to 256 or the region is empty.
pub fn area_histogram(img: &Image, region: Region, bins: usize) -> Option<[Vec<u32>; 4]> {
    if !(1..=256).contains(&bins) {
        return None;
    }
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let mut out = [vec![0u32; bins], vec![0u32; bins], vec![0u32; bins], vec![0u32; bins]];
    for y in y0..y1 {
        for &p in &img.px[y * img.w + x0..y * img.w + x1] {
            for (i, c) in channels(p).into_iter().enumerate() {
                out[i][c as usize * bins / 256] += 1;
            }
        }
    }
    Some(out)
}

/// `CIRowAverage`: the mean colour of each row of the region, top to bottom.
pub fn row_averages(img: &Image, region: Region) -> Option<Vec<Rgba>> {
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let n = (x1 - x0) as f64;
    Some(
        (y0..y1)
            .map(|y| {
                let mut sum = [0f64; 4];
                for &p in &img.px[y * img.w + x0..y * img.w + x1] {
                    for (s, c) in sum.iter_mut().zip(channels(p)) {
                        *s += c as f64;
                    }
                }
                Rgba::from_sums(sum, n)
            })
            .collect(),
    )
}

/// `CIColumnAverage`: the mean colour of each column of the region, left to right.
pub fn column_averages(img: &Image, region: Region) -> Option<Vec<Rgba>> {
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let n = (y1 - y0) as f64;
    Some(
        (x0..x1)
            .map(|x| {
                let mut sum = [0f64; 4];
                for y in y0..y1 {
                    for (s, c) in sum.iter_mut().zip(channels(img.px[y * img.w + x])) {
                        *s += c as f64;
                    }
                }
                Rgba::from_sums(sum, n)
            })
            .collect(),
    )
}

/// One cluster of `kmeans`: its mean colour and how many pixels it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cluster {
    pub mean: Rgba,
    pub count: usize,
}

fn dist2(a: [f64; 4], b: [f64; 4]) -> f64 {
    (0..4).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/// `CIKMeans`: groups the colours of the region into `means.len()` clusters. `means` are the starting
/// cluster centres (the reference `inputMeans`); each pass assigns every pixel to its nearest centre
/// and moves each centre to the mean of its pixels, and a centre that gets no pixels stays where it
/// was. Runs `passes` passes (at least one) or stops early when no centre moves. The clusters come back
/// in the order of `means`. `None` when `means` is empty or the region is empty.
pub fn kmeans(img: &Image, region: Region, means: &[Rgba], passes: usize) -> Option<Vec<Cluster>> {
    if means.is_empty() {
        return None;
    }
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let mut centres: Vec<[f64; 4]> =
        means.iter().map(|m| [m.r as f64 * 255.0, m.g as f64 * 255.0, m.b as f64 * 255.0, m.a as f64 * 255.0]).collect();
    let mut counts = vec![0usize; centres.len()];
    for _ in 0..passes.max(1) {
        let mut sums = vec![[0f64; 4]; centres.len()];
        counts.iter_mut().for_each(|c| *c = 0);
        for y in y0..y1 {
            for &p in &img.px[y * img.w + x0..y * img.w + x1] {
                let c = channels(p);
                let v = [c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64];
                let mut best = 0;
                let mut best_d = f64::INFINITY;
                for (i, &centre) in centres.iter().enumerate() {
                    let d = dist2(v, centre);
                    if d < best_d {
                        best = i;
                        best_d = d;
                    }
                }
                counts[best] += 1;
                for k in 0..4 {
                    sums[best][k] += v[k];
                }
            }
        }
        let mut moved = false;
        for i in 0..centres.len() {
            if counts[i] == 0 {
                continue;
            }
            let next = [0, 1, 2, 3].map(|k| sums[i][k] / counts[i] as f64);
            if dist2(next, centres[i]) > 1e-9 {
                moved = true;
            }
            centres[i] = next;
        }
        if !moved {
            break;
        }
    }
    Some(
        centres
            .iter()
            .zip(&counts)
            .map(|(c, &count)| Cluster { mean: Rgba::from_sums(*c, 1.0), count })
            .collect(),
    )
}

/// The `k` dominant colours of the region: k-means started from `k` pixels spread evenly through the
/// region's pixels ordered by luma, so the same image always gives the same answer. The clusters come
/// back largest first. `None` when `k` is 0 or the region is empty.
pub fn dominant_colours(img: &Image, region: Region, k: usize, passes: usize) -> Option<Vec<Cluster>> {
    if k == 0 {
        return None;
    }
    let (x0, x1, y0, y1) = region.clip(img.w, img.h)?;
    let mut pixels: Vec<u32> = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        pixels.extend_from_slice(&img.px[y * img.w + x0..y * img.w + x1]);
    }
    let luma = |p: u32| {
        let c = channels(p);
        c[0] as u32 * 77 + c[1] as u32 * 150 + c[2] as u32 * 29
    };
    pixels.sort_by_key(|&p| luma(p));
    let seeds: Vec<Rgba> = (0..k)
        .map(|i| {
            let at = if k == 1 { pixels.len() / 2 } else { i * (pixels.len() - 1) / (k - 1) };
            let c = channels(pixels[at]);
            Rgba { r: c[0] as f32 / 255.0, g: c[1] as f32 / 255.0, b: c[2] as f32 / 255.0, a: c[3] as f32 / 255.0 }
        })
        .collect();
    let mut out = kmeans(img, region, &seeds, passes)?;
    out.sort_by(|a, b| b.count.cmp(&a.count));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, px: &[u32]) -> Image {
        Image { w, h, px: px.to_vec() }
    }

    const BLACK: u32 = 0xff00_0000;
    const WHITE: u32 = 0xffff_ffff;
    const RED: u32 = 0xff00_00ff;

    #[test]
    fn average_of_black_and_white_is_grey() {
        let i = img(2, 1, &[BLACK, WHITE]);
        let a = area_average(&i, Region::whole(&i)).unwrap();
        assert!((a.r - 0.5).abs() < 1e-6 && (a.g - 0.5).abs() < 1e-6 && (a.a - 1.0).abs() < 1e-6);
    }

    #[test]
    fn regions_are_clipped_and_empty_regions_refused() {
        let i = img(2, 2, &[BLACK, WHITE, WHITE, WHITE]);
        let corner = area_average(&i, Region { x: 1, y: 1, w: 50, h: 50 }).unwrap();
        assert_eq!(corner.pack(), WHITE);
        assert!(area_average(&i, Region { x: 5, y: 0, w: 2, h: 2 }).is_none());
        assert!(area_average(&i, Region { x: 0, y: 0, w: 0, h: 2 }).is_none());
    }

    #[test]
    fn min_max_finds_the_extremes_per_channel() {
        let i = img(2, 1, &[RED, 0xff80_4000]);
        let (lo, hi) = area_min_max(&i, Region::whole(&i)).unwrap();
        assert_eq!(lo.pack(), 0xff00_0000);
        assert_eq!(hi.pack(), 0xff80_40ff);
    }

    #[test]
    fn histogram_counts_every_pixel_into_the_right_bin() {
        let i = img(3, 1, &[BLACK, WHITE, WHITE]);
        let h = area_histogram(&i, Region::whole(&i), 4).unwrap();
        assert_eq!(h[0], [1, 0, 0, 2]);
        assert_eq!(h[3], [0, 0, 0, 3]);
        assert!(area_histogram(&i, Region::whole(&i), 0).is_none());
        assert!(area_histogram(&i, Region::whole(&i), 257).is_none());
        let total: u32 = area_histogram(&i, Region::whole(&i), 256).unwrap()[1].iter().sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn row_and_column_averages_have_one_entry_per_line() {
        let i = img(2, 2, &[BLACK, WHITE, BLACK, BLACK]);
        let rows = row_averages(&i, Region::whole(&i)).unwrap();
        let cols = column_averages(&i, Region::whole(&i)).unwrap();
        assert_eq!((rows.len(), cols.len()), (2, 2));
        assert!((rows[0].r - 0.5).abs() < 1e-6 && rows[1].r == 0.0);
        assert!(cols[0].r == 0.0 && (cols[1].r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn kmeans_separates_two_colour_groups() {
        let mut px = vec![BLACK; 6];
        px.extend(vec![WHITE; 3]);
        let i = img(9, 1, &px);
        let dark = Rgba { r: 0.2, g: 0.2, b: 0.2, a: 1.0 };
        let light = Rgba { r: 0.8, g: 0.8, b: 0.8, a: 1.0 };
        let c = kmeans(&i, Region::whole(&i), &[dark, light], 5).unwrap();
        assert_eq!((c[0].count, c[1].count), (6, 3));
        assert_eq!(c[0].mean.pack(), BLACK);
        assert_eq!(c[1].mean.pack(), WHITE);
        assert!(kmeans(&i, Region::whole(&i), &[], 5).is_none());
    }

    #[test]
    fn dominant_colours_come_back_largest_first_and_repeat() {
        let mut px = vec![RED; 2];
        px.extend(vec![BLACK; 7]);
        let i = img(9, 1, &px);
        let a = dominant_colours(&i, Region::whole(&i), 2, 8).unwrap();
        assert_eq!(a[0].count, 7);
        assert_eq!(a[0].mean.pack(), BLACK);
        assert_eq!(a[1].mean.pack(), RED);
        assert_eq!(a, dominant_colours(&i, Region::whole(&i), 2, 8).unwrap());
        assert!(dominant_colours(&i, Region::whole(&i), 0, 8).is_none());
    }
}

//! Reductions, integral images and region statistics over 8-bit planes. A row
//! or column reduction returns one `f32` per line, so a sum is not clamped to
//! the 8-bit range the way a stored image would be. Integral images keep exact
//! `u64` sums, and region statistics use exact integer sums for the mean and
//! the population variance. Plain std code over `filter::Plane`, no GPU.

use crate::filter::Plane;

/// The fold applied along a row or column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    Min,
    Max,
    Sum,
    Mean,
}

/// Folds a line of values. An empty line gives 0.
fn reduce_values(values: &[u8], op: Reduce) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    match op {
        Reduce::Min => values.iter().copied().min().unwrap_or(0) as f32,
        Reduce::Max => values.iter().copied().max().unwrap_or(0) as f32,
        Reduce::Sum => values.iter().map(|&v| v as f32).sum(),
        Reduce::Mean => values.iter().map(|&v| v as f32).sum::<f32>() / values.len() as f32,
    }
}

/// One value per row, folded across the row's pixels.
pub fn reduce_rows(src: &Plane, op: Reduce) -> Vec<f32> {
    let w = src.width;
    (0..src.height)
        .map(|y| reduce_values(&src.pixels[y * w..(y + 1) * w], op))
        .collect()
}

/// One value per column, folded down the column's pixels.
pub fn reduce_columns(src: &Plane, op: Reduce) -> Vec<f32> {
    let w = src.width;
    (0..w)
        .map(|x| {
            let column: Vec<u8> = (0..src.height).map(|y| src.pixels[y * w + x]).collect();
            reduce_values(&column, op)
        })
        .collect()
}

/// The part of the rectangle (`x`, `y`, `rw`, `rh`) that lies inside a
/// `w` by `h` plane, as half-open bounds `(x0, x1, y0, y1)`. `None` when
/// nothing is left.
fn clip(w: usize, h: usize, x: usize, y: usize, rw: usize, rh: usize) -> Option<(usize, usize, usize, usize)> {
    let x0 = x.min(w);
    let y0 = y.min(h);
    let x1 = x.saturating_add(rw).min(w);
    let y1 = y.saturating_add(rh).min(h);
    (x1 > x0 && y1 > y0).then_some((x0, x1, y0, y1))
}

/// Summed-area table. `sums[y * (width + 1) + x]` holds the sum of every pixel
/// above and left of the corner (`x`, `y`), so any rectangle is four lookups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integral {
    width: usize,
    height: usize,
    sums: Vec<u64>,
}

impl Integral {
    /// The integral of the pixel values.
    pub fn new(src: &Plane) -> Self {
        Self::build(src, |v| v as u64)
    }

    /// The integral of the squared pixel values, for variance over a region.
    pub fn of_squares(src: &Plane) -> Self {
        Self::build(src, |v| v as u64 * v as u64)
    }

    fn build(src: &Plane, weight: impl Fn(u8) -> u64) -> Self {
        let (w, h) = (src.width, src.height);
        let stride = w + 1;
        let mut sums = vec![0u64; stride * (h + 1)];
        for y in 0..h {
            let mut row = 0u64;
            for x in 0..w {
                row += weight(src.pixels[y * w + x]);
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row;
            }
        }
        Integral { width: w, height: h, sums }
    }

    /// The sum over the rectangle (`x`, `y`, `rw`, `rh`), clipped to the image.
    /// An empty rectangle sums to 0.
    pub fn rect_sum(&self, x: usize, y: usize, rw: usize, rh: usize) -> u64 {
        let Some((x0, x1, y0, y1)) = clip(self.width, self.height, x, y, rw, rh) else {
            return 0;
        };
        let s = self.width + 1;
        // The partial sums never go below zero: A + B - C is the rectangle plus D.
        self.sums[y1 * s + x1] + self.sums[y0 * s + x0] - self.sums[y0 * s + x1] - self.sums[y1 * s + x0]
    }
}

/// Mean, population variance and extremes of the pixels in a region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Statistics {
    pub mean: f64,
    pub variance: f64,
    pub min: u8,
    pub max: u8,
}

/// Statistics over the rectangle (`x`, `y`, `rw`, `rh`), clipped to the plane.
/// Returns `None` when the clipped region is empty.
pub fn statistics(src: &Plane, x: usize, y: usize, rw: usize, rh: usize) -> Option<Statistics> {
    let w = src.width;
    let (x0, x1, y0, y1) = clip(w, src.height, x, y, rw, rh)?;
    let (mut n, mut sum, mut sq) = (0u64, 0u64, 0u64);
    let (mut min, mut max) = (u8::MAX, u8::MIN);
    for yy in y0..y1 {
        for xx in x0..x1 {
            let v = src.pixels[yy * w + xx];
            n += 1;
            sum += v as u64;
            sq += v as u64 * v as u64;
            min = min.min(v);
            max = max.max(v);
        }
    }
    // Variance = (n * sum of squares - sum^2) / n^2, exact in u128 (Cauchy-Schwarz keeps it >= 0).
    let numerator = n as u128 * sq as u128 - sum as u128 * sum as u128;
    let nf = n as f64;
    Some(Statistics {
        mean: sum as f64 / nf,
        variance: numerator as f64 / (nf * nf),
        min,
        max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Plane {
        Plane::new(3, 2, vec![1, 5, 3, 2, 8, 4]).unwrap()
    }

    #[test]
    fn row_reductions_fold_across_each_row() {
        let src = grid();
        assert_eq!(reduce_rows(&src, Reduce::Max), vec![5.0, 8.0]);
        assert_eq!(reduce_rows(&src, Reduce::Min), vec![1.0, 2.0]);
        assert_eq!(reduce_rows(&src, Reduce::Sum), vec![9.0, 14.0]);
        assert_eq!(reduce_rows(&src, Reduce::Mean)[0], 3.0);
    }

    #[test]
    fn column_reductions_fold_down_each_column() {
        let src = grid();
        assert_eq!(reduce_columns(&src, Reduce::Min), vec![1.0, 5.0, 3.0]);
        assert_eq!(reduce_columns(&src, Reduce::Max), vec![2.0, 8.0, 4.0]);
        assert_eq!(reduce_columns(&src, Reduce::Sum), vec![3.0, 13.0, 7.0]);
        assert_eq!(reduce_columns(&src, Reduce::Mean)[0], 1.5);
    }

    #[test]
    fn integral_rectangles_match_direct_sums() {
        let src = Plane::new(3, 3, (1..=9).collect()).unwrap();
        let integral = Integral::new(&src);
        assert_eq!(integral.rect_sum(0, 0, 3, 3), 45);
        // (1,1)=5, (2,1)=6, (1,2)=8, (2,2)=9.
        assert_eq!(integral.rect_sum(1, 1, 2, 2), 28);
        assert_eq!(integral.rect_sum(0, 0, 1, 1), 1);
    }

    #[test]
    fn integral_clips_to_the_image_and_is_zero_when_empty() {
        let src = Plane::new(3, 3, (1..=9).collect()).unwrap();
        let integral = Integral::new(&src);
        assert_eq!(integral.rect_sum(2, 2, 10, 10), 9);
        assert_eq!(integral.rect_sum(1, 1, 0, 2), 0);
        assert_eq!(integral.rect_sum(3, 0, 1, 1), 0);
    }

    #[test]
    fn squared_integral_gives_the_sum_of_squares() {
        let src = Plane::new(3, 3, (1..=9).collect()).unwrap();
        // 1^2 + ... + 9^2 = 9 * 10 * 19 / 6.
        assert_eq!(Integral::of_squares(&src).rect_sum(0, 0, 3, 3), 285);
    }

    #[test]
    fn statistics_give_mean_variance_and_extremes() {
        let src = Plane::new(2, 2, vec![0, 2, 4, 6]).unwrap();
        let s = statistics(&src, 0, 0, 2, 2).unwrap();
        assert_eq!(s.mean, 3.0);
        // Mean of squares is 14, so variance = 14 - 9 = 5.
        assert_eq!(s.variance, 5.0);
        assert_eq!((s.min, s.max), (0, 6));
    }

    #[test]
    fn statistics_clip_the_region_and_reject_an_empty_one() {
        let src = Plane::new(2, 2, vec![0, 2, 4, 6]).unwrap();
        let s = statistics(&src, 1, 0, 5, 1).unwrap();
        assert_eq!((s.mean, s.variance, s.min, s.max), (2.0, 0.0, 2, 2));
        assert!(statistics(&src, 2, 0, 1, 1).is_none());
    }
}

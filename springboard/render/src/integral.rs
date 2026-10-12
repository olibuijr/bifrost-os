//! Summed-area tables and region statistics on 8-bit single-channel planes: the sum of pixels and the sum of
//! squared pixels over any rectangle in constant time, the mean and population variance from those sums, and
//! the smallest and largest pixel in a rectangle. Everything is plain std code over `filter::Plane`, and there
//! is no app-facing entry point yet.

use crate::filter::Plane;

/// Summed-area tables of a plane. Entry (x, y) of the `(width + 1) by (height + 1)` table holds the sum of all
/// pixels strictly above and to the left of that corner, so the first row and column are zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integral {
    width: usize,
    height: usize,
    sums: Vec<u64>,
    squares: Vec<u64>,
}

impl Integral {
    /// Builds the sum table and the squared-sum table for `src`.
    pub fn new(src: &Plane) -> Self {
        let (w, h) = (src.width, src.height);
        let stride = w + 1;
        let mut sums = vec![0u64; stride * (h + 1)];
        let mut squares = vec![0u64; stride * (h + 1)];
        for y in 0..h {
            let mut row_sum = 0u64;
            let mut row_squares = 0u64;
            for x in 0..w {
                let v = src.pixels[y * w + x] as u64;
                row_sum += v;
                row_squares += v * v;
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row_sum;
                squares[(y + 1) * stride + x + 1] = squares[y * stride + x + 1] + row_squares;
            }
        }
        Integral { width: w, height: h, sums, squares }
    }

    /// The sum of the pixels in the `w` by `h` rectangle whose top left corner is (`x`, `y`). Returns `None`
    /// when the rectangle does not fit in the plane. An empty rectangle sums to 0.
    pub fn rect_sum(&self, x: usize, y: usize, w: usize, h: usize) -> Option<u64> {
        rect(&self.sums, self.width, self.height, x, y, w, h)
    }

    /// The sum of the squared pixels in the same rectangle. Returns `None` outside the plane.
    pub fn rect_squares(&self, x: usize, y: usize, w: usize, h: usize) -> Option<u64> {
        rect(&self.squares, self.width, self.height, x, y, w, h)
    }

    /// The mean and the population variance (divided by the pixel count) of the rectangle. Returns `None` for
    /// an empty rectangle or one that does not fit in the plane.
    pub fn mean_and_variance(&self, x: usize, y: usize, w: usize, h: usize) -> Option<(f64, f64)> {
        if w == 0 || h == 0 {
            return None;
        }
        let n = (w * h) as f64;
        let s = self.rect_sum(x, y, w, h)? as f64;
        let q = self.rect_squares(x, y, w, h)? as f64;
        let mean = s / n;
        // Rounding can push the difference slightly below zero for flat regions, so clamp it.
        let variance = (q / n - mean * mean).max(0.0);
        Some((mean, variance))
    }
}

/// Inclusion and exclusion on a summed-area table. The sum of the four corners is at least the two that are
/// subtracted, so the subtraction order used here cannot underflow.
fn rect(table: &[u64], width: usize, height: usize, x: usize, y: usize, w: usize, h: usize) -> Option<u64> {
    if x.checked_add(w)? > width || y.checked_add(h)? > height {
        return None;
    }
    let stride = width + 1;
    let at = |cx: usize, cy: usize| table[cy * stride + cx];
    Some(at(x + w, y + h) + at(x, y) - at(x + w, y) - at(x, y + h))
}

/// The smallest and largest pixel in the `w` by `h` rectangle at (`x`, `y`), read directly from the plane.
/// Returns `None` for an empty rectangle or one that does not fit in the plane.
pub fn min_and_max(src: &Plane, x: usize, y: usize, w: usize, h: usize) -> Option<(u8, u8)> {
    if w == 0 || h == 0 || x.checked_add(w)? > src.width || y.checked_add(h)? > src.height {
        return None;
    }
    let mut lo = u8::MAX;
    let mut hi = u8::MIN;
    for row in y..y + h {
        let start = row * src.width + x;
        for &v in &src.pixels[start..start + w] {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    Some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3 by 2 plane holding 1..=6 in reading order.
    fn sample() -> Plane {
        Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap()
    }

    #[test]
    fn rect_sums_match_the_pixels_they_cover() {
        let table = Integral::new(&sample());
        assert_eq!(table.rect_sum(0, 0, 3, 2), Some(21));
        assert_eq!(table.rect_sum(1, 0, 2, 2), Some(2 + 3 + 5 + 6));
        assert_eq!(table.rect_sum(0, 1, 1, 1), Some(4));
    }

    #[test]
    fn rect_squares_sum_the_squared_pixels() {
        let table = Integral::new(&sample());
        assert_eq!(table.rect_squares(0, 0, 3, 2), Some(91));
        assert_eq!(table.rect_squares(1, 0, 2, 2), Some(4 + 9 + 25 + 36));
    }

    #[test]
    fn rectangles_outside_the_plane_are_refused() {
        let table = Integral::new(&sample());
        assert_eq!(table.rect_sum(2, 0, 2, 1), None);
        assert_eq!(table.rect_sum(0, 1, 1, 2), None);
        assert_eq!(table.rect_squares(3, 0, 1, 1), None);
        assert_eq!(table.mean_and_variance(0, 0, 4, 1), None);
    }

    #[test]
    fn mean_and_variance_of_a_known_row() {
        // The values 2, 4, 4, 4, 5, 5, 7, 9 have mean 5 and population variance 4 (sum of squares 232).
        let src = Plane::new(8, 1, vec![2, 4, 4, 4, 5, 5, 7, 9]).unwrap();
        let table = Integral::new(&src);
        assert_eq!(table.mean_and_variance(0, 0, 8, 1), Some((5.0, 4.0)));
    }

    #[test]
    fn mean_of_an_empty_rectangle_is_none() {
        let table = Integral::new(&sample());
        assert_eq!(table.mean_and_variance(0, 0, 0, 1), None);
    }

    #[test]
    fn min_and_max_read_only_the_rectangle() {
        let src = sample();
        assert_eq!(min_and_max(&src, 1, 0, 2, 2), Some((2, 6)));
        assert_eq!(min_and_max(&src, 0, 0, 3, 2), Some((1, 6)));
        assert_eq!(min_and_max(&src, 2, 1, 1, 1), Some((6, 6)));
        assert_eq!(min_and_max(&src, 2, 0, 2, 1), None);
        assert_eq!(min_and_max(&src, 0, 0, 0, 2), None);
    }
}

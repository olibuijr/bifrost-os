//! Block motion estimation between two 8-bit planes, as plain std code. The current
//! plane is cut into square blocks (edge blocks are clipped). For each block a full
//! search over a square range of offsets finds the position in the reference plane
//! with the smallest sum of absolute differences (SAD). A candidate is used only when
//! the whole block lies inside the reference plane. Ties go to the offset that moves
//! fewest pixels (zero first), then to the first one in raster order.

use crate::filter::Plane;

/// The best match for one block: the offset into the reference plane and its SAD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionVector {
    pub dx: i32,
    pub dy: i32,
    pub sad: u32,
}

/// One vector per block, row-major, `cols` blocks across and `rows` blocks down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionField {
    pub cols: usize,
    pub rows: usize,
    pub vectors: Vec<MotionVector>,
}

/// A motion estimation session for planes of one size, with a fixed block size and
/// search range (in pixels, applied to each axis).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionEstimation {
    width: usize,
    height: usize,
    block: usize,
    range: usize,
}

/// SAD between the `bw` x `bh` block at (x0, y0) of `cur` and the block displaced by (dx, dy)
/// in `reference`. The caller guarantees the displaced block is inside the plane.
// Each argument is a distinct plane or block coordinate; a struct would only add noise to this hot helper.
#[allow(clippy::too_many_arguments)]
fn block_sad(cur: &[u8], reference: &[u8], width: usize, x0: usize, y0: usize, bw: usize, bh: usize, dx: i32, dy: i32) -> u32 {
    let rx = (x0 as i32 + dx) as usize;
    let ry = (y0 as i32 + dy) as usize;
    let mut sad = 0u32;
    for y in 0..bh {
        let c = &cur[(y0 + y) * width + x0..(y0 + y) * width + x0 + bw];
        let r = &reference[(ry + y) * width + rx..(ry + y) * width + rx + bw];
        for (a, b) in c.iter().zip(r) {
            sad += a.abs_diff(*b) as u32;
        }
    }
    sad
}

impl MotionEstimation {
    /// A session for `width` x `height` planes, `block` pixels per block and a search
    /// range of `range` pixels. None when the size or block is zero.
    pub fn new(width: usize, height: usize, block: usize, range: usize) -> Option<Self> {
        (width > 0 && height > 0 && block > 0).then_some(MotionEstimation { width, height, block, range })
    }

    /// The block grid: `cols` x `rows` blocks covering the plane.
    pub fn grid(&self) -> (usize, usize) {
        (self.width.div_ceil(self.block), self.height.div_ceil(self.block))
    }

    /// Estimates the motion of `current` relative to `reference`. None when either
    /// plane does not match the session's size.
    pub fn estimate(&self, current: &Plane, reference: &Plane) -> Option<MotionField> {
        let (w, h) = (self.width, self.height);
        if (current.width, current.height) != (w, h) || (reference.width, reference.height) != (w, h) {
            return None;
        }
        let (cols, rows) = self.grid();
        let r = self.range as i32;
        let mut vectors = Vec::with_capacity(cols * rows);
        for by in 0..rows {
            for bx in 0..cols {
                let x0 = bx * self.block;
                let y0 = by * self.block;
                let bw = self.block.min(w - x0);
                let bh = self.block.min(h - y0);
                let cur = &current.pixels;
                let rf = &reference.pixels;
                // zero offset first, so a static block keeps (0, 0) on ties
                let mut best = MotionVector { dx: 0, dy: 0, sad: block_sad(cur, rf, w, x0, y0, bw, bh, 0, 0) };
                let mut best_mag = 0i32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let rx = x0 as i32 + dx;
                        let ry = y0 as i32 + dy;
                        if rx < 0 || ry < 0 || rx as usize + bw > w || ry as usize + bh > h {
                            continue;
                        }
                        let sad = block_sad(cur, rf, w, x0, y0, bw, bh, dx, dy);
                        let mag = dx.abs() + dy.abs();
                        if sad < best.sad || (sad == best.sad && mag < best_mag) {
                            best = MotionVector { dx, dy, sad };
                            best_mag = mag;
                        }
                    }
                }
                vectors.push(best);
            }
        }
        Some(MotionField { cols, rows, vectors })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(w: usize, h: usize, px: Vec<u8>) -> Plane {
        Plane::new(w, h, px).unwrap()
    }

    #[test]
    fn a_bright_pixel_that_moved_left_is_found_by_the_block_that_holds_it() {
        // 4x4, block 2, range 1. The bright pixel moves from (1,1) in the reference to (2,1)
        // in the current plane, so block (1,0) matches at offset (-1, 0) with SAD 0.
        let mut cur = vec![0u8; 16];
        cur[1 * 4 + 2] = 255;
        let mut rf = vec![0u8; 16];
        rf[1 * 4 + 1] = 255;
        let est = MotionEstimation::new(4, 4, 2, 1).unwrap();
        let field = est.estimate(&plane(4, 4, cur), &plane(4, 4, rf)).unwrap();
        assert_eq!((field.cols, field.rows), (2, 2));
        assert_eq!(field.vectors[1], MotionVector { dx: -1, dy: 0, sad: 0 });
    }

    #[test]
    fn a_static_scene_keeps_zero_offsets() {
        let px: Vec<u8> = (0..16).map(|i| (i * 37 % 256) as u8).collect();
        let est = MotionEstimation::new(4, 4, 2, 2).unwrap();
        let field = est.estimate(&plane(4, 4, px.clone()), &plane(4, 4, px)).unwrap();
        assert!(field.vectors.iter().all(|v| v.dx == 0 && v.dy == 0 && v.sad == 0));
    }

    #[test]
    fn the_range_limits_the_search() {
        // same scene as above, but range 0 cannot reach the moved pixel
        let mut cur = vec![0u8; 16];
        cur[1 * 4 + 2] = 255;
        let mut rf = vec![0u8; 16];
        rf[1 * 4 + 1] = 255;
        let est = MotionEstimation::new(4, 4, 2, 0).unwrap();
        let field = est.estimate(&plane(4, 4, cur), &plane(4, 4, rf)).unwrap();
        assert_eq!(field.vectors[1], MotionVector { dx: 0, dy: 0, sad: 255 });
    }

    #[test]
    fn edge_blocks_are_clipped_to_the_plane() {
        // 5x3 plane with block 2: 3 columns (the last one 1 pixel wide), 2 rows (the last 1 high)
        let est = MotionEstimation::new(5, 3, 2, 0).unwrap();
        assert_eq!(est.grid(), (3, 2));
        let flat = plane(5, 3, vec![9; 15]);
        let field = est.estimate(&flat, &flat).unwrap();
        assert_eq!(field.vectors.len(), 6);
        assert!(field.vectors.iter().all(|v| v.sad == 0));
    }

    #[test]
    fn range_zero_reports_the_plain_difference_of_the_block() {
        // a 2x2 block with every pixel 3 apart: SAD = 4 * 3 = 12
        let est = MotionEstimation::new(2, 2, 2, 0).unwrap();
        let field = est.estimate(&plane(2, 2, vec![10; 4]), &plane(2, 2, vec![13; 4])).unwrap();
        assert_eq!(field.vectors, vec![MotionVector { dx: 0, dy: 0, sad: 12 }]);
    }

    #[test]
    fn bad_sizes_are_refused() {
        assert!(MotionEstimation::new(0, 4, 2, 1).is_none());
        assert!(MotionEstimation::new(4, 4, 0, 1).is_none());
        let est = MotionEstimation::new(4, 4, 2, 1).unwrap();
        assert!(est.estimate(&plane(2, 2, vec![0; 4]), &plane(4, 4, vec![0; 16])).is_none());
    }
}

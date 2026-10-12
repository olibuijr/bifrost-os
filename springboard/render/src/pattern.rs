//! Repeating tile patterns. A tile of packed ARGB pixels (0xAARRGGBB, as `Surface` stores them) is
//! placed on a grid of cells `step_x` by `step_y` apart, with the first cell's corner at `phase`.
//! Each pixel belongs to exactly one cell, so a step smaller than the tile cuts the tile at the
//! step instead of overlapping it. Pixels that fall past the tile inside a cell are transparent (0).
//! Pure: a renderer samples it per pixel or per row.

/// A tile pattern. Construct it with `new`, which checks the sizes.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    tile_w: usize,
    tile_h: usize,
    px: Vec<u32>,
    step: (i32, i32),
    phase: (i32, i32),
}

impl Pattern {
    /// A pattern from a `tile_w` by `tile_h` tile of row-major pixels. `None` when the tile is
    /// empty, the pixel count does not match the tile, or a step is not positive.
    pub fn new(tile_w: usize, tile_h: usize, px: Vec<u32>, step: (i32, i32), phase: (i32, i32)) -> Option<Self> {
        if tile_w == 0 || tile_h == 0 || px.len() != tile_w * tile_h || step.0 <= 0 || step.1 <= 0 {
            return None;
        }
        Some(Pattern { tile_w, tile_h, px, step, phase })
    }

    /// The colour at the device pixel (x, y). Transparent (0) between tiles.
    pub fn color_at(&self, x: i32, y: i32) -> u32 {
        let lx = (x as i64 - self.phase.0 as i64).rem_euclid(self.step.0 as i64);
        let ly = (y as i64 - self.phase.1 as i64).rem_euclid(self.step.1 as i64);
        if lx >= self.tile_w as i64 || ly >= self.tile_h as i64 {
            return 0;
        }
        self.px[ly as usize * self.tile_w + lx as usize]
    }

    /// The `w` by `h` block whose top-left pixel is (x, y), row-major.
    pub fn fill(&self, x: i32, y: i32, w: usize, h: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                out.push(self.color_at(x + i as i32, y + j as i32));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_by_two() -> Pattern {
        // Tile pixels: (0,0)=1, (1,0)=2, (0,1)=3, (1,1)=4, placed every 4 pixels, phase (0,0).
        Pattern::new(2, 2, vec![1, 2, 3, 4], (4, 4), (0, 0)).unwrap()
    }

    #[test]
    fn tile_pixels_are_read_row_major() {
        let p = two_by_two();
        assert_eq!(p.color_at(0, 0), 1);
        assert_eq!(p.color_at(1, 0), 2);
        assert_eq!(p.color_at(0, 1), 3);
        assert_eq!(p.color_at(1, 1), 4);
    }

    #[test]
    fn the_gap_between_tiles_is_transparent_and_the_tile_repeats() {
        let p = two_by_two();
        // Columns 2 and 3 of each 4-pixel cell lie past the 2-pixel tile.
        assert_eq!(p.color_at(2, 0), 0);
        assert_eq!(p.color_at(3, 1), 0);
        // The next cell starts at x = 4 and repeats the tile.
        assert_eq!(p.color_at(4, 0), 1);
        assert_eq!(p.color_at(5, 1), 4);
        // Negative coordinates wrap into the cell: x = -1 is column 3 of the cell at -4, a gap.
        assert_eq!(p.color_at(-1, 0), 0);
        // x = -4 is the start of a cell.
        assert_eq!(p.color_at(-4, 0), 1);
    }

    #[test]
    fn phase_moves_the_tiles() {
        let p = Pattern::new(2, 2, vec![1, 2, 3, 4], (4, 4), (1, 0)).unwrap();
        assert_eq!(p.color_at(1, 0), 1);
        assert_eq!(p.color_at(2, 0), 2);
        // Column 0 is column 3 of the cell that starts at x = -3, a gap.
        assert_eq!(p.color_at(0, 0), 0);
    }

    #[test]
    fn fill_produces_a_row_major_block() {
        let p = two_by_two();
        assert_eq!(p.fill(0, 0, 3, 2), vec![1, 2, 0, 3, 4, 0]);
        assert_eq!(p.fill(4, 0, 2, 1), vec![1, 2]);
    }

    #[test]
    fn invalid_tiles_and_steps_are_rejected() {
        assert!(Pattern::new(2, 2, vec![1, 2, 3], (4, 4), (0, 0)).is_none());
        assert!(Pattern::new(0, 2, vec![], (4, 4), (0, 0)).is_none());
        assert!(Pattern::new(2, 2, vec![1, 2, 3, 4], (0, 4), (0, 0)).is_none());
        assert!(Pattern::new(2, 2, vec![1, 2, 3, 4], (4, -1), (0, 0)).is_none());
    }
}

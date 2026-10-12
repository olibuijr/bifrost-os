//! Tiled access to a large picture, as plain std code.
//! `TileGrid` cuts a picture into tiles of a preferred size (the edge tiles are clipped to the picture) and finds
//! the tiles that cover any rectangle. `TileCache` holds the most recently used tiles and fills the others from a
//! loader the caller supplies, so a picture can be read region by region without being decoded whole. Pixels use
//! the renderer's packing (one u32 per pixel, R in the low byte). Recency is a counter, not a clock.

use crate::gfx::Image;

/// One tile's place in the picture, in pixels. `col` and `row` index the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub col: usize,
    pub row: usize,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// The tile layout of a picture of `w` by `h` pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileGrid {
    w: usize,
    h: usize,
    tile_w: usize,
    tile_h: usize,
}

impl TileGrid {
    /// A grid for a picture of `w` by `h` pixels. A zero size gives None. With no preferred size, or one with a zero
    /// side, the picture is one tile. A preferred size larger than the picture is clipped to the picture.
    pub fn new(w: usize, h: usize, preferred: Option<(usize, usize)>) -> Option<TileGrid> {
        if w == 0 || h == 0 {
            return None;
        }
        let (tile_w, tile_h) = match preferred {
            Some((tw, th)) if tw > 0 && th > 0 => (tw.min(w), th.min(h)),
            _ => (w, h),
        };
        Some(TileGrid { w, h, tile_w, tile_h })
    }

    /// Picture size in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// Tile size in pixels, before edge clipping.
    pub fn tile_size(&self) -> (usize, usize) {
        (self.tile_w, self.tile_h)
    }

    pub fn cols(&self) -> usize {
        (self.w + self.tile_w - 1) / self.tile_w
    }

    pub fn rows(&self) -> usize {
        (self.h + self.tile_h - 1) / self.tile_h
    }

    /// Number of tiles in the grid.
    pub fn count(&self) -> usize {
        self.cols() * self.rows()
    }

    /// The tile at a column and row, clipped to the picture. None outside the grid.
    pub fn tile(&self, col: usize, row: usize) -> Option<Tile> {
        if col >= self.cols() || row >= self.rows() {
            return None;
        }
        let x = col * self.tile_w;
        let y = row * self.tile_h;
        Some(Tile {
            col,
            row,
            x,
            y,
            w: self.tile_w.min(self.w - x),
            h: self.tile_h.min(self.h - y),
        })
    }

    /// Tiles that overlap a rectangle, clipped to the picture, in row-major order. An empty rectangle, or one that
    /// starts outside the picture, gives no tiles.
    pub fn covering(&self, x: usize, y: usize, w: usize, h: usize) -> Vec<Tile> {
        let x1 = x.saturating_add(w).min(self.w);
        let y1 = y.saturating_add(h).min(self.h);
        let mut out = Vec::new();
        if x >= x1 || y >= y1 {
            return out;
        }
        let c0 = x / self.tile_w;
        let c1 = (x1 - 1) / self.tile_w;
        let r0 = y / self.tile_h;
        let r1 = (y1 - 1) / self.tile_h;
        for row in r0..=r1 {
            for col in c0..=c1 {
                if let Some(t) = self.tile(col, row) {
                    out.push(t);
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
struct Entry {
    col: usize,
    row: usize,
    used: u64,
    px: Vec<u32>,
}

/// Holds up to `max_tiles` decoded tiles. A tile that is not held is filled by the loader, and the least recently
/// used tile is dropped to make room. The loader returns the tile's `w * h` pixels in row-major order, or None when
/// the source cannot supply it.
pub struct TileCache<F>
where
    F: FnMut(&Tile) -> Option<Vec<u32>>,
{
    grid: TileGrid,
    max_tiles: usize,
    load: F,
    entries: Vec<Entry>,
    clock: u64,
    loads: u64,
}

impl<F> TileCache<F>
where
    F: FnMut(&Tile) -> Option<Vec<u32>>,
{
    /// A cache over `grid` that holds at most `max_tiles` tiles. Zero tiles gives None.
    pub fn new(grid: TileGrid, max_tiles: usize, load: F) -> Option<Self> {
        if max_tiles == 0 {
            return None;
        }
        Some(TileCache { grid, max_tiles, load, entries: Vec::new(), clock: 0, loads: 0 })
    }

    /// Tiles the loader has filled so far, including ones since dropped.
    pub fn loads(&self) -> u64 {
        self.loads
    }

    /// Tiles held right now.
    pub fn resident(&self) -> usize {
        self.entries.len()
    }

    /// The pixels of one tile, loading it when it is not held. None outside the grid, or when the loader fails or
    /// returns the wrong number of pixels.
    pub fn tile_pixels(&mut self, col: usize, row: usize) -> Option<&[u32]> {
        let tile = self.grid.tile(col, row)?;
        self.clock += 1;
        let now = self.clock;
        let idx = match self.entries.iter().position(|e| e.col == col && e.row == row) {
            Some(i) => i,
            None => {
                let px = (self.load)(&tile)?;
                if px.len() != tile.w * tile.h {
                    return None;
                }
                if self.entries.len() >= self.max_tiles {
                    let oldest = self
                        .entries
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, e)| e.used)
                        .map(|(i, _)| i)?;
                    self.entries.swap_remove(oldest);
                }
                self.loads += 1;
                self.entries.push(Entry { col, row, used: now, px });
                self.entries.len() - 1
            }
        };
        self.entries[idx].used = now;
        Some(self.entries[idx].px.as_slice())
    }

    /// Copies a rectangle of the picture into a new image, loading the tiles it covers. None for an empty rectangle,
    /// one that runs past the picture, or a tile that cannot be loaded.
    pub fn read_region(&mut self, x: usize, y: usize, w: usize, h: usize) -> Option<Image> {
        if w == 0 || h == 0 {
            return None;
        }
        let (gw, gh) = self.grid.size();
        if x.checked_add(w)? > gw || y.checked_add(h)? > gh {
            return None;
        }
        let mut px = vec![0u32; w * h];
        for t in self.grid.covering(x, y, w, h) {
            let src = self.tile_pixels(t.col, t.row)?;
            let ox0 = x.max(t.x);
            let oy0 = y.max(t.y);
            let ox1 = (x + w).min(t.x + t.w);
            let oy1 = (y + h).min(t.y + t.h);
            for yy in oy0..oy1 {
                for xx in ox0..ox1 {
                    px[(yy - y) * w + (xx - x)] = src[(yy - t.y) * t.w + (xx - t.x)];
                }
            }
        }
        Some(Image { w, h, px })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_loader(t: &Tile) -> Option<Vec<u32>> {
        Some(vec![(t.col * 10 + t.row) as u32; t.w * t.h])
    }

    #[test]
    fn edge_tiles_are_clipped_to_the_picture() {
        let g = TileGrid::new(1000, 600, Some((256, 256))).unwrap();
        assert_eq!((g.cols(), g.rows(), g.count()), (4, 3, 12));
        assert_eq!(
            g.tile(3, 2),
            Some(Tile { col: 3, row: 2, x: 768, y: 512, w: 232, h: 88 })
        );
        assert_eq!(g.tile(4, 0), None);
    }

    #[test]
    fn missing_or_oversized_preferred_sizes_give_one_tile() {
        let whole = TileGrid::new(300, 200, None).unwrap();
        assert_eq!(whole.count(), 1);
        let zero_side = TileGrid::new(300, 200, Some((0, 64))).unwrap();
        assert_eq!(zero_side.tile_size(), (300, 200));
        let big = TileGrid::new(300, 200, Some((4096, 4096))).unwrap();
        assert_eq!(big.tile(0, 0), Some(Tile { col: 0, row: 0, x: 0, y: 0, w: 300, h: 200 }));
        assert!(TileGrid::new(0, 10, Some((4, 4))).is_none());
    }

    #[test]
    fn covering_lists_only_the_tiles_a_rectangle_touches() {
        let g = TileGrid::new(1000, 600, Some((256, 256))).unwrap();
        let cols: Vec<(usize, usize)> = g.covering(250, 0, 20, 10).iter().map(|t| (t.col, t.row)).collect();
        assert_eq!(cols, vec![(0, 0), (1, 0)]);
        let edge = g.covering(900, 580, 500, 500);
        assert_eq!(edge.len(), 1);
        assert_eq!((edge[0].col, edge[0].row), (3, 2));
        assert!(g.covering(1000, 0, 10, 10).is_empty(), "a rectangle starting at the right edge is empty");
    }

    #[test]
    fn cache_evicts_the_least_recently_used_tile() {
        let g = TileGrid::new(4, 4, Some((2, 2))).unwrap();
        let mut c = TileCache::new(g, 2, value_loader).unwrap();
        assert_eq!(c.tile_pixels(0, 0).unwrap()[0], 0);
        assert_eq!(c.tile_pixels(1, 0).unwrap()[0], 10);
        assert_eq!(c.loads(), 2);
        assert_eq!(c.tile_pixels(0, 0).unwrap()[0], 0, "a hit does not load again");
        assert_eq!(c.loads(), 2);
        assert_eq!(c.tile_pixels(1, 1).unwrap()[0], 11, "(1, 0) is the oldest, so it is dropped");
        assert_eq!(c.resident(), 2);
        assert_eq!(c.tile_pixels(1, 0).unwrap()[0], 10);
        assert_eq!(c.loads(), 4, "the dropped tile is loaded again");
    }

    #[test]
    fn read_region_stitches_pixels_from_several_tiles() {
        let g = TileGrid::new(4, 4, Some((2, 2))).unwrap();
        let mut c = TileCache::new(g, 4, value_loader).unwrap();
        let img = c.read_region(1, 1, 2, 2).unwrap();
        assert_eq!(img, Image { w: 2, h: 2, px: vec![0, 10, 1, 11] });
    }

    #[test]
    fn bad_regions_and_loader_failures_are_refused() {
        let g = TileGrid::new(4, 4, Some((2, 2))).unwrap();
        let mut c = TileCache::new(g, 4, value_loader).unwrap();
        assert!(c.read_region(3, 3, 2, 2).is_none(), "runs past the picture");
        assert!(c.read_region(0, 0, 0, 1).is_none(), "empty region");

        let short = |_: &Tile| Some(vec![1u32]);
        let mut bad = TileCache::new(TileGrid::new(4, 4, Some((2, 2))).unwrap(), 1, short).unwrap();
        assert!(bad.tile_pixels(0, 0).is_none(), "a wrong pixel count is not held");
        assert_eq!(bad.resident(), 0);
        assert!(TileCache::new(TileGrid::new(4, 4, None).unwrap(), 0, value_loader).is_none());
    }
}

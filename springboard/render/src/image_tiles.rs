//! Tile grids for large pictures. A picture is cut into tiles of a preferred size, so a viewer
//! can decode or draw only the tiles on screen. The grid is pure geometry with no pixels; the
//! tiles on the right and bottom edges are cut short to the picture. `TileCache` is the streaming
//! side: it holds the decoded tiles a viewer asked for, decodes the others on demand and evicts the
//! least recently used one when full.

/// One tile: its position in the grid and its rectangle on the picture, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub col: usize,
    pub row: usize,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// A `width` x `height` picture cut into tiles of `tile_w` x `tile_h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileGrid {
    width: usize,
    height: usize,
    tile_w: usize,
    tile_h: usize,
}

impl TileGrid {
    /// A grid for the picture and tile sizes. `None` when any size is zero.
    pub fn new(width: usize, height: usize, tile_w: usize, tile_h: usize) -> Option<TileGrid> {
        if width == 0 || height == 0 || tile_w == 0 || tile_h == 0 {
            return None;
        }
        Some(TileGrid { width, height, tile_w, tile_h })
    }

    /// Tiles across the picture, rounding up.
    pub fn columns(&self) -> usize {
        self.width.div_ceil(self.tile_w)
    }

    /// Tiles down the picture, rounding up.
    pub fn rows(&self) -> usize {
        self.height.div_ceil(self.tile_h)
    }

    /// Total number of tiles.
    pub fn count(&self) -> usize {
        self.columns() * self.rows()
    }

    /// The tile at `col`, `row`, or `None` past the grid.
    pub fn tile(&self, col: usize, row: usize) -> Option<Tile> {
        if col >= self.columns() || row >= self.rows() {
            return None;
        }
        let x = col * self.tile_w;
        let y = row * self.tile_h;
        Some(Tile {
            col,
            row,
            x,
            y,
            w: self.tile_w.min(self.width - x),
            h: self.tile_h.min(self.height - y),
        })
    }

    /// Every tile, row by row from the top left.
    pub fn tiles(&self) -> impl Iterator<Item = Tile> + '_ {
        let (cols, rows) = (self.columns(), self.rows());
        (0..rows).flat_map(move |r| (0..cols).filter_map(move |c| self.tile(c, r)))
    }

    /// The tiles that overlap the rectangle `x, y, w, h`, clipped to the picture. An empty list
    /// when the rectangle lies outside the picture.
    pub fn covering(&self, x: usize, y: usize, w: usize, h: usize) -> Vec<Tile> {
        let x1 = x.saturating_add(w).min(self.width);
        let y1 = y.saturating_add(h).min(self.height);
        if x >= x1 || y >= y1 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for row in y / self.tile_h..=(y1 - 1) / self.tile_h {
            for col in x / self.tile_w..=(x1 - 1) / self.tile_w {
                if let Some(t) = self.tile(col, row) {
                    out.push(t);
                }
            }
        }
        out
    }
}

/// A bounded cache of decoded tiles for a streaming tile provider. A viewer asks for a tile with
/// [`TileCache::get_or_load`]: a held tile is returned and marked as recently used, a missing tile is decoded by the
/// caller's loader and held, and the least recently used tile goes once the cache is full. The cache holds at most
/// `capacity` tiles, whatever their size.
pub struct TileCache<P> {
    capacity: usize,
    tiles: std::collections::HashMap<(usize, usize), P>,
    // Least recently used first.
    order: std::collections::VecDeque<(usize, usize)>,
}

impl<P> TileCache<P> {
    /// An empty cache that holds `capacity` tiles. None when `capacity` is zero.
    pub fn new(capacity: usize) -> Option<Self> {
        if capacity == 0 {
            return None;
        }
        Some(TileCache { capacity, tiles: std::collections::HashMap::new(), order: std::collections::VecDeque::new() })
    }

    /// The number of tiles held.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Whether no tile is held.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Whether the tile at `col`, `row` is held. Does not change the recency order.
    pub fn contains(&self, col: usize, row: usize) -> bool {
        self.tiles.contains_key(&(col, row))
    }

    /// The held tile at `tile`'s position, marked as recently used.
    pub fn get_or_load<F>(&mut self, tile: Tile, load: F) -> Option<&P>
    where
        F: FnOnce(Tile) -> Option<P>,
    {
        let key = (tile.col, tile.row);
        if self.tiles.contains_key(&key) {
            self.touch(key);
        } else {
            let pixels = load(tile)?;
            self.insert(key, pixels);
        }
        self.tiles.get(&key)
    }

    /// The tiles of `wanted` that are not held, in the order given. These are what a provider still has to load.
    pub fn missing(&self, wanted: &[Tile]) -> Vec<Tile> {
        wanted.iter().copied().filter(|t| !self.contains(t.col, t.row)).collect()
    }

    fn insert(&mut self, key: (usize, usize), pixels: P) {
        if self.tiles.insert(key, pixels).is_some() {
            self.touch(key);
        } else {
            self.order.push_back(key);
        }
        while self.tiles.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.tiles.remove(&oldest);
            }
        }
    }

    fn touch(&mut self, key: (usize, usize)) {
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
            self.order.push_back(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_counts_round_up_at_the_edges() {
        // 300 / 128 = 2.34 -> 3 columns; 200 / 128 = 1.56 -> 2 rows.
        let g = TileGrid::new(300, 200, 128, 128).unwrap();
        assert_eq!((g.columns(), g.rows(), g.count()), (3, 2, 6));
    }

    #[test]
    fn edge_tiles_are_cut_to_the_picture() {
        let g = TileGrid::new(300, 200, 128, 128).unwrap();
        // Last column starts at 256 and has 300 - 256 = 44 pixels; last row starts at 128 with
        // 200 - 128 = 72 pixels.
        assert_eq!(g.tile(2, 1), Some(Tile { col: 2, row: 1, x: 256, y: 128, w: 44, h: 72 }));
        assert_eq!(g.tile(3, 0), None);
    }

    #[test]
    fn tiles_come_row_by_row_and_cover_the_width() {
        let g = TileGrid::new(300, 200, 128, 128).unwrap();
        let order: Vec<(usize, usize)> = g.tiles().map(|t| (t.col, t.row)).collect();
        assert_eq!(order, vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]);
        let row0: usize = g.tiles().filter(|t| t.row == 0).map(|t| t.w).sum();
        assert_eq!(row0, 300, "widths of one row add up to the picture width");
    }

    #[test]
    fn zero_size_has_no_grid() {
        assert_eq!(TileGrid::new(0, 10, 4, 4), None);
        assert_eq!(TileGrid::new(10, 10, 0, 4), None);
    }

    #[test]
    fn region_inside_one_tile_gives_one_tile() {
        let g = TileGrid::new(300, 200, 128, 128).unwrap();
        // x 130..140 and y 130..140 both lie in tile column 1 and row 1.
        let hit = g.covering(130, 130, 10, 10);
        assert_eq!(hit, vec![Tile { col: 1, row: 1, x: 128, y: 128, w: 128, h: 72 }]);
    }

    #[test]
    fn region_across_a_boundary_gives_both_tiles() {
        let g = TileGrid::new(300, 200, 128, 128).unwrap();
        // x 120..140 crosses from column 0 into column 1 on row 0.
        let cols: Vec<usize> = g.covering(120, 0, 20, 1).iter().map(|t| t.col).collect();
        assert_eq!(cols, vec![0, 1]);
    }

    #[test]
    fn region_past_the_edge_is_clipped_and_outside_is_empty() {
        let g = TileGrid::new(300, 200, 128, 128).unwrap();
        // x 290..300 and y 190..200 are clipped to the bottom right tile (2, 1).
        assert_eq!(g.covering(290, 190, 100, 100), vec![Tile { col: 2, row: 1, x: 256, y: 128, w: 44, h: 72 }]);
        assert!(g.covering(400, 0, 10, 10).is_empty());
    }
}

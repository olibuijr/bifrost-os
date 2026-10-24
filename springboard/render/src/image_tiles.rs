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

/// The pixels of `tile` cut out of `image`. None when the tile reaches outside the picture.
pub fn crop_tile(image: &crate::gfx::Image, tile: Tile) -> Option<crate::gfx::Image> {
    if tile.w == 0 || tile.h == 0 || tile.x.checked_add(tile.w)? > image.w || tile.y.checked_add(tile.h)? > image.h {
        return None;
    }
    let mut out = crate::gfx::Image::new(tile.w, tile.h);
    for row in 0..tile.h {
        let from = (tile.y + row) * image.w + tile.x;
        out.px[row * tile.w..(row + 1) * tile.w].copy_from_slice(&image.px[from..from + tile.w]);
    }
    Some(out)
}

/// A decoded picture served in tiles: the pixel source behind a [`TileCache`]. A viewer asks for the region on screen
/// with [`TiledPicture::region`]; tiles it has not asked for before are cut from the picture, tiles it asked for
/// recently come from the cache, and the cache holds at most `capacity` tiles. The picture is decoded once up front
/// (the codecs decode a whole picture), so the cache bounds what a viewer holds in drawable form, not the decode.
pub struct TiledPicture {
    image: crate::gfx::Image,
    grid: TileGrid,
    cache: TileCache<crate::gfx::Image>,
    loads: usize,
}

impl TiledPicture {
    /// Cuts `image` into tiles of `tile_edge` pixels, keeping `capacity` of them. None for an empty picture, a zero
    /// edge or a zero capacity.
    pub fn new(image: crate::gfx::Image, tile_edge: usize, capacity: usize) -> Option<TiledPicture> {
        let grid = TileGrid::new(image.w, image.h, tile_edge, tile_edge)?;
        Some(TiledPicture { image, grid, cache: TileCache::new(capacity)?, loads: 0 })
    }

    /// Decodes picture file bytes (any format the codecs read) and tiles the result.
    pub fn from_bytes(bytes: &[u8], tile_edge: usize, capacity: usize) -> Option<TiledPicture> {
        TiledPicture::new(crate::cg::imageio::decode(bytes)?, tile_edge, capacity)
    }

    pub fn grid(&self) -> &TileGrid {
        &self.grid
    }

    /// How many tiles were cut from the picture so far (a tile served from the cache does not count).
    pub fn loads(&self) -> usize {
        self.loads
    }

    /// The tile at `col`, `row`, from the cache or cut from the picture. None outside the grid.
    pub fn tile(&mut self, col: usize, row: usize) -> Option<&crate::gfx::Image> {
        let t = self.grid.tile(col, row)?;
        let (image, loads) = (&self.image, &mut self.loads);
        self.cache.get_or_load(t, |t| {
            *loads += 1;
            crop_tile(image, t)
        })
    }

    /// The tiles that cover the region `x`, `y`, `w`, `h` of the picture, each with its pixels, row by row.
    pub fn region(&mut self, x: usize, y: usize, w: usize, h: usize) -> Vec<(Tile, crate::gfx::Image)> {
        let wanted = self.grid.covering(x, y, w, h);
        wanted.into_iter().filter_map(|t| self.tile(t.col, t.row).cloned().map(|img| (t, img))).collect()
    }
}

/// A picture file served in tiles without holding the decoded picture: each tile is decoded from the file when it is
/// first asked for (`cg::imageio::decode_region`) and only the cached tiles are held. A still, non-interlaced PNG is
/// read row by row, so one row besides the tile is in memory, and it may be larger than the whole-picture limit
/// (`cg::imageio::MAX_SIDE`); other formats are decoded whole for each tile and cut, which bounds what is held between
/// calls but not the decode itself.
pub struct StreamingPicture {
    bytes: Vec<u8>,
    grid: TileGrid,
    cache: TileCache<crate::gfx::Image>,
    loads: usize,
}

impl StreamingPicture {
    /// Reads the size from the header and lays the grid out; nothing is decoded yet. None for data that is not a
    /// picture, a zero edge or a zero capacity.
    pub fn new(bytes: Vec<u8>, tile_edge: usize, capacity: usize) -> Option<StreamingPicture> {
        let (w, h) = crate::cg::imageio::pixel_size(&bytes)?;
        let grid = TileGrid::new(w, h, tile_edge, tile_edge)?;
        Some(StreamingPicture { bytes, grid, cache: TileCache::new(capacity)?, loads: 0 })
    }

    pub fn grid(&self) -> &TileGrid {
        &self.grid
    }

    /// How many tiles were decoded from the file so far.
    pub fn loads(&self) -> usize {
        self.loads
    }

    /// The tile at `col`, `row`, from the cache or decoded from the file. None outside the grid or when it does not
    /// decode.
    pub fn tile(&mut self, col: usize, row: usize) -> Option<&crate::gfx::Image> {
        let t = self.grid.tile(col, row)?;
        let (bytes, loads) = (&self.bytes, &mut self.loads);
        self.cache.get_or_load(t, |t| {
            *loads += 1;
            crate::cg::imageio::decode_region(bytes, t.x, t.y, t.w, t.h)
        })
    }

    /// The tiles that cover the region `x`, `y`, `w`, `h` of the picture, each with its pixels, row by row.
    pub fn region(&mut self, x: usize, y: usize, w: usize, h: usize) -> Vec<(Tile, crate::gfx::Image)> {
        let wanted = self.grid.covering(x, y, w, h);
        wanted.into_iter().filter_map(|t| self.tile(t.col, t.row).cloned().map(|img| (t, img))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_streamed_png_gives_the_same_tiles_as_the_whole_picture() {
        let (w, h) = (70, 45);
        let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 7, 255]).collect();
        let png = crate::cg::imageio::encode::encode_png(w, h, &px, &Default::default()).unwrap();
        let whole = crate::cg::imageio::decode(&png).unwrap();
        let mut s = StreamingPicture::new(png.clone(), 32, 2).unwrap();
        assert_eq!((s.grid().columns(), s.grid().rows()), (3, 2));
        for (col, row) in [(0, 0), (2, 1), (1, 1)] {
            let t = s.grid().tile(col, row).unwrap();
            assert_eq!(s.tile(col, row).cloned(), crop_tile(&whole, t), "tile {col},{row}");
        }
        assert_eq!(s.loads(), 3);
        s.tile(1, 1);
        assert_eq!(s.loads(), 3, "a cached tile is not decoded again");
        assert_eq!(crate::cg::imageio::decode_region(&png, 60, 40, 20, 5), None, "outside the picture");
        assert!(StreamingPicture::new(b"not a picture".to_vec(), 32, 2).is_none());
    }

    fn numbered(w: usize, h: usize) -> crate::gfx::Image {
        let mut im = crate::gfx::Image::new(w, h);
        for (i, p) in im.px.iter_mut().enumerate() {
            *p = i as u32;
        }
        im
    }

    #[test]
    fn a_tile_is_the_picture_cut_at_its_place_and_nothing_outside_the_picture() {
        let im = numbered(10, 6);
        let t = Tile { col: 1, row: 0, x: 4, y: 1, w: 3, h: 2 };
        let c = crop_tile(&im, t).unwrap();
        assert_eq!((c.w, c.h), (3, 2));
        assert_eq!(c.px, vec![14, 15, 16, 24, 25, 26]);
        assert!(crop_tile(&im, Tile { col: 0, row: 0, x: 8, y: 0, w: 3, h: 1 }).is_none(), "past the right edge");
        assert!(crop_tile(&im, Tile { col: 0, row: 0, x: 0, y: 0, w: 0, h: 1 }).is_none());
    }

    #[test]
    fn a_tiled_picture_serves_the_region_and_remembers_the_tiles() {
        let mut p = TiledPicture::new(numbered(300, 200), 128, 4).unwrap();
        assert_eq!(p.grid().count(), 6);
        let hit = p.region(120, 0, 20, 1);
        assert_eq!(hit.iter().map(|(t, _)| t.col).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(p.loads(), 2);
        // the pixel at picture (130, 0) is the first one of tile column 1 plus two
        assert_eq!(hit[1].1.px[2], 130);
        let again = p.region(120, 0, 20, 1);
        assert_eq!(again.len(), 2);
        assert_eq!(p.loads(), 2, "tiles that were held are not cut again");
    }

    #[test]
    fn the_cache_bounds_the_tiles_held_and_evicts_the_oldest() {
        let mut p = TiledPicture::new(numbered(300, 200), 128, 2).unwrap();
        for col in 0..3 {
            assert!(p.tile(col, 0).is_some());
        }
        assert_eq!(p.loads(), 3);
        assert!(p.tile(2, 0).is_some() && p.tile(1, 0).is_some());
        assert_eq!(p.loads(), 3, "the two newest are still held");
        assert!(p.tile(0, 0).is_some());
        assert_eq!(p.loads(), 4, "the oldest had gone and is cut again");
        assert!(p.tile(3, 0).is_none() && p.tile(0, 2).is_none(), "outside the grid");
    }

    #[test]
    fn a_picture_file_is_decoded_and_tiled_and_bad_input_is_refused() {
        let bytes = crate::cg::imageio::tests::png_bytes(9, 5, [10, 20, 30, 255]);
        let mut p = TiledPicture::from_bytes(&bytes, 4, 8).unwrap();
        assert_eq!((p.grid().columns(), p.grid().rows()), (3, 2));
        let (t, im) = p.region(8, 4, 1, 1).remove(0);
        assert_eq!((t.w, t.h, im.w, im.h), (1, 1, 1, 1), "the corner tile is cut short");
        assert_eq!(im.px[0], crate::gfx::rgba(10, 20, 30, 255));
        assert!(TiledPicture::from_bytes(b"not a picture", 4, 8).is_none());
        assert!(TiledPicture::new(numbered(4, 4), 0, 8).is_none());
        assert!(TiledPicture::new(numbered(4, 4), 4, 0).is_none());
    }

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

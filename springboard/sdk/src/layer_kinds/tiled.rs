//! Tiled content: very large content drawn in fixed-size tiles at several levels of detail, with a tile cache and an
//! optional worker thread that draws tiles off the UI thread. A `TileSource` draws one tile; `TiledContent` decides which
//! tiles a visible region needs.
//!
//! Level 0 shows the content at its natural size. Level `n` shows it at 1/2^n; `bias` extra levels below zero show it
//! magnified (level -1 is 2x), for zoomed-in views.
use bifrost_render::gfx::Image;
use bifrost_ui::Rect;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// One tile: its level of detail and its column and row in that level's grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub level: i32,
    pub col: i32,
    pub row: i32,
}

/// Draws tiles. Called from a worker thread when drawing is asynchronous.
pub trait TileSource: Send + Sync + 'static {
    /// Draws `key`: an image of `size` pixels (edge tiles are smaller than the nominal tile). `zoom` is the pixels per
    /// content point of the tile's level (1.0 at level 0).
    fn draw_tile(&self, key: TileKey, size: (usize, usize), zoom: f32) -> Image;
}

impl<F: Fn(TileKey, (usize, usize), f32) -> Image + Send + Sync + 'static> TileSource for F {
    fn draw_tile(&self, key: TileKey, size: (usize, usize), zoom: f32) -> Image {
        self(key, size, zoom)
    }
}

/// The tile geometry of one piece of content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGrid {
    /// Content size in points.
    pub content: (f32, f32),
    /// Nominal tile size in pixels.
    pub tile: (usize, usize),
    /// Levels of detail from natural size down: 1 means only level 0.
    pub levels: u32,
    /// Extra magnified levels below level 0.
    pub bias: u32,
}

impl TileGrid {
    pub fn new(content: (f32, f32)) -> Self {
        TileGrid { content, tile: (256, 256), levels: 1, bias: 0 }
    }

    /// Pixels per content point at `level`.
    pub fn zoom(level: i32) -> f32 {
        2f32.powi(-level)
    }

    /// The level whose zoom is nearest (in powers of two) to the wanted `zoom`, kept inside the available levels.
    pub fn level_for_zoom(&self, zoom: f32) -> i32 {
        let want = if zoom > 0.0 && zoom.is_finite() { (-zoom.log2()).round() as i32 } else { 0 };
        want.clamp(-(self.bias as i32), self.levels.max(1) as i32 - 1)
    }

    /// The level's size in pixels.
    pub fn level_size(&self, level: i32) -> (usize, usize) {
        let z = TileGrid::zoom(level);
        ((self.content.0 * z).ceil().max(0.0) as usize, (self.content.1 * z).ceil().max(0.0) as usize)
    }

    /// Columns and rows of tiles at `level`.
    pub fn counts(&self, level: i32) -> (i32, i32) {
        let (w, h) = self.level_size(level);
        (w.div_ceil(self.tile.0.max(1)) as i32, h.div_ceil(self.tile.1.max(1)) as i32)
    }

    /// The pixel size of `key`: the nominal tile, cut short at the right and bottom edges.
    pub fn tile_pixels(&self, key: TileKey) -> (usize, usize) {
        let (w, h) = self.level_size(key.level);
        let (tx, ty) = (self.tile.0.max(1), self.tile.1.max(1));
        let (x, y) = (key.col.max(0) as usize * tx, key.row.max(0) as usize * ty);
        (w.saturating_sub(x).min(tx), h.saturating_sub(y).min(ty))
    }

    /// The content rect `key` covers, in points.
    pub fn tile_rect(&self, key: TileKey) -> Rect {
        let z = TileGrid::zoom(key.level);
        let (w, h) = self.tile_pixels(key);
        Rect { x: key.col as f32 * self.tile.0 as f32 / z, y: key.row as f32 * self.tile.1 as f32 / z, w: w as f32 / z, h: h as f32 / z }
    }

    /// The tiles of `level` that touch `region` (content points).
    pub fn visible(&self, region: Rect, level: i32) -> Vec<TileKey> {
        let (cols, rows) = self.counts(level);
        let z = TileGrid::zoom(level);
        let (tw, th) = (self.tile.0.max(1) as f32 / z, self.tile.1.max(1) as f32 / z);
        if region.w <= 0.0 || region.h <= 0.0 || cols == 0 || rows == 0 {
            return Vec::new();
        }
        let c0 = ((region.x / tw).floor() as i32).clamp(0, cols - 1);
        let c1 = (((region.x + region.w) / tw).ceil() as i32 - 1).clamp(0, cols - 1);
        let r0 = ((region.y / th).floor() as i32).clamp(0, rows - 1);
        let r1 = (((region.y + region.h) / th).ceil() as i32 - 1).clamp(0, rows - 1);
        let mut out = Vec::new();
        for row in r0..=r1 {
            for col in c0..=c1 {
                out.push(TileKey { level, col, row });
            }
        }
        out
    }
}

/// Finished tiles, oldest dropped first once `capacity` is passed.
#[derive(Debug)]
pub struct TileCache {
    tiles: HashMap<TileKey, Arc<Image>>,
    order: VecDeque<TileKey>,
    capacity: usize,
}

impl TileCache {
    pub fn new(capacity: usize) -> Self {
        TileCache { tiles: HashMap::new(), order: VecDeque::new(), capacity: capacity.max(1) }
    }
    pub fn get(&self, key: TileKey) -> Option<Arc<Image>> {
        self.tiles.get(&key).cloned()
    }
    pub fn insert(&mut self, key: TileKey, image: Arc<Image>) {
        if self.tiles.insert(key, image).is_none() {
            self.order.push_back(key);
        }
        while self.tiles.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.tiles.remove(&old);
            }
        }
    }
    pub fn len(&self) -> usize {
        self.tiles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.order.clear();
    }
}

/// How tiles get drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileDrawing {
    /// On the calling thread, inside `request`.
    Sync,
    /// On a worker thread; finished tiles are collected by `poll`.
    Async,
}

/// A grid, a source and a cache, with the bookkeeping of tiles that are on their way.
pub struct TiledContent {
    pub grid: TileGrid,
    pub drawing: TileDrawing,
    source: Arc<dyn TileSource>,
    cache: TileCache,
    pending: HashSet<TileKey>,
    worker: Option<(Sender<TileKey>, Receiver<(TileKey, Image)>)>,
}

impl TiledContent {
    pub fn new(grid: TileGrid, source: impl TileSource, drawing: TileDrawing) -> Self {
        TiledContent { grid, drawing, source: Arc::new(source), cache: TileCache::new(256), pending: HashSet::new(), worker: None }
    }

    /// Replaces the cache capacity (tiles kept).
    pub fn set_capacity(&mut self, capacity: usize) {
        self.cache = TileCache::new(capacity);
    }

    /// Drops every tile (the content changed) and forgets tiles still on their way.
    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.pending.clear();
        self.worker = None;
    }

    pub fn cached(&self, key: TileKey) -> Option<Arc<Image>> {
        self.cache.get(key)
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Asks for `key`. Sync drawing finishes it before returning; async drawing queues it. A tile already cached or
    /// queued is left alone.
    pub fn request(&mut self, key: TileKey) {
        if self.cache.get(key).is_some() || self.pending.contains(&key) {
            return;
        }
        match self.drawing {
            TileDrawing::Sync => {
                let img = self.source.draw_tile(key, self.grid.tile_pixels(key), TileGrid::zoom(key.level));
                self.cache.insert(key, Arc::new(img));
            }
            TileDrawing::Async => {
                if self.worker.is_none() {
                    self.worker = Some(self.spawn_worker());
                }
                if let Some((tx, _)) = &self.worker {
                    if tx.send(key).is_ok() {
                        self.pending.insert(key);
                    }
                }
            }
        }
    }

    fn spawn_worker(&self) -> (Sender<TileKey>, Receiver<(TileKey, Image)>) {
        let (req_tx, req_rx) = channel::<TileKey>();
        let (done_tx, done_rx) = channel::<(TileKey, Image)>();
        let source = self.source.clone();
        let grid = self.grid;
        std::thread::spawn(move || {
            while let Ok(key) = req_rx.recv() {
                let img = source.draw_tile(key, grid.tile_pixels(key), TileGrid::zoom(key.level));
                if done_tx.send((key, img)).is_err() {
                    break;
                }
            }
        });
        (req_tx, done_rx)
    }

    /// Collects tiles the worker has finished; true when any arrived.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        let mut done = Vec::new();
        if let Some((_, rx)) = &self.worker {
            while let Ok(item) = rx.try_recv() {
                done.push(item);
            }
        }
        for (key, img) in done {
            if self.pending.remove(&key) {
                self.cache.insert(key, Arc::new(img));
                got = true;
            }
        }
        got
    }

    /// Requests every tile of `level` that touches `region`; returns the keys in draw order.
    pub fn request_region(&mut self, region: Rect, level: i32) -> Vec<TileKey> {
        let keys = self.grid.visible(region, level);
        for k in &keys {
            self.request(*k);
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn grid() -> TileGrid {
        TileGrid { content: (1000.0, 600.0), tile: (256, 256), levels: 3, bias: 1 }
    }

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn levels_halve_the_size_and_bias_magnifies() {
        let g = grid();
        assert_eq!(g.level_size(0), (1000, 600));
        assert_eq!(g.level_size(1), (500, 300));
        assert_eq!(g.level_size(-1), (2000, 1200));
        assert_eq!(g.counts(0), (4, 3));
        assert_eq!(g.counts(1), (2, 2));
        assert_eq!(g.counts(2), (1, 1));
    }

    #[test]
    fn the_level_follows_the_zoom_inside_the_available_range() {
        let g = grid();
        assert_eq!(g.level_for_zoom(1.0), 0);
        assert_eq!(g.level_for_zoom(0.5), 1);
        assert_eq!(g.level_for_zoom(0.01), 2, "never past the coarsest level");
        assert_eq!(g.level_for_zoom(2.0), -1);
        assert_eq!(g.level_for_zoom(8.0), -1, "never past the magnified bias");
        assert_eq!(g.level_for_zoom(0.0), 0);
    }

    #[test]
    fn edge_tiles_are_cut_short() {
        let g = grid();
        assert_eq!(g.tile_pixels(TileKey { level: 0, col: 0, row: 0 }), (256, 256));
        assert_eq!(g.tile_pixels(TileKey { level: 0, col: 3, row: 2 }), (1000 - 768, 600 - 512));
        let rect = g.tile_rect(TileKey { level: 2, col: 0, row: 0 });
        assert_eq!((rect.w, rect.h), (1000.0, 600.0), "the whole level fits one tile; it covers all the content");
    }

    #[test]
    fn visible_tiles_are_those_the_region_touches() {
        let g = grid();
        let keys = g.visible(r(200.0, 0.0, 100.0, 50.0), 0);
        assert_eq!(keys.iter().map(|k| (k.col, k.row)).collect::<Vec<_>>(), vec![(0, 0), (1, 0)]);
        assert_eq!(g.visible(r(0.0, 0.0, 1000.0, 600.0), 0).len(), 12);
        assert!(g.visible(r(0.0, 0.0, 0.0, 10.0), 0).is_empty());
        // At level 1 a tile covers 512 points, at level 2 the whole content.
        assert_eq!(g.visible(r(0.0, 0.0, 1000.0, 600.0), 1).len(), 4);
        assert_eq!(g.visible(r(0.0, 0.0, 1000.0, 600.0), 2).len(), 1);
    }

    #[test]
    fn the_cache_drops_the_oldest_past_its_capacity() {
        let mut c = TileCache::new(2);
        let k = |n| TileKey { level: 0, col: n, row: 0 };
        for n in 0..3 {
            c.insert(k(n), Arc::new(Image::new(1, 1)));
        }
        assert_eq!(c.len(), 2);
        assert!(c.get(k(0)).is_none());
        assert!(c.get(k(2)).is_some());
    }

    fn marker() -> impl TileSource {
        |key: TileKey, size: (usize, usize), _z: f32| {
            let mut img = Image::new(size.0, size.1);
            img.px[0] = (key.col as u32 + 1) | (key.row as u32 + 1) << 8;
            img
        }
    }

    #[test]
    fn sync_drawing_finishes_tiles_inside_request() {
        let mut t = TiledContent::new(grid(), marker(), TileDrawing::Sync);
        let keys = t.request_region(r(0.0, 0.0, 300.0, 100.0), 0);
        assert_eq!(keys.len(), 2);
        let img = t.cached(keys[1]).unwrap();
        assert_eq!(img.px[0], 2 | 1 << 8);
        assert_eq!((img.w, img.h), (256, 256));
        assert_eq!(t.pending(), 0);
    }

    #[test]
    fn async_drawing_queues_then_delivers_on_poll() {
        let mut t = TiledContent::new(grid(), marker(), TileDrawing::Async);
        let keys = t.request_region(r(0.0, 0.0, 300.0, 100.0), 0);
        t.request(keys[0]);
        assert_eq!(t.pending(), 2, "asking again does not queue twice");
        let start = Instant::now();
        while t.pending() > 0 && start.elapsed() < Duration::from_secs(5) {
            t.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(t.pending(), 0);
        assert!(t.cached(keys[0]).is_some() && t.cached(keys[1]).is_some());
    }

    #[test]
    fn invalidate_drops_tiles_and_queued_work() {
        let mut t = TiledContent::new(grid(), marker(), TileDrawing::Sync);
        t.request_region(r(0.0, 0.0, 10.0, 10.0), 0);
        t.invalidate();
        assert!(t.cached(TileKey { level: 0, col: 0, row: 0 }).is_none());
    }
}

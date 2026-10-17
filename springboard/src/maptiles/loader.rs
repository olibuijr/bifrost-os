//! Tile loading: the cache first, then the network at the pace the tile usage policy asks for, decoded into images the
//! window blits. Fetches run on the global queue; the window thread only reads memory and applies finished tiles.

use super::cache::{TileCache, MAX_TILE_AGE};
use super::net::Net;
use super::source::{delay_before_request, TileSource, MAX_IN_FLIGHT};
use super::tiles::{TileId, TILE_SIZE};
use crate::gfx::{rgba, Image};
use crate::shell::browser::images::{decode, PixelFormat};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Decoded tiles kept in memory; past this the ones not wanted by the view are dropped.
const KEEP_TILES: usize = 400;
/// Tiles decoded from the disk cache per call, so one frame never stalls on a big view.
const DECODE_PER_CALL: usize = 16;

/// Decodes a tile image (PNG or JPEG) into premultiplied pixels.
pub fn decode_tile(bytes: &[u8]) -> Option<Image> {
    let side = TILE_SIZE as u32;
    let bmp = decode(bytes, side, side, PixelFormat::Rgba8).ok()?;
    let px = bmp.data.chunks_exact(4).map(|p| rgba(p[0], p[1], p[2], p[3])).collect();
    Some(Image { w: bmp.w as usize, h: bmp.h as usize, px })
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Fetches one tile, waits first for the request pacing, and stores it in the cache. Returns the bytes.
pub fn fetch_tile(net: &dyn Net, source: &TileSource, cache: &TileCache, last_sent: &Mutex<Option<u64>>, t: TileId) -> Result<Vec<u8>, String> {
    let wait = {
        let mut last = last_sent.lock().unwrap_or_else(|e| e.into_inner());
        let wait = delay_before_request(*last, now_ms());
        *last = Some(now_ms() + wait);
        wait
    };
    if wait > 0 {
        std::thread::sleep(std::time::Duration::from_millis(wait));
    }
    let bytes = net.get(&source.url(t), source.user_agent(), 512 * 1024).map_err(|e| e.to_string())?;
    decode_tile(&bytes).ok_or("the tile is not an image")?;
    let _ = cache.write(t, &bytes);
    Ok(bytes)
}

type Finished = Arc<Mutex<Vec<(TileId, Option<Vec<u8>>)>>>;

pub struct TileLoader {
    source: TileSource,
    cache: TileCache,
    net: Arc<dyn Net>,
    images: HashMap<TileId, Image>,
    pending: HashSet<TileId>,
    failed: HashSet<TileId>,
    /// the tiles of a Download Map still to ask for, and how many the job had
    queue: VecDeque<TileId>,
    job_total: usize,
    job_done: usize,
    finished: Finished,
    last_sent: Arc<Mutex<Option<u64>>>,
    online: bool,
    /// a cached tile older than this is fetched again
    max_age: Duration,
}

impl TileLoader {
    pub fn new(root: &Path, source: TileSource, net: Arc<dyn Net>) -> Self {
        let cache = TileCache::new(root, &source.cache_key());
        TileLoader {
            source,
            cache,
            net,
            images: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            queue: VecDeque::new(),
            job_total: 0,
            job_done: 0,
            finished: Arc::new(Mutex::new(Vec::new())),
            last_sent: Arc::new(Mutex::new(None)),
            online: true,
            max_age: MAX_TILE_AGE,
        }
    }

    /// A loader for a layer drawn over the base map (traffic): its own cache folder, and tiles that are fetched again
    /// once they are older than `max_age`.
    pub fn overlay(root: &Path, source: TileSource, net: Arc<dyn Net>, max_age: Duration) -> Self {
        let mut loader = TileLoader::new(root, source, net);
        loader.cache = TileCache::new(root, &loader.source.overlay_cache_key());
        loader.max_age = max_age;
        loader
    }

    /// How old a cached tile may be before it is fetched again.
    pub fn max_age(&self) -> Duration {
        self.max_age
    }

    /// Lets tiles that failed to load be asked for again (when a layer is switched on).
    pub fn retry_failed(&mut self) {
        self.failed.clear();
    }

    /// Another map type: its own source, cache folder and tiles.
    pub fn switch_source(&mut self, root: &Path, source: TileSource) {
        self.cache = TileCache::new(root, &source.cache_key());
        self.source = source;
        self.images.clear();
        self.pending.clear();
        self.failed.clear();
        self.queue.clear();
        self.job_total = 0;
        self.job_done = 0;
        self.finished.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn set_online(&mut self, online: bool) {
        if online && !self.online {
            self.failed.clear();
        }
        self.online = online;
    }

    pub fn image(&self, t: TileId) -> Option<&Image> {
        self.images.get(&t)
    }

    /// Makes sure the wanted tiles are loading: from the cache at once, from the network when missing or stale.
    /// True when new images arrived from the cache.
    pub fn want(&mut self, wanted: &[TileId]) -> bool {
        let mut changed = false;
        let mut decoded = 0;
        for &t in wanted {
            if !self.images.contains_key(&t) && decoded < DECODE_PER_CALL {
                if let Some((bytes, _)) = self.cache.read(t) {
                    if let Some(img) = decode_tile(&bytes) {
                        self.images.insert(t, img);
                        decoded += 1;
                        changed = true;
                    }
                }
            }
            if self.cache.is_fresh(t, self.max_age) && self.images.contains_key(&t) {
                continue;
            }
            if self.online && !self.failed.contains(&t) && !self.pending.contains(&t) {
                self.queue.push_front(t);
            }
        }
        self.dispatch();
        if self.images.len() > KEEP_TILES {
            let keep: HashSet<&TileId> = wanted.iter().collect();
            self.images.retain(|t, _| keep.contains(t));
        }
        changed
    }

    /// Starts queued fetches while fewer than `MAX_IN_FLIGHT` are running.
    fn dispatch(&mut self) {
        if !self.online {
            return;
        }
        while self.pending.len() < MAX_IN_FLIGHT {
            let Some(t) = self.queue.pop_front() else { break };
            if self.pending.contains(&t) {
                continue;
            }
            if self.job_total > 0 && self.cache.is_fresh(t, self.max_age) {
                self.job_done += 1;
                continue;
            }
            self.pending.insert(t);
            let (net, source, cache, last, done) = (self.net.clone(), self.source.clone(), self.cache.clone(), self.last_sent.clone(), self.finished.clone());
            // one thread per fetch: at most MAX_IN_FLIGHT run at once, and each is paced by `fetch_tile`
            std::thread::spawn(move || {
                let bytes = fetch_tile(&*net, &source, &cache, &last, t).ok();
                done.lock().unwrap_or_else(|e| e.into_inner()).push((t, bytes));
            });
        }
    }

    /// Applies finished fetches and starts the next queued ones. True when the picture changed.
    pub fn pump(&mut self) -> bool {
        let done = std::mem::take(&mut *self.finished.lock().unwrap_or_else(|e| e.into_inner()));
        let changed = !done.is_empty();
        for (t, bytes) in done {
            self.pending.remove(&t);
            if self.job_total > 0 && self.job_done < self.job_total {
                self.job_done += 1;
            }
            match bytes.and_then(|b| decode_tile(&b)) {
                Some(img) => {
                    self.images.insert(t, img);
                }
                None => {
                    self.failed.insert(t);
                }
            }
        }
        self.dispatch();
        if self.job_total > 0 && self.queue.is_empty() && self.pending.is_empty() {
            self.job_done = self.job_total;
        }
        changed
    }

    /// True while anything is being fetched or waits to be.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || !self.queue.is_empty()
    }

    /// Begins a Download Map: the tiles are fetched in the background with the same pacing as browsing.
    pub fn start_download(&mut self, tiles: Vec<TileId>) {
        self.job_total = tiles.len();
        self.job_done = 0;
        self.queue.extend(tiles);
        self.dispatch();
    }

    pub fn cancel_download(&mut self) {
        self.queue.clear();
        self.job_total = 0;
        self.job_done = 0;
    }

    /// (done, total) of the running download; `None` with no download.
    pub fn download_progress(&self) -> Option<(usize, usize)> {
        (self.job_total > 0).then_some((self.job_done.min(self.job_total), self.job_total))
    }

    /// True when the whole download has been fetched.
    pub fn download_finished(&self) -> bool {
        self.job_total > 0 && self.job_done >= self.job_total
    }

    pub fn end_download(&mut self) {
        self.job_total = 0;
        self.job_done = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::net::fake::Fake;
    use super::source::MapType;

    fn png() -> Vec<u8> {
        crate::pngenc::encode_rgb(4, 4, &[0xff80_4020; 16])
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("maps-loader-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_png_decodes_to_premultiplied_pixels() {
        let img = decode_tile(&png()).unwrap();
        assert_eq!((img.w, img.h), (4, 4));
        assert_eq!(img.px[0] >> 24, 255);
    }

    #[test]
    fn garbage_is_not_a_tile() {
        assert!(decode_tile(b"not an image").is_none());
    }

    #[test]
    fn a_fetched_tile_goes_to_the_cache_with_the_user_agent() {
        let root = dir("fetch");
        let net = Fake::with("https://tile.openstreetmap.org/", &png());
        let source = MapType::Standard.source("1.0.0");
        let cache = TileCache::new(&root, &source.cache_key());
        let t = TileId { z: 3, x: 4, y: 2 };
        let bytes = fetch_tile(&*net, &source, &cache, &Mutex::new(None), t).unwrap();
        assert_eq!(bytes, png());
        assert!(cache.read(t).is_some());
        let seen = net.seen.lock().unwrap();
        assert_eq!(seen[0].0, "https://tile.openstreetmap.org/3/4/2.png");
        assert!(seen[0].1.starts_with("BifrOSt-Maps/1.0.0"));
    }

    #[test]
    fn a_reply_that_is_not_an_image_is_not_cached() {
        let root = dir("bad");
        let net = Fake::with("https://tile.openstreetmap.org/", b"<html>blocked</html>");
        let source = MapType::Standard.source("1.0.0");
        let cache = TileCache::new(&root, &source.cache_key());
        let t = TileId { z: 1, x: 0, y: 0 };
        assert!(fetch_tile(&*net, &source, &cache, &Mutex::new(None), t).is_err());
        assert!(cache.read(t).is_none());
    }

    #[test]
    fn cached_tiles_show_without_the_network() {
        let root = dir("cached");
        let source = MapType::Standard.source("1.0.0");
        let t = TileId { z: 2, x: 1, y: 1 };
        TileCache::new(&root, &source.cache_key()).write(t, &png()).unwrap();
        let mut loader = TileLoader::new(&root, source, Arc::new(super::net::Offline));
        assert!(loader.want(&[t]));
        assert!(loader.image(t).is_some());
        assert!(!loader.busy(), "a fresh cached tile asks for nothing");
    }

    #[test]
    fn an_overlay_reads_its_own_folder_and_refetches_stale_tiles() {
        let root = dir("overlay");
        let source = TileSource::new("https://tile.openstreetmap.org/traffic/{z}/{x}/{y}.png", "ua").unwrap();
        let t = TileId { z: 2, x: 1, y: 1 };
        // a base tile of the same host must not show up as traffic
        TileCache::new(&root, &source.cache_key()).write(t, &png()).unwrap();
        let mut loader = TileLoader::overlay(&root, source.clone(), Arc::new(super::net::Offline), super::source::TRAFFIC_MAX_AGE);
        loader.set_online(false);
        assert!(!loader.want(&[t]));
        assert!(loader.image(t).is_none());
        TileCache::new(&root, &source.overlay_cache_key()).write(t, &png()).unwrap();
        assert!(loader.want(&[t]));
        assert!(loader.image(t).is_some());
        assert_eq!(loader.max_age(), super::source::TRAFFIC_MAX_AGE, "traffic tiles go stale sooner than map tiles");
        assert_eq!(TileLoader::new(&root, source, Arc::new(super::net::Offline)).max_age(), MAX_TILE_AGE);
    }

    #[test]
    fn offline_loader_asks_for_nothing() {
        let root = dir("off");
        let mut loader = TileLoader::new(&root, MapType::Standard.source("1.0.0"), Arc::new(super::net::Offline));
        loader.set_online(false);
        loader.want(&[TileId { z: 2, x: 0, y: 0 }]);
        assert!(!loader.busy());
    }

    #[test]
    fn a_download_counts_its_tiles_and_cancels() {
        let root = dir("job");
        let mut loader = TileLoader::new(&root, MapType::Standard.source("1.0.0"), Arc::new(super::net::Offline));
        assert_eq!(loader.download_progress(), None);
        loader.set_online(false);
        loader.start_download(vec![TileId { z: 1, x: 0, y: 0 }, TileId { z: 1, x: 1, y: 0 }]);
        assert_eq!(loader.download_progress().map(|p| p.1), Some(2));
        loader.cancel_download();
        assert_eq!(loader.download_progress(), None);
        assert!(!loader.busy());
    }
}

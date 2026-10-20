//! On-disk tile cache under the app data directory: `<root>/<source key>/<z>/<x>/<y>.png`.
//! Writes go to a temporary file and are renamed into place, so a crash never leaves a half tile.

use super::tiles::TileId;
use bifrost_core::json;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Tiles older than this are refetched when the network is up.
pub const MAX_TILE_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// The most one source's tile folder keeps on disk; past it [`TileCache::prune`] removes the oldest tiles.
pub const MAX_CACHE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TileCache {
    dir: PathBuf,
    /// the app data directory, where `areas.json` names the tiles kept offline
    root: PathBuf,
    source_key: String,
}

impl TileCache {
    /// `root` is the app data directory; `source_key` comes from `TileSource::cache_key`.
    pub fn new(root: &Path, source_key: &str) -> Self {
        Self { dir: root.join("tiles").join(source_key), root: root.to_path_buf(), source_key: source_key.to_string() }
    }

    pub fn path(&self, t: TileId) -> PathBuf {
        self.dir.join(t.z.to_string()).join(t.x.to_string()).join(format!("{}.png", t.y))
    }

    /// The cached bytes and how old the tile is. `None` when the tile is not cached or cannot be read.
    pub fn read(&self, t: TileId) -> Option<(Vec<u8>, Duration)> {
        let path = self.path(t);
        let bytes = fs::read(&path).ok()?;
        let age = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .unwrap_or(Duration::ZERO);
        Some((bytes, age))
    }

    /// Writes a tile atomically.
    pub fn write(&self, t: TileId, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(t);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("png.part");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)
    }

    /// Removes the oldest tiles (by modification time, which is when they were last fetched) until the folder is within
    /// `max_bytes`; tiles in `keep` stay whatever their age. Returns how many files were removed.
    pub fn prune(&self, max_bytes: u64, keep: &HashSet<TileId>) -> usize {
        fn entries(dir: &Path) -> impl Iterator<Item = fs::DirEntry> {
            fs::read_dir(dir).into_iter().flatten().flatten()
        }
        let mut files: Vec<(SystemTime, u64, PathBuf, TileId)> = Vec::new();
        let mut total = 0u64;
        for zd in entries(&self.dir) {
            let Some(z) = zd.file_name().to_str().and_then(|s| s.parse::<u8>().ok()) else { continue };
            for xd in entries(&zd.path()) {
                let Some(x) = xd.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
                for f in entries(&xd.path()) {
                    let name = f.file_name();
                    let Some(y) = name.to_str().and_then(|n| n.strip_suffix(".png")).and_then(|n| n.parse::<u32>().ok()) else { continue };
                    let Ok(meta) = f.metadata() else { continue };
                    if !meta.is_file() {
                        continue;
                    }
                    total += meta.len();
                    files.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), f.path(), TileId { z, x, y }));
                }
            }
        }
        if total <= max_bytes {
            return 0;
        }
        files.retain(|f| !keep.contains(&f.3));
        files.sort_by_key(|f| f.0);
        let mut removed = 0;
        for (_, len, path, _) in files {
            if total <= max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                removed += 1;
            }
        }
        removed
    }

    /// [`TileCache::prune`] to [`MAX_CACHE_BYTES`], keeping the tiles of the offline areas saved for this source.
    pub fn prune_to_limit(&self) -> usize {
        self.prune(MAX_CACHE_BYTES, &offline_tiles(&self.root, &self.source_key))
    }

    /// True when a cached tile exists and is younger than `max_age`.
    pub fn is_fresh(&self, t: TileId, max_age: Duration) -> bool {
        // the file's metadata only: the view asks this for every tile on screen each tick, so the bytes are not read
        let Ok(meta) = fs::metadata(self.path(t)) else { return false };
        let age = meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).unwrap_or(Duration::ZERO);
        age <= max_age
    }
}

/// Most tiles one Download Map may ask for. The public tile servers' usage policies forbid bulk downloads, so an
/// offline area stays small enough to be what one person browses; the pacing of `source` still applies.
pub const MAX_OFFLINE_TILES: usize = 300;
/// The tiles the offline areas saved for `source_key` keep, read from `<root>/areas.json` (the Maps app writes it: an
/// array of objects with `source`, `south`, `west`, `north`, `east`, `min_zoom` and `max_zoom`). A missing or damaged
/// file keeps nothing.
pub fn offline_tiles(root: &Path, source_key: &str) -> HashSet<TileId> {
    let Some(doc) = fs::read_to_string(root.join("areas.json")).ok().and_then(|t| json::parse(&t).ok()) else { return HashSet::new() };
    let mut out = HashSet::new();
    for entry in doc.as_array().unwrap_or_default() {
        if entry.str_field("source").as_deref() != Some(source_key) {
            continue;
        }
        let num = |k: &str| entry.get(k).and_then(json::Json::number).filter(|v| v.is_finite());
        let (Some(south), Some(west), Some(north), Some(east), Some(min), Some(max)) = (num("south"), num("west"), num("north"), num("east"), num("min_zoom"), num("max_zoom")) else { continue };
        if !(0.0..=30.0).contains(&min) || !(0.0..=30.0).contains(&max) || min > max {
            continue;
        }
        out.extend(OfflineArea { south, west, north, east, min_zoom: min as u8, max_zoom: max as u8 }.tiles());
    }
    out
}

/// An average tile, for the size estimate shown before a download starts (bytes).
pub const AVERAGE_TILE_BYTES: u64 = 18 * 1024;

/// An area to keep offline: a rectangle and the zoom levels (whole numbers) to keep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OfflineArea {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

impl OfflineArea {
    /// Every tile of the area, level by level. A rectangle across the date line is not split: `west` must be less
    /// than `east`.
    pub fn tiles(&self) -> Vec<TileId> {
        let mut out = Vec::new();
        for z in self.min_zoom..=self.max_zoom {
            let a = super::tiles::tile_at(self.north, self.west, z);
            let b = super::tiles::tile_at(self.south, self.east, z);
            let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
            let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    out.push(TileId { z, x, y });
                    if out.len() > MAX_OFFLINE_TILES {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// The tile count, and whether it is within `MAX_OFFLINE_TILES`.
    pub fn count(&self) -> (usize, bool) {
        let n = self.tiles().len();
        (n.min(MAX_OFFLINE_TILES), n <= MAX_OFFLINE_TILES)
    }
}

/// The text shown before a download starts: "126 tiles, about 2.2 MB".
pub fn download_estimate(tiles: usize) -> String {
    let bytes = tiles as u64 * AVERAGE_TILE_BYTES;
    let noun = if tiles == 1 { "tile" } else { "tiles" };
    if bytes >= 1024 * 1024 {
        format!("{tiles} {noun}, about {:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{tiles} {noun}, about {} KB", (bytes / 1024).max(1))
    }
}

impl TileCache {
    /// The tiles of `wanted` that are not cached yet (or are older than `max_age`), in order.
    pub fn missing(&self, wanted: &[TileId], max_age: Duration) -> Vec<TileId> {
        wanted.iter().copied().filter(|t| !self.is_fresh(*t, max_age)).collect()
    }
}

/// The default app data root: `$XDG_DATA_HOME/is.olibuijr.maps.desktop`, or `~/.local/share/...`.
pub fn default_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("is.olibuijr.maps.desktop"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("maps-cache-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_tile_round_trips_through_the_cache() {
        let root = temp_root("roundtrip");
        let cache = TileCache::new(&root, "tile.example.org");
        let t = TileId { z: 3, x: 5, y: 2 };
        assert!(cache.read(t).is_none());
        cache.write(t, b"png-bytes").unwrap();
        let (bytes, age) = cache.read(t).unwrap();
        assert_eq!(bytes, b"png-bytes");
        assert!(age < Duration::from_secs(60));
        assert!(cache.is_fresh(t, MAX_TILE_AGE));
        assert!(!cache.path(t).with_extension("png.part").exists());
        assert!(cache.path(t).ends_with("tiles/tile.example.org/3/5/2.png"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_offline_area_lists_its_tiles_level_by_level() {
        // a small area at zoom 10 around Reykjavik is a handful of tiles; the next level has about four times more
        let area = OfflineArea { south: 64.10, west: -22.05, north: 64.18, east: -21.85, min_zoom: 10, max_zoom: 11 };
        let tiles = area.tiles();
        assert!(tiles.iter().filter(|t| t.z == 10).count() >= 1);
        assert!(tiles.iter().filter(|t| t.z == 11).count() >= tiles.iter().filter(|t| t.z == 10).count());
        assert!(tiles.iter().all(|t| t.z == 10 || t.z == 11));
        assert!(tiles.contains(&super::super::tiles::tile_at(64.1466, -21.9426, 10)));
        assert_eq!(area.count(), (tiles.len(), true));
    }

    #[test]
    fn a_huge_area_stops_at_the_limit() {
        let world = OfflineArea { south: -80.0, west: -179.0, north: 80.0, east: 179.0, min_zoom: 8, max_zoom: 9 };
        assert_eq!(world.count(), (MAX_OFFLINE_TILES, false));
    }

    #[test]
    fn the_estimate_reads_in_kilobytes_and_megabytes() {
        assert_eq!(download_estimate(1), "1 tile, about 18 KB");
        assert_eq!(download_estimate(100), "100 tiles, about 1.8 MB");
    }

    #[test]
    fn only_missing_tiles_are_listed_for_download() {
        let root = temp_root("missing");
        let cache = TileCache::new(&root, "tile.example.org");
        let (a, b) = (TileId { z: 2, x: 1, y: 1 }, TileId { z: 2, x: 2, y: 1 });
        cache.write(a, b"x").unwrap();
        assert_eq!(cache.missing(&[a, b], MAX_TILE_AGE), vec![b]);
        let _ = fs::remove_dir_all(&root);
    }

    fn age(cache: &TileCache, t: TileId, secs_ago: u64) {
        let file = fs::File::options().write(true).open(cache.path(t)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn the_oldest_tiles_go_first_when_the_folder_is_over_its_limit() {
        let root = temp_root("prune");
        let cache = TileCache::new(&root, "tile.example.org");
        let (a, b, c) = (TileId { z: 3, x: 1, y: 1 }, TileId { z: 3, x: 2, y: 1 }, TileId { z: 4, x: 2, y: 3 });
        for t in [a, b, c] {
            cache.write(t, &[0u8; 100]).unwrap();
        }
        age(&cache, a, 3000);
        age(&cache, b, 2000);
        age(&cache, c, 1000);
        assert_eq!(cache.prune(300, &HashSet::new()), 0, "within the limit nothing goes");
        assert_eq!(cache.prune(250, &HashSet::new()), 1);
        assert!(cache.read(a).is_none() && cache.read(b).is_some() && cache.read(c).is_some(), "the oldest tile was removed");
        assert_eq!(cache.prune(0, &HashSet::new()), 2);
        assert!(cache.read(b).is_none() && cache.read(c).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tiles_kept_for_offline_use_survive_the_prune() {
        let root = temp_root("prune-keep");
        let cache = TileCache::new(&root, "tile.example.org");
        let (old, fresh) = (TileId { z: 3, x: 1, y: 1 }, TileId { z: 3, x: 2, y: 1 });
        cache.write(old, &[0u8; 100]).unwrap();
        cache.write(fresh, &[0u8; 100]).unwrap();
        age(&cache, old, 99_999);
        let keep: HashSet<TileId> = [old].into_iter().collect();
        assert_eq!(cache.prune(100, &keep), 1);
        assert!(cache.read(old).is_some(), "the offline tile stays although it is the oldest");
        assert!(cache.read(fresh).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_offline_areas_of_a_source_are_read_from_areas_json() {
        let root = temp_root("offline-tiles");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("areas.json"),
            r#"[{"name":"Home","source":"tile.example.org","south":64.10,"west":-22.05,"north":64.18,"east":-21.85,"min_zoom":10,"max_zoom":11,"tiles":9},
                {"name":"Other","source":"other.example","south":0,"west":0,"north":1,"east":1,"min_zoom":3,"max_zoom":3,"tiles":1},
                {"name":"Bad","source":"tile.example.org","south":0,"west":0,"north":1,"east":1,"min_zoom":9,"max_zoom":3}]"#,
        )
        .unwrap();
        let tiles = offline_tiles(&root, "tile.example.org");
        assert!(!tiles.is_empty());
        assert!(tiles.contains(&super::super::tiles::tile_at(64.1466, -21.9426, 10)));
        assert!(tiles.iter().all(|t| t.z == 10 || t.z == 11), "the other source's area and the bad entry add nothing");
        assert!(offline_tiles(&root, "none.example").is_empty());
        assert!(offline_tiles(&temp_root("no-such-root"), "tile.example.org").is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sources_do_not_share_tiles() {
        let root = temp_root("sources");
        let a = TileCache::new(&root, "a.example");
        let b = TileCache::new(&root, "b.example");
        let t = TileId { z: 0, x: 0, y: 0 };
        a.write(t, b"a").unwrap();
        assert!(b.read(t).is_none());
        let _ = fs::remove_dir_all(&root);
    }
}

//! On-disk tile cache under the app data directory: `<root>/<source key>/<z>/<x>/<y>.png`.
//! Writes go to a temporary file and are renamed into place, so a crash never leaves a half tile.

use super::tiles::TileId;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Tiles older than this are refetched when the network is up.
pub const MAX_TILE_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Clone, Debug)]
pub struct TileCache {
    dir: PathBuf,
}

impl TileCache {
    /// `root` is the app data directory; `source_key` comes from `TileSource::cache_key`.
    pub fn new(root: &Path, source_key: &str) -> Self {
        Self { dir: root.join("tiles").join(source_key) }
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

    /// True when a cached tile exists and is younger than `max_age`.
    pub fn is_fresh(&self, t: TileId, max_age: Duration) -> bool {
        self.read(t).is_some_and(|(_, age)| age <= max_age)
    }
}

/// Most tiles one Download Map may ask for. The public tile servers' usage policies forbid bulk downloads, so an
/// offline area stays small enough to be what one person browses; the pacing of `source` still applies.
pub const MAX_OFFLINE_TILES: usize = 300;
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
        assert!(tiles.contains(&super::tiles::tile_at(64.1466, -21.9426, 10)));
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

//! On-disk tile cache under the app data directory: `<root>/<source key>/<z>/<x>/<y>.png`.
//! Writes go to a temporary file and are renamed into place, so a crash never leaves a half tile.

use crate::tiles::TileId;
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

//! Artwork: JPEG/PNG decoding, cover-scaling to a card and an LRU cache
//! with a byte budget, plus a disk cache of the raw downloads.

use crate::gfx::Image;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Decoded artwork budget: 40 MB keeps memory use modest.
pub const BUDGET: usize = 40 * 1024 * 1024;

pub use crate::cg::imageio::decode;
pub use crate::imgdec::cover;

fn bytes_of(img: &Image) -> usize {
    img.px.len() * 4
}

/// Decoded images keyed by (item id, kind), evicting least recently used
/// ones once the byte budget is exceeded.
pub struct Cache {
    budget: usize,
    used: usize,
    tick: u64,
    map: HashMap<(String, u8), (Image, u64)>,
}

impl Cache {
    pub fn new(budget: usize) -> Self {
        Cache { budget, used: 0, tick: 0, map: HashMap::new() }
    }

    pub fn get(&mut self, id: &str, kind: u8) -> Option<&Image> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(&(id.to_string(), kind)).map(|(img, used)| {
            *used = t;
            &*img
        })
    }

    /// The image without touching its recency (for borrowed views).
    pub fn peek(&self, id: &str, kind: u8) -> Option<&Image> {
        self.map.get(&(id.to_string(), kind)).map(|(img, _)| img)
    }

    pub fn contains(&self, id: &str, kind: u8) -> bool {
        self.map.contains_key(&(id.to_string(), kind))
    }

    pub fn insert(&mut self, id: &str, kind: u8, img: Image) {
        let key = (id.to_string(), kind);
        if let Some((old, _)) = self.map.remove(&key) {
            self.used -= bytes_of(&old);
        }
        self.tick += 1;
        self.used += bytes_of(&img);
        self.map.insert(key, (img, self.tick));
        while self.used > self.budget && self.map.len() > 1 {
            let victim = self.map.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()).unwrap();
            if let Some((img, _)) = self.map.remove(&victim) {
                self.used -= bytes_of(&img);
            }
        }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Where a raw download named `name` (e.g. "123.img", "123.torrent") is kept.
pub fn disk_path(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
    dir.join("cache").join(safe)
}

pub fn disk_read(dir: &Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(disk_path(dir, name)).ok().filter(|b| !b.is_empty())
}

pub fn disk_write(dir: &Path, name: &str, bytes: &[u8]) {
    let p = disk_path(dir, name);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    #[test]
    fn decoding_and_cover_scaling_come_from_imgdec() {
        let img = decode(include_bytes!("fixtures/dot.jpg")).unwrap();
        assert_eq!((cover(&img, 4, 4).w, img.h), (4, 8));
    }

    #[test]
    fn cache_evicts_least_recently_used_within_budget() {
        let mut c = Cache::new(4 * 100 * 3); // room for three 10x10 images
        for id in ["a", "b", "c"] {
            c.insert(id, 0, solid(10, 10, 1));
        }
        assert_eq!(c.len(), 3);
        assert!(c.peek("a", 0).is_some() && c.peek("zz", 0).is_none());
        assert!(c.get("a", 0).is_some(), "touch a");
        c.insert("d", 0, solid(10, 10, 1));
        assert!(c.contains("a", 0) && c.contains("c", 0) && c.contains("d", 0));
        assert!(!c.contains("b", 0), "b was the oldest");
        assert!(c.used() <= 1200);
        c.insert("a", 1, solid(10, 10, 2));
        assert_eq!(c.get("a", 1).unwrap().px[0], 2, "kinds are separate keys");
        c.insert("huge", 0, solid(50, 50, 3));
        assert_eq!(c.len(), 1, "an oversized image evicts everything else but stays");
        assert!(!c.is_empty());
    }

    #[test]
    fn disk_cache_round_trips() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-deildu-art"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(disk_read(&dir, "1/2.img").is_none());
        disk_write(&dir, "1/2.img", b"bytes");
        assert_eq!(disk_read(&dir, "1/2.img").unwrap(), b"bytes");
        assert!(disk_path(&dir, "1/2.img").ends_with("cache/12.img"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

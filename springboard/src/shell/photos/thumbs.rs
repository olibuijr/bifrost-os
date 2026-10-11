//! Thumbnails: square covers of each picture, kept as RIMG files in a disk
//! cache bounded by file count (least recently used go first) and in a
//! memory cache bounded by bytes.

use crate::gfx::Image;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

/// Thumbnail edge in px: enough for five columns at full resolution and a
/// soft upscale at three.
pub const SIZE: usize = 256;
/// Files kept on disk (about 380 MB at `SIZE`).
pub const MAX_FILES: usize = 1500;
/// Bytes the thumbnail files may take on disk.
pub const MAX_DISK_BYTES: u64 = 192 * 1024 * 1024;
/// Writes between eviction scans: listing the directory per write is O(files).
pub const EVICT_EVERY: usize = 32;
/// Decoded thumbnails held in memory.
pub const MEM_BUDGET: usize = 24 * 1024 * 1024;

/// The RIMG bytes of `img` (magic, width, height, premultiplied pixels).
pub fn to_raw(img: &Image) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + img.px.len() * 4);
    out.extend(b"RIMG");
    out.extend((img.w as u32).to_le_bytes());
    out.extend((img.h as u32).to_le_bytes());
    out.extend(img.px.iter().flat_map(|p| p.to_le_bytes()));
    out
}

/// Which of `entries` (age stamp, size in bytes; a smaller stamp is older)
/// to delete so at most `max_files` files and `max_bytes` bytes remain.
/// Oldest go first; the newest entry is always kept. Returns indices.
pub fn eviction_victims(entries: &[(u64, u64)], max_files: usize, max_bytes: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| (entries[i].0, i));
    let (mut count, mut bytes) = (entries.len(), entries.iter().map(|e| e.1).sum::<u64>());
    let mut out = Vec::new();
    for i in order {
        if count <= 1 || (count <= max_files && bytes <= max_bytes) {
            break;
        }
        count -= 1;
        bytes -= entries[i].1;
        out.push(i);
    }
    out
}

/// Thumbnail files under one directory, evicted by file count and bytes.
#[derive(Clone, Debug)]
pub struct DiskCache {
    dir: PathBuf,
    max: usize,
    max_bytes: u64,
    /// writes since creation, shared by the clones the workers hold
    writes: Arc<AtomicUsize>,
    /// a scan runs on every n-th write
    evict_every: usize,
}

impl DiskCache {
    pub fn new(dir: impl Into<PathBuf>, max: usize) -> Self {
        DiskCache { dir: dir.into(), max: max.max(1), max_bytes: MAX_DISK_BYTES, writes: Arc::new(AtomicUsize::new(0)), evict_every: EVICT_EVERY }
    }

    /// The same cache with a different byte budget.
    pub fn with_max_bytes(mut self, bytes: u64) -> Self {
        self.max_bytes = bytes;
        self
    }

    /// The same cache scanning for victims on every n-th write (at least 1).
    pub fn with_evict_every(mut self, n: usize) -> Self {
        self.evict_every = n.max(1);
        self
    }

    pub fn path(&self, key: &str) -> PathBuf {
        let safe: String = key.chars().filter(char::is_ascii_alphanumeric).collect();
        self.dir.join(format!("{safe}.raw"))
    }

    /// The cached thumbnail, marked as just used.
    pub fn read(&self, key: &str) -> Option<Image> {
        let p = self.path(key);
        let img = Image::from_raw(&std::fs::read(&p).ok()?)?;
        if let Ok(f) = std::fs::File::options().write(true).open(&p) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(img)
    }

    /// Stores a thumbnail and drops the least recently used ones over the limit.
    pub fn write(&self, key: &str, img: &Image) {
        let _ = std::fs::create_dir_all(&self.dir);
        let p = self.path(key);
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, to_raw(img)).is_ok() && std::fs::rename(&tmp, &p).is_ok() && (self.writes.fetch_add(1, Ordering::Relaxed) + 1) % self.evict_every == 0 {
            self.evict();
        }
    }

    /// (modified time, size, path) of every cached file.
    fn files(&self) -> Vec<(SystemTime, u64, PathBuf)> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        rd.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "raw"))
            .filter_map(|e| {
                let md = e.metadata().ok()?;
                Some((md.modified().ok()?, md.len(), e.path()))
            })
            .collect()
    }

    fn evict(&self) {
        let files = self.files();
        let stamps: Vec<(u64, u64)> = files.iter().map(|(t, n, _)| (t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64), *n)).collect();
        for i in eviction_victims(&stamps, self.max, self.max_bytes) {
            let _ = std::fs::remove_file(&files[i].2);
        }
    }

    /// Bytes the cached files take.
    pub fn bytes(&self) -> u64 {
        self.files().iter().map(|f| f.1).sum()
    }

    /// Cached thumbnails on disk.
    pub fn len(&self) -> usize {
        self.files().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Decoded images keyed by string, least recently used evicted past a byte budget.
pub struct MemCache {
    budget: usize,
    max_items: usize,
    used: usize,
    tick: u64,
    map: HashMap<String, (Image, u64)>,
}

impl MemCache {
    pub fn new(budget: usize) -> Self {
        MemCache { budget, max_items: usize::MAX, used: 0, tick: 0, map: HashMap::new() }
    }

    /// Also keeps at most `n` images (at least one), whatever their size.
    pub fn with_max_items(mut self, n: usize) -> Self {
        self.max_items = n.max(1);
        self
    }

    pub fn get(&mut self, key: &str) -> Option<&Image> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(key).map(|(img, used)| {
            *used = t;
            &*img
        })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn insert(&mut self, key: &str, img: Image) {
        self.remove(key);
        self.tick += 1;
        self.used += img.px.len() * 4;
        self.map.insert(key.to_string(), (img, self.tick));
        while (self.used > self.budget || self.map.len() > self.max_items) && self.map.len() > 1 {
            let victim = self.map.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()).unwrap();
            self.remove(&victim);
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Some((old, _)) = self.map.remove(key) {
            self.used -= old.px.len() * 4;
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn used(&self) -> usize {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;
    use std::time::{Duration, UNIX_EPOCH};

    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    #[test]
    fn disk_cache_round_trips_and_evicts_least_recently_used() {
        let dir = super::super::library::tests::scratch("thumbs");
        let c = DiskCache::new(dir.join("thumbs"), 3).with_evict_every(1);
        assert!(c.is_empty() && c.read("a").is_none());
        for (i, k) in ["a", "b", "c"].iter().enumerate() {
            c.write(k, &solid(4, 4, i as u32 + 1));
            let f = std::fs::File::options().write(true).open(c.path(k)).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(1000 + i as u64)).unwrap();
        }
        assert_eq!(c.len(), 3);
        assert_eq!(c.read("a").unwrap().px[0], 1, "reading touches a");
        c.write("d", &solid(4, 4, 4));
        assert_eq!(c.len(), 3);
        assert!(c.read("b").is_none(), "b was the least recently used");
        assert!(c.read("a").is_some() && c.read("c").is_some() && c.read("d").is_some());
        assert_eq!(c.path("../x/y").file_name().unwrap(), "xy.raw", "keys are sanitised");
        assert_eq!(to_raw(&solid(1, 1, 7)).len(), 16);
        assert!(c.dir().ends_with("thumbs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mem_cache_evicts_least_recently_used_within_budget() {
        let mut c = MemCache::new(4 * 100 * 3);
        for id in ["a", "b", "c"] {
            c.insert(id, solid(10, 10, 1));
        }
        assert_eq!(c.len(), 3);
        assert!(c.get("a").is_some());
        c.insert("d", solid(10, 10, 1));
        assert!(c.contains("a") && c.contains("c") && c.contains("d") && !c.contains("b"));
        assert!(c.used() <= 1200);
        c.insert("a", solid(10, 10, pack(2, 0, 0, 255)));
        assert_eq!(c.get("a").unwrap().px[0], pack(2, 0, 0, 255), "reinserting replaces");
        c.insert("huge", solid(50, 50, 3));
        assert_eq!(c.len(), 1, "an oversized image evicts everything else but stays");
        c.remove("huge");
        assert!(c.is_empty() && c.used() == 0);
    }

    #[test]
    fn eviction_honours_both_the_file_and_the_byte_budget() {
        // stamp, size
        let e = [(5, 100), (1, 100), (3, 100), (9, 100)];
        assert!(eviction_victims(&e, 4, 400).is_empty());
        assert_eq!(eviction_victims(&e, 3, 400), [1], "oldest goes");
        assert_eq!(eviction_victims(&e, 10, 250), [1, 2], "bytes: two oldest");
        assert_eq!(eviction_victims(&e, 1, 1), [1, 2, 0], "the newest is always kept");
        let mixed = [(1, 500), (2, 10), (3, 10)];
        assert_eq!(eviction_victims(&mixed, 10, 100), [0]);
        assert!(eviction_victims(&[], 1, 1).is_empty());
        assert_eq!(eviction_victims(&[(7, 9)], 1, 1), Vec::<usize>::new());
    }

    #[test]
    fn disk_cache_drops_old_files_over_the_byte_budget() {
        let dir = super::super::library::tests::scratch("thumbs-bytes");
        let one = to_raw(&solid(4, 4, 1)).len() as u64;
        let c = DiskCache::new(dir.join("t"), 100).with_max_bytes(one * 2 + one / 2).with_evict_every(1);
        for (i, k) in ["a", "b", "c"].iter().enumerate() {
            c.write(k, &solid(4, 4, 1));
            let f = std::fs::File::options().write(true).open(c.path(k)).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(1000 + i as u64)).unwrap();
        }
        c.write("d", &solid(4, 4, 1));
        assert!(c.len() <= 2 && c.bytes() <= one * 2 + one / 2, "{} files, {} bytes", c.len(), c.bytes());
        assert!(c.read("d").is_some(), "the newest survives");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_cache_scans_for_victims_only_every_nth_write() {
        let dir = super::super::library::tests::scratch("thumbs-lazy");
        let c = DiskCache::new(dir.join("t"), 2).with_evict_every(4);
        for k in ["a", "b", "c"] {
            c.write(k, &solid(2, 2, 1));
        }
        assert_eq!(c.len(), 3, "no scan yet");
        c.write("d", &solid(2, 2, 1));
        assert_eq!(c.len(), 2, "the fourth write scans");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mem_cache_caps_the_number_of_images() {
        let mut c = MemCache::new(usize::MAX / 2).with_max_items(3);
        for id in ["a", "b", "c"] {
            c.insert(id, solid(2, 2, 1));
        }
        assert!(c.get("a").is_some());
        c.insert("d", solid(2, 2, 1));
        assert_eq!(c.len(), 3);
        assert!(c.contains("a") && c.contains("c") && c.contains("d") && !c.contains("b"), "least recently used goes");
        c.insert("e", solid(2, 2, 1));
        assert_eq!(c.len(), 3);
        assert_eq!(MemCache::new(10).with_max_items(0).max_items, 1);
    }
}

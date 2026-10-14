//! Collections the library derives by itself: Memories (a past month with enough photos), Places (photos with a
//! GPS position near each other), Duplicates (identical files) and the keyword index; and the Folders that hold
//! user albums. Everything here is plain data in and out, so the app only has to feed it.

use super::library::{civil, month_title, Photo};
use super::meta::Meta;
use crate::shell::prefs::{escape_field, unescape_field};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The most folders.
pub const MAX_FOLDERS: usize = 6;
/// The most memories listed.
pub const MAX_MEMORIES: usize = 12;
/// A month needs this many photos to become a memory.
pub const MEMORY_MIN: usize = 4;
/// Photos closer than this many degrees (about 11 km) share a place.
pub const PLACE_CELL: f64 = 0.1;

/// A folder of albums: a name and the slots (`ua.<n>`) of the user albums inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    pub albums: Vec<u8>,
}

impl Folder {
    pub fn encode(&self) -> String {
        format!("{}\t{}", escape_field(&self.name), self.albums.iter().map(u8::to_string).collect::<Vec<_>>().join(","))
    }

    pub fn decode(text: &str) -> Option<Folder> {
        // the settings file trims the value, so a folder with no albums has lost its tab
        let (name, albums) = text.split_once('\t').unwrap_or((text, ""));
        let name = unescape_field(name, 80)?;
        if name.is_empty() {
            return None;
        }
        let mut list: Vec<u8> = albums.split(',').filter_map(|a| a.trim().parse().ok()).collect();
        list.dedup();
        Some(Folder { name, albums: list })
    }

    /// Adds an album, or takes it out when it is in already; true when it is in the folder now.
    pub fn toggle(&mut self, slot: u8) -> bool {
        match self.albums.iter().position(|&a| a == slot) {
            Some(i) => {
                self.albums.remove(i);
                false
            }
            None => {
                self.albums.push(slot);
                true
            }
        }
    }

    /// The folder after user album `slot` was deleted: it is gone from the list and later slots move down one, as
    /// `albums::delete_album_writes` moves them.
    pub fn after_album_deleted(&self, slot: u8) -> Folder {
        Folder { name: self.name.clone(), albums: self.albums.iter().filter(|&&a| a != slot).map(|&a| if a > slot { a - 1 } else { a }).collect() }
    }
}

/// A past month with enough photos to look back at: its title and its photos, newest memory first. `photos` is in
/// any order; the month of `now` is left out because it is still happening.
pub fn memories(photos: &[Photo], now: i64, offset: i32) -> Vec<(String, Vec<Photo>)> {
    let current = civil(now, offset);
    let mut months: BTreeMap<(i32, u32), Vec<Photo>> = BTreeMap::new();
    for p in photos {
        let d = civil(p.mtime, offset);
        if (d.year, d.month) < (current.year, current.month) {
            months.entry((d.year, d.month)).or_default().push(p.clone());
        }
    }
    months
        .into_iter()
        .rev()
        .filter(|(_, v)| v.len() >= MEMORY_MIN)
        .take(MAX_MEMORIES)
        .map(|(_, mut v)| {
            v.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
            (month_title(&civil(v[0].mtime, offset)), v)
        })
        .collect()
}

/// The grid cell a position falls in: whole steps of `PLACE_CELL`.
pub fn place_cell(lat: f64, lon: f64) -> (i32, i32) {
    ((lat / PLACE_CELL).floor() as i32, (lon / PLACE_CELL).floor() as i32)
}

/// Photos grouped by where they were taken. `gps` holds the positions found in the files; a photo whose details
/// name a place (and do not hide it) gives the group that name. Groups with the most photos come first.
pub fn places(photos: &[Photo], gps: &HashMap<PathBuf, (f64, f64)>, metas: &HashMap<PathBuf, Meta>) -> Vec<(String, Vec<Photo>)> {
    let mut cells: BTreeMap<(i32, i32), (Vec<Photo>, Option<String>, (f64, f64))> = BTreeMap::new();
    for p in photos {
        let Some(&(lat, lon)) = gps.get(&p.path) else { continue };
        if metas.get(&p.path).is_some_and(|m| m.hide_location) {
            continue;
        }
        let entry = cells.entry(place_cell(lat, lon)).or_insert_with(|| (vec![], None, (lat, lon)));
        entry.0.push(p.clone());
        if entry.1.is_none() {
            entry.1 = metas.get(&p.path).map(|m| m.place.clone()).filter(|n| !n.is_empty());
        }
    }
    let mut out: Vec<(String, Vec<Photo>)> = cells.into_values().map(|(v, name, (lat, lon))| (name.unwrap_or_else(|| super::exif::format_gps(lat, lon)), v)).collect();
    out.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Keywords across the library: (keyword as first written, its photos), by name.
pub fn keyword_index(photos: &[Photo], metas: &HashMap<PathBuf, Meta>) -> Vec<(String, Vec<Photo>)> {
    let mut map: BTreeMap<String, (String, Vec<Photo>)> = BTreeMap::new();
    for p in photos {
        let Some(m) = metas.get(&p.path) else { continue };
        for k in &m.keywords {
            map.entry(k.to_lowercase()).or_insert_with(|| (k.clone(), vec![])).1.push(p.clone());
        }
    }
    map.into_values().collect()
}

/// A 64-bit FNV-1a hash of a file's bytes, read in blocks; None when it cannot be read.
pub fn file_hash(path: &Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            return Some(h);
        }
        for &b in &buf[..n] {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Hashes the files that share a size with another file (the others cannot be duplicates).
/// Hashes already worked out (`known`) are reused.
pub fn hash_candidates(photos: &[Photo], known: &HashMap<PathBuf, u64>) -> HashMap<PathBuf, u64> {
    let mut by_size: HashMap<u64, Vec<&Photo>> = HashMap::new();
    for p in photos {
        by_size.entry(p.size).or_default().push(p);
    }
    let mut out = HashMap::new();
    for group in by_size.into_values().filter(|g| g.len() > 1) {
        for p in group {
            if let Some(&h) = known.get(&p.path) {
                out.insert(p.path.clone(), h);
            } else if let Some(h) = file_hash(&p.path) {
                out.insert(p.path.clone(), h);
            }
        }
    }
    out
}

/// Sets of identical files: same size and same hash. Each set is oldest first, and the sets are ordered by their
/// first photo.
pub fn duplicate_groups(photos: &[Photo], hashes: &HashMap<PathBuf, u64>) -> Vec<Vec<Photo>> {
    let mut map: BTreeMap<(u64, u64), Vec<Photo>> = BTreeMap::new();
    for p in photos {
        if let Some(&h) = hashes.get(&p.path) {
            map.entry((p.size, h)).or_default().push(p.clone());
        }
    }
    let mut out: Vec<Vec<Photo>> = map
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut g| {
            g.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
            g
        })
        .collect();
    out.sort_by(|a, b| a[0].mtime.cmp(&b[0].mtime).then_with(|| a[0].path.cmp(&b[0].path)));
    out
}

/// Which photo of a duplicate set to keep: the one with the most to lose (a favourite, an edit, stars, details),
/// then the oldest. `score` rates a photo; the others are what Merge removes.
pub fn keeper(group: &[Photo], score: impl Fn(&Photo) -> u32) -> Option<usize> {
    group.iter().enumerate().max_by(|(ia, a), (ib, b)| score(a).cmp(&score(b)).then_with(|| b.mtime.cmp(&a.mtime)).then_with(|| ib.cmp(ia))).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::super::library::tests::{picture, scratch};
    use super::*;

    fn photo(name: &str, mtime: i64) -> Photo {
        Photo { path: PathBuf::from(format!("/p/{name}")), mtime, size: 10 }
    }

    const OCT_2026: i64 = 1_791_000_000;

    #[test]
    fn folders_round_trip_and_follow_deleted_albums() {
        let mut f = Folder { name: "Trips".into(), albums: vec![] };
        assert!(f.toggle(1));
        assert!(f.toggle(3));
        assert!(!f.toggle(1));
        assert_eq!(f.albums, vec![3]);
        f.toggle(0);
        f.toggle(4);
        assert_eq!(Folder::decode(&f.encode()), Some(f.clone()));
        assert_eq!(f.after_album_deleted(0).albums, vec![2, 3], "later albums move down when an earlier one is deleted");
        assert_eq!(f.after_album_deleted(3).albums, vec![0, 3]);
        assert_eq!(Folder::decode("\t1"), None);
        let empty = Folder { name: "Empty".into(), albums: vec![] };
        assert_eq!(Folder::decode(empty.encode().trim()), Some(empty), "a folder with no albums survives the trim");
    }

    #[test]
    fn memories_are_past_months_with_enough_photos_newest_first() {
        let now = OCT_2026;
        let day = 86_400;
        let mut photos = Vec::new();
        for i in 0..5 {
            photos.push(photo(&format!("aug{i}"), now - 60 * day + i * 3600));
        }
        for i in 0..2 {
            photos.push(photo(&format!("sep{i}"), now - 30 * day + i * 3600));
        }
        for i in 0..6 {
            photos.push(photo(&format!("now{i}"), now - i * 3600));
        }
        let m = memories(&photos, now, 0);
        assert_eq!(m.len(), 1, "a thin month and the running month are not memories");
        assert_eq!(m[0].1.len(), 5);
        assert!(m[0].0.contains("2026"));
        assert!(m[0].1.windows(2).all(|w| w[0].mtime <= w[1].mtime));
    }

    #[test]
    fn places_group_nearby_photos_and_respect_hidden_locations() {
        let a = photo("a", 1);
        let b = photo("b", 2);
        let c = photo("c", 3);
        let d = photo("d", 4);
        let mut gps = HashMap::new();
        gps.insert(a.path.clone(), (64.141, -21.941));
        gps.insert(b.path.clone(), (64.149, -21.931));
        gps.insert(c.path.clone(), (40.7, -74.0));
        gps.insert(d.path.clone(), (64.145, -21.935));
        let mut metas = HashMap::new();
        metas.insert(a.path.clone(), Meta { place: "Reykjavik".into(), ..Meta::default() });
        metas.insert(d.path.clone(), Meta { hide_location: true, ..Meta::default() });
        let groups = places(&[a, b, c, d], &gps, &metas);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "Reykjavik");
        assert_eq!(groups[0].1.len(), 2, "the photo with its location hidden is left out");
        assert!(groups[1].0.contains('N') && groups[1].0.contains('W'), "an unnamed place shows its position");
    }

    #[test]
    fn the_keyword_index_merges_case_and_lists_photos() {
        let a = photo("a", 1);
        let b = photo("b", 2);
        let mut metas = HashMap::new();
        metas.insert(a.path.clone(), Meta { keywords: vec!["Trip".into(), "sea".into()], ..Meta::default() });
        metas.insert(b.path.clone(), Meta { keywords: vec!["trip".into()], ..Meta::default() });
        let idx = keyword_index(&[a, b], &metas);
        assert_eq!(idx.len(), 2);
        let trip = idx.iter().find(|(k, _)| k == "Trip").unwrap();
        assert_eq!(trip.1.len(), 2);
    }

    #[test]
    fn identical_files_are_found_by_size_and_hash() {
        let dir = scratch("dupes");
        picture(&dir.join("a.png"), 1_790_000_000);
        std::fs::copy(dir.join("a.png"), dir.join("a copy.png")).unwrap();
        picture(&dir.join("other.png"), 1_790_000_100);
        let mut other = std::fs::read(dir.join("other.png")).unwrap();
        let last = other.len() - 1;
        other[last] ^= 0xFF;
        std::fs::write(dir.join("other.png"), other).unwrap();
        let photos = super::super::library::scan(&[dir.clone()]);
        assert_eq!(photos.len(), 3);
        let hashes = hash_candidates(&photos, &HashMap::new());
        let groups = duplicate_groups(&photos, &hashes);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_keeps_the_photo_with_the_most_to_lose_then_the_oldest() {
        let g = vec![photo("old", 10), photo("fav", 20), photo("new", 30)];
        assert_eq!(keeper(&g, |p| u32::from(p.name() == "fav")), Some(1));
        assert_eq!(keeper(&g, |_| 0), Some(0), "equal scores keep the oldest");
        assert_eq!(keeper(&[], |_| 0), None);
    }
}

//! The list of areas kept for offline use, saved as `areas.json` in the app data directory. Each entry records the
//! rectangle, the zoom levels and the tile source it was downloaded from, so the list can show it and Delete can
//! remove its tiles from the cache without touching tiles another area still needs.

use crate::cache::{OfflineArea, TileCache};
use crate::tiles::TileId;
use bifrost_core::json::{self, Json};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One downloaded area.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedArea {
    pub name: String,
    pub area: OfflineArea,
    /// `TileSource::cache_key` of the source the tiles came from
    pub source_key: String,
    pub tiles: usize,
}

impl SavedArea {
    /// "Reykjavik, zoom 11 to 13, 42 tiles".
    pub fn line(&self) -> String {
        let zoom = if self.area.min_zoom == self.area.max_zoom { format!("zoom {}", self.area.min_zoom) } else { format!("zoom {} to {}", self.area.min_zoom, self.area.max_zoom) };
        let noun = if self.tiles == 1 { "tile" } else { "tiles" };
        format!("{}, {zoom}, {} {noun}", self.name, self.tiles)
    }

    fn to_json(&self) -> Json {
        let a = &self.area;
        Json::Obj(vec![
            ("name".into(), Json::Str(self.name.clone())),
            ("source".into(), Json::Str(self.source_key.clone())),
            ("south".into(), Json::Num(a.south)),
            ("west".into(), Json::Num(a.west)),
            ("north".into(), Json::Num(a.north)),
            ("east".into(), Json::Num(a.east)),
            ("min_zoom".into(), Json::Num(a.min_zoom as f64)),
            ("max_zoom".into(), Json::Num(a.max_zoom as f64)),
            ("tiles".into(), Json::Num(self.tiles as f64)),
        ])
    }

    fn from_json(j: &Json) -> Option<SavedArea> {
        let num = |k: &str| j.get(k).and_then(Json::number).filter(|v| v.is_finite());
        let (min_zoom, max_zoom) = (num("min_zoom")?, num("max_zoom")?);
        if !(0.0..=30.0).contains(&min_zoom) || !(0.0..=30.0).contains(&max_zoom) || min_zoom > max_zoom {
            return None;
        }
        Some(SavedArea {
            name: j.str_field("name").unwrap_or_default(),
            source_key: j.str_field("source").filter(|s| !s.is_empty())?,
            area: OfflineArea { south: num("south")?, west: num("west")?, north: num("north")?, east: num("east")?, min_zoom: min_zoom as u8, max_zoom: max_zoom as u8 },
            tiles: num("tiles").unwrap_or(0.0).max(0.0) as usize,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Areas {
    pub list: Vec<SavedArea>,
}

impl Areas {
    /// Adds an area. The same rectangle, levels and source replaces the older entry instead of repeating it.
    pub fn add(&mut self, entry: SavedArea) {
        self.list.retain(|a| !(a.area == entry.area && a.source_key == entry.source_key));
        self.list.push(entry);
    }

    /// Removes area `index` and returns it; its tiles are deleted with `delete_tiles`.
    pub fn remove(&mut self, index: usize) -> Option<SavedArea> {
        (index < self.list.len()).then(|| self.list.remove(index))
    }

    /// The tiles of `gone` that no remaining area of the same source keeps.
    pub fn orphans(&self, gone: &SavedArea) -> Vec<TileId> {
        let kept: HashSet<TileId> = self.list.iter().filter(|a| a.source_key == gone.source_key).flat_map(|a| a.area.tiles()).collect();
        gone.area.tiles().into_iter().filter(|t| !kept.contains(t)).collect()
    }

    pub fn to_json(&self) -> String {
        Json::Arr(self.list.iter().map(SavedArea::to_json).collect()).encode()
    }

    pub fn from_json(text: &str) -> Result<Areas, String> {
        let doc = json::parse(text)?;
        Ok(Areas { list: doc.as_array().map(|a| a.iter().filter_map(SavedArea::from_json).collect()).unwrap_or_default() })
    }
}

/// Deletes cached tiles and returns how many files were removed. Missing files are fine.
pub fn delete_tiles(cache: &TileCache, tiles: &[TileId]) -> usize {
    tiles.iter().filter(|t| fs::remove_file(cache.path(**t)).is_ok()).count()
}

pub fn path(root: &Path) -> PathBuf {
    root.join("areas.json")
}

/// A missing or damaged file is an empty list.
pub fn load(root: &Path) -> Areas {
    fs::read_to_string(path(root)).ok().and_then(|t| Areas::from_json(&t).ok()).unwrap_or_default()
}

pub fn save(root: &Path, areas: &Areas) -> io::Result<()> {
    let file = path(root);
    fs::create_dir_all(root)?;
    let tmp = file.with_extension("json.part");
    fs::write(&tmp, areas.to_json())?;
    fs::rename(&tmp, &file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(south: f64, min: u8, max: u8) -> OfflineArea {
        OfflineArea { south, west: -22.05, north: south + 0.08, east: -21.85, min_zoom: min, max_zoom: max }
    }

    fn saved(name: &str, a: OfflineArea) -> SavedArea {
        SavedArea { name: name.into(), area: a, source_key: "tile.example.org".into(), tiles: a.tiles().len() }
    }

    fn root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("maps-areas-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_list_round_trips_through_the_file() {
        let r = root("roundtrip");
        let mut a = Areas::default();
        a.add(saved("Reykjavik", area(64.10, 10, 11)));
        save(&r, &a).unwrap();
        assert_eq!(load(&r), a);
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn a_damaged_file_is_an_empty_list() {
        let r = root("damaged");
        fs::create_dir_all(&r).unwrap();
        fs::write(path(&r), "not json").unwrap();
        assert!(load(&r).list.is_empty());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn the_same_area_is_listed_once() {
        let mut a = Areas::default();
        a.add(saved("First", area(64.10, 10, 11)));
        a.add(saved("Second", area(64.10, 10, 11)));
        assert_eq!(a.list.len(), 1);
        assert_eq!(a.list[0].name, "Second");
    }

    #[test]
    fn the_line_names_the_levels_and_tiles() {
        let s = SavedArea { name: "Home".into(), area: area(64.1, 11, 13), source_key: "k".into(), tiles: 42 };
        assert_eq!(s.line(), "Home, zoom 11 to 13, 42 tiles");
        let one = SavedArea { tiles: 1, area: area(64.1, 12, 12), ..s };
        assert_eq!(one.line(), "Home, zoom 12, 1 tile");
    }

    #[test]
    fn deleting_keeps_tiles_another_area_still_needs() {
        let mut a = Areas::default();
        a.add(saved("Low", area(64.10, 10, 10)));
        a.add(saved("High", area(64.10, 10, 11)));
        let low = a.remove(0).unwrap();
        assert!(a.orphans(&low).is_empty(), "the other area covers every tile of Low");
        let high = a.remove(0).unwrap();
        assert_eq!(a.orphans(&high).len(), high.area.tiles().len());
    }

    #[test]
    fn delete_removes_the_files() {
        let r = root("delete");
        let cache = TileCache::new(&r, "tile.example.org");
        let entry = saved("Home", area(64.10, 10, 10));
        for t in entry.area.tiles() {
            cache.write(t, b"x").unwrap();
        }
        let n = delete_tiles(&cache, &entry.area.tiles());
        assert_eq!(n, entry.area.tiles().len());
        assert!(cache.read(entry.area.tiles()[0]).is_none());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn a_bad_index_removes_nothing() {
        let mut a = Areas::default();
        assert!(a.remove(0).is_none());
    }
}

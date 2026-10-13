//! The person's places: Home and Work, Favorites, Recents and named Collections, saved as one JSON file in the app
//! data directory. Searched and pinned places are what the offline search runs over.

use crate::geocode::Place;
use bifrost_core::json::{self, Json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Most recent places kept.
pub const MAX_RECENTS: usize = 20;
/// Two places closer than this (degrees) are the same place.
const SAME_PLACE_DEG: f64 = 0.0002;

/// A saved place.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

impl Saved {
    pub fn new(name: &str, lat: f64, lon: f64) -> Saved {
        Saved { name: name.trim().to_string(), lat, lon }
    }

    pub fn title(&self) -> &str {
        self.name.split(',').next().unwrap_or(&self.name).trim()
    }

    fn same_as(&self, other: &Saved) -> bool {
        (self.lat - other.lat).abs() < SAME_PLACE_DEG && (self.lon - other.lon).abs() < SAME_PLACE_DEG
    }

    pub fn to_place(&self) -> Place {
        Place::new(&self.name, self.lat, self.lon)
    }

    fn to_json(&self) -> Json {
        Json::Obj(vec![("name".into(), Json::Str(self.name.clone())), ("lat".into(), Json::Num(self.lat)), ("lon".into(), Json::Num(self.lon))])
    }

    fn from_json(j: &Json) -> Option<Saved> {
        let lat = j.get("lat")?.number()?;
        let lon = j.get("lon")?.number()?;
        (lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0).then(|| Saved { name: j.str_field("name").unwrap_or_default(), lat, lon })
    }
}

/// A named list of places.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collection {
    pub name: String,
    pub places: Vec<Saved>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    pub home: Option<Saved>,
    pub work: Option<Saved>,
    pub favorites: Vec<Saved>,
    pub recents: Vec<Saved>,
    pub collections: Vec<Collection>,
}

impl Library {
    /// Adds a place to the front of the recents (a repeat moves to the front) and keeps `MAX_RECENTS`.
    pub fn add_recent(&mut self, place: Saved) {
        self.recents.retain(|p| !p.same_as(&place));
        self.recents.insert(0, place);
        self.recents.truncate(MAX_RECENTS);
    }

    pub fn clear_recents(&mut self) {
        self.recents.clear();
    }

    pub fn is_favorite(&self, lat: f64, lon: f64) -> bool {
        let probe = Saved::new("", lat, lon);
        self.favorites.iter().any(|p| p.same_as(&probe))
    }

    /// Adds the place to the favorites, or removes it when it is one already. True when it is a favorite afterwards.
    pub fn toggle_favorite(&mut self, place: Saved) -> bool {
        if self.is_favorite(place.lat, place.lon) {
            self.favorites.retain(|p| !p.same_as(&place));
            false
        } else {
            self.favorites.push(place);
            true
        }
    }

    pub fn set_home(&mut self, place: Option<Saved>) {
        self.home = place;
    }

    pub fn set_work(&mut self, place: Option<Saved>) {
        self.work = place;
    }

    /// A new empty collection. Blank and repeated names are refused; the index is returned.
    pub fn new_collection(&mut self, name: &str) -> Result<usize, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a collection needs a name".into());
        }
        if self.collections.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
            return Err(format!("there is a collection named {name} already"));
        }
        self.collections.push(Collection { name: name.to_string(), places: Vec::new() });
        Ok(self.collections.len() - 1)
    }

    pub fn remove_collection(&mut self, index: usize) -> Option<Collection> {
        (index < self.collections.len()).then(|| self.collections.remove(index))
    }

    /// Puts a place into a collection (once). False when the collection does not exist or has the place.
    pub fn add_to_collection(&mut self, index: usize, place: Saved) -> bool {
        let Some(c) = self.collections.get_mut(index) else { return false };
        if c.places.iter().any(|p| p.same_as(&place)) {
            return false;
        }
        c.places.push(place);
        true
    }

    pub fn remove_from_collection(&mut self, index: usize, place: usize) -> Option<Saved> {
        let c = self.collections.get_mut(index)?;
        (place < c.places.len()).then(|| c.places.remove(place))
    }

    /// Every saved place, for the offline search: Home, Work, favorites, collections, recents. Repeats are left out.
    pub fn all_places(&self) -> Vec<Place> {
        let mut seen: Vec<Saved> = Vec::new();
        let sources = self.home.iter().chain(self.work.iter()).chain(&self.favorites).chain(self.collections.iter().flat_map(|c| &c.places)).chain(&self.recents);
        for p in sources {
            if !seen.iter().any(|s| s.same_as(p)) {
                seen.push(p.clone());
            }
        }
        seen.iter().map(Saved::to_place).collect()
    }

    pub fn to_json(&self) -> String {
        let list = |v: &[Saved]| Json::Arr(v.iter().map(Saved::to_json).collect());
        let opt = |p: &Option<Saved>| p.as_ref().map_or(Json::Null, Saved::to_json);
        let collections = Json::Arr(
            self.collections
                .iter()
                .map(|c| Json::Obj(vec![("name".into(), Json::Str(c.name.clone())), ("places".into(), list(&c.places))]))
                .collect(),
        );
        Json::Obj(vec![
            ("home".into(), opt(&self.home)),
            ("work".into(), opt(&self.work)),
            ("favorites".into(), list(&self.favorites)),
            ("recents".into(), list(&self.recents)),
            ("collections".into(), collections),
        ])
        .encode()
    }

    /// Reads a saved library. Entries that cannot be read are skipped; text that is not JSON is an error.
    pub fn from_json(text: &str) -> Result<Library, String> {
        let doc = json::parse(text)?;
        let list = |key: &str| -> Vec<Saved> { doc.get(key).and_then(Json::as_array).map(|a| a.iter().filter_map(Saved::from_json).collect()).unwrap_or_default() };
        let collections = doc
            .get("collections")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        let name = c.str_field("name").filter(|n| !n.is_empty())?;
                        let places = c.get("places").and_then(Json::as_array).map(|p| p.iter().filter_map(Saved::from_json).collect()).unwrap_or_default();
                        Some(Collection { name, places })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Library {
            home: doc.get("home").and_then(Saved::from_json),
            work: doc.get("work").and_then(Saved::from_json),
            favorites: list("favorites"),
            recents: list("recents"),
            collections,
        })
    }
}

/// Where the library lives: `<root>/places.json`.
pub fn path(root: &Path) -> PathBuf {
    root.join("places.json")
}

/// Loads the library. A missing or unreadable file is an empty library, so a first run and a damaged file both start clean.
pub fn load(root: &Path) -> Library {
    fs::read_to_string(path(root)).ok().and_then(|t| Library::from_json(&t).ok()).unwrap_or_default()
}

/// Saves the library, writing a temporary file and renaming it into place.
pub fn save(root: &Path, library: &Library) -> io::Result<()> {
    let file = path(root);
    fs::create_dir_all(root)?;
    let tmp = file.with_extension("json.part");
    fs::write(&tmp, library.to_json())?;
    fs::rename(&tmp, &file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, lat: f64, lon: f64) -> Saved {
        Saved::new(name, lat, lon)
    }

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("maps-store-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn recents_keep_the_newest_first_without_repeats_and_are_capped() {
        let mut lib = Library::default();
        lib.add_recent(p("A", 1.0, 1.0));
        lib.add_recent(p("B", 2.0, 2.0));
        lib.add_recent(p("A again", 1.00001, 1.00001));
        assert_eq!(lib.recents.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["A again", "B"]);
        for i in 0..40 {
            lib.add_recent(p("x", 10.0 + i as f64, 5.0));
        }
        assert_eq!(lib.recents.len(), MAX_RECENTS);
        lib.clear_recents();
        assert!(lib.recents.is_empty());
    }

    #[test]
    fn a_favorite_toggles_on_and_off() {
        let mut lib = Library::default();
        assert!(lib.toggle_favorite(p("Cafe", 64.1, -21.9)));
        assert!(lib.is_favorite(64.1, -21.9));
        assert!(!lib.toggle_favorite(p("Cafe", 64.1, -21.9)));
        assert!(lib.favorites.is_empty());
    }

    #[test]
    fn collections_refuse_blank_and_repeated_names_and_repeated_places() {
        let mut lib = Library::default();
        assert!(lib.new_collection("  ").is_err());
        let i = lib.new_collection("Trip").unwrap();
        assert!(lib.new_collection("trip").is_err());
        assert!(lib.add_to_collection(i, p("Hotel", 1.0, 2.0)));
        assert!(!lib.add_to_collection(i, p("Hotel", 1.0, 2.0)));
        assert!(!lib.add_to_collection(9, p("Hotel", 1.0, 2.0)));
        assert_eq!(lib.remove_from_collection(i, 0).unwrap().name, "Hotel");
        assert!(lib.remove_from_collection(i, 0).is_none());
        assert_eq!(lib.remove_collection(i).unwrap().name, "Trip");
        assert!(lib.remove_collection(i).is_none());
    }

    #[test]
    fn the_library_round_trips_through_json_and_a_file() {
        let mut lib = Library::default();
        lib.set_home(Some(p("Home, Reykjavík", 64.1, -21.9)));
        lib.set_work(Some(p("Work", 64.2, -21.8)));
        lib.toggle_favorite(p("Cafe \"Paris\"", 64.147, -21.939));
        lib.add_recent(p("Akureyri", 65.68, -18.12));
        let i = lib.new_collection("Trip").unwrap();
        lib.add_to_collection(i, p("Hotel", 1.5, 2.5));
        let back = Library::from_json(&lib.to_json()).unwrap();
        assert_eq!(back, lib);

        let root = temp_root("roundtrip");
        assert_eq!(load(&root), Library::default(), "a missing file is an empty library");
        save(&root, &lib).unwrap();
        assert_eq!(load(&root), lib);
        fs::write(path(&root), "not json").unwrap();
        assert_eq!(load(&root), Library::default(), "a damaged file starts clean");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_entries_are_skipped_not_fatal() {
        let lib = Library::from_json(r#"{"favorites":[{"name":"ok","lat":1,"lon":2},{"name":"bad","lat":200,"lon":2},{"name":"none"}],"collections":[{"name":""},{"name":"C","places":[{"lat":3,"lon":4}]}]}"#).unwrap();
        assert_eq!(lib.favorites.len(), 1);
        assert_eq!(lib.collections.len(), 1);
        assert_eq!(lib.collections[0].places[0].name, "");
        assert!(Library::from_json("[").is_err());
    }

    #[test]
    fn offline_search_covers_everything_saved_once() {
        let mut lib = Library::default();
        lib.set_home(Some(p("Laugavegur 1, Reykjavík", 64.14, -21.93)));
        lib.add_recent(p("Laugavegur 1, Reykjavík", 64.14, -21.93));
        lib.toggle_favorite(p("Hverfisgata, Reykjavík", 64.15, -21.94));
        let all = lib.all_places();
        assert_eq!(all.len(), 2);
        assert_eq!(crate::route::search_offline(&all, "hverfis", 5).len(), 1);
    }
}

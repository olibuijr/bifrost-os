//! Photo libraries: the folders Photos can show. The ones used before are kept in the Photos preferences as
//! `lib.<n>` (newest first, one slot each, empty past the end) and the one in use as `lib.current`. Choosing a
//! library moves it to the front of the list; launching Photos opens the library in use when its folder still exists.

use crate::shell::prefs::Prefs;
use std::path::{Path, PathBuf};

/// Libraries remembered at once.
pub const MAX_LIBRARIES: usize = 8;
/// The preference holding the library in use.
pub const CURRENT: &str = "lib.current";

fn slot_key(n: usize) -> String {
    format!("lib.{n}")
}

/// The remembered libraries, newest first.
pub fn decode(prefs: &Prefs) -> Vec<PathBuf> {
    (0..MAX_LIBRARIES).filter_map(|n| prefs.get(&slot_key(n))).filter(|p| !p.is_empty()).map(PathBuf::from).collect()
}

/// The library in use, when one was chosen.
pub fn remembered(prefs: &Prefs) -> Option<PathBuf> {
    prefs.get(CURRENT).filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// Puts `path` first in `list`, drops its earlier entry and keeps at most [`MAX_LIBRARIES`].
pub fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX_LIBRARIES);
}

/// The preference writes that save `list` and the library in use. Every slot is written, so a shorter list
/// empties the slots it no longer uses.
pub fn writes(list: &[PathBuf], current: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = (0..MAX_LIBRARIES).map(|n| (slot_key(n), list.get(n).map_or_else(String::new, |p| p.display().to_string()))).collect();
    out.push((CURRENT.to_string(), current.display().to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(prefs: &mut Prefs, list: &[PathBuf], current: &Path) {
        let writes = writes(list, current);
        let pairs: Vec<(&str, &str)> = writes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        prefs.set_many(&pairs).expect("memory prefs save");
    }

    #[test]
    fn choosing_a_library_puts_it_first_and_keeps_each_once() {
        let mut list = Vec::new();
        remember(&mut list, Path::new("/home/a/Pictures"));
        remember(&mut list, Path::new("/mnt/card/Library"));
        remember(&mut list, Path::new("/home/a/Pictures"));
        assert_eq!(list, vec![PathBuf::from("/home/a/Pictures"), PathBuf::from("/mnt/card/Library")]);
    }

    #[test]
    fn the_list_is_capped_and_the_oldest_drops_out() {
        let mut list = Vec::new();
        for n in 0..(MAX_LIBRARIES + 3) {
            remember(&mut list, Path::new(&format!("/lib/{n}")));
        }
        assert_eq!(list.len(), MAX_LIBRARIES);
        assert_eq!(list[0], PathBuf::from(format!("/lib/{}", MAX_LIBRARIES + 2)));
        assert!(!list.contains(&PathBuf::from("/lib/0")));
    }

    #[test]
    fn the_list_and_the_library_in_use_survive_a_round_trip_through_prefs() {
        let mut prefs = Prefs::memory();
        let mut list = Vec::new();
        remember(&mut list, Path::new("/home/a/Pictures"));
        remember(&mut list, Path::new("/mnt/card/Library"));
        save(&mut prefs, &list, Path::new("/mnt/card/Library"));

        assert_eq!(decode(&prefs), list);
        assert_eq!(remembered(&prefs), Some(PathBuf::from("/mnt/card/Library")));
    }

    #[test]
    fn switching_back_keeps_the_known_libraries_and_empties_unused_slots() {
        let mut prefs = Prefs::memory();
        let mut list = Vec::new();
        remember(&mut list, Path::new("/a"));
        remember(&mut list, Path::new("/b"));
        save(&mut prefs, &list, Path::new("/b"));

        // switch back to the first library: the list is reordered, not grown
        remember(&mut list, Path::new("/a"));
        save(&mut prefs, &list, Path::new("/a"));
        assert_eq!(decode(&prefs), vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(remembered(&prefs), Some(PathBuf::from("/a")));

        // a shorter list clears the slot it no longer uses
        save(&mut prefs, &[PathBuf::from("/a")], Path::new("/a"));
        assert_eq!(decode(&prefs), vec![PathBuf::from("/a")]);
    }

    #[test]
    fn nothing_is_remembered_before_a_library_is_chosen() {
        let prefs = Prefs::memory();
        assert!(decode(&prefs).is_empty());
        assert_eq!(remembered(&prefs), None);
    }
}

//! Saves the contact display settings beside the contacts file, as `display.txt`:
//!
//! ```text
//! contacts-display 1
//! sort<TAB>default|family|given|none
//! name<TAB>first|last
//! ```
//!
//! A missing file means the defaults; a file without a `name` line shows first names first. Writes go to a temporary
//! file and then replace the old one.

use crate::names::NameOrder;
use bifrost_app::controls::contact_display::{ContactsDefaults, SortOrder};
use std::io;
use std::path::Path;

const HEADER: &str = "contacts-display 1";

/// Reads the display settings: the sort order and the name order. A missing file is the defaults.
pub fn load(path: &Path) -> io::Result<(ContactsDefaults, NameOrder)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((ContactsDefaults::new(), NameOrder::default())),
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(bad("Unknown display settings format"));
    }
    let mut defaults = ContactsDefaults::new();
    let mut name_order = NameOrder::default();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        match line.split('\t').collect::<Vec<_>>().as_slice() {
            ["sort", word] => defaults.set_sort_order(parse_sort(word).ok_or_else(|| bad("Unknown sort order"))?),
            ["name", word] => name_order = parse_name(word).ok_or_else(|| bad("Unknown name order"))?,
            _ => return Err(bad("Unknown line in the display settings")),
        }
    }
    Ok((defaults, name_order))
}

/// Writes the display settings.
pub fn save(path: &Path, defaults: &ContactsDefaults, name_order: NameOrder) -> io::Result<()> {
    let out = format!("{HEADER}\nsort\t{}\nname\t{}\n", sort_word(defaults.sort_order()), name_word(name_order));
    let temp = path.with_extension("txt.tmp");
    std::fs::write(&temp, out)?;
    std::fs::rename(&temp, path)
}

fn sort_word(order: SortOrder) -> &'static str {
    match order {
        SortOrder::UserDefault => "default",
        SortOrder::FamilyName => "family",
        SortOrder::GivenName => "given",
        SortOrder::None => "none",
    }
}

fn parse_sort(word: &str) -> Option<SortOrder> {
    Some(match word {
        "default" => SortOrder::UserDefault,
        "family" => SortOrder::FamilyName,
        "given" => SortOrder::GivenName,
        "none" => SortOrder::None,
        _ => return None,
    })
}

fn name_word(order: NameOrder) -> &'static str {
    match order {
        NameOrder::FirstLast => "first",
        NameOrder::LastFirst => "last",
    }
}

fn parse_name(word: &str) -> Option<NameOrder> {
    Some(match word {
        "first" => NameOrder::FirstLast,
        "last" => NameOrder::LastFirst,
        _ => return None,
    })
}

/// Reads the identifier of My Card from `mycard.txt` (one line, the identifier). A missing or unreadable file means
/// no card is marked.
pub fn load_me(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Writes My Card's identifier, or removes the file for `None`.
pub fn save_me(path: &Path, id: Option<u64>) -> io::Result<()> {
    match id {
        Some(id) => {
            let temp = path.with_extension("txt.tmp");
            std::fs::write(&temp, format!("{id}\n"))?;
            std::fs::rename(&temp, path)
        }
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

fn bad(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("contacts-display-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_a_choice_round_trips() {
        let path = dir("round").join("display.txt");
        let (defaults, names) = load(&path).unwrap();
        assert_eq!(defaults.sort_order(), SortOrder::UserDefault);
        assert_eq!(names, NameOrder::FirstLast);
        let mut defaults = ContactsDefaults::new();
        defaults.set_sort_order(SortOrder::GivenName);
        save(&path, &defaults, NameOrder::LastFirst).unwrap();
        let (loaded, names) = load(&path).unwrap();
        assert_eq!(loaded.sort_order(), SortOrder::GivenName);
        assert_eq!(names, NameOrder::LastFirst);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_file_from_before_the_name_order_shows_first_names_first() {
        let path = dir("old").join("display.txt");
        std::fs::write(&path, format!("{HEADER}\nsort\tfamily\n")).unwrap();
        let (defaults, names) = load(&path).unwrap();
        assert_eq!(defaults.sort_order(), SortOrder::FamilyName);
        assert_eq!(names, NameOrder::FirstLast);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn my_card_round_trips_and_clears() {
        let path = dir("me").join("mycard.txt");
        assert_eq!(load_me(&path), None);
        save_me(&path, Some(7)).unwrap();
        assert_eq!(load_me(&path), Some(7));
        save_me(&path, None).unwrap();
        assert_eq!(load_me(&path), None);
        save_me(&path, None).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_foreign_or_unknown_file_is_refused() {
        let path = dir("bad").join("display.txt");
        std::fs::write(&path, "nope\n").unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\nsort\tsideways\n")).unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\nname\tmiddle\n")).unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

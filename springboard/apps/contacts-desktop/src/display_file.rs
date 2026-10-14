//! Saves the contact display settings and the other preferences beside the contacts file, as `display.txt`:
//!
//! ```text
//! contacts-display 1
//! sort<TAB>default|family|given|none
//! name<TAB>first|last
//! me<TAB>contact identifier
//! notes<TAB>0|1
//! nicknames<TAB>0|1
//! template<TAB>card field code
//! import<TAB>contact identifier
//! ```
//!
//! A missing file means the defaults; a file without a `name` line shows first names first. `me` is the contact that is
//! My Card, `notes` is whether vCard export writes the notes (on by default), `nicknames` is whether a nickname is shown
//! in place of the name, each `template` line is a field new cards show, and each `import` line is a contact of the
//! last import (the Last Import list). Writes go to a temporary file and then replace the old one.

use crate::details::Detail;
use crate::names::NameOrder;
use bifrost_app::controls::contact_display::{ContactsDefaults, SortOrder};
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

const HEADER: &str = "contacts-display 1";

/// The preferences besides the sort and name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// The contact that is My Card.
    pub me: Option<u64>,
    /// Whether vCard export writes the notes of each card.
    pub export_notes: bool,
    /// Whether a contact with a nickname is shown by its nickname.
    pub prefer_nicknames: bool,
    /// The card fields a new card shows in its form.
    pub template: BTreeSet<Detail>,
    /// The contacts the last import added, for the Last Import list.
    pub last_import: BTreeSet<u64>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { me: None, export_notes: true, prefer_nicknames: false, template: BTreeSet::new(), last_import: BTreeSet::new() }
    }
}

/// Reads the display settings: the sort order, the name order and the preferences. A missing file is the defaults.
pub fn load(path: &Path) -> io::Result<(ContactsDefaults, NameOrder, Prefs)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((ContactsDefaults::new(), NameOrder::default(), Prefs::default())),
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(bad("Unknown display settings format"));
    }
    let mut defaults = ContactsDefaults::new();
    let mut name_order = NameOrder::default();
    let mut prefs = Prefs::default();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        match line.split('\t').collect::<Vec<_>>().as_slice() {
            ["sort", word] => defaults.set_sort_order(parse_sort(word).ok_or_else(|| bad("Unknown sort order"))?),
            ["name", word] => name_order = parse_name(word).ok_or_else(|| bad("Unknown name order"))?,
            ["me", id] => prefs.me = Some(id.parse().map_err(|_| bad("Unknown My Card"))?),
            ["notes", flag] => prefs.export_notes = parse_flag(flag)?,
            ["nicknames", flag] => prefs.prefer_nicknames = parse_flag(flag)?,
            ["template", code] => {
                prefs.template.insert(Detail::from_code(code).ok_or_else(|| bad("Unknown card template field"))?);
            }
            ["import", id] => {
                prefs.last_import.insert(id.parse().map_err(|_| bad("Unknown contact in the last import"))?);
            }
            _ => return Err(bad("Unknown line in the display settings")),
        }
    }
    Ok((defaults, name_order, prefs))
}

/// Writes the display settings and the preferences.
pub fn save(path: &Path, defaults: &ContactsDefaults, name_order: NameOrder, prefs: &Prefs) -> io::Result<()> {
    let mut out = format!("{HEADER}\nsort\t{}\nname\t{}\n", sort_word(defaults.sort_order()), name_word(name_order));
    if let Some(me) = prefs.me {
        out.push_str(&format!("me\t{me}\n"));
    }
    out.push_str(&format!("notes\t{}\nnicknames\t{}\n", flag_word(prefs.export_notes), flag_word(prefs.prefer_nicknames)));
    for detail in &prefs.template {
        out.push_str(&format!("template\t{}\n", detail.code()));
    }
    for id in &prefs.last_import {
        out.push_str(&format!("import\t{id}\n"));
    }
    let temp = path.with_extension("txt.tmp");
    std::fs::write(&temp, out)?;
    std::fs::rename(&temp, path)
}

fn flag_word(on: bool) -> &'static str {
    if on {
        "1"
    } else {
        "0"
    }
}

fn parse_flag(word: &str) -> io::Result<bool> {
    match word {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(bad("Unknown on/off value in the display settings")),
    }
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
        let (defaults, names, prefs) = load(&path).unwrap();
        assert_eq!(defaults.sort_order(), SortOrder::UserDefault);
        assert_eq!(names, NameOrder::FirstLast);
        assert_eq!(prefs, Prefs::default());
        assert!(prefs.export_notes, "notes are exported unless the person turns that off");
        let mut defaults = ContactsDefaults::new();
        defaults.set_sort_order(SortOrder::GivenName);
        save(&path, &defaults, NameOrder::LastFirst, &Prefs::default()).unwrap();
        let (loaded, names, prefs) = load(&path).unwrap();
        assert_eq!(loaded.sort_order(), SortOrder::GivenName);
        assert_eq!(names, NameOrder::LastFirst);
        assert_eq!(prefs, Prefs::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn my_card_and_the_preferences_round_trip() {
        let path = dir("prefs").join("display.txt");
        let prefs = Prefs {
            me: Some(7),
            export_notes: false,
            prefer_nicknames: true,
            template: BTreeSet::from([Detail::Birthday, Detail::Nickname]),
            last_import: BTreeSet::from([3, 9]),
        };
        save(&path, &ContactsDefaults::new(), NameOrder::FirstLast, &prefs).unwrap();
        let (_, _, loaded) = load(&path).unwrap();
        assert_eq!(loaded, prefs);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_file_from_before_the_name_order_shows_first_names_first() {
        let path = dir("old").join("display.txt");
        std::fs::write(&path, format!("{HEADER}\nsort\tfamily\n")).unwrap();
        let (defaults, names, prefs) = load(&path).unwrap();
        assert_eq!(defaults.sort_order(), SortOrder::FamilyName);
        assert_eq!(names, NameOrder::FirstLast);
        assert_eq!(prefs, Prefs::default());
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
        std::fs::write(&path, format!("{HEADER}\nme\tnobody\n")).unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\nnotes\tmaybe\n")).unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\ntemplate\tshoe-size\n")).unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

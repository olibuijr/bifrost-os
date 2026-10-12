//! Saves the contact display settings beside the contacts file, as `display.txt`:
//!
//! ```text
//! contacts-display 1
//! sort<TAB>default|family|given|none
//! ```
//!
//! A missing file means the defaults. Writes go to a temporary file and then replace the old one.

use bifrost_app::controls::contact_display::{ContactsDefaults, SortOrder};
use std::io;
use std::path::Path;

const HEADER: &str = "contacts-display 1";

/// Reads the display settings. A missing file is the defaults.
pub fn load(path: &Path) -> io::Result<ContactsDefaults> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ContactsDefaults::new()),
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(bad("Unknown display settings format"));
    }
    let mut defaults = ContactsDefaults::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        match line.split('\t').collect::<Vec<_>>().as_slice() {
            ["sort", word] => defaults.set_sort_order(parse_sort(word).ok_or_else(|| bad("Unknown sort order"))?),
            _ => return Err(bad("Unknown line in the display settings")),
        }
    }
    Ok(defaults)
}

/// Writes the display settings.
pub fn save(path: &Path, defaults: &ContactsDefaults) -> io::Result<()> {
    let out = format!("{HEADER}\nsort\t{}\n", sort_word(defaults.sort_order()));
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
        assert_eq!(load(&path).unwrap().sort_order(), SortOrder::UserDefault);
        let mut defaults = ContactsDefaults::new();
        defaults.set_sort_order(SortOrder::GivenName);
        save(&path, &defaults).unwrap();
        assert_eq!(load(&path).unwrap().sort_order(), SortOrder::GivenName);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_foreign_or_unknown_file_is_refused() {
        let path = dir("bad").join("display.txt");
        std::fs::write(&path, "nope\n").unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\nsort\tsideways\n")).unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

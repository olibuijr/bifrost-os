//! A tiny `key=value` settings store, persisted as a text file
//! (`/var/lib/springboard/settings.ini`). Writes go through a temporary
//! file and a rename so a power cut never leaves a half-written file.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Prefs {
    path: Option<PathBuf>,
    values: BTreeMap<String, String>,
}

impl Prefs {
    /// Loads the file at `path` (a missing or unreadable file is empty).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let values = std::fs::read_to_string(&path).map(|s| Self::parse(&s)).unwrap_or_default();
        Prefs { path: Some(path), values }
    }

    /// An in-memory store that never touches the disk.
    pub fn memory() -> Self {
        Prefs::default()
    }

    fn parse(text: &str) -> BTreeMap<String, String> {
        text.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with('#') {
                    return None;
                }
                let (k, v) = l.split_once('=')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// All pairs, sorted by key.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// A boolean pref (stored as `1`/`0`); `default` when unset.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).map_or(default, |v| v == "1")
    }

    /// A pref parsed as `T`; `None` when unset or malformed.
    pub fn get_parsed<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// Sets a value and saves (skipped when it is unchanged); keys and values are single-line text.
    pub fn set(&mut self, key: &str, value: &str) -> io::Result<()> {
        self.set_many(&[(key, value)])
    }

    /// Sets several values with a single save; nothing is written when none changed.
    pub fn set_many(&mut self, pairs: &[(&str, &str)]) -> io::Result<()> {
        let clean = |s: &str| s.replace(['\n', '\r'], " ").trim().to_string();
        let mut changed = false;
        for (k, v) in pairs {
            let (k, v) = (clean(k), clean(v));
            if self.values.get(&k) != Some(&v) {
                self.values.insert(k, v);
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    /// Forgets every value and saves (Reset All Settings).
    pub fn clear(&mut self) -> io::Result<()> {
        self.values.clear();
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text: String = self.values.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        write_atomic(path, text.as_bytes())
    }
}

pub use super::fsutil::write_atomic;

/// Escapes `\`, tab, newline and carriage return so `text` fits one tab-separated field.
pub fn escape_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_field`]; `None` on an unknown or dangling escape, or when the
/// decoded text is longer than `max` characters.
pub fn unescape_field(text: &str, max: usize) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    let mut count = 0;
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            match chars.next()? {
                '\\' => '\\',
                't' => '\t',
                'n' => '\n',
                'r' => '\r',
                _ => return None,
            }
        } else {
            c
        });
        count += 1;
        if count > max {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-prefs")).join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn parses_lines_ignoring_comments_and_blanks() {
        let v = Prefs::parse("# comment\n\nwallpaper = lake_day \nbad line\nkey=a=b\n");
        assert_eq!(v.get("wallpaper").map(String::as_str), Some("lake_day"));
        assert_eq!(v.get("key").map(String::as_str), Some("a=b"));
        assert_eq!(v.len(), 2);
        let p = Prefs { path: None, values: v };
        assert_eq!(p.entries().collect::<Vec<_>>(), vec![("key", "a=b"), ("wallpaper", "lake_day")]);
    }

    #[test]
    fn set_persists_and_load_reads_back() {
        let path = scratch("settings.ini");
        let _ = std::fs::remove_file(&path);
        let mut p = Prefs::load(&path);
        assert_eq!(p.get("wallpaper"), None);
        p.set("wallpaper", "desert_night").unwrap();
        p.set("brightness", "0.5").unwrap();
        p.set("wallpaper", "lake\nday").unwrap();
        let again = Prefs::load(&path);
        assert_eq!(again.get("wallpaper"), Some("lake day"));
        assert_eq!(again.get("brightness"), Some("0.5"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "brightness=0.5\nwallpaper=lake day\n");
    }

    #[test]
    fn unchanged_sets_skip_the_write_and_set_many_saves_once() {
        let path = scratch("many.ini");
        let _ = std::fs::remove_file(&path);
        let mut p = Prefs::load(&path);
        p.set_many(&[("a", "1"), ("b", "1")]).unwrap();
        std::fs::remove_file(&path).unwrap();
        p.set("a", "1").unwrap();
        p.set_many(&[("a", "1"), ("b", "1")]).unwrap();
        assert!(!path.exists(), "equal values do not write");
        p.set_many(&[("a", "2"), ("b", "1")]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a=2\nb=1\n");
        assert!(p.get_bool("b", false) && !p.get_bool("zz", false) && p.get_bool("zz", true));
        assert_eq!(p.get_parsed::<u32>("a"), Some(2));
        assert_eq!(p.get_parsed::<u32>("b"), Some(1));
        assert_eq!(p.get_parsed::<u32>("zz"), None);
    }

    #[test]
    fn field_escaping_round_trips() {
        for text in ["", "plain", "a\tb\nc\rd", "back\\slash", "\\n is not a newline", "trailing\\", "ünï\tcode"] {
            assert_eq!(unescape_field(&escape_field(text), usize::MAX).as_deref(), Some(text), "{text:?}");
            assert!(!escape_field(text).contains(['\t', '\n', '\r']));
        }
        assert_eq!(escape_field("a\tb"), "a\\tb");
    }

    #[test]
    fn field_unescaping_rejects_bad_escapes_and_long_text() {
        assert_eq!(unescape_field("bad\\x", 10), None);
        assert_eq!(unescape_field("dangling\\", 10), None);
        assert_eq!(unescape_field("abc", 3).as_deref(), Some("abc"));
        assert_eq!(unescape_field("abcd", 3), None);
        assert_eq!(unescape_field("a\\nb", 3).as_deref(), Some("a\nb"));
    }

    #[test]
    fn memory_store_never_writes() {
        let mut p = Prefs::memory();
        p.set("k", "v").unwrap();
        assert_eq!(p.get("k"), Some("v"));
        assert_eq!(p.path, None);
        p.clear().unwrap();
        assert_eq!(p.entries().count(), 0);
    }

    #[test]
    fn clear_empties_the_file() {
        let path = scratch("cleared.ini");
        let mut p = Prefs::load(&path);
        p.set("a", "1").unwrap();
        p.clear().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert_eq!(Prefs::load(&path).get("a"), None);
    }

    #[test]
    fn missing_file_is_empty_and_created_on_first_set() {
        let path = scratch("nested/dir/settings.ini");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let mut p = Prefs::load(&path);
        assert_eq!(p.get("x"), None);
        p.set("x", "1").unwrap();
        assert!(path.exists());
    }
}

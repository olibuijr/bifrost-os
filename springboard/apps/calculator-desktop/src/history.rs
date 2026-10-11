//! The calculation history and the calculator's few settings, persisted
//! as small text files (written through a temporary file and a rename).

use super::eval::Angle;
use super::format::Locale;
use springboard::shell::prefs::write_atomic;
use std::path::{Path, PathBuf};

pub const MAX_ENTRIES: usize = 50;
pub const HISTORY_FILE: &str = "history.tsv";
pub const SETTINGS_FILE: &str = "settings.txt";

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub expression: String,
    /// Canonical result text (ASCII, `.` decimal).
    pub result: String,
    pub value: f64,
}

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c })
        .collect()
}

fn encode(entry: &Entry) -> String {
    format!(
        "{:016x}\t{}\t{}",
        entry.value.to_bits(),
        clean(&entry.result),
        clean(&entry.expression)
    )
}

fn decode(line: &str) -> Option<Entry> {
    let mut parts = line.splitn(3, '\t');
    let bits = u64::from_str_radix(parts.next()?, 16).ok()?;
    let value = f64::from_bits(bits);
    let result = parts.next()?.to_string();
    let expression = parts.next()?.to_string();
    (value.is_finite() && !result.is_empty()).then_some(Entry { expression, result, value })
}

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Entry>,
    path: Option<PathBuf>,
}

impl History {
    /// A history that never touches the disk.
    pub fn memory() -> Self {
        Self::default()
    }

    /// Loads `path` (empty if missing or damaged lines) and saves there from now on.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut entries: Vec<Entry> = std::fs::read_to_string(&path)
            .map(|text| text.lines().filter_map(decode).collect())
            .unwrap_or_default();
        entries.truncate(MAX_ENTRIES);
        Self { entries, path: Some(path) }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// Adds `entry` as the newest one, dropping the oldest past the limit.
    pub fn push(&mut self, entry: Entry) {
        self.entries.insert(0, entry);
        self.entries.truncate(MAX_ENTRIES);
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let mut text = self.entries.iter().map(encode).collect::<Vec<_>>().join("\n");
        text.push('\n');
        if let Err(e) = write_atomic(path, &text) {
            eprintln!("springboard: calculator history: {e}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Settings {
    pub locale: Locale,
    /// Scientific keypad forced on in portrait too.
    pub scientific: bool,
    pub angle: Angle,
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
                Some(("locale", v)) => s.locale = Locale::from_code(v),
                Some(("scientific", v)) => s.scientific = v == "1",
                Some(("angle", v)) => s.angle = if v == "deg" { Angle::Deg } else { Angle::Rad },
                _ => {}
            }
        }
        s
    }

    pub fn encode(&self) -> String {
        format!(
            "locale={}\nscientific={}\nangle={}\n",
            self.locale.code(),
            self.scientific as u8,
            if self.angle == Angle::Deg { "deg" } else { "rad" }
        )
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Err(e) = write_atomic(path, &self.encode()) {
            eprintln!("springboard: calculator settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u32) -> Entry {
        Entry {
            expression: format!("{n}+1"),
            result: (n + 1).to_string(),
            value: (n + 1) as f64,
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("calc-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn newest_first_and_bounded() {
        let mut h = History::memory();
        for n in 0..(MAX_ENTRIES as u32 + 5) {
            h.push(entry(n));
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        assert_eq!(h.get(0).unwrap().result, (MAX_ENTRIES + 5).to_string());
        assert!(h.get(MAX_ENTRIES).is_none());
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn persists_across_loads_and_clear_persists_too() {
        let d = dir("persist");
        let path = d.join(HISTORY_FILE);
        {
            let mut h = History::load(&path);
            assert!(h.is_empty());
            h.push(entry(1));
            h.push(Entry { expression: "a\tb\nc".into(), result: "0.1".into(), value: 0.1 });
        }
        let h = History::load(&path);
        assert_eq!(h.len(), 2);
        assert_eq!(h.get(0).unwrap().expression, "a b c");
        assert_eq!(h.get(0).unwrap().value, 0.1);
        assert_eq!(h.get(1), Some(&entry(1)));
        let mut h = h;
        h.clear();
        assert!(History::load(&path).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let d = dir("damaged");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(HISTORY_FILE);
        let good = encode(&entry(4));
        std::fs::write(&path, format!("garbage\n\n{good}\nzz\t1\t2\n7ff8000000000000\t1\tnan\n")).unwrap();
        let h = History::load(&path);
        assert_eq!(h.entries(), &[entry(4)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn settings_round_trip_with_defaults_for_unknowns() {
        let s = Settings { locale: Locale::Is, scientific: true, angle: Angle::Deg };
        assert_eq!(Settings::parse(&s.encode()), s);
        assert_eq!(Settings::parse("junk\nlocale=zz\n"), Settings::default());
        let d = dir("settings");
        let path = d.join(SETTINGS_FILE);
        assert_eq!(Settings::load(&path), Settings::default());
        s.save(&path);
        assert_eq!(Settings::load(&path), s);
        let _ = std::fs::remove_dir_all(&d);
    }
}

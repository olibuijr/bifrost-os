//! The calculation history and the calculator's few settings, persisted
//! as small text files (written through a temporary file and a rename).

use super::eval::Angle;
use super::format::{Locale, Style, MAX_PLACES};
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

/// Seconds since the Unix epoch, now (0 when the clock is before it).
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One history line: `bits[:time]`, the result and the expression, tab separated. Lines written before the time was
/// kept have no `:time` and read as time 0 (day unknown).
fn encode(entry: &Entry, time: u64) -> String {
    format!(
        "{:016x}:{}\t{}\t{}",
        entry.value.to_bits(),
        time,
        clean(&entry.result),
        clean(&entry.expression)
    )
}

fn decode(line: &str) -> Option<(Entry, u64)> {
    let mut parts = line.splitn(3, '\t');
    let head = parts.next()?;
    let (bits, time) = match head.split_once(':') {
        Some((bits, time)) => (bits, time.parse().ok()?),
        None => (head, 0),
    };
    let bits = u64::from_str_radix(bits, 16).ok()?;
    let value = f64::from_bits(bits);
    let result = parts.next()?.to_string();
    let expression = parts.next()?.to_string();
    (value.is_finite() && !result.is_empty()).then_some((Entry { expression, result, value }, time))
}

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Entry>,
    /// When each entry was made (`now_secs`), parallel to `entries`; 0 when unknown.
    times: Vec<u64>,
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
        let mut rows: Vec<(Entry, u64)> = std::fs::read_to_string(&path)
            .map(|text| text.lines().filter_map(decode).collect())
            .unwrap_or_default();
        rows.truncate(MAX_ENTRIES);
        let (entries, times) = rows.into_iter().unzip();
        Self { entries, times, path: Some(path) }
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

    /// When entry `index` was made, in seconds since the epoch (0 when unknown).
    pub fn time(&self, index: usize) -> u64 {
        self.times.get(index).copied().unwrap_or(0)
    }

    /// Adds `entry` as the newest one, dropping the oldest past the limit.
    pub fn push(&mut self, entry: Entry) {
        self.push_at(entry, now_secs());
    }

    /// `push` with the time given (tests, imports).
    pub fn push_at(&mut self, entry: Entry, time: u64) {
        self.entries.insert(0, entry);
        self.times.insert(0, time);
        self.entries.truncate(MAX_ENTRIES);
        self.times.truncate(MAX_ENTRIES);
        self.save();
    }

    /// Deletes entry `index`; false when there is none.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.entries.remove(index);
        self.times.remove(index);
        self.save();
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.times.clear();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let mut text = self.entries.iter().zip(&self.times).map(|(e, t)| encode(e, *t)).collect::<Vec<_>>().join("\n");
        text.push('\n');
        if let Err(e) = write_atomic(path, &text) {
            eprintln!("springboard: calculator history: {e}");
        }
    }
}

/// The calendar date (year, month, day) of a day count since 1970-01-01 (proleptic Gregorian, UTC).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The caption of the day group an entry made at `time` belongs to, seen from `now`: "Today", "Yesterday" or the
/// date as `YYYY-MM-DD`; empty when the time is unknown.
pub fn day_caption(time: u64, now: u64) -> String {
    if time == 0 {
        return String::new();
    }
    let (day, today) = ((time / 86_400) as i64, (now / 86_400) as i64);
    match today - day {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            let (y, m, d) = civil_from_days(day);
            format!("{y:04}-{m:02}-{d:02}")
        }
    }
}

/// Whether entry `index` is the first of its day (the newest entry always is): it starts a new group.
pub fn starts_day(times: impl Fn(usize) -> u64, index: usize) -> bool {
    index == 0 || times(index) / 86_400 != times(index - 1) / 86_400
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub locale: Locale,
    /// Scientific keypad forced on in portrait too.
    pub scientific: bool,
    pub angle: Angle,
    /// View > Thousands Separator.
    pub thousands: bool,
    /// View > Decimal Places: fraction digits of a result, `None` for automatic.
    pub places: Option<u8>,
    /// View > Scientific Notation: results as a mantissa and an exponent.
    pub sci_notation: bool,
    /// View > RPN Mode: the basic keypad drives a stack.
    pub rpn: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            locale: Locale::default(),
            scientific: false,
            angle: Angle::default(),
            thousands: true,
            places: None,
            sci_notation: false,
            rpn: false,
        }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
                Some(("locale", v)) => s.locale = Locale::from_code(v),
                Some(("scientific", v)) => s.scientific = v == "1",
                Some(("angle", v)) => s.angle = if v == "deg" { Angle::Deg } else { Angle::Rad },
                Some(("thousands", v)) => s.thousands = v != "0",
                Some(("places", v)) => s.places = v.parse::<u8>().ok().filter(|p| *p <= MAX_PLACES),
                Some(("sci_notation", v)) => s.sci_notation = v == "1",
                Some(("rpn", v)) => s.rpn = v == "1",
                _ => {}
            }
        }
        s
    }

    pub fn encode(&self) -> String {
        format!(
            "locale={}\nscientific={}\nangle={}\nthousands={}\nplaces={}\nsci_notation={}\nrpn={}\n",
            self.locale.code(),
            self.scientific as u8,
            if self.angle == Angle::Deg { "deg" } else { "rad" },
            self.thousands as u8,
            self.places.map_or_else(|| "auto".to_string(), |p| p.to_string()),
            self.sci_notation as u8,
            self.rpn as u8
        )
    }

    /// How results are shown under these settings.
    pub fn style(&self) -> Style {
        Style { thousands: self.thousands, places: self.places, scientific: self.sci_notation }
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
    fn entries_keep_their_time_and_old_lines_read_as_unknown() {
        let d = dir("times");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(HISTORY_FILE);
        let old_line = format!("{:016x}\t5\t2+3", 5f64.to_bits());
        std::fs::write(&path, format!("{old_line}\n")).unwrap();
        let mut h = History::load(&path);
        assert_eq!(h.time(0), 0);
        h.push_at(entry(1), 1_700_000_000);
        let h = History::load(&path);
        assert_eq!(h.len(), 2);
        assert_eq!(h.time(0), 1_700_000_000);
        assert_eq!(h.time(1), 0);
        assert_eq!(h.time(9), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn one_entry_can_be_deleted_and_the_rest_keep_their_times() {
        let d = dir("remove");
        let path = d.join(HISTORY_FILE);
        let mut h = History::load(&path);
        for n in 0..3 {
            h.push_at(entry(n), 100 + u64::from(n));
        }
        assert!(h.remove(1));
        assert!(!h.remove(5));
        assert_eq!(h.len(), 2);
        assert_eq!((h.get(0).unwrap().result.as_str(), h.time(0)), ("3", 102));
        assert_eq!((h.get(1).unwrap().result.as_str(), h.time(1)), ("1", 100));
        let h = History::load(&path);
        assert_eq!(h.len(), 2);
        assert_eq!(h.time(1), 100);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn entries_group_by_day_with_captions() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let now = 1_700_000_000; // 2023-11-14
        assert_eq!(day_caption(now - 10, now), "Today");
        assert_eq!(day_caption(now - 86_400, now), "Yesterday");
        assert_eq!(day_caption(now - 5 * 86_400, now), "2023-11-09");
        assert_eq!(day_caption(0, now), "");
        let times = [now, now - 60, now - 90_000, now - 90_100];
        let starts: Vec<bool> = (0..4).map(|i| starts_day(|k| times[k], i)).collect();
        assert_eq!(starts, vec![true, false, true, false]);
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let d = dir("damaged");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(HISTORY_FILE);
        let good = encode(&entry(4), 0);
        std::fs::write(&path, format!("garbage\n\n{good}\nzz\t1\t2\n7ff8000000000000\t1\tnan\n")).unwrap();
        let h = History::load(&path);
        assert_eq!(h.entries(), &[entry(4)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn settings_round_trip_with_defaults_for_unknowns() {
        let s = Settings { locale: Locale::Is, scientific: true, angle: Angle::Deg, ..Settings::default() };
        assert_eq!(Settings::parse(&s.encode()), s);
        let view = Settings { thousands: false, places: Some(4), sci_notation: true, rpn: true, ..Settings::default() };
        assert_eq!(Settings::parse(&view.encode()), view);
        assert_eq!(view.style(), Style { thousands: false, places: Some(4), scientific: true });
        assert_eq!(Settings::parse("places=99\nthousands=1\n").places, None, "places beyond 15 are ignored");
        assert_eq!(Settings::parse("places=auto\n").places, None);
        assert!(Settings::default().thousands, "grouping is on by default");
        assert_eq!(Settings::parse("junk\nlocale=zz\n"), Settings::default());
        let d = dir("settings");
        let path = d.join(SETTINGS_FILE);
        assert_eq!(Settings::load(&path), Settings::default());
        s.save(&path);
        assert_eq!(Settings::load(&path), s);
        let _ = std::fs::remove_dir_all(&d);
    }
}

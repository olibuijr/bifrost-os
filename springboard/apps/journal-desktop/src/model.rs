//! The journal as plain data: dated entries with text and photo file names, the timeline order, search,
//! bookmarks and the JSON format of the app's file. No drawing and no file access here.

use bifrost_core::json::{self, Json};

/// Version written into the journal file.
pub const FORMAT_VERSION: f64 = 1.0;

/// Image types the journal accepts as photos (lower case, without the dot).
pub const PHOTO_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "webp", "heic"];

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

/// A calendar day in the proleptic Gregorian calendar. Days are kept in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

impl Day {
    /// The day, or `None` when the month or day does not exist.
    pub fn new(year: i64, month: u32, day: u32) -> Option<Day> {
        if (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day) {
            Some(Day { year, month, day })
        } else {
            None
        }
    }

    /// Reads `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Option<Day> {
        let mut parts = text.trim().split('-');
        let year = parts.next()?.parse::<i64>().ok()?;
        let month = parts.next()?.parse::<u32>().ok()?;
        let day = parts.next()?.parse::<u32>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Day::new(year, month, day)
    }

    /// `YYYY-MM-DD`, the form the file and the search use.
    pub fn format(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The UTC day of a Unix time in seconds.
    pub fn from_unix(secs: u64) -> Day {
        let (year, month, day) = civil_from_days((secs / 86_400) as i64);
        Day { year, month, day }
    }

    /// Days since 1970-01-01.
    pub fn to_days(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Name of the day of the week, for example "Monday".
    pub fn weekday(&self) -> &'static str {
        // 1970-01-01 was a Thursday (index 4 with Sunday as 0)
        WEEKDAYS[(self.to_days() + 4).rem_euclid(7) as usize]
    }

    /// The heading of a timeline row, for example "Monday 13 October 2026".
    pub fn long(&self) -> String {
        format!("{} {} {} {}", self.weekday(), self.day, MONTHS[(self.month - 1) as usize], self.year)
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Day count since 1970-01-01 of a civil date (inverse of `civil_from_days`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (if month > 2 { month - 3 } else { month + 9 }) as i64;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One dated entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub day: Day,
    /// Unix seconds when the entry was made and last changed.
    pub created: u64,
    pub modified: u64,
    pub text: String,
    /// File names of the photos in the app's photo directory, in the order they were added.
    pub photos: Vec<String>,
    pub bookmarked: bool,
}

/// The whole journal and the view state that is not saved (selection, search and the bookmarks filter).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Journal {
    pub entries: Vec<Entry>,
    /// The largest entry id handed out so far.
    pub next_id: u64,
    pub selected: Option<u64>,
    pub query: String,
    pub bookmarks_only: bool,
}

impl Journal {
    /// Adds an empty entry for `day`, selects it and returns its id.
    pub fn new_entry(&mut self, day: Day, now: u64) -> u64 {
        let largest = self.entries.iter().map(|e| e.id).max().unwrap_or(0);
        let id = self.next_id.max(largest) + 1;
        self.next_id = id;
        self.entries.push(Entry {
            id,
            day,
            created: now,
            modified: now,
            text: String::new(),
            photos: Vec::new(),
            bookmarked: false,
        });
        self.selected = Some(id);
        id
    }

    pub fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// Selects an entry that exists, or clears the selection with `None`. Returns false for an unknown id.
    pub fn select(&mut self, id: Option<u64>) -> bool {
        match id {
            Some(id) if self.entry(id).is_none() => false,
            _ => {
                self.selected = id;
                true
            }
        }
    }

    /// Replaces the text of an entry. Returns false when the entry does not exist.
    pub fn set_text(&mut self, id: u64, text: &str, now: u64) -> bool {
        match self.entry_mut(id) {
            Some(e) => {
                if e.text != text {
                    e.text = text.to_string();
                    e.modified = now;
                }
                true
            }
            None => false,
        }
    }

    /// Flips the bookmark of an entry and returns its new state.
    pub fn toggle_bookmark(&mut self, id: u64) -> Option<bool> {
        let e = self.entry_mut(id)?;
        e.bookmarked = !e.bookmarked;
        Some(e.bookmarked)
    }

    /// Removes an entry and returns it. When it was selected, the selection moves to the first entry of the
    /// timeline (or to nothing).
    pub fn remove(&mut self, id: u64) -> Option<Entry> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        let removed = self.entries.remove(at);
        if self.selected == Some(id) {
            self.selected = self.timeline().first().copied();
        }
        Some(removed)
    }

    /// Records a photo file name on an entry. Refused for an unknown entry, a name that is not a photo file
    /// name, or a name the entry already has.
    pub fn add_photo(&mut self, id: u64, name: &str, now: u64) -> bool {
        if !is_photo_name(name) {
            return false;
        }
        match self.entry_mut(id) {
            Some(e) if !e.photos.iter().any(|p| p == name) => {
                e.photos.push(name.to_string());
                e.modified = now;
                true
            }
            _ => false,
        }
    }

    /// A new photo file name for an entry: `<id>-<n>.<ext>`, with `n` past the photos the entry has.
    pub fn photo_name(&self, id: u64, ext: &str) -> Option<String> {
        let e = self.entry(id)?;
        let ext = ext.to_ascii_lowercase();
        if !PHOTO_EXTENSIONS.contains(&ext.as_str()) {
            return None;
        }
        let mut n = e.photos.len() + 1;
        loop {
            let name = format!("{id}-{n}.{ext}");
            if !e.photos.contains(&name) {
                return Some(name);
            }
            n += 1;
        }
    }

    /// The entries the timeline shows, newest day first and newest entry first within a day. The search and the
    /// bookmarks filter apply.
    pub fn timeline(&self) -> Vec<u64> {
        let mut shown: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| !self.bookmarks_only || e.bookmarked)
            .filter(|e| matches_query(e, &self.query))
            .collect();
        shown.sort_by(|a, b| (b.day, b.created, b.id).cmp(&(a.day, a.created, a.id)));
        shown.into_iter().map(|e| e.id).collect()
    }

    /// Keeps the selection on an entry the timeline shows: when the selected entry is hidden by the search or
    /// the filter, the first entry of the timeline is selected instead (or nothing).
    pub fn reconcile_selection(&mut self) {
        let shown = self.timeline();
        match self.selected {
            Some(id) if shown.contains(&id) => {}
            _ => self.selected = shown.first().copied(),
        }
    }
}

/// True when every word of the query appears in the entry's text or in its date (case-insensitive).
pub fn matches_query(entry: &Entry, query: &str) -> bool {
    let text = entry.text.to_lowercase();
    let date = entry.day.format();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word) || date.contains(word))
}

/// The extension of a file path, lower case, or `None` when it has none.
pub fn file_extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// True for a plain photo file name: no directories, a photo extension and a stem.
pub fn is_photo_name(name: &str) -> bool {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return false;
    }
    match file_extension(name) {
        Some(ext) => PHOTO_EXTENSIONS.contains(&ext.as_str()),
        None => false,
    }
}

/// The journal file text.
pub fn encode(journal: &Journal) -> String {
    let mut entries: Vec<&Entry> = journal.entries.iter().collect();
    entries.sort_by_key(|e| e.id);
    let entries = entries
        .into_iter()
        .map(|e| {
            Json::Obj(vec![
                ("id".into(), Json::Num(e.id as f64)),
                ("day".into(), Json::Str(e.day.format())),
                ("created".into(), Json::Num(e.created as f64)),
                ("modified".into(), Json::Num(e.modified as f64)),
                ("text".into(), Json::Str(e.text.clone())),
                ("photos".into(), Json::Arr(e.photos.iter().map(|p| Json::Str(p.clone())).collect())),
                ("bookmarked".into(), Json::Bool(e.bookmarked)),
            ])
        })
        .collect();
    Json::Obj(vec![
        ("version".into(), Json::Num(FORMAT_VERSION)),
        ("next_id".into(), Json::Num(journal.next_id as f64)),
        ("entries".into(), Json::Arr(entries)),
    ])
    .encode()
}

/// Reads the journal file. A malformed entry is skipped rather than failing the whole journal; the file itself
/// must be a JSON object with an `entries` array.
pub fn decode(text: &str) -> Result<Journal, String> {
    let root = json::parse(text)?;
    let items = root.get("entries").and_then(Json::as_array).ok_or("no entries array")?;
    let mut journal = Journal {
        next_id: root.get("next_id").and_then(Json::number).map_or(0, |v| v as u64),
        ..Journal::default()
    };
    for item in items {
        if let Some(entry) = decode_entry(item) {
            journal.entries.push(entry);
        }
    }
    let largest = journal.entries.iter().map(|e| e.id).max().unwrap_or(0);
    journal.next_id = journal.next_id.max(largest);
    Ok(journal)
}

fn decode_entry(item: &Json) -> Option<Entry> {
    let id = item.get("id").and_then(Json::number).filter(|v| *v >= 1.0)? as u64;
    let day = Day::parse(item.get("day").and_then(Json::as_str)?)?;
    let num = |key: &str| item.get(key).and_then(Json::number).map_or(0, |v| v as u64);
    let text = item.get("text").and_then(Json::as_str).unwrap_or("").to_string();
    let photos = item
        .get("photos")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| p.as_str())
        .filter(|p| is_photo_name(p))
        .map(str::to_string)
        .collect();
    let bookmarked = matches!(item.get("bookmarked"), Some(Json::Bool(true)));
    Some(Entry { id, day, created: num("created"), modified: num("modified"), text, photos, bookmarked })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal_with(days: &[(&str, &str)]) -> Journal {
        let mut j = Journal::default();
        for (i, (day, text)) in days.iter().enumerate() {
            let id = j.new_entry(Day::parse(day).unwrap(), 1_000 + i as u64);
            j.set_text(id, text, 1_000 + i as u64);
        }
        j
    }

    #[test]
    fn day_round_trips_through_unix_time() {
        assert_eq!(Day::from_unix(0), Day { year: 1970, month: 1, day: 1 });
        assert_eq!(Day::from_unix(86_400 * 20_000), Day::parse("2024-10-04").unwrap());
        let today = Day::parse("2026-10-13").unwrap();
        assert_eq!(Day::from_unix(today.to_days() as u64 * 86_400), today);
        assert_eq!(Day::parse("2000-02-29").unwrap().to_days(), 11_016);
        assert_eq!(Day::parse("2000-03-01").unwrap().to_days() - Day::parse("2000-02-28").unwrap().to_days(), 2);
    }

    #[test]
    fn day_parsing_rejects_impossible_dates() {
        assert!(Day::parse("2026-02-29").is_none());
        assert!(Day::parse("2028-02-29").is_some());
        assert!(Day::parse("2026-13-01").is_none());
        assert!(Day::parse("2026-04-31").is_none());
        assert!(Day::parse("2026-10").is_none());
        assert!(Day::parse("2026-10-13-01").is_none());
        assert!(Day::parse("hello").is_none());
    }

    #[test]
    fn day_names_match_the_calendar() {
        // 2026-10-13 was a Tuesday
        let day = Day::parse("2026-10-13").unwrap();
        assert_eq!(day.weekday(), "Tuesday");
        assert_eq!(day.long(), "Tuesday 13 October 2026");
        assert_eq!(Day::parse("1970-01-01").unwrap().weekday(), "Thursday");
    }

    #[test]
    fn timeline_lists_newest_day_first_then_newest_entry() {
        let j = journal_with(&[("2026-01-02", "a"), ("2026-01-05", "b"), ("2026-01-05", "c")]);
        let shown: Vec<&str> = j.timeline().iter().map(|id| j.entry(*id).unwrap().text.as_str()).collect();
        assert_eq!(shown, ["c", "b", "a"]);
    }

    #[test]
    fn search_matches_every_word_ignoring_case() {
        let mut j = journal_with(&[("2026-03-01", "Walked to the Harbour"), ("2026-03-02", "Rain all day")]);
        j.query = "harbour WALKED".into();
        assert_eq!(j.timeline(), vec![1]);
        j.query = "2026-03-02".into();
        assert_eq!(j.timeline(), vec![2]);
        j.query = "rain sunshine".into();
        assert!(j.timeline().is_empty());
        j.query = "   ".into();
        assert_eq!(j.timeline().len(), 2);
    }

    #[test]
    fn bookmarks_filter_limits_the_timeline() {
        let mut j = journal_with(&[("2026-03-01", "one"), ("2026-03-02", "two")]);
        assert_eq!(j.toggle_bookmark(2), Some(true));
        j.bookmarks_only = true;
        assert_eq!(j.timeline(), vec![2]);
        assert_eq!(j.toggle_bookmark(2), Some(false));
        assert!(j.timeline().is_empty());
        assert_eq!(j.toggle_bookmark(99), None);
    }

    #[test]
    fn removing_the_selected_entry_selects_the_next_one() {
        let mut j = journal_with(&[("2026-03-01", "old"), ("2026-03-02", "new")]);
        j.select(Some(1));
        let removed = j.remove(1).unwrap();
        assert_eq!(removed.text, "old");
        assert_eq!(j.selected, Some(2));
        j.remove(2);
        assert_eq!(j.selected, None);
        assert!(j.remove(2).is_none());
    }

    #[test]
    fn reconcile_moves_the_selection_off_hidden_entries() {
        let mut j = journal_with(&[("2026-03-01", "alpha"), ("2026-03-02", "beta")]);
        j.select(Some(1));
        j.query = "beta".into();
        j.reconcile_selection();
        assert_eq!(j.selected, Some(2));
        j.query = "gamma".into();
        j.reconcile_selection();
        assert_eq!(j.selected, None);
    }

    #[test]
    fn ids_are_never_reused_after_a_removal() {
        let mut j = journal_with(&[("2026-03-01", "a"), ("2026-03-01", "b")]);
        j.remove(2);
        let id = j.new_entry(Day::parse("2026-03-03").unwrap(), 5);
        assert_eq!(id, 3);
    }

    #[test]
    fn photos_must_be_image_file_names() {
        let mut j = journal_with(&[("2026-03-01", "trip")]);
        assert!(j.add_photo(1, "1-1.jpg", 2));
        assert!(!j.add_photo(1, "1-1.jpg", 2), "duplicates are refused");
        assert!(!j.add_photo(1, "../secret.png", 2), "paths are refused");
        assert!(!j.add_photo(1, "notes.txt", 2), "other file types are refused");
        assert!(!j.add_photo(7, "7-1.png", 2), "unknown entries are refused");
        assert_eq!(j.photo_name(1, "PNG").as_deref(), Some("1-2.png"));
        assert_eq!(j.photo_name(1, "txt"), None);
    }

    #[test]
    fn file_extension_and_photo_names() {
        assert_eq!(file_extension("/home/user/Pictures/Beach.JPG").as_deref(), Some("jpg"));
        assert_eq!(file_extension("C:\\pics\\sky.png").as_deref(), Some("png"));
        assert_eq!(file_extension("README"), None);
        assert_eq!(file_extension(".hidden"), None);
        assert!(is_photo_name("12-1.heic"));
        assert!(!is_photo_name(".12-1.png"));
        assert!(!is_photo_name("dir/12-1.png"));
    }

    #[test]
    fn journal_round_trips_through_its_file_format() {
        let mut j = journal_with(&[("2026-03-01", "line one\nline two \"quoted\""), ("2026-03-02", "")]);
        j.add_photo(1, "1-1.jpg", 4);
        j.toggle_bookmark(2);
        let text = encode(&j);
        let back = decode(&text).unwrap();
        assert_eq!(back.entries, j.entries);
        assert_eq!(back.next_id, 2);
        assert_eq!(back.selected, None, "the selection is view state and is not saved");
    }

    #[test]
    fn malformed_entries_are_skipped() {
        let text = r#"{"version":1,"next_id":4,"entries":[
            {"id":1,"day":"2026-03-01","text":"ok","photos":["1-1.jpg","../x.jpg"],"bookmarked":true},
            {"id":2,"day":"2026-02-30","text":"bad day"},
            {"day":"2026-03-01","text":"no id"},
            {"id":3,"day":"2026-03-04"}
        ]}"#;
        let j = decode(text).unwrap();
        assert_eq!(j.entries.len(), 2);
        let first = j.entry(1).unwrap();
        assert_eq!(first.photos, vec!["1-1.jpg".to_string()]);
        assert!(first.bookmarked);
        assert_eq!(j.entry(3).unwrap().text, "");
        assert_eq!(j.next_id, 4);
    }

    #[test]
    fn decoding_rejects_a_file_without_entries() {
        assert!(decode("[1,2]").is_err());
        assert!(decode("not json").is_err());
    }
}

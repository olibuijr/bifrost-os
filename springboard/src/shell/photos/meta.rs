//! Details a person adds to a photo: title, caption, keywords, a place name and a corrected capture time. They live
//! beside the edit recipe in `photos.ini` (one key per photo and field), never in the picture file, so the original is
//! never touched. This file also holds the search over those details and over the file facts (name, album, date,
//! favourite, edited, rating, kind), and the library-wide keyword operations of the Keyword Manager.

use super::albums::{days_from_civil, parse_date_query};
use super::library::{civil, Photo, MONTHS};
use crate::shell::prefs::Prefs;
use std::collections::BTreeMap;
use std::path::Path;

const TITLE: &str = "meta.t";
const CAPTION: &str = "meta.c";
const KEYWORDS: &str = "meta.k";
const PLACE: &str = "meta.p";
const TAKEN: &str = "meta.d";

/// Longest title or place name, in characters.
pub const MAX_TEXT: usize = 80;
/// Longest caption, in characters.
pub const MAX_CAPTION: usize = 240;
/// At most this many keywords on one photo.
pub const MAX_KEYWORDS: usize = 20;
/// Longest single keyword, in characters.
pub const MAX_KEYWORD: usize = 32;

const DAY: i64 = 86_400;

/// A field of the sidecar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Caption,
    Keywords,
    Place,
    /// the corrected capture time, Unix seconds
    Taken,
}

impl Field {
    pub const ALL: [Field; 5] = [Field::Title, Field::Caption, Field::Keywords, Field::Place, Field::Taken];

    fn kind(self) -> &'static str {
        match self {
            Field::Title => TITLE,
            Field::Caption => CAPTION,
            Field::Keywords => KEYWORDS,
            Field::Place => PLACE,
            Field::Taken => TAKEN,
        }
    }

    /// The prefs key of this field for the photo at `path`.
    pub fn key(self, path: &Path) -> String {
        format!("{}:{}", self.kind(), path.display())
    }
}

/// What a person wrote about one photo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub place: String,
    /// the corrected capture time; None keeps the file's own
    pub taken: Option<i64>,
}

impl Details {
    pub fn is_empty(&self) -> bool {
        *self == Details::default()
    }
}

/// `text` on one line: runs of white space become a single space, ends trimmed, at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Keywords from comma separated text: trimmed, no empties, no repeats (any case), at most `MAX_KEYWORDS`.
pub fn parse_keywords(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(',') {
        let word = clean(part, MAX_KEYWORD);
        if word.is_empty() || out.iter().any(|k| same(k, &word)) {
            continue;
        }
        out.push(word);
        if out.len() == MAX_KEYWORDS {
            break;
        }
    }
    out
}

/// The keywords as the text a person edits: `beach, family`.
pub fn join_keywords(keywords: &[String]) -> String {
    keywords.join(", ")
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Everything the sidecar holds for the photo at `path`.
pub fn details(prefs: &Prefs, path: &Path) -> Details {
    let text = |f: Field| prefs.get(&f.key(path)).unwrap_or("").to_string();
    Details {
        title: text(Field::Title),
        caption: text(Field::Caption),
        keywords: parse_keywords(&text(Field::Keywords)),
        place: text(Field::Place),
        taken: prefs.get(&Field::Taken.key(path)).and_then(|v| v.parse().ok()),
    }
}

/// The prefs write that stores `value` as `field` of the photo at `path`. The value is cleaned the way the field
/// needs; an empty one clears the field.
pub fn field_write(path: &Path, field: Field, value: &str) -> (String, String) {
    let stored = match field {
        Field::Title | Field::Place => clean(value, MAX_TEXT),
        Field::Caption => clean(value, MAX_CAPTION),
        Field::Keywords => join_keywords(&parse_keywords(value)),
        Field::Taken => value.trim().parse::<i64>().map(|v| v.to_string()).unwrap_or_default(),
    };
    (field.key(path), stored)
}

/// The writes that carry the sidecar of `from` over to `to` (a photo moved to Recently Deleted and back, or a
/// duplicate). The source is cleared too unless `keep_source`.
pub fn transfer_writes(prefs: &Prefs, from: &Path, to: &Path, keep_source: bool) -> Vec<(String, String)> {
    let mut writes = Vec::new();
    for f in Field::ALL {
        let source = f.key(from);
        if let Some(v) = prefs.get(&source).filter(|v| !v.is_empty()) {
            writes.push((f.key(to), v.to_string()));
            if !keep_source {
                writes.push((source, String::new()));
            }
        }
    }
    writes
}

/// Gives every photo with a corrected capture time that time, then puts the list back in library order.
pub fn apply_taken(prefs: &Prefs, photos: &mut [Photo]) {
    let mut any = false;
    for p in photos.iter_mut() {
        if let Some(t) = prefs.get(&Field::Taken.key(&p.path)).and_then(|v| v.parse::<i64>().ok()) {
            p.mtime = t;
            any = true;
        }
    }
    if any {
        photos.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
    }
}

/// "2026-10-04 14:30": a capture time as it is typed in the date sheet, in the zone `offset` seconds east of UTC.
pub fn format_taken(unix: i64, offset: i32) -> String {
    let d = civil(unix, offset);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", d.year, d.month, d.day, d.hour, d.min)
}

/// "14:30" or "14:30:15" as seconds into the day.
fn parse_clock(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let n = |i: usize| parts.get(i).map_or(Some(0), |s| s.parse::<u32>().ok());
    let (h, m, s) = (n(0)?, n(1)?, n(2)?);
    (h < 24 && m < 60 && s < 60).then_some(h * 3600 + m * 60 + s)
}

/// A capture time typed as a date, a time or both ("2026-10-04 14:30", "4 October 2026", "09:15"). A missing date
/// keeps the day of `current`, a missing time keeps its time of day. None when it is neither.
pub fn parse_taken(text: &str, offset: i32, now: i64, current: i64) -> Option<i64> {
    let mut tokens: Vec<&str> = text.split_whitespace().collect();
    let mut clock = None;
    if let Some(last) = tokens.last().copied().filter(|t| t.contains(':')) {
        clock = Some(parse_clock(last)?);
        tokens.pop();
    }
    if tokens.is_empty() && clock.is_none() {
        return None;
    }
    let day_start = if tokens.is_empty() {
        let c = civil(current, offset);
        days_from_civil(c.year, c.month, c.day) * DAY - offset as i64
    } else {
        let (start, end) = parse_date_query(&tokens.join(" "), offset, now)?;
        if end - start != DAY {
            return None;
        }
        start
    };
    let secs = clock.unwrap_or_else(|| (current + offset as i64).rem_euclid(DAY) as u32);
    Some(day_start + secs as i64)
}

// --- keyword manager ---

/// Every keyword in use on `photos` with the number of photos that carry it, by name (any case). A keyword shows
/// under the spelling met first.
pub fn keyword_counts(prefs: &Prefs, photos: &[Photo]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for p in photos {
        for k in details(prefs, &p.path).keywords {
            counts.entry(k.to_lowercase()).or_insert_with(|| (k.clone(), 0)).1 += 1;
        }
    }
    counts.into_values().collect()
}

/// The writes that rename the keyword `from` to `to` on every photo of `photos` that has it. An empty `to`, or one
/// the photo already has, folds the keyword away.
pub fn rename_keyword_writes(prefs: &Prefs, photos: &[Photo], from: &str, to: &str) -> Vec<(String, String)> {
    let to = clean(to.replace(',', " ").as_str(), MAX_KEYWORD);
    let mut writes = Vec::new();
    for p in photos {
        let have = details(prefs, &p.path).keywords;
        if !have.iter().any(|k| same(k, from)) {
            continue;
        }
        let renamed: Vec<String> = have.iter().map(|k| if same(k, from) { to.clone() } else { k.clone() }).collect();
        writes.push((Field::Keywords.key(&p.path), join_keywords(&parse_keywords(&join_keywords(&renamed)))));
    }
    writes
}

/// The writes that add `keyword` to every photo of `photos` that lacks it.
pub fn add_keyword_writes(prefs: &Prefs, photos: &[Photo], keyword: &str) -> Vec<(String, String)> {
    let keyword = clean(keyword.replace(',', " ").as_str(), MAX_KEYWORD);
    if keyword.is_empty() {
        return Vec::new();
    }
    let mut writes = Vec::new();
    for p in photos {
        let mut have = details(prefs, &p.path).keywords;
        if have.iter().any(|k| same(k, &keyword)) || have.len() >= MAX_KEYWORDS {
            continue;
        }
        have.push(keyword.clone());
        writes.push((Field::Keywords.key(&p.path), join_keywords(&have)));
    }
    writes
}

// --- search ---

/// A search typed in the toolbar field. Plain words must all appear in the file name, an album name, the title,
/// caption, place, a keyword or the date text. `#beach` or `tag:beach` asks for that keyword, `is:favorite`,
/// `is:edited` and `is:screenshot` for those photos, `kind:png` for a file type and `rating:3` for at least three
/// stars. An underscore in a keyword stands for a space (`#sun_set`). A query that is itself a date ("2026-10",
/// "4 October 2026") asks for that day, month or year.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    words: Vec<String>,
    tags: Vec<String>,
    kinds: Vec<String>,
    favourite: bool,
    edited: bool,
    screenshot: bool,
    min_rating: u8,
    date: Option<(i64, i64)>,
}

/// What a search looks at for one photo.
pub struct Subject<'a> {
    pub name: &'a str,
    pub taken: i64,
    pub favourite: bool,
    pub edited: bool,
    pub screenshot: bool,
    pub rating: u8,
    /// the names of the user albums that hold the photo
    pub albums: &'a [String],
    pub details: &'a Details,
}

impl Query {
    pub fn parse(text: &str, offset: i32, now: i64) -> Query {
        let mut q = Query::default();
        let text = text.trim();
        if let Some(range) = parse_date_query(text, offset, now) {
            q.date = Some(range);
            return q;
        }
        for token in text.split_whitespace() {
            let lower = token.to_lowercase();
            if let Some(tag) = lower.strip_prefix('#').filter(|t| !t.is_empty()) {
                q.tags.push(tag.replace('_', " "));
                continue;
            }
            let Some((key, value)) = lower.split_once(':').filter(|(_, v)| !v.is_empty()) else {
                q.words.push(lower.clone());
                continue;
            };
            match key {
                "is" if matches!(value, "favorite" | "favorites" | "favourite" | "favourites") => q.favourite = true,
                "is" if value == "edited" => q.edited = true,
                "is" if matches!(value, "screenshot" | "screenshots") => q.screenshot = true,
                "kind" => q.kinds.push(value.trim_start_matches('.').to_string()),
                "rating" if matches!(value.parse::<u8>(), Ok(1..=5)) => q.min_rating = q.min_rating.max(value.parse().unwrap_or(0)),
                "tag" | "keyword" => q.tags.push(value.replace('_', " ")),
                _ => q.words.push(lower.clone()),
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }

    /// Does the photo described by `s` match, with dates read in the zone `offset` seconds east of UTC.
    pub fn matches(&self, s: &Subject, offset: i32) -> bool {
        if let Some((from, to)) = self.date {
            return s.taken >= from && s.taken < to;
        }
        if (self.favourite && !s.favourite) || (self.edited && !s.edited) || (self.screenshot && !s.screenshot) || s.rating < self.min_rating {
            return false;
        }
        if !self.kinds.is_empty() {
            let ext = s.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
            let ext = if ext == "jpeg" { "jpg".to_string() } else { ext };
            if !self.kinds.iter().any(|k| (if k == "jpeg" { "jpg" } else { k.as_str() }) == ext) {
                return false;
            }
        }
        if !self.tags.iter().all(|t| s.details.keywords.iter().any(|k| k.to_lowercase() == *t)) {
            return false;
        }
        if self.words.is_empty() {
            return true;
        }
        let mut haystack: Vec<String> = vec![s.name.to_lowercase(), s.details.title.to_lowercase(), s.details.caption.to_lowercase(), s.details.place.to_lowercase(), date_text(s.taken, offset)];
        haystack.extend(s.albums.iter().map(|a| a.to_lowercase()));
        haystack.extend(s.details.keywords.iter().map(|k| k.to_lowercase()));
        self.words.iter().all(|w| haystack.iter().any(|h| h.contains(w.as_str())))
    }
}

/// "4 october 2026 2026-10-04": the date as words and as digits, lower case, for matching.
fn date_text(unix: i64, offset: i32) -> String {
    let d = civil(unix, offset);
    format!("{} {} {} {:04}-{:02}-{:02}", d.day, MONTHS[(d.month - 1) as usize].to_lowercase(), d.year, d.year, d.month, d.day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NOW: i64 = 1_791_073_395;

    fn photo(name: &str, mtime: i64) -> Photo {
        Photo { path: PathBuf::from(format!("/p/{name}")), mtime, size: 1 }
    }

    fn keywords(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn set(prefs: &mut Prefs, writes: &[(String, String)]) {
        let pairs: Vec<(&str, &str)> = writes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        prefs.set_many(&pairs).unwrap();
    }

    #[test]
    fn keywords_are_trimmed_unique_and_capped() {
        assert_eq!(parse_keywords(" Beach ,family,, beach , Sun  Set "), keywords(&["Beach", "family", "Sun Set"]));
        assert!(parse_keywords(" , ,").is_empty());
        let many: String = (0..30).map(|i| format!("k{i},")).collect();
        assert_eq!(parse_keywords(&many).len(), MAX_KEYWORDS);
        assert_eq!(parse_keywords(&"x".repeat(100))[0].chars().count(), MAX_KEYWORD);
        assert_eq!(join_keywords(&keywords(&["a", "b"])), "a, b");
    }

    #[test]
    fn details_round_trip_through_the_prefs_file() {
        let mut prefs = Prefs::memory();
        let path = Path::new("/p/IMG_1.jpg");
        assert!(details(&prefs, path).is_empty());
        let writes = vec![
            field_write(path, Field::Title, "  Harbour   at dusk "),
            field_write(path, Field::Caption, "The boats are back"),
            field_write(path, Field::Keywords, "sea, Boats, sea"),
            field_write(path, Field::Place, "Reykjavik"),
            field_write(path, Field::Taken, "1791000000"),
        ];
        set(&mut prefs, &writes);
        let d = details(&prefs, path);
        assert_eq!(d.title, "Harbour at dusk");
        assert_eq!(d.caption, "The boats are back");
        assert_eq!(d.keywords, keywords(&["sea", "Boats"]));
        assert_eq!(d.place, "Reykjavik");
        assert_eq!(d.taken, Some(1_791_000_000));
        // an empty value clears a field
        set(&mut prefs, &[field_write(path, Field::Title, "   ")]);
        assert_eq!(details(&prefs, path).title, "");
        assert_eq!(field_write(path, Field::Taken, "soon").1, "", "a time that is not a number stores nothing");
        assert_eq!(field_write(path, Field::Caption, &"w ".repeat(400)).1.chars().count(), MAX_CAPTION);
    }

    #[test]
    fn details_follow_a_photo_that_moves_and_stay_on_a_duplicate() {
        let mut prefs = Prefs::memory();
        let (a, b) = (Path::new("/p/a.jpg"), Path::new("/p/.trash/a.jpg"));
        set(&mut prefs, &[field_write(a, Field::Title, "Hello"), field_write(a, Field::Keywords, "x")]);
        let copy = transfer_writes(&prefs, a, b, true);
        set(&mut prefs, &copy);
        assert_eq!(details(&prefs, a).title, "Hello", "a copy keeps the source");
        assert_eq!(details(&prefs, b).keywords, keywords(&["x"]));
        let moved = transfer_writes(&prefs, b, Path::new("/p/c.jpg"), false);
        set(&mut prefs, &moved);
        assert!(details(&prefs, b).is_empty(), "a move clears the source");
        assert_eq!(details(&prefs, Path::new("/p/c.jpg")).title, "Hello");
        assert!(transfer_writes(&prefs, Path::new("/p/none.jpg"), b, false).is_empty());
    }

    #[test]
    fn a_corrected_capture_time_replaces_the_file_time_and_reorders() {
        let mut prefs = Prefs::memory();
        let mut photos = vec![photo("a.jpg", 100), photo("b.jpg", 200), photo("c.jpg", 300)];
        set(&mut prefs, &[field_write(&photos[0].path, Field::Taken, "250")]);
        apply_taken(&prefs, &mut photos);
        let names: Vec<String> = photos.iter().map(Photo::name).collect();
        assert_eq!(names, ["b.jpg", "a.jpg", "c.jpg"]);
        assert_eq!(photos[1].mtime, 250);
    }

    #[test]
    fn capture_times_are_typed_as_dates_times_or_both() {
        let current = 1_790_000_000; // 2026-09-21 14:13:20 UTC
        let at = |text: &str, offset: i32| parse_taken(text, offset, NOW, current);
        assert_eq!(at("2026-10-04 14:30", 0), Some(days_from_civil(2026, 10, 4) * DAY + 14 * 3600 + 30 * 60));
        assert_eq!(at("4 October 2026 09:15:30", 0), Some(days_from_civil(2026, 10, 4) * DAY + 9 * 3600 + 15 * 60 + 30));
        assert_eq!(at("2026-10-04", 0), Some(days_from_civil(2026, 10, 4) * DAY + 14 * 3600 + 13 * 60 + 20), "the time of day stays");
        assert_eq!(at("08:00", 0), Some(days_from_civil(2026, 9, 21) * DAY + 8 * 3600), "the day stays");
        assert_eq!(at("2026-10-04 12:00", 3600), Some(days_from_civil(2026, 10, 4) * DAY + 11 * 3600), "a zone east of UTC");
        for bad in ["", "   ", "tomorrow", "2026-10", "2026", "2026-02-30", "25:00", "10:75", "2026-10-04 10", "1:2:3:4"] {
            assert_eq!(at(bad, 0), None, "{bad:?}");
        }
        assert_eq!(format_taken(days_from_civil(2026, 10, 4) * DAY + 14 * 3600 + 30 * 60, 0), "2026-10-04 14:30");
        assert_eq!(parse_taken(&format_taken(current, 0), 0, NOW, 0), Some(current - current % 60), "the typed form reads back");
    }

    #[test]
    fn the_keyword_manager_counts_renames_and_adds() {
        let mut prefs = Prefs::memory();
        let photos = vec![photo("a.jpg", 1), photo("b.jpg", 2), photo("c.jpg", 3)];
        set(&mut prefs, &[field_write(&photos[0].path, Field::Keywords, "Beach, sun"), field_write(&photos[1].path, Field::Keywords, "beach, family, Sun")]);
        assert_eq!(keyword_counts(&prefs, &photos), [("Beach".to_string(), 2), ("family".to_string(), 1), ("sun".to_string(), 2)]);
        // renaming folds into a keyword the photo already has
        let writes = rename_keyword_writes(&prefs, &photos, "SUN", "beach");
        assert_eq!(writes.len(), 2);
        set(&mut prefs, &writes);
        assert_eq!(details(&prefs, &photos[0].path).keywords, keywords(&["Beach"]));
        assert_eq!(details(&prefs, &photos[1].path).keywords, keywords(&["beach", "family"]));
        // an empty name deletes the keyword everywhere
        set(&mut prefs, &rename_keyword_writes(&prefs, &photos, "beach", ""));
        assert_eq!(keyword_counts(&prefs, &photos), [("family".to_string(), 1)]);
        // adding skips the photos that have it
        let adds = add_keyword_writes(&prefs, &photos, "Family, friends");
        assert_eq!(adds.len(), 3, "none of the photos has it yet");
        set(&mut prefs, &adds);
        assert_eq!(details(&prefs, &photos[2].path).keywords, keywords(&["Family friends"]), "commas are not part of a keyword");
        assert_eq!(details(&prefs, &photos[1].path).keywords, keywords(&["family", "Family friends"]));
        assert!(add_keyword_writes(&prefs, &photos, "family friends").is_empty(), "now they all have it");
        assert!(add_keyword_writes(&prefs, &photos, "  ").is_empty());
    }

    fn subject<'a>(name: &'a str, taken: i64, details: &'a Details, albums: &'a [String]) -> Subject<'a> {
        Subject { name, taken, favourite: false, edited: false, screenshot: false, rating: 0, albums, details }
    }

    #[test]
    fn search_words_look_at_names_albums_dates_and_details() {
        let d = Details { title: "Harbour".into(), caption: "Boats at dusk".into(), keywords: keywords(&["Sea"]), place: "Reykjavik".into(), taken: None };
        let albums = keywords(&["Trip"]);
        let taken = days_from_civil(2026, 10, 4) * DAY;
        let s = subject("IMG_0042.JPG", taken, &d, &albums);
        let hit = |q: &str| Query::parse(q, 0, NOW).matches(&s, 0);
        for q in ["", "img_0042", "harbour", "DUSK boats", "sea", "reykjavik", "trip", "october", "oct 4", "2026-10"] {
            assert!(hit(q), "{q:?} should match");
        }
        for q in ["nothing", "harbour winter", "2025", "4 november 2026"] {
            assert!(!hit(q), "{q:?} should not match");
        }
        assert!(Query::parse("  ", 0, NOW).is_empty());
        assert!(!Query::parse("sea", 0, NOW).is_empty());
    }

    #[test]
    fn search_operators_filter_by_flag_kind_rating_and_keyword() {
        let d = Details { keywords: keywords(&["Beach"]), ..Details::default() };
        let none: Vec<String> = Vec::new();
        let mut s = subject("a.jpeg", 0, &d, &none);
        let hit = |s: &Subject, q: &str| Query::parse(q, 0, NOW).matches(s, 0);
        assert!(hit(&s, "#beach") && hit(&s, "tag:BEACH") && hit(&s, "keyword:beach"));
        let spaced = Details { keywords: keywords(&["Sun Set"]), ..Details::default() };
        let s2 = subject("b.png", 0, &spaced, &none);
        assert!(hit(&s2, "#sun_set") && hit(&s2, "tag:sun_set") && !hit(&s2, "#sun"));
        assert!(!hit(&s, "#bea"), "a tag is a whole keyword");
        assert!(hit(&s, "kind:jpg") && hit(&s, "kind:.jpeg") && !hit(&s, "kind:png"));
        assert!(!hit(&s, "is:favorite") && !hit(&s, "is:edited") && !hit(&s, "is:screenshot") && !hit(&s, "rating:1"));
        s.favourite = true;
        s.edited = true;
        s.screenshot = true;
        s.rating = 4;
        assert!(hit(&s, "is:favorites is:edited is:screenshot rating:3 rating:4"));
        assert!(!hit(&s, "rating:5"));
        // an operator with a value it does not know is a plain word
        assert!(!hit(&s, "is:old") && !hit(&s, "rating:9"));
        assert!(hit(&s, "is:favourite #beach kind:jpg"), "operators combine");
    }

    #[test]
    fn a_search_that_is_a_date_asks_for_that_period() {
        let d = Details::default();
        let none: Vec<String> = Vec::new();
        let in_oct = subject("a.png", days_from_civil(2026, 10, 15) * DAY, &d, &none);
        let in_sep = subject("b.png", days_from_civil(2026, 9, 15) * DAY, &d, &none);
        let q = Query::parse("October 2026", 0, NOW);
        assert!(q.matches(&in_oct, 0) && !q.matches(&in_sep, 0));
        let day = Query::parse("2026-09-15", 0, NOW);
        assert!(day.matches(&in_sep, 0) && !day.matches(&in_oct, 0));
        let named = subject("2026.png", 0, &d, &none);
        assert!(!Query::parse("2026", 0, NOW).matches(&named, 0), "a year is a date, not a name word");
    }
}

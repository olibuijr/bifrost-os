//! A photo's own details: title, caption, keywords, a capture date set by hand and a place name. They live in the
//! settings file beside the edit recipe, one `meta:<path>` line per photo, so the picture file itself is never
//! changed. This file also parses the text the details sheet accepts and answers the search field.

use super::albums::days_from_civil;
use super::library::{civil, Photo};
use crate::shell::prefs::{escape_field, unescape_field};

/// The longest title, caption or place kept.
pub const TEXT_MAX: usize = 200;
/// The most keywords one photo carries.
pub const KEYWORDS_MAX: usize = 24;
/// The longest keyword.
pub const KEYWORD_LEN: usize = 32;

/// What the user wrote about a photo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    /// a capture date set by hand (Unix seconds); it replaces the date in the file
    pub taken: Option<i64>,
    /// a place name typed in
    pub place: String,
    /// the location is not shared with the photo
    pub hide_location: bool,
}

impl Meta {
    pub fn is_empty(&self) -> bool {
        *self == Meta::default()
    }

    /// The single-line value stored under `meta:<path>`: a version tag, six tab-separated, escaped fields and an
    /// end mark (the settings file trims blanks at both ends of a value, so empty first and last fields need them).
    pub fn encode(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let taken = self.taken.map(|t| t.to_string()).unwrap_or_default();
        [
            "m1".to_string(),
            escape_field(&self.title),
            escape_field(&self.caption),
            escape_field(&self.keywords.join(",")),
            taken,
            escape_field(&self.place),
            if self.hide_location { "1".into() } else { String::new() },
            "~".to_string(),
        ]
        .join("\t")
    }

    /// Reads a stored value; anything malformed reads as no details.
    pub fn decode(text: &str) -> Meta {
        let f: Vec<&str> = text.split('\t').collect();
        if f.len() < 8 || f[0] != "m1" {
            return Meta::default();
        }
        let get = |i: usize| unescape_field(f[i], TEXT_MAX * 4).unwrap_or_default();
        Meta { title: get(1), caption: get(2), keywords: parse_keywords(&get(3)), taken: f[4].parse().ok(), place: get(5), hide_location: f[6] == "1" }
    }

    /// Does `keyword` (any case) belong to this photo.
    pub fn has_keyword(&self, keyword: &str) -> bool {
        self.keywords.iter().any(|k| k.eq_ignore_ascii_case(keyword))
    }

    /// Adds a keyword unless it is there already or the photo has the most; true when it was added.
    pub fn add_keyword(&mut self, keyword: &str) -> bool {
        let k = clean_keyword(keyword);
        if k.is_empty() || self.has_keyword(&k) || self.keywords.len() >= KEYWORDS_MAX {
            return false;
        }
        self.keywords.push(k);
        true
    }

    pub fn remove_keyword(&mut self, keyword: &str) -> bool {
        let before = self.keywords.len();
        self.keywords.retain(|k| !k.eq_ignore_ascii_case(keyword));
        self.keywords.len() != before
    }

    /// Renames a keyword in place; when the new name is already there the old one just goes.
    pub fn rename_keyword(&mut self, from: &str, to: &str) -> bool {
        let to = clean_keyword(to);
        if to.is_empty() || !self.has_keyword(from) {
            return false;
        }
        let had_new = self.has_keyword(&to);
        self.remove_keyword(from);
        if !had_new {
            self.keywords.push(to);
        }
        true
    }
}

/// A keyword as kept: trimmed, one line, no commas, at most `KEYWORD_LEN` characters.
pub fn clean_keyword(text: &str) -> String {
    text.replace([',', '\t', '\n', '\r'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(KEYWORD_LEN).collect()
}

/// Keywords typed as a comma-separated list: cleaned, without repeats (case ignored), at most `KEYWORDS_MAX`.
pub fn parse_keywords(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(',') {
        let k = clean_keyword(part);
        if !k.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&k)) && out.len() < KEYWORDS_MAX {
            out.push(k);
        }
    }
    out
}

/// A title, caption or place as kept: one line, trimmed, at most `TEXT_MAX` characters.
pub fn clean_text(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ").trim().chars().take(TEXT_MAX).collect()
}

/// Parses "2026-10-04", "2026-10-04 13:30" or "2026-10-04 13:30:15" as a local time in a zone `offset` seconds
/// east of UTC; Unix seconds.
pub fn parse_datetime(text: &str, offset: i32) -> Option<i64> {
    let mut parts = text.trim().splitn(2, [' ', 'T']);
    let date = parts.next()?;
    let time = parts.next().map(str::trim).filter(|t| !t.is_empty());
    let mut d = date.split('-');
    let (y, m, day): (i32, u32, u32) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    if d.next().is_some() || !(1900..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // reject a day the month does not have (31 February)
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    if days_from_civil(y, m, day) >= next {
        return None;
    }
    let (mut h, mut mi, mut s) = (0u32, 0u32, 0u32);
    if let Some(t) = time {
        let mut tp = t.split(':');
        h = tp.next()?.parse().ok()?;
        mi = tp.next().map_or(Some(0), |v| v.parse().ok())?;
        s = tp.next().map_or(Some(0), |v| v.parse().ok())?;
        if tp.next().is_some() || h > 23 || mi > 59 || s > 59 {
            return None;
        }
    }
    Some(days_from_civil(y, m, day) * 86_400 + (h * 3600 + mi * 60 + s) as i64 - offset as i64)
}

/// "2026-10-04 13:30": a date as the details sheet shows and accepts it.
pub fn format_datetime(unix: i64, offset: i32) -> String {
    let d = civil(unix, offset);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", d.year, d.month, d.day, d.hour, d.min)
}

/// Does `photo` match the search `query`: every word must be found in the file name, the title, the caption, a
/// keyword, the place, the date ("October 2026", "2026-10-04") or one of the album names `albums` lists.
pub fn matches_query(photo: &Photo, meta: &Meta, albums: &[String], offset: i32, query: &str) -> bool {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return true;
    }
    let d = civil(photo.mtime, offset);
    let mut hay = vec![photo.name(), meta.title.clone(), meta.caption.clone(), meta.place.clone(), super::library::month_title(&d), super::library::day_title(&d), format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)];
    hay.extend(meta.keywords.iter().cloned());
    hay.extend(albums.iter().cloned());
    let hay = hay.join("\n").to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn photo(name: &str, mtime: i64) -> Photo {
        Photo { path: PathBuf::from(format!("/p/{name}")), mtime, size: 1 }
    }

    #[test]
    fn details_survive_the_one_line_encoding() {
        let m = Meta { title: "Sunset\tat sea".into(), caption: "line one\nline two".into(), keywords: vec!["trip".into(), "Iceland".into()], taken: Some(1_790_000_000), place: "Reykjavik".into(), hide_location: true };
        let text = m.encode();
        assert!(!text.contains('\n'));
        assert_eq!(Meta::decode(&text), m);
        // the settings file trims a value, which must not lose empty fields
        let sparse = Meta { keywords: vec!["x".into()], ..Meta::default() };
        assert_eq!(Meta::decode(sparse.encode().trim()), sparse);
    }

    #[test]
    fn nothing_written_stores_an_empty_value_and_garbage_reads_as_nothing() {
        assert_eq!(Meta::default().encode(), "");
        assert_eq!(Meta::decode(""), Meta::default());
        assert_eq!(Meta::decode("not a record"), Meta::default());
    }

    #[test]
    fn keywords_are_cleaned_deduplicated_and_bounded() {
        assert_eq!(parse_keywords(" Trip , trip,  beach  day ,,"), vec!["Trip", "beach day"]);
        assert_eq!(clean_keyword(&"x".repeat(80)).chars().count(), KEYWORD_LEN);
        let many = (0..40).map(|i| format!("k{i}")).collect::<Vec<_>>().join(",");
        assert_eq!(parse_keywords(&many).len(), KEYWORDS_MAX);
    }

    #[test]
    fn keywords_can_be_added_removed_and_renamed() {
        let mut m = Meta::default();
        assert!(m.add_keyword("Trip"));
        assert!(!m.add_keyword("trip"), "case is ignored");
        assert!(m.add_keyword("beach"));
        assert!(m.rename_keyword("TRIP", "holiday"));
        assert_eq!(m.keywords, vec!["beach", "holiday"]);
        assert!(m.rename_keyword("beach", "Holiday"), "renaming onto an existing keyword merges them");
        assert_eq!(m.keywords, vec!["holiday"]);
        assert!(m.remove_keyword("HOLIDAY"));
        assert!(!m.remove_keyword("holiday"));
    }

    #[test]
    fn dates_parse_in_the_local_zone_and_round_trip() {
        let t = parse_datetime("2026-10-04 13:30", 3600).unwrap();
        assert_eq!(format_datetime(t, 3600), "2026-10-04 13:30");
        assert_eq!(parse_datetime("2026-10-04", 0), Some(days_from_civil(2026, 10, 4) * 86_400));
        assert_eq!(parse_datetime("2026-10-04T08:05:09", 0), Some(days_from_civil(2026, 10, 4) * 86_400 + 8 * 3600 + 5 * 60 + 9));
        assert_eq!(parse_datetime("2026-02-31", 0), None);
        assert_eq!(parse_datetime("2026-13-01", 0), None);
        assert_eq!(parse_datetime("2026-10-04 25:00", 0), None);
        assert_eq!(parse_datetime("yesterday", 0), None);
    }

    #[test]
    fn search_needs_every_word_in_any_field() {
        let p = photo("IMG_0042.jpg", days_from_civil(2026, 10, 4) * 86_400 + 40_000);
        let m = Meta { title: "Harbour".into(), keywords: vec!["boats".into()], ..Meta::default() };
        let albums = vec!["Iceland trip".to_string()];
        assert!(matches_query(&p, &m, &albums, 0, ""));
        assert!(matches_query(&p, &m, &albums, 0, "img_0042"));
        assert!(matches_query(&p, &m, &albums, 0, "harbour BOATS"));
        assert!(matches_query(&p, &m, &albums, 0, "iceland october"));
        assert!(matches_query(&p, &m, &albums, 0, "2026-10-04"));
        assert!(!matches_query(&p, &m, &albums, 0, "harbour mountains"));
    }
}

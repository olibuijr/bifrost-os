//! Pure album logic: date filters
//! typed as text or picked by month, the Recently Deleted retention and the
//! multi-select set.

use super::library::{civil, Photo, MONTHS};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Deleted photos are purged after this many days.
pub const TRASH_DAYS: i64 = 30;
const DAY: i64 = 86_400;

/// Photos with `start <= mtime < end`, order kept.
pub fn in_range(photos: &[Photo], range: (i64, i64)) -> Vec<Photo> {
    photos.iter().filter(|p| p.mtime >= range.0 && p.mtime < range.1).cloned().collect()
}

/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    crate::bare::timezone::days_from_civil(y as i64, m, d)
}

fn month_number(word: &str) -> Option<u32> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    MONTHS.iter().position(|m| m.to_ascii_lowercase().starts_with(&w)).map(|i| i as u32 + 1)
}

fn next_month(y: i32, m: u32) -> (i32, u32) {
    if m == 12 { (y + 1, 1) } else { (y, m + 1) }
}

fn valid_day(y: i32, m: u32, d: u32) -> bool {
    d >= 1 && days_from_civil(y, m, d) + 1 <= days_from_civil(next_month(y, m).0, next_month(y, m).1, 1) && (1..=12).contains(&m)
}

/// Parses "2026", "2026-10", "2026-10-04", "October 2026", "oct 2026",
/// "4 October 2026" or "4 oct" (no year: the year of `now`) into the
/// local-time range it names, as Unix seconds `[start, end)`.
pub fn parse_date_query(q: &str, offset: i32, now: i64) -> Option<(i64, i64)> {
    let q = q.trim();
    let cur_year = civil(now, offset).year;
    let span = |from: (i32, u32, u32), to: (i32, u32, u32)| {
        let s = days_from_civil(from.0, from.1, from.2) * DAY - offset as i64;
        let e = days_from_civil(to.0, to.1, to.2) * DAY - offset as i64;
        Some((s, e))
    };
    let month = |y: i32, m: u32| {
        if !(1..=12).contains(&m) || !(1900..=9999).contains(&y) {
            return None;
        }
        let (ny, nm) = next_month(y, m);
        span((y, m, 1), (ny, nm, 1))
    };
    let day = |y: i32, m: u32, d: u32| {
        if !(1900..=9999).contains(&y) || !valid_day(y, m, d) {
            return None;
        }
        let next = days_from_civil(y, m, d) + 1;
        let s = days_from_civil(y, m, d) * DAY - offset as i64;
        Some((s, next * DAY - offset as i64))
    };
    let parts: Vec<&str> = q.split(|c: char| c == '-' || c == '/' || c == '.' || c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
    let num = |s: &str| s.parse::<i32>().ok();
    match parts.as_slice() {
        [y] => {
            let y = num(y)?;
            if !(1900..=9999).contains(&y) {
                return None;
            }
            span((y, 1, 1), (y + 1, 1, 1))
        }
        [a, b] => match (num(a), num(b)) {
            (Some(y), Some(m)) if y > 31 => month(y, m as u32),
            _ => {
                if let (Some(m), Some(y)) = (month_number(a), num(b)) {
                    month(y, m)
                } else if let (Some(d), Some(m)) = (num(a), month_number(b)) {
                    day(cur_year, m, d as u32)
                } else {
                    None
                }
            }
        },
        [a, b, c] => {
            if let (Some(y), Some(m), Some(d)) = (num(a), num(b), num(c)) {
                day(y, m as u32, d as u32)
            } else if let (Some(d), Some(m), Some(y)) = (num(a), month_number(b), num(c)) {
                day(y, m, d as u32)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The month ranges present in `photos` (sorted by time), newest first, as
/// (title, range): what a month filter cycles through.
pub fn month_filters(photos: &[Photo], offset: i32) -> Vec<(String, (i64, i64))> {
    let mut out: Vec<(String, (i64, i64))> = Vec::new();
    for p in photos.iter().rev() {
        let d = civil(p.mtime, offset);
        let title = super::library::month_title(&d);
        if out.last().is_none_or(|(t, _)| *t != title) {
            let (ny, nm) = next_month(d.year, d.month);
            let s = days_from_civil(d.year, d.month, 1) * DAY - offset as i64;
            let e = days_from_civil(ny, nm, 1) * DAY - offset as i64;
            out.push((title, (s, e)));
        }
    }
    out
}

/// True once a photo deleted at `deleted_at` is due for removal.
pub fn purge_due(deleted_at: i64, now: i64) -> bool {
    now - deleted_at >= TRASH_DAYS * DAY
}

/// Whole days left before the purge (at least 0).
pub fn days_left(deleted_at: i64, now: i64) -> i64 {
    (TRASH_DAYS - (now - deleted_at).max(0) / DAY).max(0)
}

/// The multi-select set, by path so it survives rescans.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    paths: BTreeSet<PathBuf>,
    pub active: bool,
}

impl Selection {
    pub fn begin(&mut self) {
        self.active = true;
        self.paths.clear();
    }

    pub fn end(&mut self) {
        self.active = false;
        self.paths.clear();
    }

    /// Adds or removes `p`; true when it is now selected.
    pub fn toggle(&mut self, p: &Path) -> bool {
        if !self.paths.remove(p) {
            self.paths.insert(p.to_path_buf());
            return true;
        }
        false
    }

    pub fn contains(&self, p: &Path) -> bool {
        self.paths.contains(p)
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The selected paths in a stable order.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.paths.iter().cloned().collect()
    }

    /// Selects every photo of `photos`.
    pub fn select_all(&mut self, photos: &[Photo]) {
        self.paths = photos.iter().map(|p| p.path.clone()).collect();
    }

    /// Select All, or Deselect All when every photo of `photos` is already selected. True when all are selected now.
    pub fn toggle_all(&mut self, photos: &[Photo]) -> bool {
        if !photos.is_empty() && photos.iter().all(|p| self.contains(&p.path)) {
            self.paths.clear();
            false
        } else {
            self.select_all(photos);
            !photos.is_empty()
        }
    }

    /// Selects the photos between `from` and `to` (indexes into `photos`, either order), keeping what was selected.
    pub fn select_range(&mut self, photos: &[Photo], from: usize, to: usize) {
        let (a, b) = (from.min(to), from.max(to));
        for p in photos.iter().skip(a).take(b + 1 - a) {
            self.paths.insert(p.path.clone());
        }
    }

    /// Forgets paths that are no longer in `photos`.
    pub fn retain_in(&mut self, photos: &[Photo]) {
        let keep: BTreeSet<&Path> = photos.iter().map(|p| p.path.as_path()).collect();
        self.paths.retain(|p| keep.contains(p.as_path()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(name: &str, mtime: i64) -> Photo {
        Photo { path: PathBuf::from(format!("/p/{name}")), mtime, size: 1 }
    }

    // 2026-10-04 00:23:15 UTC
    const NOW: i64 = 1_791_073_395;

    #[test]
    fn date_ranges_keep_the_photos_inside_them() {
        // Screenshots and Recents moved to the smart album model (see models.rs); the range filter stays here.
        let photos = [photo("IMG_1.jpg", NOW - 40 * DAY), photo("Screenshot-2026.png", NOW - 40 * DAY), photo("screenshot_2.png", NOW - DAY), photo("IMG_2.jpg", NOW - 29 * DAY), photo("future.jpg", NOW + 10 * DAY)];
        assert_eq!(in_range(&photos, (NOW - 2 * DAY, NOW)).len(), 1);
    }

    #[test]
    fn civil_day_numbers_match_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(2026, 10, 4) * DAY, NOW - 1395);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert!(valid_day(2000, 2, 29) && !valid_day(2026, 2, 29) && !valid_day(2026, 4, 31) && !valid_day(2026, 13, 1));
    }

    #[test]
    fn date_queries_parse_to_local_ranges() {
        let oct = (days_from_civil(2026, 10, 1) * DAY, days_from_civil(2026, 11, 1) * DAY);
        assert_eq!(parse_date_query("2026-10", 0, NOW), Some(oct));
        assert_eq!(parse_date_query("October 2026", 0, NOW), Some(oct));
        assert_eq!(parse_date_query("oct 2026", 0, NOW), Some(oct));
        assert_eq!(parse_date_query("  OCT  2026 ", 0, NOW), Some(oct));
        let d4 = (days_from_civil(2026, 10, 4) * DAY, days_from_civil(2026, 10, 5) * DAY);
        assert_eq!(parse_date_query("2026-10-04", 0, NOW), Some(d4));
        assert_eq!(parse_date_query("4 October 2026", 0, NOW), Some(d4));
        assert_eq!(parse_date_query("4 oct", 0, NOW), Some(d4), "the year defaults to now's");
        assert_eq!(parse_date_query("2026", 0, NOW), Some((days_from_civil(2026, 1, 1) * DAY, days_from_civil(2027, 1, 1) * DAY)));
        let dec = parse_date_query("2026/12", 0, NOW).unwrap();
        assert_eq!(dec.1, days_from_civil(2027, 1, 1) * DAY, "December rolls into the next year");
        // an hour east of UTC, local midnight is an hour earlier
        assert_eq!(parse_date_query("2026-10-04", 3600, NOW).unwrap().0, d4.0 - 3600);
        for bad in ["", "hello", "2026-13", "2026-02-30", "32 oct", "foo 2026", "1 2 3 4", "99", "ma 2026"] {
            assert_eq!(parse_date_query(bad, 0, NOW), None, "{bad:?}");
        }
    }

    #[test]
    fn month_filters_list_months_newest_first_and_match_sections() {
        let photos = [photo("a", 1_790_000_000), photo("b", 1_790_811_000), photo("c", 1_790_907_795)];
        let f = month_filters(&photos, 0);
        assert_eq!(f.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["October 2026", "September 2026"]);
        assert_eq!(in_range(&photos, f[0].1).len(), 1);
        assert_eq!(in_range(&photos, f[1].1).len(), 2);
        assert!(month_filters(&[], 0).is_empty());
    }

    #[test]
    fn trash_is_purged_after_thirty_days() {
        assert!(!purge_due(NOW - 29 * DAY, NOW));
        assert!(purge_due(NOW - 30 * DAY, NOW));
        assert_eq!(days_left(NOW, NOW), 30);
        assert_eq!(days_left(NOW - 10 * DAY - 5, NOW), 20);
        assert_eq!(days_left(NOW - 90 * DAY, NOW), 0);
        assert_eq!(days_left(NOW + 5 * DAY, NOW), 30, "a clock step back does not extend it");
    }

    #[test]
    fn selection_toggles_and_follows_the_library() {
        let photos = [photo("a", 1), photo("b", 2), photo("c", 3)];
        let mut s = Selection::default();
        assert!(!s.active && s.is_empty());
        s.begin();
        assert!(s.active);
        assert!(s.toggle(&photos[1].path));
        assert!(s.toggle(&photos[0].path));
        assert_eq!(s.len(), 2);
        assert_eq!(s.paths(), [photos[0].path.clone(), photos[1].path.clone()]);
        assert!(!s.toggle(&photos[1].path) && !s.contains(&photos[1].path));
        s.select_all(&photos);
        assert_eq!(s.len(), 3);
        s.retain_in(&photos[..2]);
        assert_eq!(s.len(), 2);
        s.end();
        assert!(!s.active && s.is_empty());
    }

    #[test]
    fn select_all_toggles_and_ranges_extend() {
        let photos = [photo("a", 1), photo("b", 2), photo("c", 3), photo("d", 4)];
        let mut s = Selection::default();
        s.begin();
        assert!(s.toggle_all(&photos) && s.len() == 4);
        assert!(!s.toggle_all(&photos) && s.is_empty(), "a second call deselects");
        s.toggle(&photos[0].path);
        assert!(s.toggle_all(&photos) && s.len() == 4, "a partial selection completes");
        assert!(!s.toggle_all(&[]) && s.len() == 4, "nothing to select");
        s.end();
        s.begin();
        s.select_range(&photos, 3, 1);
        assert_eq!(s.paths(), [photos[1].path.clone(), photos[2].path.clone(), photos[3].path.clone()]);
        s.select_range(&photos, 0, 0);
        assert_eq!(s.len(), 4, "the range adds to the selection");
        s.select_range(&photos, 9, 12);
        assert_eq!(s.len(), 4, "a range past the end adds nothing");
    }
}

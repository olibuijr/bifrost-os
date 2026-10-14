//! Smart albums and the Filter By choices. A smart album is a name and a few conditions (date, favourite, edited,
//! rating, file type, keyword, file name) that a photo must all meet, or any of them meet. The album's list is
//! worked out again whenever the library changes, so it is never stored. Albums are kept one per settings key
//! (`sa.<n>`), the same way the user albums are.

use super::library::Photo;
use super::meta::Meta;
use crate::shell::prefs::{escape_field, unescape_field};

/// The most smart albums.
pub const MAX_SMART: usize = 8;
/// The most conditions in one album.
pub const MAX_CONDS: usize = 6;
const DAY: i64 = 86_400;

/// What a rule needs to know about a photo besides the photo itself.
pub struct Facts<'a> {
    pub favourite: bool,
    pub edited: bool,
    pub rating: u8,
    pub meta: &'a Meta,
}

/// One condition of a smart album.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    Favourite(bool),
    Edited(bool),
    /// at least this many stars
    Rating(u8),
    /// taken within the last this many days
    Recent(u32),
    /// taken in this calendar year
    Year(i32),
    /// the file name ends in this extension (lower case, no dot)
    Extension(String),
    /// a keyword that contains this text
    Keyword(String),
    /// a file name that contains this text
    Filename(String),
}

impl Cond {
    pub fn matches(&self, p: &Photo, f: &Facts, now: i64, offset: i32) -> bool {
        match self {
            Cond::Favourite(v) => f.favourite == *v,
            Cond::Edited(v) => f.edited == *v,
            Cond::Rating(n) => f.rating >= *n,
            Cond::Recent(days) => p.mtime <= now && now - p.mtime < *days as i64 * DAY,
            Cond::Year(y) => super::library::civil(p.mtime, offset).year == *y,
            Cond::Extension(e) => p.name().to_ascii_lowercase().rsplit_once('.').is_some_and(|(_, x)| x == e),
            Cond::Keyword(k) => {
                let k = k.to_lowercase();
                f.meta.keywords.iter().any(|w| w.to_lowercase().contains(&k))
            }
            Cond::Filename(t) => p.name().to_lowercase().contains(&t.to_lowercase()),
        }
    }

    /// The row text of the editor: "Favorite is yes".
    pub fn describe(&self) -> String {
        let yes = |v: bool| if v { "yes" } else { "no" };
        match self {
            Cond::Favourite(v) => format!("Favorite is {}", yes(*v)),
            Cond::Edited(v) => format!("Edited is {}", yes(*v)),
            Cond::Rating(n) => format!("Rating is {n}+ stars"),
            Cond::Recent(d) => format!("Taken in the last {d} days"),
            Cond::Year(y) => format!("Taken in {y}"),
            Cond::Extension(e) => format!("File type is {}", e.to_uppercase()),
            Cond::Keyword(k) => format!("Keyword contains {k}"),
            Cond::Filename(t) => format!("Filename contains {t}"),
        }
    }

    fn encode(&self) -> String {
        match self {
            Cond::Favourite(v) => format!("fav={}", u8::from(*v)),
            Cond::Edited(v) => format!("edit={}", u8::from(*v)),
            Cond::Rating(n) => format!("rate={n}"),
            Cond::Recent(d) => format!("recent={d}"),
            Cond::Year(y) => format!("year={y}"),
            Cond::Extension(e) => format!("ext={}", escape_cond(e)),
            Cond::Keyword(k) => format!("kw={}", escape_cond(k)),
            Cond::Filename(t) => format!("name={}", escape_cond(t)),
        }
    }

    fn decode(text: &str) -> Option<Cond> {
        let (k, v) = text.split_once('=')?;
        Some(match k {
            "fav" => Cond::Favourite(v == "1"),
            "edit" => Cond::Edited(v == "1"),
            "rate" => Cond::Rating(v.parse().ok().filter(|n| (1..=5).contains(n))?),
            "recent" => Cond::Recent(v.parse().ok().filter(|d| *d > 0)?),
            "year" => Cond::Year(v.parse().ok()?),
            "ext" => Cond::Extension(unescape_cond(v).to_ascii_lowercase()).nonempty()?,
            "kw" => Cond::Keyword(unescape_cond(v)).nonempty()?,
            "name" => Cond::Filename(unescape_cond(v)).nonempty()?,
            _ => return None,
        })
    }

    /// The condition, or None when its text is empty.
    fn nonempty(self) -> Option<Cond> {
        let empty = matches!(&self, Cond::Extension(t) | Cond::Keyword(t) | Cond::Filename(t) if t.is_empty());
        (!empty).then_some(self)
    }
}

/// Conditions are joined by `|`, so the text inside one cannot hold it.
fn escape_cond(text: &str) -> String {
    text.replace('%', "%25").replace('|', "%7C")
}

fn unescape_cond(text: &str) -> String {
    text.replace("%7C", "|").replace("%25", "%")
}

/// A smart album: a name and its conditions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Smart {
    pub name: String,
    /// any condition is enough (otherwise all must be met)
    pub any: bool,
    pub conds: Vec<Cond>,
}

impl Smart {
    pub fn new(name: &str) -> Smart {
        Smart { name: name.to_string(), any: false, conds: vec![] }
    }

    /// Does `p` belong. An album without conditions holds nothing, so a new one starts empty rather than as a copy
    /// of All Photos.
    pub fn matches(&self, p: &Photo, f: &Facts, now: i64, offset: i32) -> bool {
        if self.conds.is_empty() {
            return false;
        }
        if self.any {
            self.conds.iter().any(|c| c.matches(p, f, now, offset))
        } else {
            self.conds.iter().all(|c| c.matches(p, f, now, offset))
        }
    }

    /// Adds a condition, or replaces the one of the same kind (an album has one favourite rule, one rating rule...).
    /// False when the album already has the most.
    pub fn set(&mut self, cond: Cond) -> bool {
        match self.conds.iter().position(|c| std::mem::discriminant(c) == std::mem::discriminant(&cond)) {
            Some(i) => {
                self.conds[i] = cond;
                true
            }
            None if self.conds.len() < MAX_CONDS => {
                self.conds.push(cond);
                true
            }
            None => false,
        }
    }

    /// Drops the condition of the same kind as `like`.
    pub fn clear(&mut self, like: &Cond) {
        self.conds.retain(|c| std::mem::discriminant(c) != std::mem::discriminant(like));
    }

    /// The stored line: name, `all` or `any`, then the conditions.
    pub fn encode(&self) -> String {
        format!("{}\t{}\t{}", escape_field(&self.name), if self.any { "any" } else { "all" }, self.conds.iter().map(Cond::encode).collect::<Vec<_>>().join("|"))
    }

    pub fn decode(text: &str) -> Option<Smart> {
        let mut f = text.split('\t');
        let name = unescape_field(f.next()?, 80)?;
        if name.is_empty() {
            return None;
        }
        let any = f.next()? == "any";
        let conds = f.next().unwrap_or("").split('|').filter_map(Cond::decode).take(MAX_CONDS).collect();
        Some(Smart { name, any, conds })
    }
}

/// View > Filter By.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum KindFilter {
    #[default]
    All,
    Favourites,
    Edited,
    Photos,
    Videos,
    Rated,
    Keyworded,
}

impl KindFilter {
    pub const ALL: [KindFilter; 7] = [KindFilter::All, KindFilter::Favourites, KindFilter::Edited, KindFilter::Photos, KindFilter::Videos, KindFilter::Rated, KindFilter::Keyworded];

    pub fn label(self) -> &'static str {
        match self {
            KindFilter::All => "All Items",
            KindFilter::Favourites => "Favorites",
            KindFilter::Edited => "Edited",
            KindFilter::Photos => "Photos",
            KindFilter::Videos => "Videos",
            KindFilter::Rated => "Rated",
            KindFilter::Keyworded => "With Keywords",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            KindFilter::All => "all",
            KindFilter::Favourites => "fav",
            KindFilter::Edited => "edited",
            KindFilter::Photos => "photos",
            KindFilter::Videos => "videos",
            KindFilter::Rated => "rated",
            KindFilter::Keyworded => "kw",
        }
    }

    pub fn from_key(key: &str) -> KindFilter {
        KindFilter::ALL.into_iter().find(|k| k.key() == key).unwrap_or_default()
    }

    pub fn matches(self, p: &Photo, f: &Facts) -> bool {
        let video = is_video(&p.name());
        match self {
            KindFilter::All => true,
            KindFilter::Favourites => f.favourite,
            KindFilter::Edited => f.edited,
            KindFilter::Photos => !video,
            KindFilter::Videos => video,
            KindFilter::Rated => f.rating > 0,
            KindFilter::Keyworded => !f.meta.keywords.is_empty(),
        }
    }
}

/// A file name with a video extension.
pub fn is_video(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["mp4", "mov", "m4v", "mkv", "webm", "avi"].iter().any(|e| n.rsplit_once('.').is_some_and(|(_, x)| x == *e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NOW: i64 = 1_790_000_000;

    fn photo(name: &str, age_days: i64) -> Photo {
        Photo { path: PathBuf::from(format!("/p/{name}")), mtime: NOW - age_days * DAY, size: 1 }
    }

    fn facts<'a>(meta: &'a Meta, favourite: bool, edited: bool, rating: u8) -> Facts<'a> {
        Facts { favourite, edited, rating, meta }
    }

    #[test]
    fn every_condition_kind_matches_what_it_says() {
        let m = Meta { keywords: vec!["Iceland".into()], ..Meta::default() };
        let f = facts(&m, true, false, 4);
        let p = photo("Trip_01.JPG", 3);
        assert!(Cond::Favourite(true).matches(&p, &f, NOW, 0));
        assert!(!Cond::Edited(true).matches(&p, &f, NOW, 0));
        assert!(Cond::Rating(4).matches(&p, &f, NOW, 0));
        assert!(!Cond::Rating(5).matches(&p, &f, NOW, 0));
        assert!(Cond::Recent(7).matches(&p, &f, NOW, 0));
        assert!(!Cond::Recent(2).matches(&p, &f, NOW, 0));
        assert!(Cond::Extension("jpg".into()).matches(&p, &f, NOW, 0));
        assert!(Cond::Keyword("ice".into()).matches(&p, &f, NOW, 0));
        assert!(Cond::Filename("trip".into()).matches(&p, &f, NOW, 0));
        assert!(!Cond::Filename("beach".into()).matches(&p, &f, NOW, 0));
        let year = super::super::library::civil(p.mtime, 0).year;
        assert!(Cond::Year(year).matches(&p, &f, NOW, 0));
    }

    #[test]
    fn all_and_any_combine_the_conditions() {
        let m = Meta::default();
        let f = facts(&m, true, false, 0);
        let p = photo("a.png", 1);
        let mut s = Smart::new("Mixed");
        assert!(!s.matches(&p, &f, NOW, 0), "no conditions, no photos");
        s.set(Cond::Favourite(true));
        s.set(Cond::Rating(3));
        assert!(!s.matches(&p, &f, NOW, 0));
        s.any = true;
        assert!(s.matches(&p, &f, NOW, 0));
    }

    #[test]
    fn setting_a_kind_again_replaces_it_and_the_count_is_bounded() {
        let mut s = Smart::new("x");
        s.set(Cond::Rating(2));
        s.set(Cond::Rating(5));
        assert_eq!(s.conds, vec![Cond::Rating(5)]);
        s.clear(&Cond::Rating(1));
        assert!(s.conds.is_empty());
        for c in [Cond::Favourite(true), Cond::Edited(true), Cond::Rating(1), Cond::Recent(7), Cond::Year(2026), Cond::Keyword("a".into())] {
            assert!(s.set(c));
        }
        assert!(!s.set(Cond::Filename("z".into())), "an album holds at most six conditions");
    }

    #[test]
    fn a_smart_album_survives_its_stored_line() {
        let mut s = Smart::new("Best of | 2026");
        s.any = true;
        s.set(Cond::Keyword("a|b%".into()));
        s.set(Cond::Rating(4));
        s.set(Cond::Extension("png".into()));
        let line = s.encode();
        assert!(!line.contains('\n'));
        assert_eq!(Smart::decode(&line), Some(s));
        assert_eq!(Smart::decode(""), None);
        assert_eq!(Smart::decode("\tall\tfav=1"), None, "an album needs a name");
    }

    #[test]
    fn filter_by_picks_favourites_edited_and_kinds() {
        let m = Meta::default();
        let mk = Meta { keywords: vec!["x".into()], ..Meta::default() };
        let img = photo("a.jpg", 1);
        let mov = photo("clip.MOV", 1);
        assert!(KindFilter::All.matches(&img, &facts(&m, false, false, 0)));
        assert!(KindFilter::Favourites.matches(&img, &facts(&m, true, false, 0)));
        assert!(!KindFilter::Favourites.matches(&img, &facts(&m, false, false, 0)));
        assert!(KindFilter::Edited.matches(&img, &facts(&m, false, true, 0)));
        assert!(KindFilter::Photos.matches(&img, &facts(&m, false, false, 0)));
        assert!(!KindFilter::Photos.matches(&mov, &facts(&m, false, false, 0)));
        assert!(KindFilter::Videos.matches(&mov, &facts(&m, false, false, 0)));
        assert!(KindFilter::Rated.matches(&img, &facts(&m, false, false, 2)));
        assert!(KindFilter::Keyworded.matches(&img, &facts(&mk, false, false, 0)));
        for k in KindFilter::ALL {
            assert_eq!(KindFilter::from_key(k.key()), k);
        }
        assert_eq!(KindFilter::from_key("nonsense"), KindFilter::All);
    }
}

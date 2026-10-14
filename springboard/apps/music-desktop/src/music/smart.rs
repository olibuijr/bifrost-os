//! Smart playlists: a rule set over the library that is evaluated whenever the library or the marks change, so the
//! list stays current. Rules test a song's tags, its date added, and what the listener marked or played
//! (`Facts`, taken from `marks`). A limit and a sort order cut the result. Rules are stored one per line.
#![allow(dead_code)] // the New Smart Playlist row and the rules editor sheet wire this in next

use super::library::{sort_key, Song};

/// What the listener marked or did with a song (from `marks::Marks`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// 0 unrated, else 1 to 5
    pub rating: u8,
    pub favourite: bool,
    /// Unix seconds of the last play, if ever played
    pub last_played: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Title(Text),
    Artist(Text),
    Album(Text),
    YearIs(u32),
    YearBetween(u32, u32),
    RatingAtLeast(u8),
    Favourite(bool),
    /// Added within the last n days.
    AddedWithin(u32),
    /// Played within the last n days.
    PlayedWithin(u32),
    NeverPlayed,
    /// Longer than this many seconds.
    LongerThan(u32),
    ShorterThan(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Text {
    Contains(String),
    Is(String),
    BeginsWith(String),
    DoesNotContain(String),
}

impl Text {
    fn matches(&self, value: &str) -> bool {
        let v = value.to_lowercase();
        match self {
            Text::Contains(s) => v.contains(&s.to_lowercase()),
            Text::Is(s) => v == s.to_lowercase(),
            Text::BeginsWith(s) => v.starts_with(&s.to_lowercase()),
            Text::DoesNotContain(s) => !v.contains(&s.to_lowercase()),
        }
    }
}

impl Rule {
    pub fn matches(&self, song: &Song, facts: &Facts, now: u64) -> bool {
        let days = |n: u32| u64::from(n) * 86_400;
        match self {
            Rule::Title(t) => t.matches(&song.title),
            Rule::Artist(t) => t.matches(&song.artist) || t.matches(&song.album_artist) && !song.album_artist.is_empty(),
            Rule::Album(t) => t.matches(&song.album),
            Rule::YearIs(y) => song.year == Some(*y),
            Rule::YearBetween(a, b) => song.year.is_some_and(|y| y >= *a && y <= *b),
            Rule::RatingAtLeast(n) => facts.rating >= *n,
            Rule::Favourite(on) => facts.favourite == *on,
            Rule::AddedWithin(n) => song.added + days(*n) >= now,
            Rule::PlayedWithin(n) => facts.last_played.is_some_and(|p| p + days(*n) >= now),
            Rule::NeverPlayed => facts.last_played.is_none(),
            Rule::LongerThan(s) => song.duration_ms.is_some_and(|d| d > f64::from(*s) * 1000.0),
            Rule::ShorterThan(s) => song.duration_ms.is_some_and(|d| d < f64::from(*s) * 1000.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Title,
    Artist,
    Album,
    /// Newest first
    DateAdded,
    /// Highest first
    Rating,
    /// In the library's order, then cut at random-looking but repeatable positions (by id hash)
    Shuffled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SmartPlaylist {
    pub name: String,
    /// All rules must match when true, any one when false.
    pub all: bool,
    pub rules: Vec<Rule>,
    /// Keep at most this many songs; 0 means no limit.
    pub limit: usize,
    pub order: Order,
}

impl SmartPlaylist {
    pub fn new(name: impl Into<String>) -> Self {
        SmartPlaylist { name: name.into(), all: true, rules: vec![], limit: 0, order: Order::Title }
    }

    /// The song ids (indices into `songs`) the playlist holds now. `facts(song)` gives the listener's marks.
    pub fn select(&self, songs: &[Song], facts: impl Fn(&Song) -> Facts, now: u64) -> Vec<usize> {
        if self.rules.is_empty() {
            return vec![];
        }
        let mut hits: Vec<(usize, Facts)> = songs
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let f = facts(s);
                let ok = if self.all {
                    self.rules.iter().all(|r| r.matches(s, &f, now))
                } else {
                    self.rules.iter().any(|r| r.matches(s, &f, now))
                };
                ok.then_some((i, f))
            })
            .collect();
        match self.order {
            Order::Title => hits.sort_by_key(|(i, _)| sort_key(&songs[*i].title)),
            Order::Artist => hits.sort_by_key(|(i, _)| (sort_key(&songs[*i].artist), songs[*i].track)),
            Order::Album => hits.sort_by_key(|(i, _)| (sort_key(&songs[*i].album), songs[*i].track)),
            Order::DateAdded => hits.sort_by_key(|(i, _)| std::cmp::Reverse(songs[*i].added)),
            Order::Rating => hits.sort_by_key(|(_, f)| std::cmp::Reverse(f.rating)),
            Order::Shuffled => hits.sort_by_key(|(i, _)| (*i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 11),
        }
        let mut ids: Vec<usize> = hits.into_iter().map(|(i, _)| i).collect();
        if self.limit > 0 {
            ids.truncate(self.limit);
        }
        ids
    }

    /// The total length in seconds of the selected songs.
    pub fn total_seconds(&self, songs: &[Song], ids: &[usize]) -> f64 {
        ids.iter().filter_map(|i| songs.get(*i)).filter_map(|s| s.duration_ms).sum::<f64>() / 1000.0
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn text_field(kind: &str, t: &Text) -> String {
    let (op, v) = match t {
        Text::Contains(v) => ("contains", v),
        Text::Is(v) => ("is", v),
        Text::BeginsWith(v) => ("begins", v),
        Text::DoesNotContain(v) => ("not", v),
    };
    format!("{kind}.{op}={}", esc(v))
}

impl SmartPlaylist {
    /// One block of lines: `name`, `all`, `limit`, `order`, then one `rule` per line, ending with `end`.
    pub fn to_text(&self) -> String {
        let order = match self.order {
            Order::Title => "title",
            Order::Artist => "artist",
            Order::Album => "album",
            Order::DateAdded => "added",
            Order::Rating => "rating",
            Order::Shuffled => "shuffled",
        };
        let mut out = format!("name={}\nall={}\nlimit={}\norder={order}\n", esc(&self.name), self.all as u8, self.limit);
        for r in &self.rules {
            let line = match r {
                Rule::Title(t) => text_field("title", t),
                Rule::Artist(t) => text_field("artist", t),
                Rule::Album(t) => text_field("album", t),
                Rule::YearIs(y) => format!("year={y}"),
                Rule::YearBetween(a, b) => format!("years={a}-{b}"),
                Rule::RatingAtLeast(n) => format!("rating>={n}"),
                Rule::Favourite(on) => format!("favourite={}", *on as u8),
                Rule::AddedWithin(n) => format!("added<={n}"),
                Rule::PlayedWithin(n) => format!("played<={n}"),
                Rule::NeverPlayed => "never".to_string(),
                Rule::LongerThan(s) => format!("longer={s}"),
                Rule::ShorterThan(s) => format!("shorter={s}"),
            };
            out.push_str(&format!("rule {line}\n"));
        }
        out.push_str("end\n");
        out
    }

    /// Reads one block written by `to_text`; None when the name is missing or a rule cannot be read.
    pub fn from_text(text: &str) -> Option<SmartPlaylist> {
        let mut p = SmartPlaylist::new("");
        let mut named = false;
        for line in text.lines() {
            if line == "end" {
                break;
            }
            if let Some(rule) = line.strip_prefix("rule ") {
                p.rules.push(parse_rule(rule)?);
            } else if let Some(v) = line.strip_prefix("name=") {
                p.name = unesc(v);
                named = !p.name.trim().is_empty();
            } else if let Some(v) = line.strip_prefix("all=") {
                p.all = v == "1";
            } else if let Some(v) = line.strip_prefix("limit=") {
                p.limit = v.parse().ok()?;
            } else if let Some(v) = line.strip_prefix("order=") {
                p.order = match v {
                    "title" => Order::Title,
                    "artist" => Order::Artist,
                    "album" => Order::Album,
                    "added" => Order::DateAdded,
                    "rating" => Order::Rating,
                    "shuffled" => Order::Shuffled,
                    _ => return None,
                };
            }
        }
        named.then_some(p)
    }
}

fn parse_rule(s: &str) -> Option<Rule> {
    if s == "never" {
        return Some(Rule::NeverPlayed);
    }
    if let Some(v) = s.strip_prefix("rating>=") {
        return Some(Rule::RatingAtLeast(v.parse().ok()?));
    }
    if let Some(v) = s.strip_prefix("added<=") {
        return Some(Rule::AddedWithin(v.parse().ok()?));
    }
    if let Some(v) = s.strip_prefix("played<=") {
        return Some(Rule::PlayedWithin(v.parse().ok()?));
    }
    let (key, value) = s.split_once('=')?;
    let text = |op: &str, value: &str| -> Option<Text> {
        let v = unesc(value);
        Some(match op {
            "contains" => Text::Contains(v),
            "is" => Text::Is(v),
            "begins" => Text::BeginsWith(v),
            "not" => Text::DoesNotContain(v),
            _ => return None,
        })
    };
    match key.split_once('.') {
        Some(("title", op)) => Some(Rule::Title(text(op, value)?)),
        Some(("artist", op)) => Some(Rule::Artist(text(op, value)?)),
        Some(("album", op)) => Some(Rule::Album(text(op, value)?)),
        Some(_) => None,
        None => match key {
            "year" => Some(Rule::YearIs(value.parse().ok()?)),
            "years" => {
                let (a, b) = value.split_once('-')?;
                Some(Rule::YearBetween(a.parse().ok()?, b.parse().ok()?))
            }
            "favourite" => Some(Rule::Favourite(value == "1")),
            "longer" => Some(Rule::LongerThan(value.parse().ok()?)),
            "shorter" => Some(Rule::ShorterThan(value.parse().ok()?)),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn song(id: usize, title: &str, artist: &str, year: u32, added: u64, secs: f64) -> Song {
        Song {
            id,
            path: PathBuf::from(format!("/m/{id}.mp3")),
            title: title.into(),
            artist: artist.into(),
            album: format!("Album {id}"),
            album_artist: String::new(),
            track: Some(id as u32),
            year: Some(year),
            duration_ms: Some(secs * 1000.0),
            has_cover: false,
            added,
        }
    }

    const NOW: u64 = 10_000_000;
    const DAY: u64 = 86_400;

    fn library() -> Vec<Song> {
        vec![
            song(0, "Alpha", "Ann", 1999, NOW - 2 * DAY, 200.0),
            song(1, "Beta", "Bob", 2005, NOW - 40 * DAY, 400.0),
            song(2, "Gamma", "Ann", 2015, NOW - DAY, 100.0),
        ]
    }

    fn no_facts(_: &Song) -> Facts {
        Facts::default()
    }

    #[test]
    fn text_and_year_rules() {
        let songs = library();
        let mut p = SmartPlaylist::new("Ann 2000s");
        p.rules = vec![Rule::Artist(Text::Is("ann".into())), Rule::YearBetween(2000, 2020)];
        assert_eq!(p.select(&songs, no_facts, NOW), vec![2]);
        p.all = false;
        assert_eq!(p.select(&songs, no_facts, NOW), vec![0, 1, 2]);
    }

    #[test]
    fn date_and_marks_rules() {
        let songs = library();
        let facts = |s: &Song| Facts {
            rating: if s.id == 1 { 5 } else { 0 },
            favourite: s.id == 0,
            last_played: (s.id == 2).then_some(NOW - 3 * DAY),
        };
        let mut p = SmartPlaylist::new("x");
        p.rules = vec![Rule::AddedWithin(7)];
        assert_eq!(p.select(&songs, facts, NOW), vec![0, 2]);
        p.rules = vec![Rule::RatingAtLeast(4)];
        assert_eq!(p.select(&songs, facts, NOW), vec![1]);
        p.rules = vec![Rule::Favourite(true)];
        assert_eq!(p.select(&songs, facts, NOW), vec![0]);
        p.rules = vec![Rule::PlayedWithin(7)];
        assert_eq!(p.select(&songs, facts, NOW), vec![2]);
        p.rules = vec![Rule::NeverPlayed];
        assert_eq!(p.select(&songs, facts, NOW), vec![0, 1]);
    }

    #[test]
    fn order_and_limit() {
        let songs = library();
        let mut p = SmartPlaylist::new("x");
        p.rules = vec![Rule::LongerThan(50)];
        p.order = Order::DateAdded;
        assert_eq!(p.select(&songs, no_facts, NOW), vec![2, 0, 1]);
        p.limit = 2;
        assert_eq!(p.select(&songs, no_facts, NOW), vec![2, 0]);
        p.limit = 0;
        p.order = Order::Title;
        assert_eq!(p.select(&songs, no_facts, NOW), vec![0, 1, 2]);
        p.rules = vec![Rule::ShorterThan(150)];
        let ids = p.select(&songs, no_facts, NOW);
        assert_eq!(ids, vec![2]);
        assert_eq!(p.total_seconds(&songs, &ids), 100.0);
    }

    #[test]
    fn empty_rule_set_selects_nothing() {
        assert!(SmartPlaylist::new("x").select(&library(), no_facts, NOW).is_empty());
    }

    #[test]
    fn text_round_trip() {
        let mut p = SmartPlaylist::new("Mix\twith\\odd");
        p.all = false;
        p.limit = 25;
        p.order = Order::Rating;
        p.rules = vec![
            Rule::Title(Text::Contains("love".into())),
            Rule::Artist(Text::DoesNotContain("a=b".into())),
            Rule::Album(Text::BeginsWith("The".into())),
            Rule::YearIs(1999),
            Rule::YearBetween(1990, 1999),
            Rule::RatingAtLeast(3),
            Rule::Favourite(false),
            Rule::AddedWithin(30),
            Rule::PlayedWithin(7),
            Rule::NeverPlayed,
            Rule::LongerThan(60),
            Rule::ShorterThan(600),
        ];
        assert_eq!(SmartPlaylist::from_text(&p.to_text()), Some(p));
        assert_eq!(SmartPlaylist::from_text("name=x\nrule bogus\nend\n"), None);
        assert_eq!(SmartPlaylist::from_text("all=1\nend\n"), None);
    }
}

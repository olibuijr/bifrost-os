//! Smart playlists: a list defined by rules over the library that fills itself. A playlist holds a match mode (all
//! rules or any rule) and rules of the form field, operator, value; the songs it shows are worked out on demand from
//! the library and the listening data, so it follows added files, new ratings and fresh plays at once.
//!
//! Each smart playlist is a small text file, `<dir>/<name>.smart`:
//!
//! ```text
//! smart-playlist 1
//! match all
//! rule<TAB>artist<TAB>contains<TAB>abba
//! rule<TAB>rating<TAB>greater<TAB>3
//! ```
//!
//! Pure file and rule logic, no UI.

use super::library::Song;
use super::playlists::clean_name;
use super::stats::Stats;
use std::path::PathBuf;

const SECONDS_PER_DAY: u64 = 86_400;

/// How a field's value is compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// words in a tag
    Text,
    /// a whole number
    Number,
    /// yes or no
    Flag,
}

/// What a rule looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Artist,
    Album,
    Year,
    Rating,
    Plays,
    /// whole days since the file was added
    DaysSinceAdded,
    /// whole days since the song last started; a song that never played counts as infinitely long ago
    DaysSincePlayed,
    Favourite,
}

impl Field {
    pub const ALL: [Field; 9] = [Field::Title, Field::Artist, Field::Album, Field::Year, Field::Rating, Field::Plays, Field::DaysSinceAdded, Field::DaysSincePlayed, Field::Favourite];

    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::Artist => "Artist",
            Field::Album => "Album",
            Field::Year => "Year",
            Field::Rating => "Rating",
            Field::Plays => "Plays",
            Field::DaysSinceAdded => "Days Since Added",
            Field::DaysSincePlayed => "Days Since Played",
            Field::Favourite => "Favourite",
        }
    }

    /// The word the file stores the field under.
    pub fn key(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Artist => "artist",
            Field::Album => "album",
            Field::Year => "year",
            Field::Rating => "rating",
            Field::Plays => "plays",
            Field::DaysSinceAdded => "added",
            Field::DaysSincePlayed => "played",
            Field::Favourite => "favourite",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn kind(self) -> Kind {
        match self {
            Field::Title | Field::Artist | Field::Album => Kind::Text,
            Field::Favourite => Kind::Flag,
            _ => Kind::Number,
        }
    }

    /// The operators that make sense for the field, first one the default.
    pub fn ops(self) -> &'static [Op] {
        match self.kind() {
            Kind::Text => &[Op::Contains, Op::DoesNotContain, Op::Is, Op::IsNot, Op::StartsWith],
            Kind::Number => &[Op::Is, Op::IsNot, Op::Greater, Op::Less],
            Kind::Flag => &[Op::Is],
        }
    }

    /// The value a fresh rule on the field starts with.
    pub fn default_value(self) -> &'static str {
        match self {
            Field::Rating => "3",
            Field::DaysSinceAdded | Field::DaysSincePlayed => "30",
            Field::Year => "2000",
            Field::Plays => "0",
            Field::Favourite => "1",
            Field::Title | Field::Artist | Field::Album => "",
        }
    }

    /// The next field in the list, wrapping.
    pub fn next(self) -> Field {
        let i = Field::ALL.iter().position(|f| *f == self).map_or(0, |i| i + 1);
        Field::ALL[i % Field::ALL.len()]
    }
}

/// How a rule compares the song's value with the rule's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Contains,
    DoesNotContain,
    Is,
    IsNot,
    StartsWith,
    Greater,
    Less,
}

impl Op {
    pub fn label(self) -> &'static str {
        match self {
            Op::Contains => "contains",
            Op::DoesNotContain => "does not contain",
            Op::Is => "is",
            Op::IsNot => "is not",
            Op::StartsWith => "starts with",
            Op::Greater => "is greater than",
            Op::Less => "is less than",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Op::Contains => "contains",
            Op::DoesNotContain => "excludes",
            Op::Is => "is",
            Op::IsNot => "isnot",
            Op::StartsWith => "starts",
            Op::Greater => "greater",
            Op::Less => "less",
        }
    }

    pub fn from_key(key: &str) -> Option<Op> {
        [Op::Contains, Op::DoesNotContain, Op::Is, Op::IsNot, Op::StartsWith, Op::Greater, Op::Less].into_iter().find(|o| o.key() == key)
    }
}

/// One condition: `field op value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub field: Field,
    pub op: Op,
    pub value: String,
}

impl Rule {
    /// A rule on `field` with its first operator and its starting value.
    pub fn new(field: Field) -> Rule {
        Rule { field, op: field.ops()[0], value: field.default_value().to_string() }
    }

    /// Moves the rule to the next field; the operator and the value are kept when they still make sense.
    pub fn cycle_field(&mut self) {
        let next = self.field.next();
        let kind_changed = next.kind() != self.field.kind();
        self.field = next;
        if !next.ops().contains(&self.op) {
            self.op = next.ops()[0];
        }
        if kind_changed || self.value.is_empty() {
            self.value = next.default_value().to_string();
        }
    }

    /// Moves the rule to the field's next operator.
    pub fn cycle_op(&mut self) {
        let ops = self.field.ops();
        let at = ops.iter().position(|o| *o == self.op).map_or(0, |i| (i + 1) % ops.len());
        self.op = ops[at];
    }

    /// Flips a yes or no value.
    pub fn toggle_flag(&mut self) {
        self.value = if self.value == "1" { "0".to_string() } else { "1".to_string() };
    }

    /// The value as the editor shows it.
    pub fn value_text(&self) -> String {
        if self.field.kind() == Kind::Flag {
            let answer = if self.value == "1" { "Yes" } else { "No" };
            return answer.to_string();
        }
        self.value.clone()
    }

    /// Whether `song` satisfies the rule. `now` is Unix seconds. A rule with no usable value (an empty text, a number
    /// that does not parse) keeps every song, so a rule that is still being typed does not empty the list.
    pub fn matches(&self, song: &Song, stats: &Stats, now: u64) -> bool {
        match self.field.kind() {
            Kind::Text => {
                let needle = self.value.trim().to_lowercase();
                if needle.is_empty() {
                    return true;
                }
                let hay = match self.field {
                    Field::Title => &song.title,
                    Field::Artist => &song.artist,
                    _ => &song.album,
                }
                .to_lowercase();
                match self.op {
                    Op::Contains => hay.contains(&needle),
                    Op::DoesNotContain => !hay.contains(&needle),
                    Op::Is => hay == needle,
                    Op::IsNot => hay != needle,
                    Op::StartsWith => hay.starts_with(&needle),
                    Op::Greater | Op::Less => true,
                }
            }
            Kind::Number => {
                let Ok(want) = self.value.trim().parse::<i64>() else { return true };
                let have = match self.field {
                    Field::Year => song.year.map_or(0, i64::from),
                    Field::Rating => i64::from(stats.rating(&song.path)),
                    Field::Plays => i64::from(stats.plays(&song.path)),
                    Field::DaysSinceAdded => (now.saturating_sub(song.added) / SECONDS_PER_DAY) as i64,
                    _ => match stats.last_played(&song.path) {
                        0 => i64::MAX,
                        last => (now.saturating_sub(last) / SECONDS_PER_DAY) as i64,
                    },
                };
                match self.op {
                    Op::Is | Op::Contains | Op::StartsWith => have == want,
                    Op::IsNot | Op::DoesNotContain => have != want,
                    Op::Greater => have > want,
                    Op::Less => have < want,
                }
            }
            Kind::Flag => stats.is_favourite(&song.path) == (self.value == "1"),
        }
    }
}

/// A playlist defined by rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartPlaylist {
    pub name: String,
    /// every rule must match (true) or any one rule (false)
    pub all: bool,
    pub rules: Vec<Rule>,
}

impl SmartPlaylist {
    /// A playlist with one starting rule, on the artist.
    pub fn new(name: &str) -> Self {
        SmartPlaylist { name: name.to_string(), all: true, rules: vec![Rule::new(Field::Artist)] }
    }

    /// Whether `song` belongs to the playlist.
    pub fn matches(&self, song: &Song, stats: &Stats, now: u64) -> bool {
        if self.all {
            self.rules.iter().all(|r| r.matches(song, stats, now))
        } else {
            self.rules.iter().any(|r| r.matches(song, stats, now))
        }
    }

    /// The ids of the songs that belong, in library order.
    pub fn select(&self, songs: &[Song], stats: &Stats, now: u64) -> Vec<usize> {
        songs.iter().filter(|s| self.matches(s, stats, now)).map(|s| s.id).collect()
    }

    /// The text saved to disk.
    pub fn to_text(&self) -> String {
        let mut out = format!("smart-playlist 1\nmatch {}\n", if self.all { "all" } else { "any" });
        for r in &self.rules {
            let value: String = r.value.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            out.push_str(&format!("rule\t{}\t{}\t{}\n", r.field.key(), r.op.key(), value));
        }
        out
    }

    /// Reads a saved playlist; lines it does not understand are skipped.
    pub fn from_text(name: &str, text: &str) -> SmartPlaylist {
        let mut list = SmartPlaylist { name: name.to_string(), all: true, rules: Vec::new() };
        for line in text.lines() {
            if let Some(mode) = line.strip_prefix("match ") {
                list.all = mode.trim() != "any";
            } else if let Some(rest) = line.strip_prefix("rule\t") {
                let mut f = rest.splitn(3, '\t');
                if let (Some(field), Some(op), Some(value)) = (f.next().and_then(Field::from_key), f.next().and_then(Op::from_key), f.next()) {
                    if field.ops().contains(&op) {
                        list.rules.push(Rule { field, op, value: value.to_string() });
                    }
                }
            }
        }
        list
    }
}

/// The smart playlists kept in a folder, by name.
#[derive(Clone, Debug, Default)]
pub struct SmartPlaylists {
    dir: PathBuf,
    lists: Vec<SmartPlaylist>,
}

impl SmartPlaylists {
    /// Reads every smart playlist in `dir` (which may not exist yet).
    pub fn open(dir: PathBuf) -> Self {
        let mut lists = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "smart") {
                    if let (Some(name), Ok(text)) = (p.file_stem().map(|s| s.to_string_lossy().into_owned()), std::fs::read_to_string(&p)) {
                        let mut list = SmartPlaylist::from_text(&name, &text);
                        if list.rules.is_empty() {
                            list.rules.push(Rule::new(Field::Artist));
                        }
                        lists.push(list);
                    }
                }
            }
        }
        lists.sort_by_key(|l| l.name.to_lowercase());
        SmartPlaylists { dir, lists }
    }

    pub fn lists(&self) -> &[SmartPlaylist] {
        &self.lists
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.smart"))
    }

    fn save(&self, i: usize) {
        let Some(l) = self.lists.get(i) else { return };
        let _ = std::fs::create_dir_all(&self.dir);
        let tmp = self.dir.join(format!(".{}.tmp", l.name));
        if std::fs::write(&tmp, l.to_text()).is_ok() {
            let _ = std::fs::rename(&tmp, self.file(&l.name));
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.lists.iter().position(|l| l.name.to_lowercase() == name.to_lowercase())
    }

    /// Makes a playlist with one starting rule; the index it lands at, or None when the name is unusable or taken.
    pub fn create(&mut self, name: &str) -> Option<usize> {
        let name = clean_name(name)?;
        if self.position(&name).is_some() {
            return None;
        }
        self.lists.push(SmartPlaylist::new(&name));
        self.lists.sort_by_key(|l| l.name.to_lowercase());
        let at = self.position(&name)?;
        self.save(at);
        Some(at)
    }

    /// The first free "Smart Playlist", "Smart Playlist 2", ... name.
    pub fn fresh_name(&self) -> String {
        (1..).map(|n| if n == 1 { "Smart Playlist".to_string() } else { format!("Smart Playlist {n}") }).find(|n| self.position(n).is_none()).unwrap_or_default()
    }

    /// Renames playlist `i`; false when the name is unusable or taken by another.
    pub fn rename(&mut self, i: usize, name: &str) -> bool {
        let Some(name) = clean_name(name) else { return false };
        if i >= self.lists.len() || self.position(&name).is_some_and(|p| p != i) {
            return false;
        }
        let old = self.file(&self.lists[i].name.clone());
        let _ = std::fs::remove_file(old);
        self.lists[i].name = name.clone();
        self.save(i);
        self.lists.sort_by_key(|l| l.name.to_lowercase());
        true
    }

    pub fn delete(&mut self, i: usize) {
        if i < self.lists.len() {
            let l = self.lists.remove(i);
            let _ = std::fs::remove_file(self.file(&l.name));
        }
    }

    /// Flips between matching all rules and any rule.
    pub fn toggle_match(&mut self, i: usize) -> bool {
        let Some(l) = self.lists.get_mut(i) else { return false };
        l.all = !l.all;
        self.save(i);
        true
    }

    /// Adds a rule after the last one, on the next field the playlist does not use yet.
    pub fn add_rule(&mut self, i: usize) -> bool {
        let Some(l) = self.lists.get_mut(i) else { return false };
        let field = Field::ALL.into_iter().find(|f| l.rules.iter().all(|r| r.field != *f)).unwrap_or(Field::Title);
        l.rules.push(Rule::new(field));
        self.save(i);
        true
    }

    /// Takes rule `r` out; the last rule stays (a playlist always has one).
    pub fn remove_rule(&mut self, i: usize, r: usize) -> bool {
        match self.lists.get_mut(i) {
            Some(l) if l.rules.len() > 1 && r < l.rules.len() => {
                l.rules.remove(r);
                self.save(i);
                true
            }
            _ => false,
        }
    }

    /// Applies `change` to rule `r` and saves.
    fn change_rule(&mut self, i: usize, r: usize, change: impl FnOnce(&mut Rule)) -> bool {
        let Some(rule) = self.lists.get_mut(i).and_then(|l| l.rules.get_mut(r)) else { return false };
        change(rule);
        self.save(i);
        true
    }

    pub fn cycle_field(&mut self, i: usize, r: usize) -> bool {
        self.change_rule(i, r, Rule::cycle_field)
    }

    pub fn cycle_op(&mut self, i: usize, r: usize) -> bool {
        self.change_rule(i, r, Rule::cycle_op)
    }

    pub fn toggle_flag(&mut self, i: usize, r: usize) -> bool {
        self.change_rule(i, r, Rule::toggle_flag)
    }

    /// Sets the value of rule `r`; control characters are dropped.
    pub fn set_value(&mut self, i: usize, r: usize, value: &str) -> bool {
        let value: String = value.chars().filter(|c| !c.is_control()).collect();
        self.change_rule(i, r, |rule| rule.value = value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dir(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-smart").join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn song(id: usize, title: &str, artist: &str, year: Option<u32>, added: u64) -> Song {
        Song { id, path: format!("/m/{id}.mp3").into(), title: title.into(), artist: artist.into(), album: "Album".into(), album_artist: artist.into(), track: None, year, duration_ms: None, has_cover: false, added }
    }

    const DAY: u64 = SECONDS_PER_DAY;
    const NOW: u64 = 100 * DAY;

    fn songs() -> Vec<Song> {
        vec![
            song(0, "Waterloo", "Abba", Some(1974), NOW - 2 * DAY),
            song(1, "Money, Money, Money", "Abba", Some(1976), NOW - 40 * DAY),
            song(2, "Time", "Pink Floyd", Some(1973), NOW - 10 * DAY),
            song(3, "Untitled", "Unknown", None, NOW - 90 * DAY),
        ]
    }

    fn rule(field: Field, op: Op, value: &str) -> Rule {
        Rule { field, op, value: value.to_string() }
    }

    fn list(all: bool, rules: Vec<Rule>) -> SmartPlaylist {
        SmartPlaylist { name: "T".into(), all, rules }
    }

    #[test]
    fn text_rules_compare_without_regard_to_case() {
        let (s, st) = (songs(), Stats::default());
        let pick = |r: Rule| list(true, vec![r]).select(&s, &st, NOW);
        assert_eq!(pick(rule(Field::Artist, Op::Contains, "ABB")), [0, 1]);
        assert_eq!(pick(rule(Field::Artist, Op::DoesNotContain, "abba")), [2, 3]);
        assert_eq!(pick(rule(Field::Title, Op::Is, "time")), [2]);
        assert_eq!(pick(rule(Field::Title, Op::IsNot, "time")), [0, 1, 3]);
        assert_eq!(pick(rule(Field::Title, Op::StartsWith, "money")), [1]);
        assert_eq!(pick(rule(Field::Artist, Op::Contains, "  ")), [0, 1, 2, 3], "an empty value keeps every song");
    }

    #[test]
    fn number_rules_read_year_added_and_the_listening_data() {
        let s = songs();
        let mut st = Stats::default();
        st.set_rating(Path::new("/m/0.mp3"), 5);
        st.set_rating(Path::new("/m/2.mp3"), 2);
        st.record_play(Path::new("/m/2.mp3"), NOW - 3 * DAY);
        st.record_play(Path::new("/m/2.mp3"), NOW - 3 * DAY);
        let pick = |r: Rule| list(true, vec![r]).select(&s, &st, NOW);
        assert_eq!(pick(rule(Field::Year, Op::Less, "1975")), [0, 2, 3], "a song without a year reads as 0");
        assert_eq!(pick(rule(Field::Year, Op::Is, "0")), [3]);
        assert_eq!(pick(rule(Field::Rating, Op::Greater, "3")), [0]);
        assert_eq!(pick(rule(Field::Plays, Op::Greater, "1")), [2]);
        assert_eq!(pick(rule(Field::DaysSinceAdded, Op::Less, "30")), [0, 2], "added in the last 30 days");
        assert_eq!(pick(rule(Field::DaysSincePlayed, Op::Less, "7")), [2]);
        assert_eq!(pick(rule(Field::DaysSincePlayed, Op::Greater, "7")), [0, 1, 3], "never played is long ago");
        assert_eq!(pick(rule(Field::Year, Op::Less, "soon")), [0, 1, 2, 3], "a number that does not parse keeps every song");
    }

    #[test]
    fn favourite_is_a_yes_or_no_rule() {
        let s = songs();
        let mut st = Stats::default();
        st.toggle_favourite(Path::new("/m/1.mp3"));
        let mut r = Rule::new(Field::Favourite);
        assert_eq!(list(true, vec![r.clone()]).select(&s, &st, NOW), [1]);
        r.toggle_flag();
        assert_eq!(r.value_text(), "No");
        assert_eq!(list(true, vec![r]).select(&s, &st, NOW), [0, 2, 3]);
    }

    #[test]
    fn all_and_any_combine_the_rules() {
        let (s, st) = (songs(), Stats::default());
        let rules = vec![rule(Field::Artist, Op::Contains, "abba"), rule(Field::Year, Op::Greater, "1975")];
        assert_eq!(list(true, rules.clone()).select(&s, &st, NOW), [1]);
        assert_eq!(list(false, rules).select(&s, &st, NOW), [0, 1]);
    }

    #[test]
    fn the_list_follows_the_listening_data_as_it_changes() {
        let (s, mut st) = (songs(), Stats::default());
        let loved = list(true, vec![Rule::new(Field::Favourite)]);
        assert!(loved.select(&s, &st, NOW).is_empty());
        st.toggle_favourite(Path::new("/m/3.mp3"));
        assert_eq!(loved.select(&s, &st, NOW), [3]);
    }

    #[test]
    fn changing_the_field_keeps_what_still_fits() {
        let mut r = rule(Field::Title, Op::StartsWith, "abc");
        r.cycle_field();
        assert_eq!((r.field, r.op, r.value.as_str()), (Field::Artist, Op::StartsWith, "abc"), "text to text keeps the operator and value");
        let mut r = rule(Field::Album, Op::Contains, "x");
        r.cycle_field();
        assert_eq!((r.field, r.op, r.value.as_str()), (Field::Year, Op::Is, "2000"), "text to number starts over");
        let mut r = rule(Field::Rating, Op::Greater, "4");
        r.cycle_op();
        assert_eq!(r.op, Op::Less);
        r.cycle_op();
        assert_eq!(r.op, Op::Is, "the operators wrap");
        let mut last = Rule::new(Field::Favourite);
        last.cycle_field();
        assert_eq!(last.field, Field::Title, "the fields wrap");
        assert!(last.field.ops().contains(&last.op));
    }

    #[test]
    fn every_field_and_operator_has_a_stored_name_that_reads_back() {
        for f in Field::ALL {
            assert_eq!(Field::from_key(f.key()), Some(f));
            assert!(!f.ops().is_empty());
            assert!(f.ops().iter().all(|o| Op::from_key(o.key()) == Some(*o)));
            assert_eq!(Rule::new(f).field, f);
        }
        assert_eq!(Field::from_key("nope"), None);
        assert_eq!(Op::from_key("nope"), None);
    }

    #[test]
    fn text_round_trips_and_skips_what_it_cannot_read() {
        let mut l = list(false, vec![rule(Field::Artist, Op::Contains, "ab ba"), rule(Field::Rating, Op::Greater, "3")]);
        l.rules[0].value.push('\t');
        let back = SmartPlaylist::from_text("T", &l.to_text());
        assert!(!back.all);
        assert_eq!(back.rules, [rule(Field::Artist, Op::Contains, "ab ba "), rule(Field::Rating, Op::Greater, "3")], "a tab in a value is stored as a space");
        let odd = SmartPlaylist::from_text("T", "smart-playlist 1\nmatch all\nrule\tbogus\tis\tx\nrule\trating\tcontains\t3\nrule\tyear\tis\t1999\nnoise\n");
        assert_eq!(odd.rules, [rule(Field::Year, Op::Is, "1999")], "an unknown field and an operator the field lacks are dropped");
    }

    #[test]
    fn the_store_creates_edits_and_survives_a_reopen() {
        let d = dir("store");
        let mut p = SmartPlaylists::open(d.clone());
        assert!(p.lists().is_empty());
        assert_eq!(p.fresh_name(), "Smart Playlist");
        let i = p.create("Loved").unwrap();
        assert!(p.create("loved").is_none(), "names are unique, case aside");
        assert!(p.create("").is_none());
        assert_eq!(p.fresh_name(), "Smart Playlist");
        assert!(p.add_rule(i));
        assert_eq!(p.lists()[i].rules.len(), 2);
        assert_eq!(p.lists()[i].rules[1].field, Field::Title, "the first field not used yet");
        assert!(p.cycle_op(i, 1));
        assert!(p.set_value(i, 1, "ti\ttle"));
        assert!(p.toggle_match(i));
        assert!(!p.cycle_field(i, 9) && !p.set_value(9, 0, "x"));
        assert!(p.remove_rule(i, 0));
        assert!(!p.remove_rule(i, 0), "the last rule stays");
        let back = SmartPlaylists::open(d.clone());
        assert_eq!(back.lists().len(), 1);
        assert!(!back.lists()[0].all);
        assert_eq!(back.lists()[0].rules, [rule(Field::Title, Op::DoesNotContain, "title")]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rename_moves_the_file_and_delete_removes_it() {
        let d = dir("rename");
        let mut p = SmartPlaylists::open(d.clone());
        p.create("One").unwrap();
        p.create("Two").unwrap();
        assert!(!p.rename(0, "Two"), "taken");
        assert!(!p.rename(0, "/"), "unusable");
        assert!(p.rename(0, "Uno"));
        assert!(!d.join("One.smart").exists() && d.join("Uno.smart").exists());
        assert_eq!(SmartPlaylists::open(d.clone()).lists().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Two", "Uno"]);
        let i = p.lists().iter().position(|l| l.name == "Uno").unwrap();
        p.delete(i);
        assert!(!d.join("Uno.smart").exists());
        assert_eq!(p.lists().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}

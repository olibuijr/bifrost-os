//! `TimeZone`: zones read from the system's TZif files (`/usr/share/zoneinfo`) in pure Rust.
//!
//! The explicit transition table answers dates up to its last entry; later dates use the POSIX rule in
//! the file's footer (`EST5EDT,M3.2.0,M11.1.0`), which is how current "slim" files describe every year
//! after the table ends. Only the `Mm.w.d` rule form is understood; a footer in another form leaves the
//! last table entry in force.
//!
//! Instants are Unix seconds (UTC); offsets are seconds east of UTC like `secondsFromGMT(for:)`.

use crate::calendar_system::{civil_from_days, days_from_civil};
use std::path::Path;

/// The system zone database.
pub const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

#[derive(Clone, Debug, PartialEq, Eq)]
struct TType {
    offset: i64,
    dst: bool,
    abbr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Trans {
    month: i64,
    week: i64,
    day: i64,
    /// seconds after local midnight, may be negative or beyond 24 hours
    time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DstRule {
    abbr: String,
    offset: i64,
    start: Trans,
    end: Trans,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    std_abbr: String,
    std_offset: i64,
    dst: Option<DstRule>,
}

/// What is in force at an instant.
struct Info {
    offset: i64,
    dst: bool,
    abbr: String,
}

/// A time zone.
#[derive(Clone, Debug)]
pub struct TimeZone {
    id: String,
    types: Vec<TType>,
    transitions: Vec<(i64, usize)>,
    rule: Option<Rule>,
}

// ---------------------------------------------------------------- POSIX rule

struct Cursor<'a> {
    s: &'a [u8],
    p: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.p).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn number(&mut self) -> Option<i64> {
        let start = self.p;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.p += 1;
        }
        std::str::from_utf8(&self.s[start..self.p]).ok()?.parse().ok()
    }
    fn name(&mut self) -> Option<String> {
        if self.eat(b'<') {
            let start = self.p;
            while self.peek()? != b'>' {
                self.p += 1;
            }
            let n = std::str::from_utf8(&self.s[start..self.p]).ok()?.to_string();
            self.p += 1;
            return Some(n);
        }
        let start = self.p;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.p += 1;
        }
        (self.p - start >= 3).then(|| String::from_utf8_lossy(&self.s[start..self.p]).into_owned())
    }
    /// `[+-]hh[:mm[:ss]]` in seconds, sign applied.
    fn hms(&mut self) -> Option<i64> {
        let sign = if self.eat(b'-') {
            -1
        } else {
            self.eat(b'+');
            1
        };
        let mut secs = self.number()? * 3600;
        if self.eat(b':') {
            secs += self.number()? * 60;
            if self.eat(b':') {
                secs += self.number()?;
            }
        }
        Some(sign * secs)
    }
    fn transition(&mut self) -> Option<Trans> {
        if !self.eat(b'M') {
            return None;
        }
        let month = self.number()?;
        self.eat(b'.').then_some(())?;
        let week = self.number()?;
        self.eat(b'.').then_some(())?;
        let day = self.number()?;
        let time = if self.eat(b'/') { self.hms()? } else { 7200 };
        Some(Trans { month, week, day, time })
    }
}

fn parse_rule(text: &str) -> Option<Rule> {
    let mut c = Cursor { s: text.as_bytes(), p: 0 };
    let std_abbr = c.name()?;
    let std_offset = -c.hms()?;
    if c.peek().is_none() {
        return Some(Rule { std_abbr, std_offset, dst: None });
    }
    let abbr = c.name()?;
    let offset = if matches!(c.peek(), Some(b',') | None) { std_offset + 3600 } else { -c.hms()? };
    c.eat(b',').then_some(())?;
    let start = c.transition()?;
    c.eat(b',').then_some(())?;
    let end = c.transition()?;
    Some(Rule { std_abbr, std_offset, dst: Some(DstRule { abbr, offset, start, end }) })
}

/// Days since 1970-01-01 of the `week`th `weekday` (0 = Sunday) of the month; week 5 is the last.
fn nth_weekday(y: i64, m: i64, week: i64, weekday: i64) -> i64 {
    let first = days_from_civil(y, m, 1);
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let len = days_from_civil(ny, nm, 1) - first;
    let first_weekday = (first + 4).rem_euclid(7);
    let mut day = 1 + (weekday - first_weekday).rem_euclid(7) + (week - 1) * 7;
    if day > len {
        day -= 7;
    }
    first + day - 1
}

impl Rule {
    /// UTC instants of the DST start and end in `year`.
    fn year_transitions(&self, year: i64) -> Option<(i64, i64)> {
        let d = self.dst.as_ref()?;
        let start = nth_weekday(year, d.start.month, d.start.week, d.start.day) * 86_400 + d.start.time - self.std_offset;
        let end = nth_weekday(year, d.end.month, d.end.week, d.end.day) * 86_400 + d.end.time - d.offset;
        Some((start, end))
    }

    fn info_at(&self, t: i64) -> Info {
        if let Some(d) = &self.dst {
            let year = civil_from_days((t + self.std_offset).div_euclid(86_400)).0;
            if let Some((start, end)) = self.year_transitions(year) {
                let in_dst = if start < end { t >= start && t < end } else { t >= start || t < end };
                if in_dst {
                    return Info { offset: d.offset, dst: true, abbr: d.abbr.clone() };
                }
            }
        }
        Info { offset: self.std_offset, dst: false, abbr: self.std_abbr.clone() }
    }
}

// ---------------------------------------------------------------- TZif

fn be(d: &[u8], at: usize, n: usize) -> Option<i64> {
    let b = d.get(at..at + n)?;
    let mut v: i64 = if b[0] & 0x80 != 0 { -1 } else { 0 };
    for &x in b {
        v = (v << 8) | i64::from(x);
    }
    Some(v)
}

struct Counts {
    time: usize,
    typ: usize,
    chars: usize,
    body_len: usize,
}

/// The header at `p`: its version byte and counts.
fn header(d: &[u8], p: usize, tsize: usize) -> Option<(u8, Counts)> {
    if d.get(p..p + 4)? != b"TZif" {
        return None;
    }
    let version = *d.get(p + 4)?;
    let n = |i: usize| be(d, p + 20 + 4 * i, 4).map(|v| v as usize & 0xffff_ffff);
    let (isut, isstd, leap, time, typ, chars) = (n(0)?, n(1)?, n(2)?, n(3)?, n(4)?, n(5)?);
    let body_len = time * tsize + time + typ * 6 + chars + leap * (tsize + 4) + isstd + isut;
    Some((version, Counts { time, typ, chars, body_len }))
}

impl TimeZone {
    /// Coordinated Universal Time.
    pub fn utc() -> Self {
        TimeZone { id: "UTC".into(), types: vec![TType { offset: 0, dst: false, abbr: "UTC".into() }], transitions: Vec::new(), rule: None }
    }

    /// A zone from a POSIX rule such as `EST5EDT,M3.2.0,M11.1.0` (no transition table).
    pub fn from_posix_rule(identifier: &str, rule: &str) -> Option<Self> {
        let r = parse_rule(rule)?;
        let types = vec![TType { offset: r.std_offset, dst: false, abbr: r.std_abbr.clone() }];
        Some(TimeZone { id: identifier.into(), types, transitions: Vec::new(), rule: Some(r) })
    }

    /// `TimeZone(identifier:)` from `/usr/share/zoneinfo`; None for an unknown identifier.
    pub fn new(identifier: &str) -> Option<Self> {
        Self::in_dir(Path::new(ZONEINFO_DIR), identifier)
    }

    /// Like `new`, over another zone database directory.
    pub fn in_dir(dir: &Path, identifier: &str) -> Option<Self> {
        if identifier.is_empty() || identifier.starts_with('/') || identifier.split('/').any(|p| p == "..") {
            return None;
        }
        let bytes = std::fs::read(dir.join(identifier)).ok()?;
        Self::from_tzif(identifier, &bytes)
    }

    /// Parses a TZif file (versions 1 to 4).
    pub fn from_tzif(identifier: &str, d: &[u8]) -> Option<Self> {
        let (version, h1) = header(d, 0, 4)?;
        let (tsize, body_at, counts, footer_at) = if version == 0 {
            (4, 44, h1, None)
        } else {
            let p2 = 44 + h1.body_len;
            let (_, h2) = header(d, p2, 8)?;
            let end = p2 + 44 + h2.body_len;
            (8, p2 + 44, h2, Some(end))
        };
        let mut transitions = Vec::with_capacity(counts.time);
        let idx_at = body_at + counts.time * tsize;
        let types_at = idx_at + counts.time;
        let chars_at = types_at + counts.typ * 6;
        let chars = d.get(chars_at..chars_at + counts.chars)?;
        let mut types = Vec::with_capacity(counts.typ);
        for i in 0..counts.typ {
            let at = types_at + i * 6;
            let abbr_at = *d.get(at + 5)? as usize;
            let tail = chars.get(abbr_at..)?;
            let abbr = String::from_utf8_lossy(&tail[..tail.iter().position(|&b| b == 0).unwrap_or(tail.len())]).into_owned();
            types.push(TType { offset: be(d, at, 4)?, dst: *d.get(at + 4)? != 0, abbr });
        }
        if types.is_empty() {
            return None;
        }
        for i in 0..counts.time {
            let t = be(d, body_at + i * tsize, tsize)?;
            let ty = *d.get(idx_at + i)? as usize;
            if ty >= types.len() {
                return None;
            }
            transitions.push((t, ty));
        }
        let rule = footer_at.and_then(|at| {
            let rest = d.get(at..)?;
            let rest = rest.strip_prefix(b"\n")?;
            let line = &rest[..rest.iter().position(|&b| b == b'\n')?];
            parse_rule(std::str::from_utf8(line).ok()?)
        });
        Some(TimeZone { id: identifier.into(), types, transitions, rule })
    }

    /// `knownTimeZoneIdentifiers`: every zone file under `/usr/share/zoneinfo`, sorted.
    pub fn known_time_zone_identifiers() -> Vec<String> {
        let root = Path::new(ZONEINFO_DIR);
        let mut out = Vec::new();
        collect_zones(root, "", &mut out);
        out.sort();
        out
    }

    /// `identifier`.
    pub fn identifier(&self) -> &str {
        &self.id
    }

    /// The type before the first transition: the first standard-time type.
    fn initial(&self) -> &TType {
        self.types.iter().find(|t| !t.dst).unwrap_or(&self.types[0])
    }

    fn info_at(&self, t: i64) -> Info {
        let own = |ty: &TType| Info { offset: ty.offset, dst: ty.dst, abbr: ty.abbr.clone() };
        match self.transitions.last() {
            None => match &self.rule {
                Some(r) => r.info_at(t),
                None => own(self.initial()),
            },
            Some(&(last, ty)) => {
                if t < self.transitions[0].0 {
                    own(self.initial())
                } else if t >= last {
                    match &self.rule {
                        Some(r) => r.info_at(t),
                        None => own(&self.types[ty]),
                    }
                } else {
                    let i = self.transitions.partition_point(|x| x.0 <= t) - 1;
                    own(&self.types[self.transitions[i].1])
                }
            }
        }
    }

    /// `secondsFromGMT(for:)`
    pub fn seconds_from_gmt(&self, unix: i64) -> i64 {
        self.info_at(unix).offset
    }

    /// `abbreviation(for:)` ("EDT", "GMT", "+1030").
    pub fn abbreviation(&self, unix: i64) -> String {
        self.info_at(unix).abbr
    }

    /// `isDaylightSavingTime(for:)`
    pub fn is_daylight_saving_time(&self, unix: i64) -> bool {
        self.info_at(unix).dst
    }

    /// `daylightSavingTimeOffset(for:)`: the extra seconds in force, 0 in standard time.
    pub fn daylight_saving_time_offset(&self, unix: i64) -> i64 {
        let info = self.info_at(unix);
        if !info.dst {
            return 0;
        }
        if let (Some(r), Some(&(last, _))) = (&self.rule, self.transitions.last()) {
            if unix >= last {
                return info.offset - r.std_offset;
            }
        }
        if let Some(r) = self.rule.as_ref().filter(|_| self.transitions.is_empty()) {
            return info.offset - r.std_offset;
        }
        let upto = self.transitions.partition_point(|x| x.0 <= unix);
        let standard = self.transitions[..upto].iter().rev().map(|&(_, ty)| &self.types[ty]).find(|ty| !ty.dst).unwrap_or_else(|| self.initial());
        info.offset - standard.offset
    }

    /// `nextDaylightSavingTimeTransition(after:)`: the first instant after `unix` where daylight saving
    /// starts or ends; None for a zone without daylight saving.
    pub fn next_daylight_saving_time_transition(&self, unix: i64) -> Option<i64> {
        let mut prev_dst = self.initial().dst;
        for &(at, ty) in &self.transitions {
            let dst = self.types[ty].dst;
            if at > unix && dst != prev_dst {
                return Some(at);
            }
            prev_dst = dst;
        }
        let rule = self.rule.as_ref()?;
        rule.dst.as_ref()?;
        let year = civil_from_days((unix + rule.std_offset).div_euclid(86_400)).0;
        (year - 1..=year + 2).filter_map(|y| rule.year_transitions(y)).flat_map(|(a, b)| [a, b]).filter(|&t| t > unix).min()
    }
}

fn collect_zones(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let id = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let path = e.path();
        if path.is_dir() {
            if prefix.is_empty() && matches!(name.as_str(), "posix" | "right") {
                continue;
            }
            collect_zones(&path, &id, out);
        } else if std::fs::read(&path).is_ok_and(|b| b.starts_with(b"TZif")) {
            out.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, m: i64, d: i64, h: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600
    }

    fn zone(id: &str) -> TimeZone {
        TimeZone::new(id).unwrap_or_else(|| panic!("{id} is not in {ZONEINFO_DIR}"))
    }

    #[test]
    fn posix_rules_parse() {
        let r = parse_rule("EST5EDT,M3.2.0,M11.1.0").unwrap();
        assert_eq!((r.std_abbr.as_str(), r.std_offset), ("EST", -18_000));
        let d = r.dst.unwrap();
        assert_eq!((d.abbr.as_str(), d.offset, d.start.month, d.start.week, d.start.time), ("EDT", -14_400, 3, 2, 7200));
        let lh = parse_rule("<+1030>-10:30<+11>-11,M10.1.0,M4.1.0").unwrap();
        assert_eq!((lh.std_abbr.as_str(), lh.std_offset), ("+1030", 37_800));
        assert_eq!(lh.dst.unwrap().offset, 39_600);
        assert_eq!(parse_rule("GMT0").unwrap().dst, None);
        assert_eq!(parse_rule("EST5EDT,J60,J300"), None);
    }

    #[test]
    fn the_nth_weekday_finds_dst_days() {
        // second Sunday of March 2026 is the 8th; last Sunday of October 2026 is the 25th
        assert_eq!(nth_weekday(2026, 3, 2, 0), days_from_civil(2026, 3, 8));
        assert_eq!(nth_weekday(2026, 10, 5, 0), days_from_civil(2026, 10, 25));
    }

    #[test]
    fn a_rule_zone_switches_at_the_right_instants() {
        let ny = TimeZone::from_posix_rule("America/New_York", "EST5EDT,M3.2.0,M11.1.0").unwrap();
        let spring = at(2026, 3, 8, 7);
        assert_eq!(ny.seconds_from_gmt(spring - 1), -18_000);
        assert_eq!(ny.seconds_from_gmt(spring), -14_400);
        let fall = at(2026, 11, 1, 6);
        assert_eq!(ny.abbreviation(fall - 1), "EDT");
        assert_eq!(ny.abbreviation(fall), "EST");
        assert_eq!(ny.next_daylight_saving_time_transition(at(2026, 1, 1, 0)), Some(spring));
        assert_eq!(ny.next_daylight_saving_time_transition(spring), Some(fall));
        assert_eq!(ny.daylight_saving_time_offset(at(2026, 7, 1, 0)), 3600);
        assert_eq!(ny.daylight_saving_time_offset(at(2026, 1, 1, 0)), 0);
    }

    #[test]
    fn utc_has_no_offset_or_transitions() {
        let u = TimeZone::utc();
        assert_eq!((u.identifier(), u.seconds_from_gmt(123), u.abbreviation(0)), ("UTC", 0, "UTC".to_string()));
        assert_eq!(u.next_daylight_saving_time_transition(0), None);
    }

    #[test]
    fn reykjavik_is_gmt_all_year() {
        let z = zone("Atlantic/Reykjavik");
        for t in [at(2026, 1, 1, 0), at(2026, 7, 1, 0), at(1960, 7, 1, 0)] {
            assert_eq!(z.seconds_from_gmt(t) % 3600, 0);
        }
        assert_eq!(z.seconds_from_gmt(at(2026, 7, 1, 0)), 0);
        assert_eq!(z.abbreviation(at(2026, 7, 1, 0)), "GMT");
        assert!(!z.is_daylight_saving_time(at(2026, 7, 1, 0)));
        assert_eq!(z.next_daylight_saving_time_transition(at(2026, 1, 1, 0)), None);
    }

    #[test]
    fn new_york_has_est_and_edt() {
        let z = zone("America/New_York");
        assert_eq!((z.seconds_from_gmt(at(2026, 1, 15, 12)), z.abbreviation(at(2026, 1, 15, 12))), (-18_000, "EST".to_string()));
        assert_eq!((z.seconds_from_gmt(at(2026, 7, 1, 12)), z.abbreviation(at(2026, 7, 1, 12))), (-14_400, "EDT".to_string()));
        assert!(z.is_daylight_saving_time(at(2026, 7, 1, 12)));
        assert_eq!(z.daylight_saving_time_offset(at(2026, 7, 1, 12)), 3600);
        assert_eq!(z.next_daylight_saving_time_transition(at(2026, 1, 1, 0)), Some(at(2026, 3, 8, 7)));
        assert_eq!(z.next_daylight_saving_time_transition(at(2026, 3, 8, 7)), Some(at(2026, 11, 1, 6)));
    }

    #[test]
    fn lord_howe_shifts_by_thirty_minutes() {
        let z = zone("Australia/Lord_Howe");
        assert_eq!((z.seconds_from_gmt(at(2026, 1, 15, 0)), z.abbreviation(at(2026, 1, 15, 0))), (39_600, "+11".to_string()));
        assert_eq!((z.seconds_from_gmt(at(2026, 7, 15, 0)), z.abbreviation(at(2026, 7, 15, 0))), (37_800, "+1030".to_string()));
        assert_eq!(z.daylight_saving_time_offset(at(2026, 1, 15, 0)), 1800);
        assert_eq!(z.daylight_saving_time_offset(at(2026, 7, 15, 0)), 0);
        assert!(z.next_daylight_saving_time_transition(at(2026, 7, 15, 0)).is_some());
    }

    #[test]
    fn unknown_and_unsafe_identifiers_are_rejected() {
        assert!(TimeZone::new("Mars/Olympus_Mons").is_none());
        assert!(TimeZone::new("../../etc/passwd").is_none());
        assert!(TimeZone::new("/etc/passwd").is_none());
        assert!(TimeZone::new("").is_none());
        assert!(TimeZone::from_tzif("x", b"not a zone").is_none());
    }

    #[test]
    fn known_identifiers_list_the_database() {
        let ids = TimeZone::known_time_zone_identifiers();
        for id in ["Atlantic/Reykjavik", "America/New_York", "Australia/Lord_Howe", "Europe/London"] {
            assert!(ids.iter().any(|i| i == id), "{id}");
        }
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
        assert!(!ids.iter().any(|i| i.starts_with("posix/") || i.starts_with("right/")));
    }
}

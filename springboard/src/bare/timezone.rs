//! Time zones: TZif files read from the
//! system tzdata (the combined `tzdata` file or a zoneinfo tree) give the
//! UTC offset and daylight saving at any instant. `sys::local_time` asks
//! `offset_at`; Settings › Date & Time picks the zone with `set`.
//!
//! The zone of the machine is /etc/localtime (a link into
//! /usr/share/zoneinfo; `system_name`), `set` passes a changed choice on to
//! `timedatectl set-timezone`, and `timedatectl` also says whether systemd-timesyncd
//! keeps the clock (`timedatectl_show`, `set_ntp`; net::ntp stays out of its way).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub const DEFAULT: &str = "Atlantic/Reykjavik";

/// Combined database files (index, then zone blobs); none on a standard Linux system.
const COMBINED: [&str; 0] = [];
/// One TZif file per zone.
const TREES: [&str; 1] = ["/usr/share/zoneinfo"];
const REGIONS: [&str; 10] = ["Africa", "America", "Antarctica", "Arctic", "Asia", "Atlantic", "Australia", "Europe", "Indian", "Pacific"];

const DAY: i64 = 86_400;

// ---- civil time ----

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil (year, month, day) for days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (era * 400 + yoe + (m <= 2) as i64, m, d)
}

/// Weekday (0 = Sunday) of a day count since 1970-01-01 (a Thursday).
pub fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// Days in month `m` of year `y`.
pub fn days_in_month(y: i64, m: u32) -> u32 {
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    (next - days_from_civil(y, m, 1)) as u32
}

/// (hour, min, sec, wday, mday, mon 0-11) of a Unix time already shifted
/// to local time.
pub fn civil(local: i64) -> (i32, i32, i32, i32, i32, i32) {
    let days = local.div_euclid(DAY);
    let secs = local.rem_euclid(DAY) as i32;
    let (_, m, d) = civil_from_days(days);
    (secs / 3600, secs / 60 % 60, secs % 60, weekday(days) as i32, d as i32, m as i32 - 1)
}

// ---- POSIX TZ rule (the TZif footer) ----

/// "Mm.w.d/time": day `wday` of week `week` (5 = last) of `month`, at
/// `time` seconds after local midnight.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spec {
    month: u32,
    week: u32,
    wday: u32,
    time: i32,
}

impl Spec {
    /// Seconds since the epoch of the change in `year`, as a local time.
    fn local_secs(&self, year: i64) -> i64 {
        let first = days_from_civil(year, self.month, 1);
        let mut dom = 1 + (self.wday as i64 - weekday(first)).rem_euclid(7) as u32 + (self.week - 1) * 7;
        while dom > days_in_month(year, self.month) {
            dom -= 7;
        }
        (first + dom as i64 - 1) * DAY + self.time as i64
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    /// seconds east of UTC
    std: i32,
    dst: Option<(i32, Spec, Spec)>,
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        let hit = self.peek() == Some(c);
        self.i += hit as usize;
        hit
    }

    fn number(&mut self) -> Option<i32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }

    /// A zone abbreviation: letters, or anything between `<` and `>`.
    fn name(&mut self) -> Option<()> {
        if self.eat(b'<') {
            while self.peek()? != b'>' {
                self.i += 1;
            }
            self.i += 1;
            return Some(());
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        (self.i - start >= 3).then_some(())
    }

    /// [+-]hh[:mm[:ss]] in seconds.
    fn hms(&mut self) -> Option<i32> {
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

    fn spec(&mut self) -> Option<Spec> {
        if !self.eat(b'M') {
            return None; // Julian-day rules are not used by the tz database
        }
        let month = self.number()? as u32;
        self.eat(b'.');
        let week = self.number()? as u32;
        self.eat(b'.');
        let wday = self.number()? as u32;
        let time = if self.eat(b'/') { self.hms()? } else { 7200 };
        ((1..=12).contains(&month) && (1..=5).contains(&week) && wday <= 6).then_some(Spec { month, week, wday, time })
    }
}

impl Rule {
    /// Parses a POSIX TZ string such as `CET-1CEST,M3.5.0,M10.5.0/3`.
    fn parse(tz: &str) -> Option<Rule> {
        let mut c = Cursor { s: tz.as_bytes(), i: 0 };
        c.name()?;
        let std = -c.hms()?; // POSIX counts hours west of UTC
        if c.peek().is_none() {
            return Some(Rule { std, dst: None });
        }
        c.name()?;
        let dst = if matches!(c.peek(), Some(b',') | None) { std + 3600 } else { -c.hms()? };
        let (start, end) = if c.eat(b',') {
            let start = c.spec()?;
            c.eat(b',').then_some(())?;
            (start, c.spec()?)
        } else {
            (Spec { month: 3, week: 2, wday: 0, time: 7200 }, Spec { month: 11, week: 1, wday: 0, time: 7200 })
        };
        Some(Rule { std, dst: Some((dst, start, end)) })
    }

    /// (offset east of UTC, is daylight time) at a Unix time.
    fn at(&self, unix: i64) -> (i32, bool) {
        let Some((dst, start, end)) = self.dst else { return (self.std, false) };
        let (year, ..) = civil_from_days((unix + self.std as i64).div_euclid(DAY));
        // spring forward is stated in standard time, fall back in daylight time
        let on = start.local_secs(year) - self.std as i64;
        let off = end.local_secs(year) - dst as i64;
        let daylight = if on < off { unix >= on && unix < off } else { !(unix >= off && unix < on) };
        if daylight {
            (dst, true)
        } else {
            (self.std, false)
        }
    }
}

// ---- TZif ----

/// One zone's transitions, as parsed from a TZif file.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    transitions: Vec<i64>,
    /// type index for each transition
    kinds: Vec<u8>,
    /// (seconds east of UTC, is daylight time)
    types: Vec<(i32, bool)>,
    /// governs everything after the last transition
    rule: Option<Rule>,
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

impl Zone {
    /// Parses a TZif file (version 2+ preferred, version 1 accepted).
    pub fn parse(b: &[u8]) -> Option<Zone> {
        if b.get(..4)? != b"TZif" {
            return None;
        }
        // counts: isut, isstd, leap, time, type, char
        let counts = |at: usize| -> Option<[usize; 6]> {
            let mut c = [0usize; 6];
            for (k, v) in c.iter_mut().enumerate() {
                *v = be32(b, at + 20 + 4 * k)? as usize;
            }
            Some(c)
        };
        let c1 = counts(0)?;
        let v1_len = c1[3] * 5 + c1[4] * 6 + c1[5] + c1[2] * 8 + c1[1] + c1[0];
        let v2 = b.get(4).is_some_and(|&v| v >= b'2') && b.len() >= 44 + v1_len + 44;
        let (head, wide) = if v2 { (44 + v1_len, 8) } else { (0, 4) };
        let [isut, isstd, leap, timecnt, typecnt, charcnt] = if v2 { counts(head)? } else { c1 };
        // the data follows its header either way
        let mut at = head + 44;
        let mut transitions = Vec::with_capacity(timecnt);
        for k in 0..timecnt {
            transitions.push(if wide == 8 { be64(b, at + 8 * k)? } else { be32(b, at + 4 * k)? as i32 as i64 });
        }
        at += timecnt * wide;
        let kinds = b.get(at..at + timecnt)?.to_vec();
        at += timecnt;
        let mut types = Vec::with_capacity(typecnt);
        for k in 0..typecnt {
            let o = at + 6 * k;
            types.push((be32(b, o)? as i32, *b.get(o + 4)? != 0));
        }
        at += typecnt * 6 + charcnt + leap * (wide + 4) + isstd + isut;
        if types.is_empty() || kinds.iter().any(|&k| k as usize >= types.len()) {
            return None;
        }
        let rule = if v2 && b.get(at) == Some(&b'\n') {
            let tail = &b[at + 1..];
            let end = tail.iter().position(|&c| c == b'\n').unwrap_or(tail.len());
            std::str::from_utf8(&tail[..end]).ok().and_then(Rule::parse)
        } else {
            None
        };
        Some(Zone { transitions, kinds, types, rule })
    }

    /// (offset east of UTC in seconds, is daylight time) at a Unix time.
    pub fn at(&self, unix: i64) -> (i32, bool) {
        match self.transitions.last() {
            Some(&last) if unix >= last => match &self.rule {
                Some(rule) => rule.at(unix),
                None => self.types[*self.kinds.last().expect("kind per transition") as usize],
            },
            Some(_) => {
                let n = self.transitions.partition_point(|&t| t <= unix);
                // before the first transition the first standard type applies
                match n.checked_sub(1) {
                    Some(k) => self.types[self.kinds[k] as usize],
                    None => self.types.iter().copied().find(|t| !t.1).unwrap_or(self.types[0]),
                }
            }
            None => match &self.rule {
                Some(rule) => rule.at(unix),
                None => self.types[0],
            },
        }
    }
}

// ---- the tz database on disk ----

/// A name from the picker or the settings file must not walk out of the tree.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('/') && !name.split('/').any(|p| p.is_empty() || p == "." || p == "..")
}

/// (name, offset, length) of every zone in a combined tzdata file: a 24
/// byte header (12 byte magic, index, data and trailer offsets, big-endian),
/// then 52 byte entries of name[40], start, length and a spare word.
fn combined_index(b: &[u8]) -> Option<Vec<(String, usize, usize)>> {
    if !b.starts_with(b"tzdata") {
        return None;
    }
    let (index, data) = (be32(b, 12)? as usize, be32(b, 16)? as usize);
    let mut out = Vec::new();
    for entry in b.get(index..data)?.chunks_exact(52) {
        let name_len = entry[..40].iter().position(|&c| c == 0).unwrap_or(40);
        let name = String::from_utf8_lossy(&entry[..name_len]).into_owned();
        out.push((name, data + be32(entry, 40)? as usize, be32(entry, 44)? as usize));
    }
    Some(out)
}

fn read_combined(path: &str, name: &str) -> Option<Vec<u8>> {
    let b = std::fs::read(path).ok()?;
    let (_, start, len) = combined_index(&b)?.into_iter().find(|(n, ..)| n == name)?;
    b.get(start..start + len).map(<[u8]>::to_vec)
}

/// The TZif bytes of a zone, from whichever database exists.
fn read_zone(name: &str) -> Option<Vec<u8>> {
    if !valid_name(name) {
        return None;
    }
    COMBINED.iter().find_map(|p| read_combined(p, name)).or_else(|| TREES.iter().find_map(|t| std::fs::read(format!("{t}/{name}")).ok()))
}

/// Loads and parses a zone by name ("Atlantic/Reykjavik").
pub fn load(name: &str) -> Option<Zone> {
    Zone::parse(&read_zone(name)?)
}

fn wanted(name: &str) -> bool {
    name == "UTC" || name.split('/').next().is_some_and(|r| REGIONS.contains(&r)) && name.contains('/')
}

fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let full = if prefix.is_empty() { name } else { format!("{prefix}/{}", e.file_name().to_string_lossy()) };
        if e.path().is_dir() {
            walk(&e.path(), &full, out);
        } else if wanted(&full) {
            out.push(full);
        }
    }
}

/// Every selectable zone name, sorted ("Africa/Abidjan" ... "UTC").
pub fn list_zones() -> Vec<String> {
    let mut names: Vec<String> = COMBINED
        .iter()
        .find_map(|p| combined_index(&std::fs::read(p).ok()?))
        .map(|index| index.into_iter().map(|(n, ..)| n).filter(|n| wanted(n)).collect())
        .unwrap_or_default();
    if names.is_empty() {
        for t in TREES {
            walk(std::path::Path::new(t), "", &mut names);
            if !names.is_empty() {
                break;
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

// ---- the current zone ----

static ZONE: Mutex<Option<(String, Zone)>> = Mutex::new(None);
static TRIED_DEFAULT: AtomicBool = AtomicBool::new(false);
/// Only the shell (System Settings) changes the system zone; apps and tests just use one in-process.
static PERSIST: AtomicBool = AtomicBool::new(false);

/// Lets `set` hand a changed zone to timedatectl. The shell calls this once at startup.
pub fn persist_to_system(on: bool) {
    PERSIST.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Makes `name` the zone `local_time` reports in; false (and no change)
/// when the database has no such zone. A zone that differs from the system's
/// is also handed to timedatectl, so the choice survives a restart of the shell.
pub fn set(name: &str) -> bool {
    let Some(zone) = load(name) else { return false };
    if let Ok(mut z) = ZONE.lock() {
        *z = Some((name.to_string(), zone));
    }
    if PERSIST.load(std::sync::atomic::Ordering::Relaxed) && system_name().is_some_and(|s| s != name) {
        let name = name.to_string();
        crate::bare::sys::spawn_named("set-timezone", move || {
            let _ = crate::privd_client::connect().set_timezone(&name);
        });
    }
    true
}

/// The zone name behind a /etc/localtime link target ("/usr/share/zoneinfo/Europe/Berlin" or a
/// relative "../usr/share/zoneinfo/Europe/Berlin"); None for anything outside a zoneinfo tree.
pub fn name_from_link(target: &std::path::Path) -> Option<String> {
    let t = target.to_str()?;
    let name = t.split_once("zoneinfo/")?.1;
    valid_name(name).then(|| name.to_string())
}

/// The machine's zone on a desktop: /etc/localtime's link, else /etc/timezone.
pub fn system_name() -> Option<String> {
    std::fs::read_link("/etc/localtime").ok().and_then(|t| name_from_link(&t)).or_else(|| {
        let n = std::fs::read_to_string("/etc/timezone").ok()?.trim().to_string();
        valid_name(&n).then_some(n)
    })
}

/// `timedatectl show -p PROP --value`, e.g. "NTP" (timesyncd enabled) or "NTPSynchronized".
pub fn timedatectl_show(prop: &str) -> Option<String> {
    let out = std::process::Command::new("timedatectl").args(["show", "-p", prop, "--value"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// timesyncd's sync flag without a process or a bus call (the Date & Time page polls it): systemd-timesyncd
/// touches `/run/systemd/timesync/synchronized` once the clock is synchronized. `None` without timesyncd.
pub fn timesyncd_synchronized() -> Option<&'static str> {
    timesyncd_enabled()?;
    Some(if std::path::Path::new("/run/systemd/timesync/synchronized").exists() { "yes" } else { "no" })
}

/// Whether network time is on (timedatectl's `NTP`), read from the unit's enablement links instead of
/// asking timedated, which a bus call would start and leave to time out again.
pub fn timesyncd_enabled() -> Option<&'static str> {
    let unit = "systemd-timesyncd.service";
    let lib = ["/usr/lib/systemd/system", "/lib/systemd/system"].iter().any(|d| std::path::Path::new(&format!("{d}/{unit}")).exists());
    if !lib {
        return None;
    }
    let wants = ["sysinit.target.wants", "multi-user.target.wants"];
    let on = wants.iter().any(|w| std::fs::symlink_metadata(format!("/etc/systemd/system/{w}/{unit}")).is_ok());
    Some(if on { "yes" } else { "no" })
}

/// Turns systemd's network time on or off (`timedatectl set-ntp`) when it differs; best effort.
pub fn set_ntp(on: bool) {
    crate::bare::sys::spawn_named("set-ntp", move || {
        if timedatectl_show("NTP").is_some_and(|v| (v == "yes") != on) {
            let _ = crate::privd_client::connect().set_ntp(on);
        }
    });
}

/// The active zone's name, `DEFAULT` until one is loaded.
pub fn current_name() -> String {
    ZONE.lock().ok().and_then(|z| z.as_ref().map(|(n, _)| n.clone())).unwrap_or_else(|| DEFAULT.to_string())
}

/// Seconds east of UTC at `unix` in the active zone; `None` when no zone
/// database is readable (the caller falls back to libc).
pub fn offset_at(unix: i64) -> Option<i32> {
    if ZONE.lock().ok()?.is_none() && !TRIED_DEFAULT.swap(true, Ordering::Relaxed) {
        set(DEFAULT);
    }
    ZONE.lock().ok()?.as_ref().map(|(_, z)| z.at(unix).0)
}

/// "UTC+1", "UTC-3:30" for an offset in seconds.
pub fn offset_label(secs: i32) -> String {
    let (sign, a) = if secs < 0 { ('-', -secs) } else { ('+', secs) };
    match a / 60 % 60 {
        0 => format!("UTC{sign}{}", a / 3600),
        m => format!("UTC{sign}{}:{m:02}", a / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localtime_links_give_zone_names() {
        let p = std::path::Path::new;
        assert_eq!(name_from_link(p("/usr/share/zoneinfo/Europe/Berlin")).as_deref(), Some("Europe/Berlin"));
        assert_eq!(name_from_link(p("../usr/share/zoneinfo/Atlantic/Reykjavik")).as_deref(), Some("Atlantic/Reykjavik"));
        assert_eq!(name_from_link(p("/etc/elsewhere")), None);
        assert_eq!(name_from_link(p("/usr/share/zoneinfo/../../shadow")), None);
    }

    fn unix(y: i64, m: u32, d: u32, hour: i64) -> i64 {
        days_from_civil(y, m, d) * DAY + hour * 3600
    }

    #[test]
    fn civil_fields_of_a_known_instant() {
        // 2024-07-04 15:30:20 UTC was a Thursday
        assert_eq!(civil(unix(2024, 7, 4, 15) + 30 * 60 + 20), (15, 30, 20, 4, 4, 6));
        assert_eq!(civil(-1), (23, 59, 59, 3, 31, 11));
    }

    #[test]
    fn posix_rule_follows_eu_daylight_saving() {
        let r = Rule::parse("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        assert_eq!(r.at(unix(2024, 1, 15, 12)), (3600, false));
        assert_eq!(r.at(unix(2024, 7, 1, 12)), (7200, true));
        // 2024-03-31 01:00 UTC is the switch
        assert_eq!(r.at(unix(2024, 3, 31, 0)).0, 3600);
        assert_eq!(r.at(unix(2024, 3, 31, 1)).0, 7200);
        // 2024-10-27 01:00 UTC is the switch back
        assert_eq!(r.at(unix(2024, 10, 27, 0)).0, 7200);
        assert_eq!(r.at(unix(2024, 10, 27, 1)).0, 3600);
    }

    #[test]
    fn posix_rule_handles_the_southern_hemisphere_and_fixed_zones() {
        let r = Rule::parse("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap();
        assert_eq!(r.at(unix(2024, 1, 15, 0)), (39_600, true));
        assert_eq!(r.at(unix(2024, 7, 15, 0)), (36_000, false));
        let g = Rule::parse("GMT0").unwrap();
        assert_eq!(g.at(unix(2024, 7, 1, 0)), (0, false));
        let n = Rule::parse("<-03>3").unwrap();
        assert_eq!(n.at(0), (-10_800, false));
        let us = Rule::parse("EST5EDT,M3.2.0,M11.1.0").unwrap();
        assert_eq!(us.at(unix(2024, 7, 1, 12)), (-14_400, true));
        assert_eq!(us.at(unix(2024, 12, 1, 12)), (-18_000, false));
        assert_eq!(Rule::parse("nonsense"), None);
    }

    /// A version 2 TZif: UTC until t=1000, then +1 h standard, footer "XXX-2".
    fn blob() -> Vec<u8> {
        let mut b = Vec::new();
        let head = |b: &mut Vec<u8>, counts: [u32; 6]| {
            b.extend_from_slice(b"TZif2");
            b.extend_from_slice(&[0; 15]);
            for c in counts {
                b.extend_from_slice(&c.to_be_bytes());
            }
        };
        // empty version 1 block
        head(&mut b, [0, 0, 0, 0, 1, 4]);
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(b"UTC\0");
        head(&mut b, [0, 0, 0, 1, 2, 8]);
        b.extend_from_slice(&1000i64.to_be_bytes());
        b.push(1);
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(&3600i32.to_be_bytes());
        b.extend_from_slice(&[0, 4]);
        b.extend_from_slice(b"UTC\0ABC\0");
        b.extend_from_slice(b"\nXXX-2\n");
        b
    }

    #[test]
    fn tzif_transitions_then_the_footer_rule() {
        let z = Zone::parse(&blob()).unwrap();
        assert_eq!(z.at(999), (0, false));
        assert_eq!(z.at(1000), (7200, false), "after the last transition the footer rule rules");
        assert_eq!(Zone::parse(b"nope"), None);
    }

    #[test]
    fn combined_database_is_indexed_by_name() {
        let mut b = b"tzdata2024a\0".to_vec();
        b.extend_from_slice(&24u32.to_be_bytes());
        b.extend_from_slice(&76u32.to_be_bytes());
        b.extend_from_slice(&76u32.to_be_bytes());
        let mut entry = [0u8; 52];
        entry[..9].copy_from_slice(b"Asia/Test");
        entry[44..48].copy_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(&entry);
        b.extend_from_slice(b"abc");
        assert_eq!(combined_index(&b), Some(vec![("Asia/Test".to_string(), 76, 3)]));
    }

    #[test]
    fn names_are_validated_and_filtered() {
        assert!(valid_name("Atlantic/Reykjavik") && !valid_name("../etc/passwd") && !valid_name("/etc") && !valid_name(""));
        assert!(wanted("Europe/Paris") && wanted("UTC") && !wanted("posix/Europe/Paris") && !wanted("zone.tab") && !wanted("Europe"));
    }

    #[test]
    fn offsets_are_labelled() {
        assert_eq!(offset_label(0), "UTC+0");
        assert_eq!(offset_label(3600), "UTC+1");
        assert_eq!(offset_label(-12_600), "UTC-3:30");
    }
}

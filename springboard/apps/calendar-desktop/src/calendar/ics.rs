//! iCalendar (RFC 5545) reader and writer for events: line folding, TEXT
//! escaping, DATE / DATE-TIME (floating, UTC, TZID), DURATION, RRULE, EXDATE,
//! VALARM triggers and the fixed offsets of VTIMEZONE blocks.
use super::invitees;
use super::recur::{self, Rule};
use super::store::CalendarEvent;
use super::Date;
use bifrost_app::controls::calendar_participant::Participant;

const MAX_EVENTS: usize = 5000;
const MAX_LINE: usize = 75;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Local {
    pub date: Date,
    pub minute: u16,
}

impl Local {
    pub fn new(date: Date, minute: u16) -> Self {
        Local { date, minute }
    }

    pub fn add_minutes(self, minutes: i64) -> Local {
        let total = self.date.days() * 1440 + self.minute as i64 + minutes;
        Local {
            date: Date::from_days(total.div_euclid(1440)),
            minute: total.rem_euclid(1440) as u16,
        }
    }

    pub fn minutes_since(self, other: Local) -> i64 {
        (self.date.days() - other.date.days()) * 1440 + self.minute as i64 - other.minute as i64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IcsEvent {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub notes: String,
    pub all_day: bool,
    pub start: Local,
    /// All-day events: exclusive end date, minute 0 (as in ICS).
    pub end: Local,
    /// None = floating, Some("UTC") = UTC, otherwise an IANA/VTIMEZONE name.
    pub tzid: Option<String>,
    pub rule: Option<Rule>,
    pub exdates: Vec<Date>,
    /// Minutes before the start, one per alert.
    pub alarms: Vec<u32>,
    pub color: Option<String>,
    /// Invitees and their responses (ATTENDEE lines), read-only.
    pub attendees: Vec<Participant>,
}

impl IcsEvent {
    pub fn new(uid: &str, summary: &str, start: Local, end: Local) -> Self {
        IcsEvent {
            uid: uid.into(),
            summary: summary.into(),
            location: String::new(),
            notes: String::new(),
            all_day: false,
            start,
            end,
            tzid: None,
            rule: None,
            exdates: vec![],
            alarms: vec![],
            color: None,
            attendees: vec![],
        }
    }

    pub fn duration_minutes(&self) -> i64 {
        self.end.minutes_since(self.start).max(0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IcsCalendar {
    pub name: String,
    pub events: Vec<IcsEvent>,
    /// (TZID, standard UTC offset in minutes) from VTIMEZONE blocks.
    pub timezones: Vec<(String, i32)>,
}

// ---------------------------------------------------------------- reading

/// Joins folded lines (CRLF or LF followed by a space or tab).
pub fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut parts = vec![];
    let mut quoted = false;
    let mut from = 0;
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == sep && !quoted {
            parts.push(&s[from..i]);
            from = i + c.len_utf8();
        }
    }
    parts.push(&s[from..]);
    parts
}

struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Line {
    fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

fn parse_line(line: &str) -> Option<Line> {
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == ':' && !quoted {
            colon = Some(i);
            break;
        }
    }
    let colon = colon?;
    let head = split_unquoted(&line[..colon], ';');
    let name = head[0].trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = head[1..]
        .iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    Some(Line { name, params, value: line[colon + 1..].to_string() })
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Parsed DATE or DATE-TIME: (value, all_day, is_utc).
fn parse_when(value: &str, params_value: Option<&str>) -> Option<(Local, bool, bool)> {
    let v = value.trim();
    if !v.is_ascii() {
        return None;
    }
    if params_value == Some("DATE") || v.len() == 8 {
        return Some((Local::new(recur::parse_ymd(v)?, 0), true, false));
    }
    if v.len() < 15 || v.as_bytes()[8] != b'T' {
        return None;
    }
    let date = recur::parse_ymd(v)?;
    let num = |a: usize, b: usize| -> Option<u16> {
        v[a..b].bytes().all(|c| c.is_ascii_digit()).then(|| v[a..b].parse().ok())?
    };
    let (h, m) = (num(9, 11)?, num(11, 13)?);
    if h > 23 || m > 59 {
        return None;
    }
    Some((Local::new(date, h * 60 + m), false, v.ends_with('Z')))
}

/// ISO 8601 duration to signed minutes (seconds are truncated).
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.as_bytes().first()? {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    let s = s.strip_prefix('P')?;
    let (mut total, mut num, mut time, mut any) = (0i64, String::new(), false, false);
    for c in s.chars() {
        match c {
            'T' => time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                any = true;
                total += n * match (unit, time) {
                    ('W', false) => 10_080,
                    ('D', false) => 1440,
                    ('H', true) => 60,
                    ('M', true) => 1,
                    ('S', true) => 0,
                    _ => return None,
                };
                if unit == 'S' {
                    total += n / 60;
                }
            }
        }
    }
    (num.is_empty() && any).then_some(sign * total)
}

fn parse_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let sign = match s.as_bytes().first()? {
        b'-' => -1,
        b'+' => 1,
        _ => return None,
    };
    let d = &s[1..];
    if d.len() < 4 || !d.is_ascii() || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(sign * (d[0..2].parse::<i32>().ok()? * 60 + d[2..4].parse::<i32>().ok()?))
}

#[derive(Default)]
struct Builder {
    uid: String,
    summary: String,
    location: String,
    notes: String,
    start: Option<(Local, bool, bool)>,
    start_tz: Option<String>,
    end: Option<Local>,
    duration: Option<i64>,
    rule: Option<Rule>,
    exdates: Vec<Date>,
    alarms: Vec<u32>,
    color: Option<String>,
    attendees: Vec<Participant>,
}

impl Builder {
    fn finish(self, serial: usize) -> Option<IcsEvent> {
        let (start, all_day, utc) = self.start?;
        let end = match (self.end, self.duration) {
            (Some(e), _) if e >= start => e,
            (_, Some(d)) if d >= 0 => start.add_minutes(d),
            _ if all_day => start.add_minutes(1440),
            _ => start,
        };
        let tzid = if utc { Some("UTC".to_string()) } else if all_day { None } else { self.start_tz };
        let mut alarms = self.alarms;
        alarms.sort_unstable();
        alarms.dedup();
        Some(IcsEvent {
            uid: if self.uid.is_empty() { format!("imported-{serial}@springboard") } else { self.uid },
            summary: self.summary,
            location: self.location,
            notes: self.notes,
            all_day,
            start,
            end,
            tzid,
            rule: self.rule,
            exdates: self.exdates,
            alarms,
            color: self.color,
            attendees: self.attendees,
        })
    }
}

/// Lenient parser: malformed events are skipped, the rest is kept.
pub fn parse(text: &str) -> IcsCalendar {
    let mut cal = IcsCalendar::default();
    let mut stack: Vec<String> = vec![];
    let mut cur: Option<Builder> = None;
    let mut tz_name: Option<String> = None;
    let mut tz_std: Option<i32> = None;
    let mut tz_any: Option<i32> = None;
    for raw in unfold(text) {
        let Some(line) = parse_line(&raw) else { continue };
        match line.name.as_str() {
            "BEGIN" => {
                let kind = line.value.trim().to_ascii_uppercase();
                match kind.as_str() {
                    "VEVENT" if cal.events.len() < MAX_EVENTS => cur = Some(Builder::default()),
                    "VTIMEZONE" => {
                        tz_name = None;
                        tz_std = None;
                        tz_any = None;
                    }
                    _ => {}
                }
                stack.push(kind);
                continue;
            }
            "END" => {
                let kind = line.value.trim().to_ascii_uppercase();
                while let Some(top) = stack.pop() {
                    if top == kind {
                        break;
                    }
                }
                match kind.as_str() {
                    "VEVENT" => {
                        if let Some(b) = cur.take() {
                            if let Some(e) = b.finish(cal.events.len()) {
                                cal.events.push(e);
                            }
                        }
                    }
                    "VTIMEZONE" => {
                        if let (Some(n), Some(o)) = (tz_name.take(), tz_std.or(tz_any)) {
                            cal.timezones.push((n, o));
                        }
                    }
                    _ => {}
                }
                continue;
            }
            _ => {}
        }
        let top = stack.last().map(String::as_str).unwrap_or("");
        match top {
            "VCALENDAR" if line.name == "X-WR-CALNAME" => cal.name = unescape(&line.value),
            "VTIMEZONE" if line.name == "TZID" => tz_name = Some(line.value.trim().to_string()),
            "STANDARD" | "DAYLIGHT" if line.name == "TZOFFSETTO" => {
                if let Some(o) = parse_offset(&line.value) {
                    tz_any.get_or_insert(o);
                    if top == "STANDARD" {
                        tz_std.get_or_insert(o);
                    }
                }
            }
            "VALARM" if line.name == "TRIGGER" => {
                if let (Some(b), Some(m)) = (&mut cur, parse_duration(&line.value)) {
                    if line.param("VALUE") != Some("DATE-TIME") && line.param("RELATED") != Some("END") && m <= 0 {
                        b.alarms.push((-m) as u32);
                    }
                }
            }
            "VEVENT" => {
                let Some(b) = &mut cur else { continue };
                match line.name.as_str() {
                    "UID" => b.uid = unescape(&line.value),
                    "SUMMARY" => b.summary = unescape(&line.value),
                    "LOCATION" => b.location = unescape(&line.value),
                    "DESCRIPTION" => b.notes = unescape(&line.value),
                    "COLOR" => b.color = Some(line.value.trim().to_string()),
                    "DTSTART" => {
                        b.start = parse_when(&line.value, line.param("VALUE"));
                        b.start_tz = line.param("TZID").map(str::to_string);
                    }
                    "DTEND" => b.end = parse_when(&line.value, line.param("VALUE")).map(|w| w.0),
                    "DURATION" => b.duration = parse_duration(&line.value),
                    "RRULE" => b.rule = recur::parse_rrule(&line.value),
                    "ATTENDEE" => {
                        let (cn, role, cutype, partstat) = (line.param("CN"), line.param("ROLE"), line.param("CUTYPE"), line.param("PARTSTAT"));
                        if let Some(p) = invitees::attendee(&line.value, cn, role, cutype, partstat) {
                            b.attendees.push(p);
                        }
                    }
                    "EXDATE" => {
                        for v in line.value.split(',') {
                            if let Some((l, _, _)) = parse_when(v, line.param("VALUE")) {
                                b.exdates.push(l.date);
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    cal
}

// ---------------------------------------------------------------- writing

/// Folds a content line at 75 octets without splitting a character.
pub fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3);
    let mut used = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if used + n > MAX_LINE {
            out.push_str("\r\n ");
            used = 1;
        }
        out.push(c);
        used += n;
    }
    out.push_str("\r\n");
    out
}

pub fn format_trigger(minutes: u32) -> String {
    match minutes {
        0 => "PT0S".into(),
        m if m % 1440 == 0 => format!("-P{}D", m / 1440),
        m if m % 60 == 0 => format!("-PT{}H", m / 60),
        m => format!("-PT{m}M"),
    }
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time (DTSTAMP).
pub fn stamp(unix: i64) -> String {
    let c = springboard::shell::clock::tz::civil(unix, 0);
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", c.year, c.month, c.day, c.hour, c.min, c.sec)
}

fn format_local(l: Local) -> String {
    format!("{}T{:02}{:02}00", recur::format_ymd(l.date), l.minute / 60, l.minute % 60)
}

fn format_offset(minutes: i32) -> String {
    let a = minutes.abs();
    format!("{}{:02}{:02}", if minutes < 0 { '-' } else { '+' }, a / 60, a % 60)
}

fn tz_param(tzid: &str) -> String {
    if tzid.contains([';', ':', ',']) {
        format!("TZID=\"{tzid}\"")
    } else {
        format!("TZID={tzid}")
    }
}

/// `(parameter suffix, value)` of a DTSTART/DTEND style property.
fn when_field(l: Local, e: &IcsEvent) -> (String, String) {
    if e.all_day {
        (";VALUE=DATE".into(), recur::format_ymd(l.date))
    } else {
        match e.tzid.as_deref() {
            Some("UTC") => (String::new(), format!("{}Z", format_local(l))),
            Some(tz) => (format!(";{}", tz_param(tz)), format_local(l)),
            None => (String::new(), format_local(l)),
        }
    }
}

fn write_event(out: &mut String, e: &IcsEvent, stamp: &str) {
    let mut add = |s: String| out.push_str(&fold(&s));
    add("BEGIN:VEVENT".into());
    add(format!("UID:{}", escape(&e.uid)));
    add(format!("DTSTAMP:{stamp}"));
    let (p, v) = when_field(e.start, e);
    add(format!("DTSTART{p}:{v}"));
    let (p, v) = when_field(e.end, e);
    add(format!("DTEND{p}:{v}"));
    add(format!("SUMMARY:{}", escape(&e.summary)));
    if !e.location.is_empty() {
        add(format!("LOCATION:{}", escape(&e.location)));
    }
    if !e.notes.is_empty() {
        add(format!("DESCRIPTION:{}", escape(&e.notes)));
    }
    for p in &e.attendees {
        add(invitees::attendee_line(p));
    }
    if let Some(r) = &e.rule {
        add(format!("RRULE:{}", recur::to_rrule(r)));
    }
    if !e.exdates.is_empty() {
        let (p, _) = when_field(e.start, e);
        let values: Vec<String> = e
            .exdates
            .iter()
            .map(|&d| {
                when_field(Local::new(d, e.start.minute), e).1
            })
            .collect();
        add(format!("EXDATE{p}:{}", values.join(",")));
    }
    if let Some(c) = &e.color {
        add(format!("COLOR:{c}"));
    }
    for &m in &e.alarms {
        add("BEGIN:VALARM".into());
        add("ACTION:DISPLAY".into());
        add(format!("DESCRIPTION:{}", escape(&e.summary)));
        add(format!("TRIGGER:{}", format_trigger(m)));
        add("END:VALARM".into());
    }
    add("END:VEVENT".into());
}

/// Serialises a calendar; `stamp` is the DTSTAMP value (see [`stamp`]).
pub fn write(cal: &IcsCalendar, stamp: &str) -> String {
    let mut out = String::new();
    let add = |out: &mut String, s: String| out.push_str(&fold(&s));
    add(&mut out, "BEGIN:VCALENDAR".into());
    add(&mut out, "VERSION:2.0".into());
    add(&mut out, "PRODID:-//BifrOSt//Calendar//EN".into());
    add(&mut out, "CALSCALE:GREGORIAN".into());
    if !cal.name.is_empty() {
        add(&mut out, format!("X-WR-CALNAME:{}", escape(&cal.name)));
    }
    let mut written: Vec<&str> = vec![];
    for e in &cal.events {
        let Some(tz) = e.tzid.as_deref().filter(|t| *t != "UTC" && !e.all_day) else { continue };
        let Some(&(_, off)) = cal.timezones.iter().find(|(n, _)| n == tz) else { continue };
        if written.contains(&tz) {
            continue;
        }
        written.push(tz);
        add(&mut out, "BEGIN:VTIMEZONE".into());
        add(&mut out, format!("TZID:{tz}"));
        add(&mut out, "BEGIN:STANDARD".into());
        add(&mut out, "DTSTART:19700101T000000".into());
        add(&mut out, format!("TZOFFSETFROM:{}", format_offset(off)));
        add(&mut out, format!("TZOFFSETTO:{}", format_offset(off)));
        add(&mut out, "END:STANDARD".into());
        add(&mut out, "END:VTIMEZONE".into());
    }
    for e in &cal.events {
        write_event(&mut out, e, stamp);
    }
    add(&mut out, "END:VCALENDAR".into());
    out
}

/// One event as a complete .ics file (share / export a single event).
pub fn write_single(e: &IcsEvent, timezones: &[(String, i32)], stamp: &str) -> String {
    let cal = IcsCalendar { name: String::new(), events: vec![e.clone()], timezones: timezones.to_vec() };
    write(&cal, stamp)
}

// ------------------------------------------------- store event conversion

pub fn from_calendar_event(e: &CalendarEvent) -> IcsEvent {
    let start = Local::new(e.date, if e.all_day { 0 } else { e.minute });
    let end = if e.all_day { start.add_minutes(1440) } else { start.add_minutes(e.duration as i64) };
    let mut ev = IcsEvent::new(&format!("{}@springboard.tab", e.id), &e.title, start, end);
    ev.all_day = e.all_day;
    ev
}

/// Multi-day events are cut to 24 hours, the store's maximum.
pub fn to_calendar_event(e: &IcsEvent, id: u64) -> Option<CalendarEvent> {
    let title: String = e.summary.chars().filter(|c| !c.is_control()).take(160).collect();
    let title = title.trim().to_string();
    let ev = CalendarEvent {
        id,
        date: e.start.date,
        minute: if e.all_day { 0 } else { e.start.minute },
        duration: if e.all_day { 1440 } else { e.duration_minutes().clamp(15, 1440) as u16 },
        all_day: e.all_day,
        title,
        location: String::new(),
        notes: String::new(),
        repeat: 0,
        alert: 0,
    };
    ev.valid().then_some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }
    fn l(date: Date, h: u16, m: u16) -> Local {
        Local::new(date, h * 60 + m)
    }

    #[test]
    fn unfold_handles_crlf_lf_and_tabs() {
        let lines = unfold("A:1\r\nB:long\r\n  text\nC:x\n\ty\n");
        assert_eq!(lines, vec!["A:1", "B:long text", "C:xy"]);
    }

    #[test]
    fn escape_round_trip() {
        let s = "a, b; c\\d\nline";
        assert_eq!(escape(s), "a\\, b\\; c\\\\d\\nline");
        assert_eq!(unescape(&escape(s)), s);
    }

    #[test]
    fn fold_limits_octets_and_keeps_chars() {
        let long = format!("SUMMARY:{}", "\u{00fe}".repeat(80));
        let folded = fold(&long);
        for part in folded.split("\r\n").filter(|p| !p.is_empty()) {
            assert!(part.len() <= 75, "{} octets", part.len());
        }
        assert_eq!(unfold(&folded), vec![long]);
    }

    #[test]
    fn duration_parsing() {
        assert_eq!(parse_duration("PT1H30M"), Some(90));
        assert_eq!(parse_duration("-PT15M"), Some(-15));
        assert_eq!(parse_duration("P1D"), Some(1440));
        assert_eq!(parse_duration("P1W"), Some(10_080));
        assert_eq!(parse_duration("P1DT2H"), Some(1560));
        assert_eq!(parse_duration("PT0S"), Some(0));
        assert_eq!(parse_duration("P"), None);
        assert_eq!(parse_duration("PT5"), None);
        assert_eq!(parse_duration("1H"), None);
    }

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Work\r\n\
BEGIN:VTIMEZONE\r\nTZID:Atlantic/Reykjavik\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\n\
TZOFFSETFROM:+0000\r\nTZOFFSETTO:+0000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:a@x\r\nDTSTART;TZID=Atlantic/Reykjavik:20261005T090000\r\n\
DTEND;TZID=Atlantic/Reykjavik:20261005T103000\r\nSUMMARY:Stand\\, up\r\n\
LOCATION:Room 4\r\nDESCRIPTION:Line one\\nLine\r\n  two\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6\r\nEXDATE;TZID=Atlantic/Reykjavik:20261007T090000\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-P1D\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b@x\r\nDTSTART;VALUE=DATE:20261010\r\nDTEND;VALUE=DATE:20261012\r\nSUMMARY:Trip\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:c@x\r\nDTSTART:20261011T120000Z\r\nDURATION:PT45M\r\nSUMMARY:Call\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:bad\r\nSUMMARY:No start\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parses_sample_calendar() {
        let cal = parse(SAMPLE);
        assert_eq!(cal.name, "Work");
        assert_eq!(cal.timezones, vec![("Atlantic/Reykjavik".to_string(), 0)]);
        assert_eq!(cal.events.len(), 3, "event without DTSTART is skipped");
        let a = &cal.events[0];
        assert_eq!(a.uid, "a@x");
        assert_eq!(a.summary, "Stand, up");
        assert_eq!(a.location, "Room 4");
        assert_eq!(a.notes, "Line one\nLine two");
        assert_eq!(a.start, l(d(2026, 10, 5), 9, 0));
        assert_eq!(a.end, l(d(2026, 10, 5), 10, 30));
        assert_eq!(a.tzid.as_deref(), Some("Atlantic/Reykjavik"));
        assert_eq!(a.rule.as_ref().unwrap().count, Some(6));
        assert_eq!(a.exdates, vec![d(2026, 10, 7)]);
        assert_eq!(a.alarms, vec![15, 1440]);
        let b = &cal.events[1];
        assert!(b.all_day);
        assert_eq!(b.end.date, d(2026, 10, 12));
        let c = &cal.events[2];
        assert_eq!(c.tzid.as_deref(), Some("UTC"));
        assert_eq!(c.duration_minutes(), 45);
    }

    #[test]
    fn write_then_parse_round_trips() {
        let cal = parse(SAMPLE);
        let text = write(&cal, "20261005T120000Z");
        assert!(text.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(text.contains("BEGIN:VTIMEZONE\r\nTZID:Atlantic/Reykjavik\r\n"));
        assert!(text.contains("DTSTAMP:20261005T120000Z\r\n"));
        assert!(text.contains("RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6\r\n"));
        assert!(text.contains("TRIGGER:-P1D\r\n"));
        assert!(text.contains("DTSTART;VALUE=DATE:20261010\r\n"));
        assert!(text.contains("DTSTART:20261011T120000Z\r\n"));
        for part in text.split("\r\n") {
            assert!(part.len() <= 75);
        }
        assert_eq!(parse(&text), cal);
    }

    #[test]
    fn long_titles_survive_folding() {
        let title = "Quarterly planning, with \u{00e9}clair \u{1f370} and more ".repeat(5);
        let mut e = IcsEvent::new("u", &title, l(d(2026, 10, 5), 9, 0), l(d(2026, 10, 5), 10, 0));
        e.notes = "x;y\nz".into();
        let text = write_single(&e, &[], "20261005T000000Z");
        let back = parse(&text);
        assert_eq!(back.events[0].summary, title);
        assert_eq!(back.events[0].notes, "x;y\nz");
    }

    #[test]
    fn missing_uid_is_generated_and_end_defaults() {
        let cal = parse("BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20261010\nSUMMARY:S\nEND:VEVENT\nEND:VCALENDAR\n");
        let e = &cal.events[0];
        assert!(e.uid.starts_with("imported-"));
        assert_eq!(e.end.date, d(2026, 10, 11));
    }

    #[test]
    fn garbage_is_rejected_gracefully() {
        assert!(parse("").events.is_empty());
        assert!(parse("not ics at all").events.is_empty());
        let cal = parse("BEGIN:VEVENT\nDTSTART:2026\nSUMMARY:x\nEND:VEVENT\n");
        assert!(cal.events.is_empty());
    }

    #[test]
    fn quoted_params_may_contain_separators() {
        let line = parse_line("DTSTART;TZID=\"A;B:C\":20261005T090000").unwrap();
        assert_eq!(line.param("TZID"), Some("A;B:C"));
        assert_eq!(line.value, "20261005T090000");
    }

    #[test]
    fn store_event_conversion() {
        let e = CalendarEvent {
            id: 7,
            date: d(2026, 10, 5),
            minute: 600,
            duration: 90,
            all_day: false,
            title: "Dentist".into(),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert: 0,
        };
        let ics = from_calendar_event(&e);
        assert_eq!(ics.uid, "7@springboard.tab");
        assert_eq!(ics.end, l(d(2026, 10, 5), 11, 30));
        assert_eq!(to_calendar_event(&ics, 7), Some(e));
        let mut all_day = from_calendar_event(&CalendarEvent {
            id: 8,
            date: d(2026, 10, 31),
            minute: 0,
            duration: 1440,
            all_day: true,
            title: "Halloween".into(),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert: 0,
        });
        assert_eq!(all_day.end.date, d(2026, 11, 1));
        all_day.end = all_day.end.add_minutes(3 * 1440);
        assert_eq!(to_calendar_event(&all_day, 8).unwrap().duration, 1440);
        let mut blank = ics.clone();
        blank.summary = "  ".into();
        assert_eq!(to_calendar_event(&blank, 9), None);
    }

    #[test]
    fn local_arithmetic_crosses_days() {
        let a = l(d(2026, 12, 31), 23, 30);
        assert_eq!(a.add_minutes(45), l(d(2027, 1, 1), 0, 15));
        assert_eq!(a.add_minutes(-1440 - 30), l(d(2026, 12, 30), 23, 0));
        assert_eq!(l(d(2027, 1, 1), 0, 15).minutes_since(a), 45);
    }

    #[test]
    fn stamp_format() {
        assert_eq!(stamp(0), "19700101T000000Z");
        assert_eq!(stamp(86_400 + 3_661), "19700102T010101Z");
    }

    #[test]
    fn trigger_format() {
        assert_eq!(format_trigger(0), "PT0S");
        assert_eq!(format_trigger(15), "-PT15M");
        assert_eq!(format_trigger(120), "-PT2H");
        assert_eq!(format_trigger(2880), "-P2D");
    }
}

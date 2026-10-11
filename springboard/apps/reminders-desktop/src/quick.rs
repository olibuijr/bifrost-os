//! Quick-add parsing: "buy milk tomorrow 9am", "pay rent every month",
//! "call mom in 2 hours !!". Pure Rust, no locale tables beyond English.
use springboard::shell::clock::tz::{civil_from_days, days_from_civil};
use crate::repeat::{add_months, Repeat, Unit};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quick {
    pub title: String,
    pub due: Option<i64>,
    /// minutes after local midnight
    pub time: Option<u16>,
    pub repeat: Option<Repeat>,
    /// 0 none, 1 low, 2 medium, 3 high
    pub priority: u8,
}

#[derive(Default)]
struct Hit {
    day: Option<i64>,
    time: Option<u16>,
    repeat: Option<Repeat>,
    prio: u8,
    rel_min: Option<i64>,
}

const WEEKDAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];
const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn weekday_of(tok: &str) -> Option<i64> {
    if tok.len() < 3 {
        return None;
    }
    let tok = if tok == "tues" || tok == "thur" || tok == "thurs" || tok == "weds" {
        &tok[..3]
    } else {
        tok
    };
    WEEKDAYS
        .iter()
        .position(|w| *w == tok || (tok.len() == 3 && w.starts_with(tok)))
        .map(|i| i as i64)
}
fn month_of(tok: &str) -> Option<u32> {
    let tok = if tok == "sept" { "sep" } else { tok };
    if tok.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| *m == tok || (tok.len() == 3 && m.starts_with(tok)))
        .map(|i| i as u32 + 1)
}
fn day_number(tok: &str) -> Option<u32> {
    let t = ["st", "nd", "rd", "th"]
        .iter()
        .find_map(|s| tok.strip_suffix(s))
        .unwrap_or(tok);
    let n: u32 = t.parse().ok()?;
    (1..=31).contains(&n).then_some(n)
}
/// The next date strictly after `today` that falls on weekday `wd` (Mon = 0).
fn next_weekday(today: i64, wd: i64) -> i64 {
    let cur = (today + 3).rem_euclid(7);
    let delta = (wd - cur).rem_euclid(7);
    today + if delta == 0 { 7 } else { delta }
}
fn unit_of(tok: &str) -> Option<Unit> {
    let t = tok.strip_suffix('s').unwrap_or(tok);
    match t {
        "day" => Some(Unit::Day),
        "week" => Some(Unit::Week),
        "month" => Some(Unit::Month),
        "year" => Some(Unit::Year),
        _ => None,
    }
}
fn count_of(tok: &str) -> Option<u32> {
    match tok {
        "a" | "an" | "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        _ => tok.parse().ok().filter(|n| (1..=999).contains(n)),
    }
}

/// A clock time at `j`; the second value is how many tokens it used.
fn clock(toks: &[String], j: usize, bare: bool) -> Option<(u16, usize)> {
    let tok = toks.get(j)?.as_str();
    match tok {
        "noon" => return Some((720, 1)),
        "midnight" => return Some((0, 1)),
        _ => {}
    }
    let (body, mut ampm) = if let Some(b) = tok.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = tok.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (tok, None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse::<u16>().ok()?, m.parse::<u16>().ok()?),
        Some(_) => return None,
        None => {
            if ampm.is_none() && !bare {
                return None;
            }
            (body.parse::<u16>().ok()?, 0)
        }
    };
    let mut used = 1;
    if ampm.is_none() {
        match toks.get(j + 1).map(String::as_str) {
            Some("am") => {
                ampm = Some(false);
                used = 2;
            }
            Some("pm") => {
                ampm = Some(true);
                used = 2;
            }
            _ => {}
        }
    }
    if m > 59 {
        return None;
    }
    let h = match ampm {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            h % 12 + if pm { 12 } else { 0 }
        }
        None => {
            if h > 23 {
                return None;
            }
            h
        }
    };
    Some((h * 60 + m, used))
}

fn match_at(toks: &[String], j: usize, today: i64, bare: bool) -> Option<(usize, Hit)> {
    let tok = toks.get(j)?.as_str();
    let next = toks.get(j + 1).map(String::as_str);
    let day = |d: i64| Hit { day: Some(d), ..Hit::default() };
    match tok {
        "today" => return Some((1, day(today))),
        "tonight" => {
            return Some((1, Hit { day: Some(today), time: Some(20 * 60), ..Hit::default() }))
        }
        "tomorrow" | "tmrw" | "tmr" => return Some((1, day(today + 1))),
        "daily" => return Some((1, Hit { repeat: Some(Repeat::new(Unit::Day, 1)), ..Hit::default() })),
        "weekly" => return Some((1, Hit { repeat: Some(Repeat::new(Unit::Week, 1)), ..Hit::default() })),
        "monthly" => return Some((1, Hit { repeat: Some(Repeat::new(Unit::Month, 1)), ..Hit::default() })),
        "yearly" | "annually" => {
            return Some((1, Hit { repeat: Some(Repeat::new(Unit::Year, 1)), ..Hit::default() }))
        }
        "next" => {
            if next == Some("week") {
                return Some((2, day(today + 7)));
            }
            if let Some(wd) = next.and_then(weekday_of) {
                return Some((2, day(next_weekday(today, wd))));
            }
            return None;
        }
        "in" => {
            let n = count_of(next?)?;
            let unit = toks.get(j + 2)?.as_str();
            let n64 = n as i64;
            let hit = if unit.starts_with("min") {
                Hit { rel_min: Some(n64), ..Hit::default() }
            } else if unit.starts_with("hour") || unit == "hr" || unit == "hrs" {
                Hit { rel_min: Some(n64 * 60), ..Hit::default() }
            } else {
                match unit_of(unit)? {
                    Unit::Day => day(today + n64),
                    Unit::Week => day(today + 7 * n64),
                    Unit::Month => day(add_months(today, n64)),
                    Unit::Year => day(add_months(today, 12 * n64)),
                }
            };
            return Some((3, hit));
        }
        "every" => {
            let n1 = next?;
            if let Some(unit) = unit_of(n1) {
                return Some((2, Hit { repeat: Some(Repeat::new(unit, 1)), ..Hit::default() }));
            }
            if n1 == "other" {
                let unit = unit_of(toks.get(j + 2)?)?;
                return Some((3, Hit { repeat: Some(Repeat::new(unit, 2)), ..Hit::default() }));
            }
            if n1 == "weekday" {
                return Some((2, Hit { repeat: Some(Repeat::new(Unit::Day, 1)), ..Hit::default() }));
            }
            if let Some(wd) = weekday_of(n1) {
                return Some((
                    2,
                    Hit {
                        day: Some(next_weekday(today, wd)),
                        repeat: Some(Repeat::new(Unit::Week, 1)),
                        ..Hit::default()
                    },
                ));
            }
            let n = count_of(n1)?;
            let unit = unit_of(toks.get(j + 2)?)?;
            return Some((3, Hit { repeat: Some(Repeat::new(unit, n)), ..Hit::default() }));
        }
        _ => {}
    }
    if !tok.is_empty() && tok.len() <= 3 && tok.chars().all(|c| c == '!') {
        return Some((1, Hit { prio: tok.len() as u8, ..Hit::default() }));
    }
    if let Some(wd) = weekday_of(tok) {
        return Some((1, day(next_weekday(today, wd))));
    }
    if let Some((time, used)) = clock(toks, j, bare) {
        return Some((used, Hit { time: Some(time), ..Hit::default() }));
    }
    // "oct 12", "october 12th" / "12 oct", "12th october"
    let (month, dnum, used) = if let (Some(m), Some(d)) = (month_of(tok), next.and_then(day_number)) {
        (m, d, 2)
    } else if let (Some(d), Some(m)) = (day_number(tok), next.and_then(month_of)) {
        (m, d, 2)
    } else {
        return None;
    };
    let (year, _, _) = civil_from_days(today);
    let mut date = days_from_civil(year, month, dnum);
    if civil_from_days(date).2 != dnum {
        return None;
    }
    if date < today {
        date = days_from_civil(year + 1, month, dnum);
        if civil_from_days(date).2 != dnum {
            return None;
        }
    }
    Some((used, day(date)))
}

/// Parses `text` against the local `today` (day number) and `now_min`
/// (minutes since local midnight). Unparsed words become the title; when
/// nothing would be left, the whole text is the title and nothing is parsed.
pub fn parse(text: &str, today: i64, now_min: u16) -> Quick {
    let words: Vec<&str> = text.split_whitespace().collect();
    let lower: Vec<String> = words
        .iter()
        .map(|w| w.trim_end_matches([',', '.', ';']).to_lowercase())
        .collect();
    let mut hit = Hit::default();
    let mut keep: Vec<&str> = vec![];
    let mut i = 0;
    while i < words.len() {
        let prep = matches!(lower[i].as_str(), "on" | "at" | "by" | "due" | "from");
        if prep {
            if let Some((n, h)) = match_at(&lower, i + 1, today, lower[i] == "at") {
                merge(&mut hit, h);
                i += 1 + n;
                continue;
            }
        }
        if let Some((n, h)) = match_at(&lower, i, today, false) {
            merge(&mut hit, h);
            i += n;
            continue;
        }
        keep.push(words[i]);
        i += 1;
    }
    let title = keep.join(" ");
    if title.is_empty() {
        return Quick {
            title: words.join(" "),
            ..Quick::default()
        };
    }
    let mut due = hit.day;
    let mut time = hit.time;
    if let Some(rel) = hit.rel_min {
        let total = today * 1440 + now_min as i64 + rel;
        due = Some(total.div_euclid(1440));
        time = Some(total.rem_euclid(1440) as u16);
    } else if due.is_none() {
        if let Some(t) = time {
            due = Some(if t > now_min { today } else { today + 1 });
        } else if hit.repeat.is_some() {
            due = Some(today);
        }
    }
    Quick {
        title,
        due,
        time,
        repeat: hit.repeat,
        priority: hit.prio,
    }
}

fn merge(into: &mut Hit, from: Hit) {
    into.day = into.day.or(from.day);
    into.time = into.time.or(from.time);
    into.repeat = into.repeat.or(from.repeat);
    into.rel_min = into.rel_min.or(from.rel_min);
    into.prio = into.prio.max(from.prio);
}

#[cfg(test)]
mod tests {
    use super::*;
    // 2026-10-05 is a Monday.
    fn today() -> i64 {
        days_from_civil(2026, 10, 5)
    }
    fn p(s: &str) -> Quick {
        parse(s, today(), 10 * 60)
    }
    #[test]
    fn tomorrow_with_time() {
        let q = p("buy milk tomorrow 9am");
        assert_eq!(q.title, "buy milk");
        assert_eq!(q.due, Some(today() + 1));
        assert_eq!(q.time, Some(9 * 60));
    }
    #[test]
    fn plain_text_is_untouched() {
        let q = p("buy milk");
        assert_eq!(q, Quick { title: "buy milk".into(), ..Quick::default() });
    }
    #[test]
    fn only_a_date_phrase_stays_a_title() {
        let q = p("tomorrow");
        assert_eq!(q.title, "tomorrow");
        assert_eq!(q.due, None);
    }
    #[test]
    fn weekdays_are_strictly_in_the_future() {
        assert_eq!(p("gym monday").due, Some(today() + 7));
        assert_eq!(p("gym on fri").due, Some(today() + 4));
        assert_eq!(p("gym next tuesday").due, Some(today() + 1));
        assert_eq!(p("review next week").due, Some(today() + 7));
    }
    #[test]
    fn times_in_several_forms() {
        assert_eq!(p("call at 15:30").time, Some(15 * 60 + 30));
        assert_eq!(p("call 9:30pm").time, Some(21 * 60 + 30));
        assert_eq!(p("call at 5 pm").time, Some(17 * 60));
        assert_eq!(p("call at 7").time, Some(7 * 60));
        assert_eq!(p("lunch at noon").time, Some(12 * 60));
        assert_eq!(p("lunch 12am").time, Some(0));
        assert_eq!(p("call 13pm").time, None);
        assert_eq!(p("buy 3 apricots").time, None);
    }
    #[test]
    fn time_without_day_picks_next_occurrence() {
        assert_eq!(p("stretch 11am").due, Some(today()));
        assert_eq!(p("stretch 8am").due, Some(today() + 1));
    }
    #[test]
    fn relative_offsets() {
        let q = p("laundry in 2 hours");
        assert_eq!((q.due, q.time), (Some(today()), Some(12 * 60)));
        let q = parse("laundry in 90 minutes", today(), 23 * 60);
        assert_eq!((q.due, q.time), (Some(today() + 1), Some(30)));
        assert_eq!(p("renew in 3 days").due, Some(today() + 3));
        assert_eq!(p("renew in a week").due, Some(today() + 7));
        assert_eq!(p("renew in 2 months").due, Some(days_from_civil(2026, 12, 5)));
    }
    #[test]
    fn month_day_dates_roll_to_next_year() {
        assert_eq!(p("party oct 12").due, Some(days_from_civil(2026, 10, 12)));
        assert_eq!(p("party 3 march").due, Some(days_from_civil(2027, 3, 3)));
        assert_eq!(p("party feb 30").due, None);
        assert_eq!(p("taxes April 15th").due, Some(days_from_civil(2027, 4, 15)));
    }
    #[test]
    fn repeat_phrases() {
        let q = p("pay rent every month");
        assert_eq!(q.repeat, Some(Repeat::new(Unit::Month, 1)));
        assert_eq!(q.due, Some(today()));
        assert_eq!(q.title, "pay rent");
        assert_eq!(p("water plants every 3 days").repeat, Some(Repeat::new(Unit::Day, 3)));
        assert_eq!(p("sweep every other week").repeat, Some(Repeat::new(Unit::Week, 2)));
        let q = p("standup every monday 9:15am");
        assert_eq!(q.repeat, Some(Repeat::new(Unit::Week, 1)));
        assert_eq!(q.due, Some(today() + 7));
        assert_eq!(q.time, Some(9 * 60 + 15));
        assert_eq!(p("floss daily").repeat, Some(Repeat::new(Unit::Day, 1)));
    }
    #[test]
    fn priority_marks() {
        let q = p("fix bug !!!");
        assert_eq!((q.priority, q.title.as_str()), (3, "fix bug"));
        assert_eq!(p("fix ! bug").priority, 1);
    }
    #[test]
    fn tonight_and_punctuation() {
        let q = p("movie tonight,");
        assert_eq!(q.time, Some(20 * 60));
        assert_eq!(q.due, Some(today()));
        assert_eq!(q.title, "movie");
    }
}

//! New Event from Text: reads a sentence such as "Lunch with Ana next Tuesday at 12:30 at Laugavegur 5" into an
//! event's title, date, start time and location. The explicit dates and the street addresses are the shared
//! detector's (`bifrost_app::controls::detect`, the rules the Notes app uses); the weekdays, "today", "tomorrow" and
//! the clock times are English, like the rest of the calendar's text. Only the first line of the text is read.
use bifrost_app::controls::detect::{self, Kind};
use springboard::shell::calendar::Date;

/// What a sentence says about an event. A field the sentence does not mention is `None` (the editor keeps its
/// defaults: the selected day, 9:00, no location).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quick {
    /// The words before the first scheduling phrase (a date, a day, a time or "at" a place).
    pub title: String,
    pub date: Option<Date>,
    /// Minute of the day the event starts.
    pub minute: Option<u16>,
    /// The first street address, as written.
    pub location: Option<String>,
}

/// Weekday names, Sunday first (the index `Date::weekday` returns).
const WEEKDAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
/// "tonight" with no time given starts at 19:00.
const EVENING: u16 = 19 * 60;

/// The words of `line` as byte ranges, without trailing sentence punctuation.
fn words(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices().chain(std::iter::once((line.len(), ' '))) {
        match (start, c.is_whitespace()) {
            (None, false) => start = Some(i),
            (Some(s), true) => {
                let mut end = i;
                while end > s && line[..end].ends_with([',', '.', ';', ':', '!', '?']) {
                    end -= 1;
                }
                if end > s {
                    out.push((s, end));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn weekday(word: &str) -> Option<usize> {
    WEEKDAYS.iter().position(|d| *d == word)
}

/// The date of weekday `wd` after `today`. `strict` skips today itself ("next Tuesday" said on a Tuesday is a week on).
fn next_weekday(today: Date, wd: usize, strict: bool) -> Date {
    let delta = (wd + 7 - today.weekday()) % 7;
    let delta = if delta == 0 && strict { 7 } else { delta };
    today.shift_days(delta as i32)
}

/// `2026-10-20` or `20.10.2026`, a real calendar date.
fn parse_date(text: &str) -> Option<Date> {
    let (year, month, day) = if text.len() == 10 && text.as_bytes()[4] == b'-' {
        let mut p = text.split('-');
        (p.next()?, p.next()?, p.next()?)
    } else {
        let mut p = text.split('.');
        let (d, m, y) = (p.next()?, p.next()?, p.next()?);
        (y, m, d)
    };
    let date = Date { year: year.parse().ok()?, month: month.parse().ok()?, day: day.parse().ok()? };
    date.valid().then_some(date)
}

/// A clock time at word `i`: "12:30", "12:30pm", "5pm", "noon", or "5 pm" (two words). The minute of the day and the
/// number of words used. A bare "5" is not a time.
fn clock(lows: &[String], i: usize) -> Option<(u16, usize)> {
    let w = lows.get(i)?.as_str();
    if w == "noon" {
        return Some((720, 1));
    }
    let (body, mut pm) = match w.strip_suffix("pm") {
        Some(b) => (b, Some(true)),
        None => match w.strip_suffix("am") {
            Some(b) => (b, Some(false)),
            None => (w, None),
        },
    };
    let mut used = 1;
    let (hour, minute) = match body.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse::<u16>().ok()?, m.parse::<u16>().ok()?),
        Some(_) => return None,
        None => {
            if pm.is_none() {
                pm = match lows.get(i + 1).map(String::as_str) {
                    Some("am") => Some(false),
                    Some("pm") => Some(true),
                    _ => return None,
                };
                used = 2;
            }
            (body.parse::<u16>().ok()?, 0)
        }
    };
    let minute = match pm {
        Some(pm) => {
            if !(1..=12).contains(&hour) || minute > 59 {
                return None;
            }
            (hour % 12 + if pm { 12 } else { 0 }) * 60 + minute
        }
        None => {
            if hour > 23 || minute > 59 {
                return None;
            }
            hour * 60 + minute
        }
    };
    Some((minute, used))
}

/// Reads `text` (its first line) as an event, with `today` as the day "today", "tomorrow" and the weekdays refer to.
pub fn parse(text: &str, today: Date) -> Quick {
    let line = text.lines().next().unwrap_or("").trim();
    let toks = words(line);
    let lows: Vec<String> = toks.iter().map(|&(s, e)| line[s..e].to_lowercase()).collect();
    let dates = detect::find(line, &[Kind::Date]);
    let location = detect::find(line, &[Kind::Address]).into_iter().next();
    let mut quick = Quick { location: location.as_ref().map(|m| m.text(line).to_string()), ..Quick::default() };
    // where the scheduling phrase begins: everything before it is the title
    let mut start: Option<usize> = None;
    let mut tonight = false;
    let mut i = 0;
    while i < toks.len() {
        let at = toks[i].0;
        let w = lows[i].as_str();
        let next = lows.get(i + 1).map(String::as_str);
        // an explicit date: 2026-10-20 or 20.10.2026
        if let Some(m) = dates.iter().find(|m| m.range.start == at) {
            if let Some(d) = parse_date(m.text(line)) {
                quick.date = Some(d);
                start.get_or_insert(at);
                while i < toks.len() && toks[i].0 < m.range.end {
                    i += 1;
                }
                continue;
            }
        }
        let mut eat = 1;
        if w == "today" || w == "tonight" {
            quick.date = Some(today);
            tonight |= w == "tonight";
            start.get_or_insert(at);
        } else if w == "tomorrow" {
            quick.date = Some(today.shift_days(1));
            start.get_or_insert(at);
        } else if let Some(wd) = next.and_then(weekday).filter(|_| matches!(w, "next" | "this" | "on")) {
            // "next Tuesday", "this Friday", "on Monday"
            quick.date = Some(next_weekday(today, wd, w == "next"));
            start.get_or_insert(at);
            eat = 2;
        } else if let Some(wd) = weekday(w).filter(|_| i > 0) {
            // a bare weekday, not the first word (so "Friday drinks" keeps its title)
            quick.date = Some(next_weekday(today, wd, false));
            start.get_or_insert(at);
        } else if w == "at" {
            let after = toks.get(i + 1).map(|t| t.0);
            if let Some((minute, used)) = clock(&lows, i + 1) {
                quick.minute.get_or_insert(minute);
                start.get_or_insert(at);
                eat = 1 + used;
            } else if location.as_ref().is_some_and(|m| Some(m.range.start) == after) {
                start.get_or_insert(at);
            }
        } else if let Some((minute, used)) = clock(&lows, i) {
            quick.minute.get_or_insert(minute);
            start.get_or_insert(at);
            eat = used;
        }
        i += eat;
    }
    if tonight {
        quick.minute.get_or_insert(EVENING);
    }
    quick.title = match start {
        Some(s) => line[..s].trim().to_string(),
        None => line.to_string(),
    };
    quick
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tuesday 2026-10-13 (`Date::weekday` 2).
    fn today() -> Date {
        Date { year: 2026, month: 10, day: 13 }
    }

    #[test]
    fn the_example_sentence_fills_every_field() {
        let q = parse("Lunch with Ana next Tuesday at 12:30 at Laugavegur 5", today());
        assert_eq!(q.title, "Lunch with Ana");
        // "next Tuesday" said on a Tuesday is a week on
        assert_eq!(q.date, Some(Date { year: 2026, month: 10, day: 20 }));
        assert_eq!(q.minute, Some(12 * 60 + 30));
        assert_eq!(q.location.as_deref(), Some("Laugavegur 5"));
    }

    #[test]
    fn tomorrow_and_a_pm_time() {
        let q = parse("Dentist tomorrow 3pm", today());
        assert_eq!(q.title, "Dentist");
        assert_eq!(q.date, Some(Date { year: 2026, month: 10, day: 14 }));
        assert_eq!(q.minute, Some(15 * 60));
        assert_eq!(q.location, None);
    }

    #[test]
    fn a_bare_weekday_is_the_next_one_including_today() {
        let q = parse("Team sync Tuesday", today());
        assert_eq!(q.date, Some(today()));
        let q = parse("Team sync Friday", today());
        assert_eq!(q.date, Some(Date { year: 2026, month: 10, day: 16 }));
    }

    #[test]
    fn next_weekday_skips_today_only_when_strict() {
        assert_eq!(next_weekday(today(), 2, true), Date { year: 2026, month: 10, day: 20 });
        assert_eq!(next_weekday(today(), 3, true), Date { year: 2026, month: 10, day: 14 });
    }

    #[test]
    fn an_explicit_date_and_a_twenty_four_hour_time() {
        let q = parse("Flight home 2026-10-30 18:05", today());
        assert_eq!(q.title, "Flight home");
        assert_eq!(q.date, Some(Date { year: 2026, month: 10, day: 30 }));
        assert_eq!(q.minute, Some(18 * 60 + 5));
        let q = parse("Board meeting 30.10.2026 at 9:00", today());
        assert_eq!(q.date, Some(Date { year: 2026, month: 10, day: 30 }));
        assert_eq!(q.minute, Some(9 * 60));
    }

    #[test]
    fn noon_and_tonight_defaults() {
        assert_eq!(parse("Lunch at noon", today()).minute, Some(720));
        let q = parse("Drinks tonight", today());
        assert_eq!(q.date, Some(today()));
        assert_eq!(q.minute, Some(19 * 60));
        // a stated time wins over the evening default
        assert_eq!(parse("Drinks tonight at 8am", today()).minute, Some(8 * 60));
    }

    #[test]
    fn a_sentence_without_scheduling_is_only_a_title() {
        let q = parse("Buy milk", today());
        assert_eq!(q, Quick { title: "Buy milk".into(), ..Quick::default() });
        assert_eq!(parse("", today()), Quick::default());
    }

    #[test]
    fn a_leading_weekday_stays_in_the_title() {
        let q = parse("Friday drinks at 17:00", today());
        assert_eq!(q.title, "Friday drinks");
        assert_eq!(q.date, None);
        assert_eq!(q.minute, Some(17 * 60));
    }

    #[test]
    fn at_without_a_time_or_address_stays_in_the_title() {
        let q = parse("Dinner at Ana's tomorrow", today());
        assert_eq!(q.title, "Dinner at Ana's");
        assert_eq!(q.date, Some(Date { year: 2026, month: 10, day: 14 }));
    }

    #[test]
    fn only_the_first_line_is_read() {
        let q = parse("Standup 9:30\nbring the notes tomorrow", today());
        assert_eq!(q.title, "Standup");
        assert_eq!(q.minute, Some(9 * 60 + 30));
        assert_eq!(q.date, None);
    }

    #[test]
    fn invalid_clock_values_are_not_times() {
        assert_eq!(clock(&["13:00pm".to_string()], 0), None);
        assert_eq!(clock(&["25:00".to_string()], 0), None);
        assert_eq!(clock(&["5".to_string()], 0), None);
        assert_eq!(clock(&["5".to_string(), "pm".to_string()], 0), Some((17 * 60, 2)));
    }

    #[test]
    fn an_impossible_date_is_not_a_date() {
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-02-29"), None);
        assert_eq!(parse_date("2028-02-29"), Some(Date { year: 2028, month: 2, day: 29 }));
    }
}

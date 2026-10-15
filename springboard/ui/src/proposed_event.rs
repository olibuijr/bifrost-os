//! Proposed meeting time in a message: finds phrases such as "Friday at 3pm", "tomorrow 14:30" or "Monday 1pm for an hour"
//! and returns the day, start and length a new calendar event would be prefilled with.
//!
//! Pure text: no clock is read and no calendar store is touched. The day is relative (today, tomorrow or a weekday), so
//! the caller turns it into a date with its own calendar. A time with no day means today. A time needs a colon or an
//! am/pm marker, so a bare number such as a room number is not taken for a time.

use std::ops::Range;

/// Length of a proposed event when the message gives none.
pub const DEFAULT_DURATION_MINUTES: u16 = 60;

/// The longest proposed event: one day.
const MAX_DURATION_MINUTES: u32 = 24 * 60;

/// The day a proposal refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Day {
    Today,
    Tomorrow,
    /// A weekday, 0 for Monday through 6 for Sunday. The next such day counted from today, today included.
    Weekday(u8),
}

/// A meeting time found in text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposedEvent {
    pub day: Day,
    /// Minutes after midnight, 0 to 1439.
    pub start_minute: u16,
    pub duration_minutes: u16,
    /// The bytes of the source text the proposal was read from.
    pub span: Range<usize>,
}

impl ProposedEvent {
    /// Days from a day of the week (0 for Monday) to the proposed day. Today gives 0, tomorrow 1, and a weekday the
    /// number of days forward to its next occurrence, so a proposal for today's weekday gives 0.
    pub fn days_from(&self, today_weekday: u8) -> u8 {
        match self.day {
            Day::Today => 0,
            Day::Tomorrow => 1,
            Day::Weekday(day) => (day + 7 - today_weekday % 7) % 7,
        }
    }
}

struct Token {
    start: usize,
    end: usize,
    text: String,
}

const WEEKDAYS: [(&str, u8); 7] = [
    ("monday", 0),
    ("tuesday", 1),
    ("wednesday", 2),
    ("thursday", 3),
    ("friday", 4),
    ("saturday", 5),
    ("sunday", 6),
];

/// Splits text on white space. Trailing punctuation is left out of each token and its span.
fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(start) = open.take() {
                out.push(token(text, start, i));
            }
        } else if open.is_none() {
            open = Some(i);
        }
    }
    if let Some(start) = open {
        out.push(token(text, start, text.len()));
    }
    out
}

fn token(text: &str, start: usize, end: usize) -> Token {
    let raw = &text[start..end];
    let trimmed = raw.trim_end_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | ')'));
    Token { start, end: start + trimmed.len(), text: trimmed.to_lowercase() }
}

/// The day a word names: today, tomorrow, a weekday or its three-letter abbreviation.
fn day_of(word: &str) -> Option<Day> {
    match word {
        "today" => Some(Day::Today),
        "tomorrow" => Some(Day::Tomorrow),
        _ => WEEKDAYS
            .iter()
            .find(|(name, _)| *name == word || (word.len() == 3 && name.starts_with(word)))
            .map(|&(_, number)| Day::Weekday(number)),
    }
}

/// Reads a clock time from `word`, with `next` as the following word for a separate am or pm.
/// Returns the minute after midnight and whether `next` was used.
fn clock(word: &str, next: Option<&str>) -> Option<(u16, bool)> {
    let (body, meridiem, used_next) = if let Some(body) = word.strip_suffix("am") {
        (body, Some(false), false)
    } else if let Some(body) = word.strip_suffix("pm") {
        (body, Some(true), false)
    } else {
        match next {
            Some("am") => (word, Some(false), true),
            Some("pm") => (word, Some(true), true),
            _ => (word, None, false),
        }
    };
    let (hour, minute) = match body.split_once(':') {
        Some((hour, minute)) => (hour.parse::<u16>().ok()?, minute.parse::<u16>().ok()?),
        None if meridiem.is_some() => (body.parse::<u16>().ok()?, 0),
        None => return None,
    };
    if minute > 59 {
        return None;
    }
    let hour24 = match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            match (hour, pm) {
                (12, false) => 0,
                (12, true) => 12,
                (hour, false) => hour,
                (hour, true) => hour + 12,
            }
        }
        None if hour <= 23 => hour,
        None => return None,
    };
    Some((hour24 * 60 + minute, used_next))
}

/// Reads "for an hour", "for 45 minutes" or "for 2 hours" at `at`. Returns the minutes and the index of the last token used.
fn duration_after(tokens: &[Token], at: usize) -> Option<(u16, usize)> {
    if tokens.get(at)?.text != "for" {
        return None;
    }
    let amount = tokens.get(at + 1)?.text.as_str();
    let value = if amount == "an" || amount == "a" { 1 } else { amount.parse::<u32>().ok()? };
    let unit_at = at + 2;
    let per_unit = match tokens.get(unit_at)?.text.as_str() {
        "hour" | "hours" | "hr" | "hrs" => 60,
        "minute" | "minutes" | "min" | "mins" => 1,
        _ => return None,
    };
    let minutes = value.saturating_mul(per_unit);
    if minutes == 0 || minutes > MAX_DURATION_MINUTES {
        return None;
    }
    Some((minutes as u16, unit_at))
}

/// The first meeting time in `text`, or `None` when the text has no clock time.
pub fn find_proposal(text: &str) -> Option<ProposedEvent> {
    let toks = tokens(text);
    for start in 0..toks.len() {
        let mut day = Day::Today;
        let mut at = start;
        if let Some(named) = day_of(&toks[start].text) {
            day = named;
            at = start + 1;
            if toks.get(at).is_some_and(|t| t.text == "at") {
                at += 1;
            }
        } else if toks[start].text == "at" {
            at = start + 1;
        }
        let Some(time) = toks.get(at) else { continue };
        let next = toks.get(at + 1).map(|t| t.text.as_str());
        let Some((start_minute, used_meridiem)) = clock(&time.text, next) else { continue };
        let mut last = if used_meridiem { at + 1 } else { at };
        let mut duration_minutes = DEFAULT_DURATION_MINUTES;
        if let Some((minutes, end)) = duration_after(&toks, last + 1) {
            duration_minutes = minutes;
            last = end;
        }
        return Some(ProposedEvent {
            day,
            start_minute,
            duration_minutes,
            span: toks[start].start..toks[last].end,
        });
    }
    None
}

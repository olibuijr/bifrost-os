//! Recurring events (`EKRecurrenceRule`, `EKRecurrenceEnd`, `EKRecurrenceDayOfWeek`, `EKEvent.occurrenceDate`,
//! `EKEvent.isDetached`, `EKSpan`, `EKEventStore.events(matching:)`). A pure model: no clock, no I/O.
//!
//! ```ignore
//! let mut store = EventStore::new();
//! let mut e = Event::new("1", "Standup", DateTime::new(2026, 10, 5, 9, 0), 30);
//! e.recurrence_rule = RecurrenceRule::parse("FREQ=WEEKLY;BYDAY=MO,WE");
//! store.save(e);
//! let p = store.predicate_for_events(DateTime::new(2026, 10, 1, 0, 0), DateTime::new(2026, 10, 31, 0, 0), None);
//! let list = store.events_matching(&p); // one `Occurrence` per repeat in the window
//! ```
//! Times are floating: a wall-clock minute of the day on a calendar date, so an event at 09:00 stays at 09:00 on
//! both sides of a daylight-saving change. Weeks start on Monday. Days are numbered like `EKWeekday`: Sunday is 1.
//! Month days and set positions count from the end when negative.

const MAX_PERIODS: i64 = 200_000;
const MINUTES_PER_DAY: i64 = 1440;

/// A calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        Date { year, month, day }
    }

    /// Days since 1970-01-01.
    fn days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let shifted = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * shifted + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days(z: i64) -> Self {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Date { year, month, day }
    }

    fn month_len(year: i32, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
            _ => 28,
        }
    }

    /// Sunday is 1, as `EKWeekday`.
    pub fn weekday(self) -> u8 {
        ((self.days() + 4).rem_euclid(7) + 1) as u8
    }

    fn plus_days(self, n: i64) -> Date {
        Date::from_days(self.days() + n)
    }
}

/// A floating wall-clock time: a date and a minute of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub date: Date,
    pub minute: u16,
}

impl DateTime {
    pub fn new(year: i32, month: u8, day: u8, hour: u16, minute: u16) -> Self {
        DateTime { date: Date::new(year, month, day), minute: hour * 60 + minute }
    }

    /// Minutes since 1970-01-01 00:00, floating.
    fn total_minutes(self) -> i64 {
        self.date.days() * MINUTES_PER_DAY + i64::from(self.minute)
    }

    fn from_total_minutes(m: i64) -> Self {
        DateTime { date: Date::from_days(m.div_euclid(MINUTES_PER_DAY)), minute: m.rem_euclid(MINUTES_PER_DAY) as u16 }
    }
}

/// `EKRecurrenceFrequency`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// `EKRecurrenceDayOfWeek`: a weekday (Sunday 1) and an optional week number (0 means every one, 2 the second,
/// -1 the last) used by monthly and yearly rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecurrenceDayOfWeek {
    pub day_of_the_week: u8,
    pub week_number: i8,
}

impl RecurrenceDayOfWeek {
    pub fn new(day_of_the_week: u8) -> Self {
        RecurrenceDayOfWeek { day_of_the_week, week_number: 0 }
    }

    pub fn with_week_number(day_of_the_week: u8, week_number: i8) -> Self {
        RecurrenceDayOfWeek { day_of_the_week, week_number }
    }
}

/// `EKRecurrenceEnd`: `EndDate` is the last day an occurrence may start on, `OccurrenceCount` the total including
/// the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecurrenceEnd {
    EndDate(Date),
    OccurrenceCount(u32),
}

/// `EKRecurrenceRule`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceRule {
    pub frequency: Frequency,
    pub interval: u32,
    pub days_of_the_week: Vec<RecurrenceDayOfWeek>,
    pub days_of_the_month: Vec<i8>,
    pub months_of_the_year: Vec<u8>,
    pub set_positions: Vec<i16>,
    pub recurrence_end: Option<RecurrenceEnd>,
}

const DAY_CODES: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

impl RecurrenceRule {
    pub fn new(frequency: Frequency, interval: u32) -> Self {
        RecurrenceRule {
            frequency,
            interval: interval.max(1),
            days_of_the_week: vec![],
            days_of_the_month: vec![],
            months_of_the_year: vec![],
            set_positions: vec![],
            recurrence_end: None,
        }
    }

    /// The rule for the shell's stored repeat choice (0 never, 1 daily, 2 weekly, 3 every 2 weeks, 4 monthly,
    /// 5 yearly).
    pub fn from_repeat_choice(choice: u8) -> Option<Self> {
        match choice {
            1 => Some(Self::new(Frequency::Daily, 1)),
            2 => Some(Self::new(Frequency::Weekly, 1)),
            3 => Some(Self::new(Frequency::Weekly, 2)),
            4 => Some(Self::new(Frequency::Monthly, 1)),
            5 => Some(Self::new(Frequency::Yearly, 1)),
            _ => None,
        }
    }

    /// Parses an RFC 5545 RRULE value (the `RRULE:` prefix is optional): FREQ, INTERVAL, COUNT, UNTIL (`YYYYMMDD`,
    /// a time part is ignored), BYDAY, BYMONTHDAY, BYMONTH, BYSETPOS. Unknown parts are ignored.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix("RRULE:").unwrap_or(text);
        let mut rule = RecurrenceRule::new(Frequency::Daily, 1);
        let mut have_frequency = false;
        for part in text.split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=')?;
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    rule.frequency = match value.to_ascii_uppercase().as_str() {
                        "DAILY" => Frequency::Daily,
                        "WEEKLY" => Frequency::Weekly,
                        "MONTHLY" => Frequency::Monthly,
                        "YEARLY" => Frequency::Yearly,
                        _ => return None,
                    };
                    have_frequency = true;
                }
                "INTERVAL" => {
                    let n: u32 = value.parse().ok()?;
                    if !(1..=1000).contains(&n) {
                        return None;
                    }
                    rule.interval = n;
                }
                "COUNT" => {
                    let n: u32 = value.parse().ok()?;
                    if !(1..=100_000).contains(&n) {
                        return None;
                    }
                    rule.recurrence_end = Some(RecurrenceEnd::OccurrenceCount(n));
                }
                "UNTIL" => rule.recurrence_end = Some(RecurrenceEnd::EndDate(parse_ymd(value)?)),
                "BYDAY" => {
                    for item in value.split(',') {
                        rule.days_of_the_week.push(parse_day(item)?);
                    }
                }
                "BYMONTHDAY" => {
                    for item in value.split(',') {
                        let n: i8 = item.parse().ok()?;
                        if n == 0 || !(-31..=31).contains(&n) {
                            return None;
                        }
                        rule.days_of_the_month.push(n);
                    }
                }
                "BYMONTH" => {
                    for item in value.split(',') {
                        let n: u8 = item.parse().ok()?;
                        if !(1..=12).contains(&n) {
                            return None;
                        }
                        rule.months_of_the_year.push(n);
                    }
                }
                "BYSETPOS" => {
                    for item in value.split(',') {
                        let n: i16 = item.parse().ok()?;
                        if n == 0 || !(-366..=366).contains(&n) {
                            return None;
                        }
                        rule.set_positions.push(n);
                    }
                }
                _ => {}
            }
        }
        have_frequency.then_some(rule)
    }

    /// The RFC 5545 RRULE value; `parse(&r.format())` gives `r` back.
    pub fn format(&self) -> String {
        let freq = match self.frequency {
            Frequency::Daily => "DAILY",
            Frequency::Weekly => "WEEKLY",
            Frequency::Monthly => "MONTHLY",
            Frequency::Yearly => "YEARLY",
        };
        let mut s = format!("FREQ={freq}");
        if self.interval != 1 {
            s += &format!(";INTERVAL={}", self.interval);
        }
        if !self.days_of_the_week.is_empty() {
            let items: Vec<String> = self
                .days_of_the_week
                .iter()
                .map(|d| {
                    let num = if d.week_number == 0 { String::new() } else { d.week_number.to_string() };
                    format!("{num}{}", DAY_CODES[usize::from(d.day_of_the_week + 6) % 7])
                })
                .collect();
            s += &format!(";BYDAY={}", items.join(","));
        }
        if !self.days_of_the_month.is_empty() {
            let items: Vec<String> = self.days_of_the_month.iter().map(|n| n.to_string()).collect();
            s += &format!(";BYMONTHDAY={}", items.join(","));
        }
        if !self.months_of_the_year.is_empty() {
            let items: Vec<String> = self.months_of_the_year.iter().map(|n| n.to_string()).collect();
            s += &format!(";BYMONTH={}", items.join(","));
        }
        if !self.set_positions.is_empty() {
            let items: Vec<String> = self.set_positions.iter().map(|n| n.to_string()).collect();
            s += &format!(";BYSETPOS={}", items.join(","));
        }
        match self.recurrence_end {
            Some(RecurrenceEnd::EndDate(d)) => s += &format!(";UNTIL={:04}{:02}{:02}", d.year, d.month, d.day),
            Some(RecurrenceEnd::OccurrenceCount(n)) => s += &format!(";COUNT={n}"),
            None => {}
        }
        s
    }
}

fn parse_ymd(s: &str) -> Option<Date> {
    if s.len() < 8 || !s.is_char_boundary(8) || !s[..8].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let d = Date::new(s[0..4].parse().ok()?, s[4..6].parse().ok()?, s[6..8].parse().ok()?);
    ((1..=12).contains(&d.month) && d.day >= 1 && d.day <= Date::month_len(d.year, d.month)).then_some(d)
}

fn parse_day(item: &str) -> Option<RecurrenceDayOfWeek> {
    let item = item.trim().to_ascii_uppercase();
    if item.len() < 2 || !item.is_ascii() {
        return None;
    }
    let (num, code) = item.split_at(item.len() - 2);
    let index = DAY_CODES.iter().position(|c| *c == code)?;
    let week_number: i8 = if num.is_empty() {
        0
    } else {
        let n: i8 = num.parse().ok()?;
        if n == 0 || !(-53..=53).contains(&n) {
            return None;
        }
        n
    };
    Some(RecurrenceDayOfWeek::with_week_number(index as u8 + 1, week_number))
}

/// The day of the month a (possibly negative) BYMONTHDAY value names, or `None` when the month is too short.
fn month_day(n: i8, len: u8) -> Option<u8> {
    let d = if n > 0 { i16::from(n) } else { i16::from(len) + i16::from(n) + 1 };
    (1..=i16::from(len)).contains(&d).then_some(d as u8)
}

/// Every candidate date of one month under the rule's day filters, ascending. `default_day` is used when the rule
/// names no day (a monthly rule repeats on the start's day; a month without that day is skipped).
fn month_candidates(rule: &RecurrenceRule, year: i32, month: u8, default_day: u8) -> Vec<Date> {
    let len = Date::month_len(year, month);
    let weekday_of = |d: u8| Date::new(year, month, d).weekday();
    let mut by_week: Vec<u8> = Vec::new();
    for w in &rule.days_of_the_week {
        let all: Vec<u8> = (1..=len).filter(|&d| weekday_of(d) == w.day_of_the_week).collect();
        let n = all.len() as i16;
        let k = i16::from(w.week_number);
        if k == 0 {
            by_week.extend(all);
        } else if k > 0 {
            if let Some(&d) = all.get((k - 1) as usize) {
                by_week.push(d);
            }
        } else if -k <= n {
            by_week.push(all[(n + k) as usize]);
        }
    }
    let by_month_day: Vec<u8> = rule.days_of_the_month.iter().filter_map(|&n| month_day(n, len)).collect();
    let mut out: Vec<u8> = match (rule.days_of_the_week.is_empty(), rule.days_of_the_month.is_empty()) {
        (true, true) => {
            if default_day <= len {
                vec![default_day]
            } else {
                vec![]
            }
        }
        (false, true) => by_week,
        (true, false) => by_month_day,
        (false, false) => by_week.into_iter().filter(|d| by_month_day.contains(d)).collect(),
    };
    out.sort_unstable();
    out.dedup();
    out.into_iter().map(|d| Date::new(year, month, d)).collect()
}

/// The dates of period `k` (counted from the period holding `start`), ascending, after BYSETPOS.
fn period_candidates(rule: &RecurrenceRule, start: Date, k: i64) -> Vec<Date> {
    let step = k * i64::from(rule.interval);
    let since_monday = (i64::from(start.weekday()) + 5) % 7;
    let mut dates: Vec<Date> = match rule.frequency {
        Frequency::Daily => {
            let d = start.plus_days(step);
            let week_ok = rule.days_of_the_week.is_empty() || rule.days_of_the_week.iter().any(|w| w.day_of_the_week == d.weekday());
            let month_day_ok = rule.days_of_the_month.is_empty()
                || rule.days_of_the_month.iter().any(|&n| month_day(n, Date::month_len(d.year, d.month)) == Some(d.day));
            if week_ok && month_day_ok {
                vec![d]
            } else {
                vec![]
            }
        }
        Frequency::Weekly => {
            let first = start.plus_days(-since_monday + step * 7);
            if rule.days_of_the_week.is_empty() {
                vec![first.plus_days(since_monday)]
            } else {
                rule.days_of_the_week.iter().map(|w| first.plus_days((i64::from(w.day_of_the_week) + 5) % 7)).collect()
            }
        }
        Frequency::Monthly => {
            let m0 = i64::from(start.year) * 12 + i64::from(start.month) - 1 + step;
            month_candidates(rule, m0.div_euclid(12) as i32, (m0.rem_euclid(12) + 1) as u8, start.day)
        }
        Frequency::Yearly => {
            let year = start.year + step as i32;
            let months: Vec<u8> = if rule.months_of_the_year.is_empty() { vec![start.month] } else { rule.months_of_the_year.clone() };
            months.into_iter().flat_map(|m| month_candidates(rule, year, m, start.day)).collect()
        }
    };
    if !rule.months_of_the_year.is_empty() {
        dates.retain(|d| rule.months_of_the_year.contains(&d.month));
    }
    dates.sort();
    dates.dedup();
    if !rule.set_positions.is_empty() {
        let n = dates.len() as i32;
        let mut picked: Vec<Date> = rule
            .set_positions
            .iter()
            .filter_map(|&p| {
                let i = if p > 0 { i32::from(p) - 1 } else { n + i32::from(p) };
                if (0..n).contains(&i) {
                    Some(dates[i as usize])
                } else {
                    None
                }
            })
            .collect();
        picked.sort();
        picked.dedup();
        dates = picked;
    }
    dates
}

/// The start dates of every repeat of `rule` that begin on or after `start` and no later than `last` (inclusive),
/// honouring the end. The first date is always `start`.
pub fn occurrence_dates(rule: &RecurrenceRule, start: Date, last: Date) -> Vec<Date> {
    let mut out = Vec::new();
    let limit = match rule.recurrence_end {
        Some(RecurrenceEnd::EndDate(d)) => d.min(last),
        _ => last,
    };
    let max_count = match rule.recurrence_end {
        Some(RecurrenceEnd::OccurrenceCount(n)) => n as usize,
        _ => usize::MAX,
    };
    let mut seen = 0usize;
    for k in 0..MAX_PERIODS {
        let mut dates = period_candidates(rule, start, k);
        if k == 0 {
            dates.retain(|d| *d >= start);
            if dates.first() != Some(&start) {
                dates.insert(0, start);
            }
        }
        for d in dates {
            if seen >= max_count || d > limit {
                return out;
            }
            seen += 1;
            out.push(d);
        }
        if seen >= max_count {
            return out;
        }
        if matches!(period_candidates(rule, start, k + 1).first(), Some(d) if *d > limit) {
            return out;
        }
    }
    out
}

/// One replaced repeat of a recurring event: the occurrence that was at `occurrence_date` now has its own time,
/// length and title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detached {
    pub occurrence_date: DateTime,
    pub start_date: DateTime,
    pub duration_minutes: u32,
    pub title: String,
}

/// `EKEvent` with its recurrence data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub identifier: String,
    pub title: String,
    pub calendar: String,
    pub start_date: DateTime,
    pub duration_minutes: u32,
    pub recurrence_rule: Option<RecurrenceRule>,
    /// Repeats skipped (EXDATE), by their original start.
    pub excluded_dates: Vec<DateTime>,
    pub detached: Vec<Detached>,
}

impl Event {
    pub fn new(identifier: &str, title: &str, start_date: DateTime, duration_minutes: u32) -> Self {
        Event {
            identifier: identifier.into(),
            title: title.into(),
            calendar: String::new(),
            start_date,
            duration_minutes,
            recurrence_rule: None,
            excluded_dates: vec![],
            detached: vec![],
        }
    }

    pub fn has_recurrence_rules(&self) -> bool {
        self.recurrence_rule.is_some()
    }
}

/// One event as it appears in a date range: `EKEvent` with `occurrenceDate` and `isDetached`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub event_identifier: String,
    pub title: String,
    pub calendar: String,
    /// The original start of this repeat, even when a detached copy moved it.
    pub occurrence_date: DateTime,
    pub start_date: DateTime,
    pub end_date: DateTime,
    pub is_detached: bool,
}

/// `EKSpan`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    ThisEvent,
    FutureEvents,
}

/// `predicateForEvents(withStart:end:calendars:)`. `calendars` of `None` matches every calendar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventPredicate {
    pub start: DateTime,
    pub end: DateTime,
    pub calendars: Option<Vec<String>>,
}

/// The expansion of one event: every occurrence that overlaps `[start, end)`, ascending by start.
pub fn occurrences(event: &Event, start: DateTime, end: DateTime) -> Vec<Occurrence> {
    let length = i64::from(event.duration_minutes);
    let make = |occurrence_date: DateTime| -> Occurrence {
        let (start_date, duration, title, is_detached) = match event.detached.iter().find(|d| d.occurrence_date == occurrence_date) {
            Some(d) => (d.start_date, i64::from(d.duration_minutes), d.title.clone(), true),
            None => (occurrence_date, length, event.title.clone(), false),
        };
        Occurrence {
            event_identifier: event.identifier.clone(),
            title,
            calendar: event.calendar.clone(),
            occurrence_date,
            start_date,
            end_date: DateTime::from_total_minutes(start_date.total_minutes() + duration),
            is_detached,
        }
    };
    let bases: Vec<DateTime> = match &event.recurrence_rule {
        None => vec![event.start_date],
        Some(rule) => {
            // A detached repeat may move later than its original day, so look a month past the window.
            let last = end.date.plus_days(31);
            occurrence_dates(rule, event.start_date.date, last)
                .into_iter()
                .map(|date| DateTime { date, minute: event.start_date.minute })
                .collect()
        }
    };
    let mut out: Vec<Occurrence> = bases
        .into_iter()
        .filter(|b| !event.excluded_dates.contains(b))
        .map(make)
        .filter(|o| o.start_date < end && (o.end_date > start || (o.end_date == o.start_date && o.start_date >= start)))
        .collect();
    out.sort_by_key(|o| (o.start_date, o.occurrence_date));
    out
}

/// `EKEventStore`: the events and their range queries.
#[derive(Clone, Debug, Default)]
pub struct EventStore {
    events: Vec<Event>,
}

impl EventStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the event, or replaces the one with the same identifier.
    pub fn save(&mut self, event: Event) {
        match self.events.iter_mut().find(|e| e.identifier == event.identifier) {
            Some(slot) => *slot = event,
            None => self.events.push(event),
        }
    }

    pub fn event(&self, identifier: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.identifier == identifier)
    }

    pub fn predicate_for_events(&self, start: DateTime, end: DateTime, calendars: Option<&[String]>) -> EventPredicate {
        EventPredicate { start, end, calendars: calendars.map(<[String]>::to_vec) }
    }

    /// `events(matching:)`: every occurrence in the window, ascending by start.
    pub fn events_matching(&self, predicate: &EventPredicate) -> Vec<Occurrence> {
        let mut out: Vec<Occurrence> = self
            .events
            .iter()
            .filter(|e| predicate.calendars.as_ref().map_or(true, |c| c.contains(&e.calendar)))
            .flat_map(|e| occurrences(e, predicate.start, predicate.end))
            .collect();
        out.sort_by(|a, b| (a.start_date, &a.event_identifier).cmp(&(b.start_date, &b.event_identifier)));
        out
    }

    /// `remove(_:span:)` for the repeat that started at `occurrence_date`. `ThisEvent` skips that repeat;
    /// `FutureEvents` ends the series the day before it (the whole event goes when it is the first repeat).
    pub fn remove(&mut self, identifier: &str, occurrence_date: DateTime, span: Span) {
        let Some(i) = self.events.iter().position(|e| e.identifier == identifier) else { return };
        let first = self.events[i].start_date;
        let repeats = self.events[i].recurrence_rule.is_some();
        match span {
            Span::ThisEvent if !repeats => {
                self.events.remove(i);
            }
            Span::ThisEvent => {
                let e = &mut self.events[i];
                e.detached.retain(|d| d.occurrence_date != occurrence_date);
                if !e.excluded_dates.contains(&occurrence_date) {
                    e.excluded_dates.push(occurrence_date);
                }
            }
            Span::FutureEvents if !repeats || occurrence_date.date <= first.date => {
                self.events.remove(i);
            }
            Span::FutureEvents => {
                if let Some(rule) = &mut self.events[i].recurrence_rule {
                    rule.recurrence_end = Some(RecurrenceEnd::EndDate(occurrence_date.date.plus_days(-1)));
                }
            }
        }
    }

    /// Replaces one repeat (`save(_:span: .thisEvent)` on a repeat that was edited): it becomes detached.
    pub fn detach(&mut self, identifier: &str, detached: Detached) {
        if let Some(e) = self.events.iter_mut().find(|e| e.identifier == identifier) {
            e.detached.retain(|d| d.occurrence_date != detached.occurrence_date);
            e.detached.push(detached);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day)
    }

    fn at(y: i32, m: u8, day: u8, h: u16) -> DateTime {
        DateTime::new(y, m, day, h, 0)
    }

    fn rule(text: &str) -> RecurrenceRule {
        RecurrenceRule::parse(text).expect("rule parses")
    }

    fn days(r: &str, start: Date, last: Date) -> Vec<Date> {
        occurrence_dates(&rule(r), start, last)
    }

    #[test]
    fn weekday_numbers_follow_ekweekday() {
        assert_eq!(d(2026, 10, 4).weekday(), 1); // Sunday
        assert_eq!(d(2026, 10, 5).weekday(), 2); // Monday
        assert_eq!(d(2026, 10, 10).weekday(), 7); // Saturday
    }

    #[test]
    fn date_arithmetic_round_trips() {
        assert_eq!(d(2026, 2, 28).plus_days(1), d(2026, 3, 1));
        assert_eq!(d(2024, 2, 28).plus_days(1), d(2024, 2, 29));
        assert_eq!(d(2026, 12, 31).plus_days(1), d(2027, 1, 1));
    }

    #[test]
    fn daily_with_an_interval_steps_by_it() {
        let got = days("FREQ=DAILY;INTERVAL=3", d(2026, 10, 1), d(2026, 10, 12));
        assert_eq!(got, vec![d(2026, 10, 1), d(2026, 10, 4), d(2026, 10, 7), d(2026, 10, 10)]);
    }

    #[test]
    fn weekly_by_day_lists_each_named_weekday() {
        let got = days("FREQ=WEEKLY;BYDAY=MO,WE", d(2026, 10, 5), d(2026, 10, 14));
        assert_eq!(got, vec![d(2026, 10, 5), d(2026, 10, 7), d(2026, 10, 12), d(2026, 10, 14)]);
    }

    #[test]
    fn weekly_without_days_repeats_on_the_start_weekday() {
        let got = days("FREQ=WEEKLY;INTERVAL=2", d(2026, 10, 7), d(2026, 11, 10));
        assert_eq!(got, vec![d(2026, 10, 7), d(2026, 10, 21), d(2026, 11, 4)]);
    }

    #[test]
    fn weekly_days_before_the_start_in_its_week_are_not_occurrences() {
        let got = days("FREQ=WEEKLY;BYDAY=MO,FR", d(2026, 10, 7), d(2026, 10, 16));
        assert_eq!(got, vec![d(2026, 10, 7), d(2026, 10, 9), d(2026, 10, 12), d(2026, 10, 16)]);
    }

    #[test]
    fn monthly_by_month_day_counts_negative_days_from_the_end() {
        let got = days("FREQ=MONTHLY;BYMONTHDAY=1,-1", d(2026, 1, 1), d(2026, 3, 31));
        assert_eq!(got, vec![d(2026, 1, 1), d(2026, 1, 31), d(2026, 2, 1), d(2026, 2, 28), d(2026, 3, 1), d(2026, 3, 31)]);
    }

    #[test]
    fn monthly_on_the_31st_skips_shorter_months() {
        let got = days("FREQ=MONTHLY", d(2026, 1, 31), d(2026, 5, 31));
        assert_eq!(got, vec![d(2026, 1, 31), d(2026, 3, 31), d(2026, 5, 31)]);
    }

    #[test]
    fn monthly_by_set_position_picks_the_last_weekday_of_the_month() {
        let got = days("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", d(2026, 8, 31), d(2026, 11, 30));
        assert_eq!(got, vec![d(2026, 8, 31), d(2026, 9, 30), d(2026, 10, 30), d(2026, 11, 30)]);
    }

    #[test]
    fn monthly_second_tuesday_uses_the_week_number() {
        let got = days("FREQ=MONTHLY;BYDAY=2TU", d(2026, 10, 13), d(2026, 12, 31));
        assert_eq!(got, vec![d(2026, 10, 13), d(2026, 11, 10), d(2026, 12, 8)]);
    }

    #[test]
    fn yearly_by_month_and_day_repeats_each_year() {
        let got = days("FREQ=YEARLY;BYMONTH=2,6;BYMONTHDAY=29", d(2024, 2, 29), d(2028, 12, 31));
        assert_eq!(
            got,
            vec![d(2024, 2, 29), d(2024, 6, 29), d(2025, 6, 29), d(2026, 6, 29), d(2027, 6, 29), d(2028, 2, 29), d(2028, 6, 29)]
        );
    }

    #[test]
    fn yearly_without_parts_repeats_on_the_start_date() {
        let got = days("FREQ=YEARLY", d(2024, 3, 5), d(2027, 3, 5));
        assert_eq!(got, vec![d(2024, 3, 5), d(2025, 3, 5), d(2026, 3, 5), d(2027, 3, 5)]);
    }

    #[test]
    fn an_occurrence_count_ends_the_series_after_that_many() {
        let got = days("FREQ=DAILY;COUNT=3", d(2026, 10, 1), d(2027, 1, 1));
        assert_eq!(got, vec![d(2026, 10, 1), d(2026, 10, 2), d(2026, 10, 3)]);
    }

    #[test]
    fn an_end_date_is_the_last_day_an_occurrence_may_start_on() {
        let got = days("FREQ=DAILY;UNTIL=20261003", d(2026, 10, 1), d(2027, 1, 1));
        assert_eq!(got, vec![d(2026, 10, 1), d(2026, 10, 2), d(2026, 10, 3)]);
    }

    #[test]
    fn the_first_occurrence_is_the_start_even_when_the_rule_does_not_match_it() {
        let got = days("FREQ=WEEKLY;BYDAY=FR", d(2026, 10, 7), d(2026, 10, 16));
        assert_eq!(got, vec![d(2026, 10, 7), d(2026, 10, 9), d(2026, 10, 16)]);
    }

    #[test]
    fn rules_format_and_parse_back() {
        for text in [
            "FREQ=DAILY",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10",
            "FREQ=MONTHLY;BYDAY=-1FR;BYSETPOS=1;UNTIL=20271231",
            "FREQ=YEARLY;BYMONTHDAY=-1;BYMONTH=2,6",
        ] {
            assert_eq!(rule(text).format(), text);
        }
    }

    #[test]
    fn rule_fields_mirror_the_reference_names() {
        let r = rule("RRULE:FREQ=MONTHLY;BYDAY=2TU;BYSETPOS=1;COUNT=4");
        assert_eq!(r.frequency, Frequency::Monthly);
        assert_eq!(r.days_of_the_week, vec![RecurrenceDayOfWeek::with_week_number(3, 2)]);
        assert_eq!(r.set_positions, vec![1]);
        assert_eq!(r.recurrence_end, Some(RecurrenceEnd::OccurrenceCount(4)));
    }

    #[test]
    fn bad_rules_are_refused() {
        assert!(RecurrenceRule::parse("INTERVAL=2").is_none());
        assert!(RecurrenceRule::parse("FREQ=HOURLY").is_none());
        assert!(RecurrenceRule::parse("FREQ=DAILY;BYDAY=XX").is_none());
        assert!(RecurrenceRule::parse("FREQ=DAILY;UNTIL=20261301").is_none());
    }

    #[test]
    fn the_stored_repeat_choices_map_to_rules() {
        assert_eq!(RecurrenceRule::from_repeat_choice(0), None);
        assert_eq!(RecurrenceRule::from_repeat_choice(3).unwrap().format(), "FREQ=WEEKLY;INTERVAL=2");
        assert_eq!(RecurrenceRule::from_repeat_choice(5).unwrap().format(), "FREQ=YEARLY");
    }

    fn weekly_standup() -> Event {
        let mut e = Event::new("a", "Standup", at(2026, 10, 5, 9), 30);
        e.recurrence_rule = Some(rule("FREQ=WEEKLY;BYDAY=MO,WE"));
        e
    }

    #[test]
    fn a_plain_event_appears_once_inside_the_window() {
        let e = Event::new("a", "Lunch", at(2026, 10, 5, 12), 60);
        assert_eq!(occurrences(&e, at(2026, 10, 5, 0), at(2026, 10, 6, 0)).len(), 1);
        assert!(occurrences(&e, at(2026, 10, 6, 0), at(2026, 10, 7, 0)).is_empty());
    }

    #[test]
    fn a_recurring_event_expands_inside_the_window_only() {
        let got = occurrences(&weekly_standup(), at(2026, 10, 12, 0), at(2026, 10, 19, 0));
        let starts: Vec<DateTime> = got.iter().map(|o| o.start_date).collect();
        assert_eq!(starts, vec![at(2026, 10, 12, 9), at(2026, 10, 14, 9)]);
        assert!(got.iter().all(|o| !o.is_detached && o.occurrence_date == o.start_date));
        assert_eq!(got[0].end_date, DateTime::new(2026, 10, 12, 9, 30));
    }

    #[test]
    fn an_occurrence_that_began_before_the_window_but_overlaps_it_is_included() {
        let mut e = Event::new("n", "Night shift", at(2026, 10, 5, 22), 600);
        e.recurrence_rule = Some(rule("FREQ=DAILY"));
        let got = occurrences(&e, at(2026, 10, 6, 0), at(2026, 10, 6, 6));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].occurrence_date, at(2026, 10, 5, 22));
    }

    #[test]
    fn excluded_dates_are_skipped() {
        let mut e = weekly_standup();
        e.excluded_dates.push(at(2026, 10, 12, 9));
        let got = occurrences(&e, at(2026, 10, 12, 0), at(2026, 10, 19, 0));
        assert_eq!(got.iter().map(|o| o.start_date).collect::<Vec<_>>(), vec![at(2026, 10, 14, 9)]);
    }

    #[test]
    fn excluded_repeats_still_count_toward_the_occurrence_count() {
        let mut e = Event::new("c", "Three", at(2026, 10, 1, 9), 30);
        e.recurrence_rule = Some(rule("FREQ=DAILY;COUNT=3"));
        e.excluded_dates.push(at(2026, 10, 2, 9));
        let got = occurrences(&e, at(2026, 10, 1, 0), at(2026, 11, 1, 0));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn a_detached_repeat_replaces_one_occurrence_and_keeps_its_occurrence_date() {
        let mut e = weekly_standup();
        e.detached.push(Detached {
            occurrence_date: at(2026, 10, 14, 9),
            start_date: at(2026, 10, 15, 14),
            duration_minutes: 60,
            title: "Standup (moved)".into(),
        });
        let got = occurrences(&e, at(2026, 10, 12, 0), at(2026, 10, 19, 0));
        assert_eq!(got.len(), 2);
        assert!(!got[0].is_detached);
        let moved = &got[1];
        assert!(moved.is_detached);
        assert_eq!(moved.occurrence_date, at(2026, 10, 14, 9));
        assert_eq!(moved.start_date, at(2026, 10, 15, 14));
        assert_eq!(moved.end_date, at(2026, 10, 15, 15));
        assert_eq!(moved.title, "Standup (moved)");
    }

    #[test]
    fn a_floating_time_keeps_its_wall_clock_hour_across_a_daylight_saving_change() {
        let mut e = Event::new("d", "Daily", at(2026, 3, 27, 9), 30);
        e.recurrence_rule = Some(rule("FREQ=DAILY"));
        let got = occurrences(&e, at(2026, 3, 27, 0), at(2026, 4, 3, 0));
        assert_eq!(got.len(), 7);
        assert!(got.iter().all(|o| o.start_date.minute == 9 * 60));
    }

    #[test]
    fn the_store_merges_events_sorted_and_filters_by_calendar() {
        let mut store = EventStore::new();
        let mut work = weekly_standup();
        work.calendar = "Work".into();
        let mut home = Event::new("h", "Dinner", at(2026, 10, 12, 18), 90);
        home.calendar = "Home".into();
        store.save(work);
        store.save(home);
        let all = store.predicate_for_events(at(2026, 10, 12, 0), at(2026, 10, 19, 0), None);
        let ids: Vec<String> = store.events_matching(&all).into_iter().map(|o| o.event_identifier).collect();
        assert_eq!(ids, vec!["a", "h", "a"]);
        let home_only = ["Home".to_string()];
        let only = store.predicate_for_events(at(2026, 10, 12, 0), at(2026, 10, 19, 0), Some(&home_only));
        assert_eq!(store.events_matching(&only).len(), 1);
    }

    #[test]
    fn removing_this_event_skips_one_repeat_and_future_events_ends_the_series() {
        let mut store = EventStore::new();
        store.save(weekly_standup());
        store.remove("a", at(2026, 10, 12, 9), Span::ThisEvent);
        let p = store.predicate_for_events(at(2026, 10, 12, 0), at(2026, 10, 26, 0), None);
        assert_eq!(store.events_matching(&p).len(), 3);
        store.remove("a", at(2026, 10, 19, 9), Span::FutureEvents);
        assert_eq!(store.events_matching(&p).len(), 1);
        store.remove("a", at(2026, 10, 5, 9), Span::FutureEvents);
        assert!(store.event("a").is_none());
    }

    #[test]
    fn detaching_a_repeat_through_the_store_marks_it() {
        let mut store = EventStore::new();
        store.save(weekly_standup());
        store.detach(
            "a",
            Detached { occurrence_date: at(2026, 10, 12, 9), start_date: at(2026, 10, 12, 10), duration_minutes: 30, title: "Standup".into() },
        );
        let p = store.predicate_for_events(at(2026, 10, 12, 0), at(2026, 10, 13, 0), None);
        let got = store.events_matching(&p);
        assert_eq!(got.len(), 1);
        assert!(got[0].is_detached);
    }
}

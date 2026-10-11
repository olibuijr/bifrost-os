//! RFC 5545 RRULE subset: FREQ, INTERVAL, COUNT, UNTIL, BYDAY (with ordinals),
//! BYMONTHDAY (negative allowed), BYMONTH. Weeks start on Monday (WebST=MO).
//! Pure date arithmetic, no I/O.
use super::Date;

const MAX_PERIODS: i64 = 40_000;
const MAX_OUT: usize = 5_000;
const DAY_CODES: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    /// (ordinal, weekday) with weekday 0 = Sunday; ordinal 0 = every one,
    /// 2 = second, -1 = last.
    pub by_day: Vec<(i8, u8)>,
    pub by_month_day: Vec<i8>,
    pub by_month: Vec<u8>,
    /// Inclusive last day.
    pub until: Option<Date>,
    /// Total occurrences including the first.
    pub count: Option<u32>,
}

impl Rule {
    pub fn new(freq: Freq) -> Self {
        Rule {
            freq,
            interval: 1,
            by_day: vec![],
            by_month_day: vec![],
            by_month: vec![],
            until: None,
            count: None,
        }
    }
}

/// `YYYYMMDD` (extra characters such as a time part are ignored).
pub fn parse_ymd(s: &str) -> Option<Date> {
    if s.len() < 8 || !s.is_char_boundary(8) || !s[..8].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let d = Date {
        year: s[0..4].parse().ok()?,
        month: s[4..6].parse().ok()?,
        day: s[6..8].parse().ok()?,
    };
    d.valid().then_some(d)
}

pub fn format_ymd(d: Date) -> String {
    format!("{:04}{:02}{:02}", d.year, d.month, d.day)
}

pub fn parse_rrule(text: &str) -> Option<Rule> {
    let text = text.trim();
    let text = text.strip_prefix("RRULE:").unwrap_or(text);
    let mut rule = Rule::new(Freq::Daily);
    let mut have_freq = false;
    for part in text.split(';').filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=')?;
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => {
                rule.freq = match value.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                };
                have_freq = true;
            }
            "INTERVAL" => {
                rule.interval = value.parse().ok().filter(|&n| (1..=1000).contains(&n))?;
            }
            "COUNT" => {
                rule.count = Some(value.parse().ok().filter(|&n| (1..=10_000).contains(&n))?);
            }
            "UNTIL" => rule.until = Some(parse_ymd(value)?),
            "BYDAY" => {
                for item in value.split(',') {
                    rule.by_day.push(parse_byday(item)?);
                }
            }
            "BYMONTHDAY" => {
                for item in value.split(',') {
                    let n: i8 = item.parse().ok()?;
                    if n == 0 || !(-31..=31).contains(&n) {
                        return None;
                    }
                    rule.by_month_day.push(n);
                }
            }
            "BYMONTH" => {
                for item in value.split(',') {
                    let n: u8 = item.parse().ok()?;
                    if !(1..=12).contains(&n) {
                        return None;
                    }
                    rule.by_month.push(n);
                }
            }
            _ => {}
        }
    }
    have_freq.then_some(rule)
}

fn parse_byday(item: &str) -> Option<(i8, u8)> {
    let item = item.trim().to_ascii_uppercase();
    if item.len() < 2 || !item.is_ascii() {
        return None;
    }
    let (num, code) = item.split_at(item.len() - 2);
    let wd = DAY_CODES.iter().position(|c| *c == code)? as u8;
    let nth: i8 = if num.is_empty() {
        0
    } else {
        num.parse().ok().filter(|n: &i8| *n != 0 && (-53..=53).contains(n))?
    };
    Some((nth, wd))
}

pub fn to_rrule(rule: &Rule) -> String {
    let freq = match rule.freq {
        Freq::Daily => "DAILY",
        Freq::Weekly => "WEEKLY",
        Freq::Monthly => "MONTHLY",
        Freq::Yearly => "YEARLY",
    };
    let mut s = format!("FREQ={freq}");
    if rule.interval != 1 {
        s += &format!(";INTERVAL={}", rule.interval);
    }
    if !rule.by_day.is_empty() {
        let items: Vec<String> = rule
            .by_day
            .iter()
            .map(|&(n, wd)| {
                let num = if n == 0 { String::new() } else { n.to_string() };
                format!("{num}{}", DAY_CODES[wd as usize % 7])
            })
            .collect();
        s += &format!(";BYDAY={}", items.join(","));
    }
    if !rule.by_month_day.is_empty() {
        let items: Vec<String> = rule.by_month_day.iter().map(|n| n.to_string()).collect();
        s += &format!(";BYMONTHDAY={}", items.join(","));
    }
    if !rule.by_month.is_empty() {
        let items: Vec<String> = rule.by_month.iter().map(|n| n.to_string()).collect();
        s += &format!(";BYMONTH={}", items.join(","));
    }
    if let Some(u) = rule.until {
        s += &format!(";UNTIL={}", format_ymd(u));
    }
    if let Some(c) = rule.count {
        s += &format!(";COUNT={c}");
    }
    s
}

/// Short label for the editor's Repeat row.
pub fn describe(rule: Option<&Rule>) -> String {
    let Some(r) = rule else {
        return "Never".into();
    };
    let unit = match r.freq {
        Freq::Daily => "day",
        Freq::Weekly => "week",
        Freq::Monthly => "month",
        Freq::Yearly => "year",
    };
    match r.interval {
        1 => format!("Every {unit}"),
        n => format!("Every {n} {unit}s"),
    }
}

fn last_day() -> i64 {
    Date { year: 9999, month: 12, day: 31 }.days()
}

fn month_dates(year: i64, month: u32, rule: &Rule, start: Date) -> Vec<Date> {
    let first = Date { year, month, day: 1 };
    let len = first.month_len();
    let mut out = vec![];
    let weekday_of = |day: u32| Date { year, month, day }.weekday() as u8;
    if !rule.by_day.is_empty() && rule.by_month_day.is_empty() {
        for &(nth, wd) in &rule.by_day {
            let all: Vec<u32> = (1..=len).filter(|&d| weekday_of(d) == wd).collect();
            if nth == 0 {
                out.extend(all);
            } else if nth > 0 {
                out.extend(all.get(nth as usize - 1).copied());
            } else if let Some(i) = all.len().checked_sub(nth.unsigned_abs() as usize) {
                out.extend(all.get(i).copied());
            }
        }
    } else if !rule.by_month_day.is_empty() {
        for &n in &rule.by_month_day {
            let day = if n > 0 { n as i32 } else { len as i32 + 1 + n as i32 };
            if day >= 1 && day <= len as i32 {
                let day = day as u32;
                if rule.by_day.is_empty() || rule.by_day.iter().any(|&(_, wd)| wd == weekday_of(day)) {
                    out.push(day);
                }
            }
        }
    } else if start.day <= len {
        out.push(start.day);
    }
    out.sort_unstable();
    out.dedup();
    out.into_iter().map(|day| Date { year, month, day }).collect()
}

/// First day of period `n`, or None past the supported range.
fn period_start(rule: &Rule, start: Date, n: i64) -> Option<Date> {
    let int = rule.interval as i64;
    let in_range = |d: i64| (d <= last_day()).then(|| Date::from_days(d));
    match rule.freq {
        Freq::Daily => in_range(start.days() + n * int),
        Freq::Weekly => in_range(start.days() - ((start.weekday() as i64 + 6) % 7) + 7 * n * int),
        Freq::Monthly => {
            let idx = start.year * 12 + start.month as i64 - 1 + n * int;
            (idx / 12 <= 9999).then(|| Date { year: idx / 12, month: (idx % 12) as u32 + 1, day: 1 })
        }
        Freq::Yearly => {
            let year = start.year + n * int;
            (year <= 9999).then_some(Date { year, month: 1, day: 1 })
        }
    }
}

fn period_dates(rule: &Rule, start: Date, n: i64) -> Vec<Date> {
    let Some(p) = period_start(rule, start, n) else {
        return vec![];
    };
    match rule.freq {
        Freq::Daily => {
            let ok = (rule.by_day.is_empty()
                || rule.by_day.iter().any(|&(_, wd)| wd as usize == p.weekday()))
                && (rule.by_month.is_empty() || rule.by_month.contains(&(p.month as u8)))
                && (rule.by_month_day.is_empty()
                    || rule.by_month_day.iter().any(|&m| {
                        let len = p.month_len() as i32;
                        let day = if m > 0 { m as i32 } else { len + 1 + m as i32 };
                        day == p.day as i32
                    }));
            if ok { vec![p] } else { vec![] }
        }
        Freq::Weekly => {
            let wds: Vec<u8> = if rule.by_day.is_empty() {
                vec![start.weekday() as u8]
            } else {
                rule.by_day.iter().map(|&(_, wd)| wd).collect()
            };
            (0..7)
                .map(|off| p.shift_days(off))
                .filter(|d| wds.contains(&(d.weekday() as u8)))
                .filter(|d| rule.by_month.is_empty() || rule.by_month.contains(&(d.month as u8)))
                .collect()
        }
        Freq::Monthly => {
            if !rule.by_month.is_empty() && !rule.by_month.contains(&(p.month as u8)) {
                return vec![];
            }
            month_dates(p.year, p.month, rule, start)
        }
        Freq::Yearly => {
            let months: Vec<u32> = if rule.by_month.is_empty() {
                vec![start.month]
            } else {
                let mut m: Vec<u32> = rule.by_month.iter().map(|&m| m as u32).collect();
                m.sort_unstable();
                m.dedup();
                m
            };
            months.into_iter().flat_map(|m| month_dates(p.year, m, rule, start)).collect()
        }
    }
}

/// Occurrence dates of `rule` anchored at `start`, within `from..=to`,
/// without `exceptions`. COUNT is applied before exceptions and windowing, as
/// in RFC 5545. Output is capped at 5000 dates.
pub fn expand(rule: &Rule, start: Date, exceptions: &[Date], from: Date, to: Date) -> Vec<Date> {
    let mut out = vec![];
    if !start.valid() || rule.interval == 0 {
        return out;
    }
    let last = rule.until.map_or(to, |u| u.min(to));
    let int = rule.interval as i64;
    let mut n = 0;
    if rule.count.is_none() {
        let gap = from.days() - start.days();
        n = match rule.freq {
            Freq::Daily => (gap / int - 1).max(0),
            Freq::Weekly => (gap / (7 * int) - 1).max(0),
            _ => 0,
        };
    }
    let mut emitted = 0u32;
    let end_n = n + MAX_PERIODS;
    while n < end_n {
        let Some(p) = period_start(rule, start, n) else {
            break;
        };
        if p > last {
            break;
        }
        for d in period_dates(rule, start, n) {
            if d < start {
                continue;
            }
            if d > last || rule.count.is_some_and(|c| emitted >= c) {
                return out;
            }
            emitted += 1;
            if d >= from && !exceptions.contains(&d) {
                out.push(d);
                if out.len() >= MAX_OUT {
                    return out;
                }
            }
        }
        n += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }
    fn all(rule: &Rule, start: Date) -> Vec<Date> {
        expand(rule, start, &[], start, d(2035, 12, 31))
    }

    #[test]
    fn weekday_anchor() {
        assert_eq!(d(2026, 10, 5).weekday(), 1);
    }

    #[test]
    fn daily_interval_count() {
        let mut r = Rule::new(Freq::Daily);
        r.interval = 2;
        r.count = Some(3);
        assert_eq!(all(&r, d(2026, 10, 5)), vec![d(2026, 10, 5), d(2026, 10, 7), d(2026, 10, 9)]);
    }

    #[test]
    fn daily_until_is_inclusive() {
        let mut r = Rule::new(Freq::Daily);
        r.until = Some(d(2026, 10, 7));
        assert_eq!(all(&r, d(2026, 10, 5)), vec![d(2026, 10, 5), d(2026, 10, 6), d(2026, 10, 7)]);
    }

    #[test]
    fn window_skips_ahead_without_count() {
        let r = Rule::new(Freq::Daily);
        let got = expand(&r, d(2026, 1, 1), &[], d(2026, 10, 5), d(2026, 10, 7));
        assert_eq!(got, vec![d(2026, 10, 5), d(2026, 10, 6), d(2026, 10, 7)]);
        let mut w = Rule::new(Freq::Weekly);
        w.interval = 2;
        let got = expand(&w, d(2026, 1, 5), &[], d(2026, 10, 1), d(2026, 10, 31));
        assert!(got.iter().all(|x| x.weekday() == 1 && (x.days() - d(2026, 1, 5).days()) % 14 == 0));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn weekly_weekday_set() {
        let mut r = Rule::new(Freq::Weekly);
        r.by_day = vec![(0, 1), (0, 3), (0, 5)];
        r.count = Some(5);
        let want = vec![d(2026, 10, 5), d(2026, 10, 7), d(2026, 10, 9), d(2026, 10, 12), d(2026, 10, 14)];
        assert_eq!(all(&r, d(2026, 10, 5)), want);
    }

    #[test]
    fn weekly_interval_two() {
        let mut r = Rule::new(Freq::Weekly);
        r.interval = 2;
        r.by_day = vec![(0, 1), (0, 4)];
        r.count = Some(4);
        let want = vec![d(2026, 10, 5), d(2026, 10, 8), d(2026, 10, 19), d(2026, 10, 22)];
        assert_eq!(all(&r, d(2026, 10, 5)), want);
    }

    #[test]
    fn weekly_defaults_to_start_weekday() {
        let mut r = Rule::new(Freq::Weekly);
        r.count = Some(2);
        assert_eq!(all(&r, d(2026, 10, 7)), vec![d(2026, 10, 7), d(2026, 10, 14)]);
    }

    #[test]
    fn monthly_by_date_skips_short_months() {
        let mut r = Rule::new(Freq::Monthly);
        r.count = Some(4);
        let want = vec![d(2026, 1, 31), d(2026, 3, 31), d(2026, 5, 31), d(2026, 7, 31)];
        assert_eq!(all(&r, d(2026, 1, 31)), want);
    }

    #[test]
    fn monthly_last_day() {
        let mut r = Rule::new(Freq::Monthly);
        r.by_month_day = vec![-1];
        r.count = Some(3);
        let want = vec![d(2026, 1, 31), d(2026, 2, 28), d(2026, 3, 31)];
        assert_eq!(all(&r, d(2026, 1, 31)), want);
    }

    #[test]
    fn monthly_last_friday() {
        let mut r = Rule::new(Freq::Monthly);
        r.by_day = vec![(-1, 5)];
        r.count = Some(3);
        assert_eq!(all(&r, d(2026, 10, 30)), vec![d(2026, 10, 30), d(2026, 11, 27), d(2026, 12, 25)]);
    }

    #[test]
    fn monthly_second_tuesday() {
        let mut r = Rule::new(Freq::Monthly);
        r.by_day = vec![(2, 2)];
        r.count = Some(3);
        assert_eq!(all(&r, d(2026, 10, 13)), vec![d(2026, 10, 13), d(2026, 11, 10), d(2026, 12, 8)]);
    }

    #[test]
    fn yearly_default_and_leap_day() {
        let mut r = Rule::new(Freq::Yearly);
        r.until = Some(d(2029, 12, 31));
        let want = vec![d(2026, 10, 5), d(2027, 10, 5), d(2028, 10, 5), d(2029, 10, 5)];
        assert_eq!(all(&r, d(2026, 10, 5)), want);
        let mut leap = Rule::new(Freq::Yearly);
        leap.count = Some(2);
        assert_eq!(all(&leap, d(2024, 2, 29)), vec![d(2024, 2, 29), d(2028, 2, 29)]);
    }

    #[test]
    fn yearly_by_month_set() {
        let mut r = Rule::new(Freq::Yearly);
        r.by_month = vec![3, 9];
        r.count = Some(3);
        assert_eq!(all(&r, d(2026, 3, 1)), vec![d(2026, 3, 1), d(2026, 9, 1), d(2027, 3, 1)]);
    }

    #[test]
    fn exceptions_are_skipped_but_counted() {
        let mut r = Rule::new(Freq::Daily);
        r.count = Some(3);
        let got = expand(&r, d(2026, 10, 5), &[d(2026, 10, 6)], d(2026, 10, 1), d(2026, 12, 31));
        assert_eq!(got, vec![d(2026, 10, 5), d(2026, 10, 7)]);
    }

    #[test]
    fn window_bounds() {
        let r = Rule::new(Freq::Daily);
        let got = expand(&r, d(2026, 10, 5), &[], d(2026, 10, 6), d(2026, 10, 7));
        assert_eq!(got, vec![d(2026, 10, 6), d(2026, 10, 7)]);
        assert!(expand(&r, d(2026, 10, 5), &[], d(2026, 10, 9), d(2026, 10, 8)).is_empty());
    }

    #[test]
    fn expansion_is_bounded() {
        let r = Rule::new(Freq::Daily);
        let got = expand(&r, d(2000, 1, 1), &[], d(2000, 1, 1), d(9999, 12, 31));
        assert_eq!(got.len(), MAX_OUT);
    }

    #[test]
    fn parse_and_format_round_trip() {
        let text = "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10";
        let r = parse_rrule(text).unwrap();
        assert_eq!(r.freq, Freq::Weekly);
        assert_eq!(r.interval, 2);
        assert_eq!(r.by_day, vec![(0, 1), (0, 3)]);
        assert_eq!(r.count, Some(10));
        assert_eq!(to_rrule(&r), text);
        let m = parse_rrule("RRULE:FREQ=MONTHLY;BYDAY=-1FR;UNTIL=20271231T000000Z").unwrap();
        assert_eq!(m.by_day, vec![(-1, 5)]);
        assert_eq!(m.until, Some(d(2027, 12, 31)));
        assert_eq!(to_rrule(&m), "FREQ=MONTHLY;BYDAY=-1FR;UNTIL=20271231");
    }

    #[test]
    fn parse_rejects_bad_rules() {
        assert!(parse_rrule("INTERVAL=2").is_none());
        assert!(parse_rrule("FREQ=HOURLY").is_none());
        assert!(parse_rrule("FREQ=DAILY;INTERVAL=0").is_none());
        assert!(parse_rrule("FREQ=DAILY;BYDAY=XX").is_none());
        assert!(parse_rrule("FREQ=MONTHLY;BYMONTHDAY=0").is_none());
        assert!(parse_rrule("FREQ=YEARLY;BYMONTH=13").is_none());
    }

    #[test]
    fn describe_labels() {
        assert_eq!(describe(None), "Never");
        let mut r = Rule::new(Freq::Weekly);
        assert_eq!(describe(Some(&r)), "Every week");
        r.interval = 2;
        assert_eq!(describe(Some(&r)), "Every 2 weeks");
    }
}

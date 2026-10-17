//! The Birthdays widget: Contacts gives the shell a timeline for Notification Centre's widget area through the
//! `widgets_timeline` service, one entry per local day for the next week. An entry names the people whose birthday
//! falls on that day (with the age they turn when the year is known), or the next birthday and how far away it is.
//!
//! The birthday text is the card field written by `details::normalize_birthday`: `YYYY-MM-DD`, or the year-less
//! `--MM-DD`. A card whose birthday is anything else is left out. Only names and dates go to the shell, and they stay on
//! this machine. The shell asks for a fresh timeline when the last entry's day arrives; while Contacts is running the
//! timeline is also rebuilt whenever a card changes or the day rolls over (`Widget::sync` sends only a changed timeline).

use bifrost_app::calendar_system::{civil_from_days, days_from_civil};
use bifrost_app::Ctx;
use bifrost_app::widgets_timeline::{self, Entry, ReloadPolicy};

/// The widget kind the shell stores the timeline under.
pub const KIND: &str = "birthdays";
/// Days of entries sent at once.
pub const DAYS: i64 = 7;
/// People named in one entry before "and N more".
const NAMED: usize = 3;

/// A card with a birthday: its display name and the birthday text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub birthday: String,
}

/// A parsed birthday: the year when known, the month (1 to 12) and the day.
pub fn parse_birthday(text: &str) -> Option<(Option<i64>, i64, i64)> {
    let t = text.trim();
    let (year, rest) = if let Some(rest) = t.strip_prefix("--") {
        (None, rest.replace('-', ""))
    } else {
        let (y, rest) = t.split_once('-')?;
        if y.len() != 4 {
            return None;
        }
        (Some(y.parse::<i64>().ok()?), rest.replace('-', ""))
    };
    if rest.len() != 4 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (m, d) = (rest[..2].parse::<i64>().ok()?, rest[2..].parse::<i64>().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((year, m, d))
}

/// One birthday that is coming up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upcoming {
    pub name: String,
    /// Days from the reference day (0 is that day itself).
    pub in_days: i64,
    /// The age reached on that day, when the birth year is known and the person was born before it.
    pub turns: Option<i64>,
}

/// The birthdays on or after `today` (days since 1970-01-01, local), soonest first and then by name. A 29 February
/// birthday falls on 1 March in a year with no 29 February.
pub fn upcoming(people: &[Person], today: i64) -> Vec<Upcoming> {
    let (this_year, _, _) = civil_from_days(today);
    let mut out: Vec<Upcoming> = people
        .iter()
        .filter_map(|p| {
            let (born, m, d) = parse_birthday(&p.birthday)?;
            let mut year = this_year;
            let mut on = days_from_civil(year, m, d);
            if on < today {
                year += 1;
                on = days_from_civil(year, m, d);
            }
            let (occurs_in, _, _) = civil_from_days(on);
            let turns = born.map(|b| occurs_in - b).filter(|age| *age > 0);
            Some(Upcoming { name: p.name.clone(), in_days: on - today, turns })
        })
        .collect();
    out.sort_by(|a, b| a.in_days.cmp(&b.in_days).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

fn who(u: &Upcoming) -> String {
    match u.turns {
        Some(age) => format!("{} ({age})", u.name),
        None => u.name.clone(),
    }
}

/// The title and text the widget shows on the local day `day`: the people whose birthday it is, else the next one.
pub fn summary(people: &[Person], day: i64) -> (String, String) {
    let all = upcoming(people, day);
    let Some(first) = all.first() else {
        return ("Birthdays".into(), "No birthdays saved".into());
    };
    let group: Vec<&Upcoming> = all.iter().take_while(|u| u.in_days == first.in_days).collect();
    let mut names: Vec<String> = group.iter().take(NAMED).map(|u| who(u)).collect();
    if group.len() > NAMED {
        names.push(format!("and {} more", group.len() - NAMED));
    }
    let names = names.join(", ");
    match first.in_days {
        0 => ("Birthday today".into(), names),
        1 => ("Birthday tomorrow".into(), names),
        n => ("Next birthday".into(), format!("{names}, in {n} days")),
    }
}

/// The timeline: one entry for each of the next [`DAYS`] local days starting with the day of `now` (unix seconds) in the
/// zone `offset` (seconds east of UTC). Empty when no card has a usable birthday.
pub fn entries(people: &[Person], now: i64, offset: i32) -> Vec<Entry> {
    if !people.iter().any(|p| parse_birthday(&p.birthday).is_some()) {
        return Vec::new();
    }
    let today = (now + offset as i64).div_euclid(86_400);
    (0..DAYS)
        .map(|k| {
            let (title, text) = summary(people, today + k);
            Entry { date: ((today + k) * 86_400 - offset as i64) as f64, title, text }
        })
        .collect()
}

/// What Contacts last sent to the shell.
#[derive(Default)]
pub struct Widget {
    sent: Option<Vec<Entry>>,
}

impl Widget {
    /// Brings the shell's timeline in line with the cards. Sends nothing when it has not changed; removes the timeline
    /// when the last birthday goes.
    pub fn sync(&mut self, ctx: &Ctx, people: &[Person], now: i64, offset: i32) {
        let next = entries(people, now, offset);
        match (&self.sent, next.is_empty()) {
            (None, true) => {}
            (Some(_), true) => {
                widgets_timeline::remove(ctx, KIND);
                self.sent = None;
            }
            (Some(old), false) if *old == next => {}
            (_, false) => {
                let reload_at = ((now + offset as i64).div_euclid(86_400) + DAYS) * 86_400 - offset as i64;
                widgets_timeline::reload(ctx, KIND, &next, ReloadPolicy::After(reload_at as f64));
                self.sent = Some(next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, birthday: &str) -> Person {
        Person { name: name.into(), birthday: birthday.into() }
    }

    fn day(y: i64, m: i64, d: i64) -> i64 {
        days_from_civil(y, m, d)
    }

    #[test]
    fn both_birthday_forms_parse_and_anything_else_does_not() {
        assert_eq!(parse_birthday("1990-05-17"), Some((Some(1990), 5, 17)));
        assert_eq!(parse_birthday("--05-17"), Some((None, 5, 17)));
        assert_eq!(parse_birthday("--0517"), Some((None, 5, 17)));
        for bad in ["May 17", "", "1990-13-01", "1990-00-10", "90-05-17", "--05-32", "1990-5-7"] {
            assert_eq!(parse_birthday(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_next_occurrence_and_the_age_turned_are_worked_out() {
        let people = [person("Ada", "1990-05-17"), person("Bob", "--12-25"), person("Cy", "2025-05-17")];
        let got = upcoming(&people, day(2026, 5, 17));
        assert_eq!(got[0], Upcoming { name: "Ada".into(), in_days: 0, turns: Some(36) });
        assert_eq!(got[1].name, "Cy");
        assert_eq!(got[1].turns, Some(1));
        assert_eq!(got[2], Upcoming { name: "Bob".into(), in_days: day(2026, 12, 25) - day(2026, 5, 17), turns: None });
        // a day later the birthday has passed, so it is next year's
        let after = upcoming(&[person("Ada", "1990-05-17")], day(2026, 5, 18));
        assert_eq!(after[0].in_days, day(2027, 5, 17) - day(2026, 5, 18));
        assert_eq!(after[0].turns, Some(37));
        // born today or in the future: no age
        assert_eq!(upcoming(&[person("Newborn", "2026-05-17")], day(2026, 5, 17))[0].turns, None);
    }

    #[test]
    fn a_leap_day_birthday_falls_on_the_first_of_march_in_other_years() {
        let got = upcoming(&[person("Leap", "--02-29")], day(2027, 2, 1));
        assert_eq!(got[0].in_days, day(2027, 3, 1) - day(2027, 2, 1));
        let leap = upcoming(&[person("Leap", "--02-29")], day(2028, 2, 1));
        assert_eq!(leap[0].in_days, 28);
    }

    #[test]
    fn the_summary_names_todays_birthdays_or_the_next_one() {
        let people = [person("Ada", "1990-05-17"), person("Bob", "--05-17"), person("Cy", "--05-20")];
        assert_eq!(summary(&people, day(2026, 5, 17)), ("Birthday today".to_string(), "Ada (36), Bob".to_string()));
        assert_eq!(summary(&people, day(2026, 5, 16)), ("Birthday tomorrow".to_string(), "Ada (36), Bob".to_string()));
        assert_eq!(summary(&people, day(2026, 5, 18)), ("Next birthday".to_string(), "Cy, in 2 days".to_string()));
        let crowd: Vec<Person> = (0..5).map(|i| person(&format!("P{i}"), "--06-01")).collect();
        assert_eq!(summary(&crowd, day(2026, 6, 1)).1, "P0, P1, P2, and 2 more");
        assert_eq!(summary(&[], day(2026, 6, 1)).1, "No birthdays saved");
    }

    #[test]
    fn the_timeline_has_one_entry_per_local_day_starting_at_local_midnight() {
        let people = [person("Ada", "--05-18")];
        // 2026-05-17 23:30 local in a zone 2 hours east of UTC is 21:30 UTC
        let now = day(2026, 5, 17) * 86_400 + 21 * 3600 + 1800;
        let list = entries(&people, now, 7200);
        assert_eq!(list.len(), DAYS as usize);
        assert_eq!(list[0].date, (day(2026, 5, 17) * 86_400 - 7200) as f64, "local midnight, as a UTC instant");
        assert_eq!(list[0].title, "Birthday tomorrow");
        assert_eq!(list[1].title, "Birthday today");
        assert!(list.windows(2).all(|w| w[1].date - w[0].date == 86_400.0));
        assert!(entries(&[person("X", "May 17")], now, 0).is_empty(), "no usable birthday, no timeline");
    }
}

//! Settings > Battery > Schedule: the repeating times at which the computer wakes from sleep or starts from off.
//! The model is `bifrost_core::wake_schedule`; this file holds what the page shows, how a tap edits the schedule and
//! how the schedule is kept (one `wake_schedule` pref, entries separated by `;`). After every edit the shell sends
//! `Effect::WakeSchedule` with the next moment, and the binary arms the real-time clock for it through privd's
//! `power.wake_alarm`.

use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use super::prefs::Prefs;
use bifrost_core::wake_schedule::{weekday_of_day, Action as WakeAction, Entry, Schedule, Weekdays};

/// The pref that keeps the schedule.
pub const PREF: &str = "wake_schedule";

/// The time a new entry starts with: 07:00 on weekdays.
const NEW_MINUTE: u16 = 7 * 60;

/// What a row of the Schedule page does. The number is the entry's index in the schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeEdit {
    Add,
    Time(usize),
    Days(usize),
    Kind(usize),
    Enabled(usize),
    Remove(usize),
}

/// Reads the schedule from the prefs (empty when unset or damaged).
pub fn load(p: &Prefs) -> Schedule {
    Schedule::from_text(&p.get(PREF).unwrap_or("").replace(';', "\n"))
}

/// Keeps the schedule in the prefs.
pub fn save(s: &Schedule, p: &mut Prefs) {
    let _ = p.set(PREF, &s.to_text().trim_end().replace('\n', ";"));
}

/// The Unix time to arm the clock for (0 = none): the next enabled entry after `now`, with `utc_offset` seconds east of UTC.
pub fn next_at(s: &Schedule, now: i64, utc_offset: i64) -> i64 {
    s.next(now, utc_offset).map_or(0, |m| m.at)
}

/// The set that follows `days` when the Repeat row is tapped: Every day, Weekdays, Weekend, and round again.
pub fn next_days(days: Weekdays) -> Weekdays {
    match days {
        Weekdays::EVERY_DAY => Weekdays::WEEKDAYS,
        Weekdays::WEEKDAYS => Weekdays::WEEKEND,
        _ => Weekdays::EVERY_DAY,
    }
}

/// Applies an edit that needs no time picker. Returns false when the entry does not exist.
pub fn apply(s: &mut Schedule, edit: WakeEdit) -> bool {
    match edit {
        WakeEdit::Add => {
            s.add(Entry::new(Weekdays::WEEKDAYS, NEW_MINUTE, WakeAction::Wake));
            true
        }
        WakeEdit::Remove(i) => s.remove(i),
        WakeEdit::Enabled(i) => match s.entries().get(i).map(|e| e.enabled) {
            Some(on) => s.set_enabled(i, !on),
            None => false,
        },
        WakeEdit::Days(i) => match s.entries().get(i).copied() {
            Some(old) => replace(s, i, Entry { days: next_days(old.days), ..old }),
            None => false,
        },
        WakeEdit::Kind(i) => match s.entries().get(i).copied() {
            Some(old) => {
                let action = if old.action == WakeAction::Wake { WakeAction::PowerOn } else { WakeAction::Wake };
                replace(s, i, Entry { action, ..old })
            }
            None => false,
        },
        // the time is chosen on the picker: see `set_minute`
        WakeEdit::Time(i) => i < s.entries().len(),
    }
}

/// Sets entry `index` to start at `minute` of the day.
pub fn set_minute(s: &mut Schedule, index: usize, minute: u16) -> bool {
    match s.entries().get(index).copied() {
        Some(old) => replace(s, index, Entry { minute_of_day: minute.min(1439), ..old }),
        None => false,
    }
}

/// The minute of the day entry `index` starts at.
pub fn minute_of(s: &Schedule, index: usize) -> u16 {
    s.entries().get(index).map_or(NEW_MINUTE, |e| e.minute_of_day)
}

/// Replaces entry `index`, keeping the order of the others.
fn replace(s: &mut Schedule, index: usize, entry: Entry) -> bool {
    let mut all: Vec<Entry> = s.entries().to_vec();
    if index >= all.len() {
        return false;
    }
    all[index] = entry;
    *s = Schedule::new();
    for e in all {
        s.add(e);
    }
    true
}

fn kind_label(a: WakeAction) -> &'static str {
    match a {
        WakeAction::Wake => "Wake",
        WakeAction::PowerOn => "Power On",
    }
}

const WEEKDAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// The value of the Schedule row on the Battery page.
pub fn summary(s: &Schedule) -> String {
    match s.entries().iter().filter(|e| e.enabled).count() {
        0 => "None".into(),
        1 => "1 time".into(),
        n => format!("{n} times"),
    }
}

/// The footer line: when the computer starts itself next.
pub fn next_label(s: &Schedule, now: i64, utc_offset: i64, clock_24h: bool) -> String {
    match s.next(now, utc_offset) {
        None => "Nothing is scheduled. The computer only starts when you press the power button.".into(),
        Some(m) => {
            let local = m.at + utc_offset;
            let day = WEEKDAY_NAMES[usize::from(weekday_of_day(local.div_euclid(86_400)) % 7)];
            let minute = local.rem_euclid(86_400) / 60;
            format!("Next: {day} at {}. The computer starts itself at the times above, from sleep or from off.", crate::status::clock_in((minute / 60) as i32, (minute % 60) as i32, clock_24h))
        }
    }
}

/// The page: one group per entry (time, repeat, action, enabled, remove), then Add Time.
pub fn spec(s: &Schedule, clock_24h: bool, now: i64, utc_offset: i64) -> Spec {
    let mut groups = Vec::new();
    for (i, e) in s.entries().iter().enumerate() {
        let time = crate::status::clock_in(i32::from(e.minute_of_day / 60), i32::from(e.minute_of_day % 60), clock_24h);
        groups.push(
            Group::new(vec![
                Item::new("Time").value(time).action(Action::Wake(WakeEdit::Time(i))),
                Item::new("Repeat").value(e.days.label()).action(Action::Wake(WakeEdit::Days(i))),
                Item::new("Action").value(kind_label(e.action)).action(Action::Wake(WakeEdit::Kind(i))),
                Item::new("Enabled").switch(e.enabled, Action::Wake(WakeEdit::Enabled(i))),
                Item::new("Remove").tint(Tint::Red).action(Action::Wake(WakeEdit::Remove(i))),
            ])
            .header(&format!("TIME {}", i + 1)),
        );
    }
    groups.push(Group::new(vec![Item::new("Add Time").tint(Tint::Blue).action(Action::Wake(WakeEdit::Add))]).footer(&next_label(s, now, utc_offset, clock_24h)));
    Spec { id: PageId::WakeSchedule, title: "Schedule".into(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_apply_and_round_trip_through_the_pref() {
        let mut s = Schedule::new();
        assert!(apply(&mut s, WakeEdit::Add));
        assert!(apply(&mut s, WakeEdit::Days(0)));
        assert_eq!(s.entries()[0].days, Weekdays::WEEKEND);
        assert!(apply(&mut s, WakeEdit::Kind(0)));
        assert_eq!(s.entries()[0].action, WakeAction::PowerOn);
        assert!(set_minute(&mut s, 0, 6 * 60 + 30));
        assert!(!apply(&mut s, WakeEdit::Remove(3)));
        let mut p = Prefs::memory();
        save(&s, &mut p);
        assert_eq!(load(&p), s);
        assert_eq!(summary(&s), "1 time");
        apply(&mut s, WakeEdit::Enabled(0));
        assert_eq!(summary(&s), "None");
    }

    #[test]
    fn next_moment_is_found_and_described() {
        let mut s = Schedule::new();
        s.add(Entry::new(Weekdays::EVERY_DAY, 7 * 60, WakeAction::Wake));
        // 1970-01-01 00:00 UTC, a Thursday
        assert_eq!(next_at(&s, 0, 0), 7 * 3600);
        assert!(next_label(&s, 0, 0, true).starts_with("Next: Thursday at 7:00"));
        assert_eq!(next_at(&Schedule::new(), 0, 0), 0);
    }
}

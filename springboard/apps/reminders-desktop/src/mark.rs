//! Mark Due Date As: Today, Tomorrow, This Weekend, Next Week, and Mark All Overdue to Today. Each acts on a set of
//! reminder ids and returns the previous (due, time) pairs so one undo step restores them.
#![allow(dead_code)] // the Mark Due Date As submenu wires these in the next step

use crate::model::Reminder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkDue {
    Today,
    Tomorrow,
    ThisWeekend,
    NextWeek,
}

impl MarkDue {
    pub const ALL: [Self; 4] = [Self::Today, Self::Tomorrow, Self::ThisWeekend, Self::NextWeek];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Tomorrow => "Tomorrow",
            Self::ThisWeekend => "This Weekend",
            Self::NextWeek => "Next Week",
        }
    }

    /// The local day this mark means, from today's day number. The week starts on Monday: This Weekend is the coming
    /// Saturday (today when today is Saturday or Sunday), Next Week is the Monday after today.
    pub fn day(self, today: i64) -> i64 {
        let weekday = (today + 3).rem_euclid(7); // 1970-01-01 was a Thursday; Monday is 0
        match self {
            Self::Today => today,
            Self::Tomorrow => today + 1,
            Self::ThisWeekend => {
                if weekday >= 5 {
                    today
                } else {
                    today + (5 - weekday)
                }
            }
            Self::NextWeek => today + (7 - weekday),
        }
    }
}

/// What a reminder had before a mark, for undo.
pub type Previous = (u64, Option<i64>, Option<u16>);

/// Sets the due day of the reminders in `ids` (a time of day is kept). Completed reminders are left alone.
pub fn mark_due(items: &mut [Reminder], ids: &[u64], mark: MarkDue, today: i64) -> Vec<Previous> {
    let day = mark.day(today);
    let mut before = vec![];
    for r in items.iter_mut().filter(|r| ids.contains(&r.id) && !r.completed) {
        if r.due != Some(day) {
            before.push((r.id, r.due, r.time));
            r.due = Some(day);
        }
    }
    before
}

/// Moves every open reminder due before today to today, keeping its time of day.
pub fn overdue_to_today(items: &mut [Reminder], today: i64) -> Vec<Previous> {
    let mut before = vec![];
    for r in items.iter_mut().filter(|r| !r.completed) {
        if r.due.is_some_and(|d| d < today) {
            before.push((r.id, r.due, r.time));
            r.due = Some(today);
        }
    }
    before
}

/// Puts back what a mark changed.
pub fn undo(items: &mut [Reminder], before: &[Previous]) {
    for (id, due, time) in before {
        if let Some(r) = items.iter_mut().find(|r| r.id == *id) {
            r.due = *due;
            r.time = *time;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::clock::tz::days_from_civil;

    // 2026-10-14 is a Wednesday
    fn wednesday() -> i64 {
        days_from_civil(2026, 10, 14)
    }

    #[test]
    fn days_follow_the_week() {
        let t = wednesday();
        assert_eq!(MarkDue::Today.day(t), t);
        assert_eq!(MarkDue::Tomorrow.day(t), t + 1);
        assert_eq!(MarkDue::ThisWeekend.day(t), days_from_civil(2026, 10, 17));
        assert_eq!(MarkDue::NextWeek.day(t), days_from_civil(2026, 10, 19));
    }

    #[test]
    fn weekend_stays_when_it_is_the_weekend() {
        let saturday = days_from_civil(2026, 10, 17);
        let sunday = saturday + 1;
        assert_eq!(MarkDue::ThisWeekend.day(saturday), saturday);
        assert_eq!(MarkDue::ThisWeekend.day(sunday), sunday);
        assert_eq!(MarkDue::NextWeek.day(sunday), days_from_civil(2026, 10, 19));
        let monday = days_from_civil(2026, 10, 19);
        assert_eq!(MarkDue::NextWeek.day(monday), days_from_civil(2026, 10, 26));
    }

    #[test]
    fn marking_changes_only_the_selection_and_undoes() {
        let t = wednesday();
        let mut items = vec![Reminder::new(1, "a"), Reminder::new(2, "b"), Reminder::new(3, "c")];
        items[0].due = Some(t - 3);
        items[0].time = Some(600);
        items[2].completed = true;
        let before = mark_due(&mut items, &[1, 3], MarkDue::Tomorrow, t);
        assert_eq!(items[0].due, Some(t + 1));
        assert_eq!(items[0].time, Some(600));
        assert_eq!(items[1].due, None);
        assert_eq!(items[2].due, None);
        assert_eq!(before, vec![(1, Some(t - 3), Some(600))]);
        undo(&mut items, &before);
        assert_eq!(items[0].due, Some(t - 3));
    }

    #[test]
    fn overdue_moves_to_today_in_one_step() {
        let t = wednesday();
        let mut items = vec![Reminder::new(1, "a"), Reminder::new(2, "b"), Reminder::new(3, "c")];
        items[0].due = Some(t - 5);
        items[1].due = Some(t + 2);
        items[2].due = Some(t - 1);
        items[2].completed = true;
        let before = overdue_to_today(&mut items, t);
        assert_eq!(items[0].due, Some(t));
        assert_eq!(items[1].due, Some(t + 2));
        assert_eq!(items[2].due, Some(t - 1));
        assert_eq!(before.len(), 1);
    }
}

//! Custom smart lists: a name and a set of rules over date, time of day, priority, tags, lists and the flag.
//! `SmartDef::matches` is the filter the editor sheet builds; `to_line`/`from_line` is its storage form (one
//! line each, so the store can keep them beside the lists).
#![allow(dead_code)] // the filter editor sheet and the sidebar section wire this in next

use crate::model::{tag_key, Reminder};

/// A due-date test. Days are local calendar days (`today` is passed in).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DueTest {
    Any,
    /// Due today.
    Today,
    /// Due before today.
    Overdue,
    /// Due from tomorrow on.
    Upcoming,
    /// Due within the next `n` days, today included.
    Within(u32),
    /// Has no due date.
    None,
    Between(i64, i64),
}

/// A time-of-day test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeTest {
    Any,
    Morning,
    Afternoon,
    Evening,
    /// Has no time of day.
    Unset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartDef {
    pub id: u64,
    pub name: String,
    pub due: DueTest,
    pub time: TimeTest,
    /// Minimum priority (0 = any, 1 low, 2 medium, 3 high).
    pub min_priority: u8,
    pub flagged_only: bool,
    /// Tags to match (lower case, no `#`).
    pub tags: Vec<String>,
    /// All of the tags when true, any of them when false.
    pub all_tags: bool,
    /// Lists to include; empty means every list.
    pub include_lists: Vec<u64>,
    pub exclude_lists: Vec<u64>,
    pub show_completed: bool,
}

impl SmartDef {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        SmartDef {
            id,
            name: name.into(),
            due: DueTest::Any,
            time: TimeTest::Any,
            min_priority: 0,
            flagged_only: false,
            tags: vec![],
            all_tags: false,
            include_lists: vec![],
            exclude_lists: vec![],
            show_completed: false,
        }
    }

    pub fn matches(&self, r: &Reminder, today: i64) -> bool {
        if r.completed && !self.show_completed {
            return false;
        }
        if self.flagged_only && !r.flagged {
            return false;
        }
        if r.priority < self.min_priority {
            return false;
        }
        if !self.include_lists.is_empty() && !self.include_lists.contains(&r.list) {
            return false;
        }
        if self.exclude_lists.contains(&r.list) {
            return false;
        }
        if !self.due_ok(r.due, today) || !self.time_ok(r.time) {
            return false;
        }
        self.tags_ok(r)
    }

    fn due_ok(&self, due: Option<i64>, today: i64) -> bool {
        match (self.due, due) {
            (DueTest::Any, _) => true,
            (DueTest::None, d) => d.is_none(),
            (_, None) => false,
            (DueTest::Today, Some(d)) => d == today,
            (DueTest::Overdue, Some(d)) => d < today,
            (DueTest::Upcoming, Some(d)) => d > today,
            (DueTest::Within(n), Some(d)) => d >= today && d <= today + n as i64,
            (DueTest::Between(a, b), Some(d)) => d >= a && d <= b,
        }
    }

    fn time_ok(&self, time: Option<u16>) -> bool {
        match (self.time, time) {
            (TimeTest::Any, _) => true,
            (TimeTest::Unset, t) => t.is_none(),
            (_, None) => false,
            (TimeTest::Morning, Some(m)) => m < 12 * 60,
            (TimeTest::Afternoon, Some(m)) => (12 * 60..18 * 60).contains(&m),
            (TimeTest::Evening, Some(m)) => m >= 18 * 60,
        }
    }

    fn tags_ok(&self, r: &Reminder) -> bool {
        if self.tags.is_empty() {
            return true;
        }
        let have: Vec<u64> = r.tags().iter().map(|t| tag_key(t)).collect();
        let has = |t: &String| have.contains(&tag_key(t));
        if self.all_tags {
            self.tags.iter().all(has)
        } else {
            self.tags.iter().any(has)
        }
    }

    /// The reminders that match, in their stored order.
    pub fn select<'a>(&self, items: &'a [Reminder], today: i64) -> Vec<&'a Reminder> {
        items.iter().filter(|r| self.matches(r, today)).collect()
    }

    pub fn count(&self, items: &[Reminder], today: i64) -> usize {
        items.iter().filter(|r| self.matches(r, today)).count()
    }

    /// One tab-separated line: id, name, due, time, priority, flagged, all_tags, completed, tags, include, exclude.
    pub fn to_line(&self) -> String {
        let due = match self.due {
            DueTest::Any => "any".to_string(),
            DueTest::Today => "today".into(),
            DueTest::Overdue => "overdue".into(),
            DueTest::Upcoming => "upcoming".into(),
            DueTest::Within(n) => format!("within:{n}"),
            DueTest::None => "none".into(),
            DueTest::Between(a, b) => format!("between:{a}:{b}"),
        };
        let time = match self.time {
            TimeTest::Any => "any",
            TimeTest::Morning => "morning",
            TimeTest::Afternoon => "afternoon",
            TimeTest::Evening => "evening",
            TimeTest::Unset => "unset",
        };
        let ids = |v: &[u64]| v.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.id,
            self.name.replace(['\t', '\n'], " "),
            due,
            time,
            self.min_priority,
            self.flagged_only as u8,
            self.all_tags as u8,
            self.show_completed as u8,
            self.tags.join(","),
            ids(&self.include_lists),
            ids(&self.exclude_lists),
        )
    }

    pub fn from_line(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 11 {
            return None;
        }
        let due = match f[2].split(':').collect::<Vec<_>>().as_slice() {
            ["any"] => DueTest::Any,
            ["today"] => DueTest::Today,
            ["overdue"] => DueTest::Overdue,
            ["upcoming"] => DueTest::Upcoming,
            ["none"] => DueTest::None,
            ["within", n] => DueTest::Within(n.parse().ok()?),
            ["between", a, b] => DueTest::Between(a.parse().ok()?, b.parse().ok()?),
            _ => return None,
        };
        let time = match f[3] {
            "any" => TimeTest::Any,
            "morning" => TimeTest::Morning,
            "afternoon" => TimeTest::Afternoon,
            "evening" => TimeTest::Evening,
            "unset" => TimeTest::Unset,
            _ => return None,
        };
        let ids = |s: &str| -> Option<Vec<u64>> {
            if s.is_empty() {
                return Some(vec![]);
            }
            s.split(',').map(|n| n.parse().ok()).collect()
        };
        Some(SmartDef {
            id: f[0].parse().ok()?,
            name: f[1].to_string(),
            due,
            time,
            min_priority: f[4].parse::<u8>().ok()?.min(3),
            flagged_only: f[5] == "1",
            all_tags: f[6] == "1",
            show_completed: f[7] == "1",
            tags: f[8].split(',').filter(|t| !t.is_empty()).map(str::to_string).collect(),
            include_lists: ids(f[9])?,
            exclude_lists: ids(f[10])?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 20_000;

    fn r(id: u64, title: &str) -> Reminder {
        Reminder::new(id, title)
    }

    #[test]
    fn due_tests() {
        let mut a = r(1, "a");
        let mut def = SmartDef::new(1, "d");
        def.due = DueTest::Overdue;
        assert!(!def.matches(&a, T));
        a.due = Some(T - 1);
        assert!(def.matches(&a, T));
        def.due = DueTest::Within(3);
        a.due = Some(T + 3);
        assert!(def.matches(&a, T));
        a.due = Some(T + 4);
        assert!(!def.matches(&a, T));
        def.due = DueTest::None;
        a.due = None;
        assert!(def.matches(&a, T));
    }

    #[test]
    fn time_priority_flag_and_completed() {
        let mut a = r(1, "a");
        let mut def = SmartDef::new(1, "d");
        def.time = TimeTest::Afternoon;
        a.time = Some(13 * 60);
        assert!(def.matches(&a, T));
        a.time = Some(9 * 60);
        assert!(!def.matches(&a, T));
        def.time = TimeTest::Any;
        def.min_priority = 2;
        a.priority = 1;
        assert!(!def.matches(&a, T));
        a.priority = 3;
        def.flagged_only = true;
        assert!(!def.matches(&a, T));
        a.flagged = true;
        assert!(def.matches(&a, T));
        a.completed = true;
        assert!(!def.matches(&a, T));
        def.show_completed = true;
        assert!(def.matches(&a, T));
    }

    #[test]
    fn tags_any_and_all() {
        let a = r(1, "pay rent #home #money");
        let mut def = SmartDef::new(1, "d");
        def.tags = vec!["home".into(), "work".into()];
        assert!(def.matches(&a, T));
        def.all_tags = true;
        assert!(!def.matches(&a, T));
        def.tags = vec!["Home".into(), "money".into()];
        assert!(def.matches(&a, T));
    }

    #[test]
    fn list_include_and_exclude() {
        let mut a = r(1, "a");
        a.list = 2;
        let mut def = SmartDef::new(1, "d");
        def.include_lists = vec![1, 2];
        assert!(def.matches(&a, T));
        def.exclude_lists = vec![2];
        assert!(!def.matches(&a, T));
        def.include_lists = vec![3];
        def.exclude_lists = vec![];
        assert!(!def.matches(&a, T));
    }

    #[test]
    fn lines_round_trip() {
        let mut def = SmartDef::new(7, "Work this week");
        def.due = DueTest::Between(100, 200);
        def.time = TimeTest::Morning;
        def.min_priority = 2;
        def.flagged_only = true;
        def.tags = vec!["work".into(), "urgent".into()];
        def.all_tags = true;
        def.include_lists = vec![1, 4];
        def.exclude_lists = vec![9];
        def.show_completed = true;
        assert_eq!(SmartDef::from_line(&def.to_line()), Some(def));
        assert_eq!(SmartDef::from_line("junk"), None);
        assert_eq!(SmartDef::from_line("1\tx\twithin:z\tany\t0\t0\t0\t0\t\t\t"), None);
    }

    #[test]
    fn select_keeps_order() {
        let mut items = vec![r(1, "a"), r(2, "b"), r(3, "c")];
        items[1].flagged = true;
        items[2].flagged = true;
        let mut def = SmartDef::new(1, "d");
        def.flagged_only = true;
        let ids: Vec<u64> = def.select(&items, T).iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(def.count(&items, T), 2);
    }
}

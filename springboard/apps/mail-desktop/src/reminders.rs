//! Message > Remind Me. A reminded message leaves the list until its time; then it comes back as unread and raises a
//! notification. A reminder is keyed by the message's Message-ID (UID values change when a message moves), and the list is kept
//! in the app's data directory, one reminder a line: the due time in Unix seconds, a tab, the Message-ID.

use std::path::Path;

/// File name of the reminders in the app's data directory.
pub const FILE: &str = "reminders.txt";

/// A reminder older than this many seconds past its due time, whose message was never found, is dropped.
pub const STALE_SECS: i64 = 30 * 86_400;

/// The choices of the Remind Me submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    InAnHour,
    Tomorrow,
    InThreeDays,
    NextWeek,
}

impl When {
    pub const ALL: [When; 4] = [When::InAnHour, When::Tomorrow, When::InThreeDays, When::NextWeek];

    pub fn label(self) -> &'static str {
        match self {
            When::InAnHour => "In 1 Hour",
            When::Tomorrow => "Tomorrow",
            When::InThreeDays => "In 3 Days",
            When::NextWeek => "Next Week",
        }
    }

    pub fn seconds(self) -> i64 {
        match self {
            When::InAnHour => 3_600,
            When::Tomorrow => 86_400,
            When::InThreeDays => 3 * 86_400,
            When::NextWeek => 7 * 86_400,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub message_id: String,
    /// Unix seconds.
    pub due: i64,
}

/// Adds a reminder, replacing any earlier one for the same message.
pub fn set(list: &mut Vec<Reminder>, message_id: &str, due: i64) {
    list.retain(|r| r.message_id != message_id);
    list.push(Reminder { message_id: message_id.to_string(), due });
}

/// True while a reminder keeps the message out of the list: one exists and its time has not come.
pub fn hides(list: &[Reminder], message_id: &str, now: i64) -> bool {
    list.iter().any(|r| r.message_id == message_id && r.due > now)
}

/// The reminders whose time has come.
pub fn due(list: &[Reminder], now: i64) -> Vec<Reminder> {
    list.iter().filter(|r| r.due <= now).cloned().collect()
}

pub fn parse(text: &str) -> Vec<Reminder> {
    text.lines()
        .filter_map(|line| {
            let (due, id) = line.split_once('\t')?;
            let id = id.trim();
            (!id.is_empty()).then(|| Reminder { message_id: id.to_string(), due: due.trim().parse().ok()? })
        })
        .collect()
}

pub fn serialize(list: &[Reminder]) -> String {
    list.iter().filter(|r| !r.message_id.contains(['\r', '\n', '\t'])).map(|r| format!("{}\t{}\n", r.due, r.message_id)).collect()
}

pub fn load(path: &Path) -> Vec<Reminder> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(path: &Path, list: &[Reminder]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serialize(list)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reminder_hides_its_message_until_it_is_due() {
        let mut list = Vec::new();
        set(&mut list, "<a@x>", 1_000);
        assert!(hides(&list, "<a@x>", 999));
        assert!(!hides(&list, "<a@x>", 1_000), "due messages are back");
        assert!(!hides(&list, "<b@x>", 0));
        assert_eq!(due(&list, 999), vec![]);
        assert_eq!(due(&list, 1_000), vec![Reminder { message_id: "<a@x>".into(), due: 1_000 }]);
    }

    #[test]
    fn setting_a_reminder_again_replaces_the_old_time() {
        let mut list = Vec::new();
        set(&mut list, "<a@x>", 10);
        set(&mut list, "<a@x>", 20);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].due, 20);
    }

    #[test]
    fn reminders_round_trip_and_bad_lines_are_skipped() {
        let list = vec![Reminder { message_id: "<a@x>".into(), due: 5 }, Reminder { message_id: "<b@x>".into(), due: 9 }];
        assert_eq!(parse(&serialize(&list)), list);
        assert_eq!(parse("x\t<a>\n7\n8\t\n"), vec![]);
    }

    #[test]
    fn the_choices_are_later_and_in_order() {
        let secs: Vec<i64> = When::ALL.iter().map(|w| w.seconds()).collect();
        assert!(secs.windows(2).all(|w| w[0] < w[1]));
    }
}

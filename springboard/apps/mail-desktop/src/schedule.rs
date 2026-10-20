//! Things Mail does later: messages that wait to be sent (Send Later, and the undo-send delay) and reminders that bring a
//! message back (Message > Remind Me). Both are kept in a JSON file in the app's data directory, so a restart does not
//! lose them. Times are Unix seconds.

use crate::prefs::write_json;
use bifrost_core::json::{self, Json};
use std::path::Path;

/// File name in the app's data directory.
pub const FILE: &str = "mail-queue.json";
/// How long a message that could not be sent waits before the next try, in seconds.
pub const RETRY_SECS: i64 = 300;
/// How many times a queued message is tried before it is given up.
pub const MAX_ATTEMPTS: u32 = 5;
/// Longest time Send Later and Remind Me take: one year.
pub const MAX_DELAY_SECS: i64 = 365 * 24 * 3600;

/// A message waiting to be sent: already rendered, with its envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
    pub id: u64,
    /// When it may go out.
    pub due: i64,
    pub account: String,
    pub from: String,
    pub recipients: Vec<String>,
    /// The message as SMTP sends it (see `Draft::render`).
    pub text: String,
    /// How many times sending it failed so far.
    pub attempts: u32,
}

/// A message to bring back to the top of the inbox at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub account: String,
    pub mailbox: String,
    pub message_id: String,
    pub subject: String,
    pub due: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queue {
    pub outbox: Vec<Queued>,
    pub reminders: Vec<Reminder>,
    next_id: u64,
}

/// A waiting time such as `30m`, `2 hours`, `1d` or `1w 2d 3h`, in seconds. Units: s, m (minutes), h, d, w, with the
/// usual long forms. A bare number is minutes. Zero, nothing and more than a year are errors.
pub fn parse_delay(text: &str) -> Result<i64, String> {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() {
        return Err("Enter a time, for example 30m, 2h or 1d".into());
    }
    let mut total: i64 = 0;
    let mut chars = lower.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut digits = String::new();
        while let Some(c) = chars.peek().filter(|c| c.is_ascii_digit()) {
            digits.push(*c);
            chars.next();
        }
        if digits.is_empty() {
            return Err(format!("{} is not a time. Try 30m, 2h or 1d", text.trim()));
        }
        let amount: i64 = digits.parse().map_err(|_| "That time is too long".to_string())?;
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut unit = String::new();
        while let Some(c) = chars.peek().filter(|c| c.is_ascii_alphabetic()) {
            unit.push(*c);
            chars.next();
        }
        let seconds = match unit.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "" | "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86_400,
            "w" | "wk" | "week" | "weeks" => 604_800,
            other => return Err(format!("{other} is not a unit of time. Use s, m, h, d or w")),
        };
        total = total.saturating_add(amount.saturating_mul(seconds));
        if total > MAX_DELAY_SECS {
            return Err("That is more than a year away".into());
        }
    }
    if total <= 0 {
        return Err("Enter a time after now".into());
    }
    Ok(total)
}

impl Queue {
    /// A fresh id for a queued message.
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Takes the messages that are due out of the outbox, oldest first.
    pub fn take_due_messages(&mut self, now: i64) -> Vec<Queued> {
        let (mut due, rest): (Vec<Queued>, Vec<Queued>) = std::mem::take(&mut self.outbox).into_iter().partition(|q| q.due <= now);
        self.outbox = rest;
        due.sort_by_key(|q| (q.due, q.id));
        due
    }

    /// Takes the reminders that are due out of the list, earliest first.
    pub fn take_due_reminders(&mut self, now: i64) -> Vec<Reminder> {
        let (mut due, rest): (Vec<Reminder>, Vec<Reminder>) = std::mem::take(&mut self.reminders).into_iter().partition(|r| r.due <= now);
        self.reminders = rest;
        due.sort_by_key(|r| r.due);
        due
    }

    /// Removes a queued message (Undo Send). Returns whether it was still waiting.
    pub fn cancel(&mut self, id: u64) -> bool {
        let before = self.outbox.len();
        self.outbox.retain(|q| q.id != id);
        self.outbox.len() != before
    }

    /// Drops the reminders of one message (Message > Ignore Follow-Up). Returns how many were dropped.
    pub fn drop_reminders(&mut self, account: &str, message_id: &str) -> usize {
        let before = self.reminders.len();
        self.reminders.retain(|r| !(r.account == account && r.message_id == message_id));
        before - self.reminders.len()
    }

    /// The time the next queued message or reminder is due.
    pub fn next_due(&self) -> Option<i64> {
        self.outbox.iter().map(|q| q.due).chain(self.reminders.iter().map(|r| r.due)).min()
    }

    pub fn to_json(&self) -> Json {
        let outbox = self
            .outbox
            .iter()
            .map(|q| {
                Json::Obj(vec![
                    ("id".into(), Json::Num(q.id as f64)),
                    ("due".into(), Json::Num(q.due as f64)),
                    ("account".into(), Json::Str(q.account.clone())),
                    ("from".into(), Json::Str(q.from.clone())),
                    ("recipients".into(), Json::Arr(q.recipients.iter().map(|r| Json::Str(r.clone())).collect())),
                    ("text".into(), Json::Str(q.text.clone())),
                    ("attempts".into(), Json::Num(f64::from(q.attempts))),
                ])
            })
            .collect();
        let reminders = self
            .reminders
            .iter()
            .map(|r| {
                Json::Obj(vec![
                    ("account".into(), Json::Str(r.account.clone())),
                    ("mailbox".into(), Json::Str(r.mailbox.clone())),
                    ("message_id".into(), Json::Str(r.message_id.clone())),
                    ("subject".into(), Json::Str(r.subject.clone())),
                    ("due".into(), Json::Num(r.due as f64)),
                ])
            })
            .collect();
        Json::Obj(vec![
            ("next_id".into(), Json::Num(self.next_id as f64)),
            ("outbox".into(), Json::Arr(outbox)),
            ("reminders".into(), Json::Arr(reminders)),
        ])
    }

    /// Reads the queue back. An entry that lacks a field it needs is dropped.
    pub fn from_json(json: &Json) -> Queue {
        let text = |item: &Json, key: &str| item.get(key).and_then(Json::as_str).unwrap_or("").to_string();
        let number = |item: &Json, key: &str| item.get(key).and_then(Json::number).unwrap_or(0.0);
        let mut queue = Queue { next_id: number(json, "next_id") as u64, ..Queue::default() };
        for item in json.get("outbox").and_then(Json::as_array).unwrap_or(&[]) {
            let recipients: Vec<String> = item.get("recipients").and_then(Json::as_array).unwrap_or(&[]).iter().filter_map(Json::as_str).map(str::to_string).collect();
            let entry = Queued {
                id: number(item, "id") as u64,
                due: number(item, "due") as i64,
                account: text(item, "account"),
                from: text(item, "from"),
                recipients,
                text: text(item, "text"),
                attempts: number(item, "attempts") as u32,
            };
            if entry.id != 0 && !entry.account.is_empty() && !entry.recipients.is_empty() && !entry.text.is_empty() {
                queue.next_id = queue.next_id.max(entry.id);
                queue.outbox.push(entry);
            }
        }
        for item in json.get("reminders").and_then(Json::as_array).unwrap_or(&[]) {
            let entry = Reminder {
                account: text(item, "account"),
                mailbox: text(item, "mailbox"),
                message_id: text(item, "message_id"),
                subject: text(item, "subject"),
                due: number(item, "due") as i64,
            };
            if !entry.account.is_empty() && !entry.mailbox.is_empty() && !entry.message_id.is_empty() {
                queue.reminders.push(entry);
            }
        }
        queue
    }
}

/// Reads the queue file. A missing file is an empty queue; a damaged one is kept aside (see [`load_checked`]).
pub fn load(path: &Path) -> Queue {
    load_checked(path).0
}

/// Reads the queue file, and says what happened to a file that could not be read. A missing file is an empty queue with
/// nothing to say. A file that cannot be read or parsed is renamed to `<name>.bad` (`.bad-2`, ... when that exists), so
/// the next save does not overwrite the messages in it, and the queue starts empty with a note for the person.
pub fn load_checked(path: &Path) -> (Queue, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Queue::default(), None),
        Err(e) => return (Queue::default(), set_aside(path, &e.to_string())),
    };
    match json::parse(&text) {
        Ok(j) => (Queue::from_json(&j), None),
        Err(e) => (Queue::default(), set_aside(path, &format!("{e:?}"))),
    }
}

/// Renames a damaged queue file out of the way and describes it for the status line.
fn set_aside(path: &Path, why: &str) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let mut aside = path.with_file_name(format!("{name}.bad"));
    let mut n = 2;
    while aside.exists() {
        aside = path.with_file_name(format!("{name}.bad-{n}"));
        n += 1;
    }
    let kept = std::fs::rename(path, &aside).map(|()| aside.display().to_string());
    Some(match kept {
        Ok(kept) => format!("The outbox file was damaged and could not be read. It was kept as {kept}"),
        Err(e) => format!("The outbox file was damaged ({why}) and could not be set aside: {e}"),
    })
}

/// Writes the queue file.
pub fn save(path: &Path, queue: &Queue) -> Result<(), String> {
    write_json(path, &queue.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(id: u64, due: i64) -> Queued {
        Queued { id, due, account: "work".into(), from: "me@example.com".into(), recipients: vec!["a@example.com".into()], text: "Subject: x\r\n\r\nhi\r\n".into(), attempts: 0 }
    }

    fn reminder(due: i64) -> Reminder {
        Reminder { account: "work".into(), mailbox: "INBOX".into(), message_id: "<m@x>".into(), subject: "Plan".into(), due }
    }

    #[test]
    fn delays_read_numbers_and_units() {
        assert_eq!(parse_delay("30m"), Ok(1800));
        assert_eq!(parse_delay("30"), Ok(1800), "a bare number is minutes");
        assert_eq!(parse_delay(" 2 hours "), Ok(7200));
        assert_eq!(parse_delay("1d"), Ok(86_400));
        assert_eq!(parse_delay("1w 2d 3h"), Ok(604_800 + 2 * 86_400 + 3 * 3600));
        assert_eq!(parse_delay("90 SECONDS"), Ok(90));
    }

    #[test]
    fn nonsense_and_extremes_are_refused_with_a_reason() {
        assert!(parse_delay("").unwrap_err().contains("Enter a time"));
        assert!(parse_delay("soon").unwrap_err().contains("is not a time"));
        assert!(parse_delay("5 fortnights").unwrap_err().contains("fortnights is not a unit"));
        assert!(parse_delay("0m").unwrap_err().contains("after now"));
        assert!(parse_delay("400d").unwrap_err().contains("more than a year"));
        assert!(parse_delay("99999999999999999999d").is_err());
    }

    #[test]
    fn due_messages_come_out_oldest_first_and_the_rest_wait() {
        let mut queue = Queue::default();
        queue.outbox = vec![queued(3, 300), queued(1, 100), queued(2, 900)];
        let due = queue.take_due_messages(300);
        assert_eq!(due.iter().map(|q| q.id).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(queue.outbox.len(), 1);
        assert_eq!(queue.next_due(), Some(900));
        assert!(queue.take_due_messages(300).is_empty());
    }

    #[test]
    fn reminders_and_messages_share_the_next_due_time() {
        let mut queue = Queue::default();
        assert_eq!(queue.next_due(), None);
        queue.outbox.push(queued(1, 500));
        queue.reminders = vec![reminder(200), reminder(800)];
        assert_eq!(queue.next_due(), Some(200));
        assert_eq!(queue.take_due_reminders(250).len(), 1);
        assert_eq!(queue.next_due(), Some(500));
    }

    #[test]
    fn undo_cancels_only_a_message_that_still_waits() {
        let mut queue = Queue::default();
        let id = queue.new_id();
        queue.outbox.push(queued(id, 10));
        assert!(queue.cancel(id));
        assert!(!queue.cancel(id));
        assert_ne!(queue.new_id(), id, "ids are not reused");
    }

    #[test]
    fn the_queue_survives_a_round_trip_through_its_file_and_keeps_ids_unique() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-queue-{}", std::process::id()));
        let path = dir.join(FILE);
        assert_eq!(load(&path), Queue::default());
        let mut queue = Queue::default();
        let id = queue.new_id();
        queue.outbox.push(queued(id, 1_800_000_000));
        queue.reminders.push(reminder(1_800_000_100));
        save(&path, &queue).unwrap();
        let mut back = load(&path);
        assert_eq!(back, queue);
        assert!(back.new_id() > id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_queue_file_is_kept_aside_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-queue-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        assert_eq!(load_checked(&path), (Queue::default(), None), "a missing file is not an error");
        std::fs::write(&path, "{ not json").unwrap();
        let (queue, note) = load_checked(&path);
        assert_eq!(queue, Queue::default());
        assert!(note.unwrap().contains(".bad"));
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(dir.join(format!("{FILE}.bad"))).unwrap(), "{ not json");
        // a second damaged file does not replace the first
        std::fs::write(&path, "[ also bad").unwrap();
        let _ = load_checked(&path);
        assert_eq!(std::fs::read_to_string(dir.join(format!("{FILE}.bad-2"))).unwrap(), "[ also bad");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_without_what_they_need_are_dropped() {
        let json = bifrost_core::json::parse(r#"{"outbox":[{"id":1,"account":"a","recipients":[],"text":"x"},{"id":0,"account":"a","recipients":["b@c"],"text":"x"}],"reminders":[{"account":"a"}]}"#).unwrap();
        let queue = Queue::from_json(&json);
        assert!(queue.outbox.is_empty() && queue.reminders.is_empty());
    }
}

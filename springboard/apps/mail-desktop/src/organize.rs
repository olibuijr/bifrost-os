//! Organising mail: the favorites bar of pinned mailboxes, smart mailboxes (saved searches), Remind Me and Send
//! Later queues, the category classifier (primary, transactions, updates, promotions), and find in the message body.
//! Pure std plus `search` and `message`; each list has a line-based stored form.

use crate::message::parse_headers;
use crate::search::{Doc, Query};

/// Pinned mailboxes, in the order the favorites bar shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favorites {
    names: Vec<String>,
}

impl Favorites {
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Pins a mailbox; false when it is already there.
    pub fn add(&mut self, name: &str) -> bool {
        if self.names.iter().any(|n| n == name) {
            return false;
        }
        self.names.push(name.to_string());
        true
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.names.len();
        self.names.retain(|n| n != name);
        self.names.len() != before
    }

    /// Moves a favorite to `to` (clamped).
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.names.len() {
            let item = self.names.remove(from);
            self.names.insert(to.min(self.names.len()), item);
        }
    }

    /// A mailbox was renamed: the favorite follows it.
    pub fn rename(&mut self, from: &str, to: &str) {
        for n in &mut self.names {
            if n == from {
                *n = to.to_string();
            }
        }
    }

    pub fn to_text(&self) -> String {
        self.names.iter().map(|n| format!("{n}\n")).collect()
    }

    pub fn from_text(text: &str) -> Favorites {
        let mut f = Favorites::default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            f.add(line);
        }
        f
    }
}

/// A saved search shown in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartMailbox {
    pub name: String,
    pub query: String,
}

impl SmartMailbox {
    /// Whether a message belongs to the mailbox.
    pub fn matches(&self, doc: &Doc) -> bool {
        Query::parse(&self.query).matches(doc)
    }

    /// The indices of the messages that belong to the mailbox.
    pub fn select(&self, docs: &[Doc]) -> Vec<usize> {
        let q = Query::parse(&self.query);
        docs.iter().enumerate().filter(|(_, d)| q.matches(d)).map(|(i, _)| i).collect()
    }
}

pub fn smart_to_text(list: &[SmartMailbox]) -> String {
    list.iter().map(|m| format!("{}\t{}\n", m.name.replace(['\t', '\n'], " "), m.query.replace(['\t', '\n'], " "))).collect()
}

pub fn smart_from_text(text: &str) -> Vec<SmartMailbox> {
    text.lines()
        .filter_map(|l| {
            let (name, query) = l.split_once('\t')?;
            (!name.trim().is_empty()).then(|| SmartMailbox { name: name.trim().to_string(), query: query.to_string() })
        })
        .collect()
}

/// A message that comes back to the top of the list at `due` (Unix seconds): Remind Me.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remind {
    pub message_id: String,
    pub due: i64,
}

/// A message held back until `due` (Unix seconds): Send Later. `raw` is the rendered message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheduled {
    pub id: u64,
    pub due: i64,
    pub raw: String,
}

/// Items whose time has come, earliest first (the indices into `due_times`).
pub fn due_indices(due_times: &[i64], now: i64) -> Vec<usize> {
    let mut v: Vec<usize> = due_times.iter().enumerate().filter(|(_, d)| **d <= now).map(|(i, _)| i).collect();
    v.sort_by_key(|i| due_times[*i]);
    v
}

pub fn due_reminders(list: &[Remind], now: i64) -> Vec<&Remind> {
    let times: Vec<i64> = list.iter().map(|r| r.due).collect();
    due_indices(&times, now).into_iter().map(|i| &list[i]).collect()
}

pub fn due_sends(list: &[Scheduled], now: i64) -> Vec<&Scheduled> {
    let times: Vec<i64> = list.iter().map(|s| s.due).collect();
    due_indices(&times, now).into_iter().map(|i| &list[i]).collect()
}

/// The time for the Remind Me menu items, from a local midnight-based day number and a clock: `later_today` is three
/// hours on, `tonight` 20:00 today, `tomorrow` 08:00 tomorrow, `next_week` 08:00 on the Monday after.
pub fn remind_time(choice: &str, now: i64, offset_secs: i64) -> Option<i64> {
    let local = now + offset_secs;
    let day = local.div_euclid(86_400);
    let at = |d: i64, hour: i64| d * 86_400 + hour * 3600 - offset_secs;
    let weekday = (day + 3).rem_euclid(7); // Monday = 0
    match choice {
        "later_today" => Some(now + 3 * 3600),
        "tonight" => Some(at(day, 20).max(now + 60)),
        "tomorrow" => Some(at(day + 1, 8)),
        "next_week" => Some(at(day + 7 - weekday, 8)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Primary,
    Transactions,
    Updates,
    Promotions,
}

impl Category {
    pub const ALL: [Category; 4] = [Self::Primary, Self::Transactions, Self::Updates, Self::Promotions];

    pub fn label(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Transactions => "Transactions",
            Self::Updates => "Updates",
            Self::Promotions => "Promotions",
        }
    }
}

/// Sorts a message into a category from its raw header block (and subject). Bulk mail is told by `List-Unsubscribe`,
/// `List-Id` or `Precedence: bulk`; a transaction by words such as receipt, invoice, order; an update by notification
/// or newsletter sender patterns; everything else is Primary. Replies in a conversation stay Primary.
pub fn categorize(raw_headers: &str) -> Category {
    let h = parse_headers(raw_headers);
    let get = |name: &str| h.iter().find(|(n, _)| n == name).map(|(_, v)| v.to_lowercase()).unwrap_or_default();
    if !get("in-reply-to").is_empty() {
        return Category::Primary;
    }
    let subject = get("subject");
    let from = get("from");
    let bulk = !get("list-unsubscribe").is_empty() || !get("list-id").is_empty() || get("precedence").contains("bulk");
    let has = |text: &str, words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&subject, &["receipt", "invoice", "your order", "order #", "payment", "booking confirmation", "shipped", "delivery"]) {
        return Category::Transactions;
    }
    if has(&from, &["noreply", "no-reply", "notifications@", "notification@", "alerts@", "mailer-daemon"]) && !bulk {
        return Category::Updates;
    }
    if bulk && has(&(subject.clone() + " " + &from), &["sale", "% off", "deal", "offer", "discount", "promo", "marketing", "newsletter@", "news@"]) {
        return Category::Promotions;
    }
    if bulk {
        return Category::Updates;
    }
    Category::Primary
}

/// Every match of `needle` in `body` as `(start, end)` byte ranges, case-insensitive, for the find bar's highlights.
pub fn find_in_body(body: &str, needle: &str) -> Vec<(usize, usize)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return vec![];
    }
    let (hay, pat) = (body.to_lowercase(), needle.to_lowercase());
    if hay.len() != body.len() {
        return vec![]; // case folding changed byte lengths, so the ranges would not line up with `body`
    }
    hay.match_indices(&pat).map(|(i, m)| (i, i + m.len())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorites_order_and_text() {
        let mut f = Favorites::default();
        assert!(f.add("Inbox"));
        assert!(!f.add("Inbox"));
        f.add("Sent");
        f.add("Work");
        f.reorder(2, 0);
        assert_eq!(f.names(), ["Work", "Inbox", "Sent"]);
        f.rename("Sent", "Outbox");
        assert!(f.remove("Inbox"));
        assert_eq!(Favorites::from_text(&f.to_text()), f);
    }

    #[test]
    fn smart_mailboxes_select_with_queries() {
        let docs = [
            Doc { from: "ann@shop.is", to: "me@x.is", subject: "Order shipped", body: "on its way" },
            Doc { from: "bob@y.is", to: "me@x.is", subject: "Lunch?", body: "noon" },
        ];
        let m = SmartMailbox { name: "Shop".into(), query: "from:shop".into() };
        assert_eq!(m.select(&docs), vec![0]);
        assert!(m.matches(&docs[0]) && !m.matches(&docs[1]));
        let list = vec![m];
        assert_eq!(smart_from_text(&smart_to_text(&list)), list);
    }

    #[test]
    fn due_items_come_earliest_first() {
        let list = vec![
            Remind { message_id: "<a>".into(), due: 300 },
            Remind { message_id: "<b>".into(), due: 100 },
            Remind { message_id: "<c>".into(), due: 900 },
        ];
        let ids: Vec<&str> = due_reminders(&list, 400).iter().map(|r| r.message_id.as_str()).collect();
        assert_eq!(ids, ["<b>", "<a>"]);
        let sends = vec![Scheduled { id: 1, due: 50, raw: "x".into() }];
        assert_eq!(due_sends(&sends, 49).len(), 0);
        assert_eq!(due_sends(&sends, 50).len(), 1);
    }

    #[test]
    fn remind_times() {
        // 2026-10-15 (Thursday) 12:00 UTC
        let now = 1_792_065_600;
        assert_eq!(remind_time("later_today", now, 0), Some(now + 10_800));
        assert_eq!(remind_time("tonight", now, 0), Some(now + 8 * 3600));
        assert_eq!(remind_time("tomorrow", now, 0), Some(now + 20 * 3600));
        // next Monday 08:00 is 4 days on, less the four hours from 12:00 back to 08:00
        assert_eq!(remind_time("next_week", now, 0), Some(now + 4 * 86_400 - 4 * 3600));
        assert_eq!(remind_time("bogus", now, 0), None);
    }

    #[test]
    fn categories() {
        let c = |raw: &str| categorize(raw);
        assert_eq!(c("Subject: Your order #123 has shipped\nFrom: shop@x.is\n"), Category::Transactions);
        assert_eq!(c("Subject: Big sale\nFrom: news@x.is\nList-Unsubscribe: <mailto:u@x.is>\n"), Category::Promotions);
        assert_eq!(c("Subject: New login\nFrom: no-reply@x.is\n"), Category::Updates);
        assert_eq!(c("Subject: Weekly digest\nFrom: team@x.is\nList-Id: <t.x.is>\n"), Category::Updates);
        assert_eq!(c("Subject: Re: lunch\nFrom: bob@y.is\nIn-Reply-To: <1@y.is>\n"), Category::Primary);
        assert_eq!(c("Subject: hello\nFrom: bob@y.is\n"), Category::Primary);
    }

    #[test]
    fn body_find() {
        assert_eq!(find_in_body("Hello hello HELLO", "hello"), vec![(0, 5), (6, 11), (12, 17)]);
        assert!(find_in_body("abc", "  ").is_empty());
        assert!(find_in_body("abc", "x").is_empty());
    }
}

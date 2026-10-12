//! Endpoint security events as a source: each (kind, phase) pair the core model defines, as log rows, so the table,
//! the search and the detail pane show them like any other message. The kernel feed is not wired yet
//! (`bifrost_core::security_event` says so), so no event happens here: a row says whether the subscription wants
//! that event. Filtering is the search: `category:Socket` keeps one category, and `subscribed` or `not` finds the
//! events the subscription holds or lacks. The rows carry no date, since nothing has happened.

use crate::model::{Level, LogEntry};
use crate::source::Source;
use bifrost_core::security_event::{Category, Event, Kind, Phase, Subscription};

/// The process and subsystem the rows name (the rows are not written by a process).
pub const PROCESS: &str = "endpoint-security";
pub const SUBSYSTEM: &str = "is.olibuijr.endpoint-security";
/// The footer note while the source is shown.
pub const NOTE: &str = "No event feed yet. These rows are the events BifrOSt can subscribe to.";

/// The name of a category, as the Category column shows it.
pub fn category_label(category: Category) -> &'static str {
    match category {
        Category::Socket => "Socket",
        Category::Pseudoterminal => "Pseudoterminal",
        Category::Clock => "Clock",
        Category::Kernel => "Kernel",
    }
}

/// The name of an event kind.
pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::UnixSocketBind => "Unix socket bind",
        Kind::UnixSocketConnect => "Unix socket connect",
        Kind::SocketSuspendResume => "Socket suspend and resume",
        Kind::PtyGrant => "Pseudoterminal grant",
        Kind::PtyClose => "Pseudoterminal close",
        Kind::SetTime => "Set time",
        Kind::KextLoad => "Kernel extension load",
        Kind::KextUnload => "Kernel extension unload",
    }
}

/// The name of a phase.
pub fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Authorize => "Authorize",
        Phase::Notify => "Notify",
    }
}

/// Every valid (kind, phase) pair, in declaration order.
pub fn events() -> Vec<Event> {
    Kind::ALL.iter().flat_map(|&kind| kind.phases().iter().map(move |&phase| Event { kind, phase })).collect()
}

/// The subscription the Console opens with: every category, so every event is wanted.
pub fn default_subscription() -> Subscription {
    let mut sub = Subscription::new();
    for category in [Category::Socket, Category::Pseudoterminal, Category::Clock, Category::Kernel] {
        sub.subscribe_category(category);
    }
    sub
}

/// The message of one row: the kind, the phase and whether the subscription wants it.
fn message(event: Event, sub: &Subscription) -> String {
    let want = if sub.wants(event) { "subscribed" } else { "not subscribed" };
    format!("{} ({}): {}", kind_label(event.kind), phase_label(event.phase), want)
}

/// One row per event: a waiting event (Authorize) is Default, a reported one (Notify) is Info.
fn row(event: Event, sub: &Subscription) -> LogEntry {
    LogEntry {
        date: 0,
        process: PROCESS.into(),
        pid: 0,
        subsystem: SUBSYSTEM.into(),
        category: category_label(event.kind.category()).into(),
        level: if event.needs_verdict() { Level::Notice } else { Level::Info },
        message: message(event, sub),
    }
}

/// The rows for every event, in declaration order, against one subscription.
pub fn rows(sub: &Subscription) -> Vec<LogEntry> {
    events().into_iter().map(|event| row(event, sub)).collect()
}

/// The detail text for one event: whether the source waits for a verdict, its category and the subscription.
pub fn explain(event: Event, sub: &Subscription) -> String {
    let verdict = if event.needs_verdict() {
        "Waits for a verdict before the action proceeds."
    } else {
        "Reports the event after it happens. It cannot be refused."
    };
    let want = if sub.wants(event) { "subscribed" } else { "not subscribed" };
    format!("{verdict}\nCategory: {}\nSubscription: {want}", category_label(event.kind.category()))
}

/// The security rows as a source. The newest `limit` rows are kept, as the other sources keep the newest entries.
pub struct Security;

impl Source for Security {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        let mut all = rows(&default_subscription());
        if all.len() > limit {
            all.drain(..all.len() - limit);
        }
        Ok(all)
    }

    fn detail(&mut self, e: &LogEntry) -> Option<String> {
        let sub = default_subscription();
        events().into_iter().find(|&event| message(event, &sub) == e.message).map(|event| explain(event, &sub))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(kind: Kind, phase: Phase) -> Event {
        Event { kind, phase }
    }

    #[test]
    fn there_are_thirteen_valid_events() {
        // bind 2, connect 2, suspend-resume 2, grant 1, close 1, set time 2, load 2, unload 1
        assert_eq!(events().len(), 13);
        assert!(events().iter().all(|e| e.exists()));
    }

    #[test]
    fn the_default_subscription_wants_every_row() {
        let sub = default_subscription();
        assert_eq!(sub.len(), 13);
        let all = rows(&sub);
        assert_eq!(all.len(), 13);
        assert!(all.iter().all(|r| r.message.contains(": subscribed")));
        assert!(all.iter().all(|r| !r.message.contains("not subscribed")));
    }

    #[test]
    fn a_partial_subscription_marks_the_rows_it_lacks() {
        let mut sub = Subscription::new();
        sub.subscribe(Kind::SetTime, Phase::Authorize).unwrap();
        let all = rows(&sub);
        let wanted: Vec<&LogEntry> = all.iter().filter(|r| r.message.contains(": subscribed")).collect();
        assert_eq!(wanted.len(), 1);
        assert_eq!(wanted[0].message, "Set time (Authorize): subscribed");
        assert_eq!(all.iter().filter(|r| r.message.ends_with("not subscribed")).count(), 12);
    }

    #[test]
    fn each_row_takes_its_category_and_a_level_from_its_phase() {
        let all = rows(&default_subscription());
        let grant = all.iter().find(|r| r.message.starts_with("Pseudoterminal grant")).unwrap();
        assert_eq!(grant.category, "Pseudoterminal");
        assert_eq!(grant.level, Level::Info, "a notify-only event is Info");
        let bind = all.iter().find(|r| r.message == "Unix socket bind (Authorize): subscribed").unwrap();
        assert_eq!(bind.category, "Socket");
        assert_eq!(bind.level, Level::Notice, "an event that waits for a verdict is Default");
        assert_eq!(bind.subsystem, SUBSYSTEM);
        assert_eq!(bind.date, 0);
    }

    #[test]
    fn the_detail_says_whether_the_event_waits_for_a_verdict() {
        let sub = default_subscription();
        let text = explain(pair(Kind::SetTime, Phase::Authorize), &sub);
        assert!(text.starts_with("Waits for a verdict"), "{text}");
        assert!(text.contains("Category: Clock"), "{text}");
        let text = explain(pair(Kind::KextUnload, Phase::Notify), &sub);
        assert!(text.starts_with("Reports the event after it happens"), "{text}");
        assert!(text.ends_with("Subscription: subscribed"), "{text}");
    }

    #[test]
    fn the_source_keeps_the_newest_rows_within_the_limit() {
        let mut source = Security;
        let all = rows(&default_subscription());
        assert_eq!(source.load(5).unwrap(), all[8..].to_vec());
        assert_eq!(source.load(100).unwrap(), all);
        let first = all[0].clone();
        assert!(source.detail(&first).unwrap().contains("Waits for a verdict"));
        let other = LogEntry { message: "no such row".into(), ..LogEntry::default() };
        assert!(source.detail(&other).is_none());
    }
}

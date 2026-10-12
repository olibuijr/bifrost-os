//! The window's log-position, walk, subsystem and signpost behaviour, tested on the window's plain state (no kit `Ui`
//! is needed for these: `rebuild`, the Log menu actions and the footer are state, and the drawing reads them).

use crate::console::{Console, View};
use crate::model::{Level, LogEntry};

fn msg(date: i64, subsystem: &str, message: &str) -> LogEntry {
    LogEntry { date, process: "notes".into(), pid: 1, subsystem: subsystem.into(), category: "sync".into(), level: Level::Notice, message: message.into() }
}

fn console_with(entries: Vec<LogEntry>) -> Console {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    for e in entries {
        c.buffer.push(e);
    }
    c.rebuild();
    c
}

#[test]
fn show_messages_after_selected_keeps_only_the_newer_messages_and_toggles_back() {
    let mut c = console_with(vec![msg(10, "a.b", "one"), msg(20, "a.b", "two"), msg(30, "a.b", "three")]);
    c.selected = Some(0);
    c.toggle_after();
    assert_eq!(c.visible, vec![1, 2]);
    assert!(c.after.is_some());
    c.toggle_after();
    assert_eq!(c.visible, vec![0, 1, 2], "Show All Messages returns to the whole list");
    assert!(c.after.is_none());
}

#[test]
fn show_messages_after_with_nothing_selected_changes_nothing() {
    let mut c = console_with(vec![msg(10, "a.b", "one"), msg(20, "a.b", "two")]);
    c.toggle_after();
    assert!(c.after.is_none());
    assert_eq!(c.visible, vec![0, 1]);
}

#[test]
fn the_signposts_view_lists_only_signposts_and_totals_their_durations() {
    let mut c = console_with(vec![
        msg(10, "a.b", "shell ready"),
        msg(20, "a.b", "interval view.load id=9 40 us"),
        msg(30, "a.b", "interval view.load id=10 60 us"),
        msg(40, "a.b", "event tap id=3"),
    ]);
    c.view = View::Signposts;
    c.rebuild();
    assert_eq!(c.visible, vec![1, 2, 3]);
    let footer = c.footer_text();
    assert!(footer.contains("view.load 2x"), "{footer}");
    assert!(footer.contains("100 us total"), "{footer}");
    assert!(footer.contains("longest 60 us"), "{footer}");
}

#[test]
fn show_only_this_subsystem_adds_a_subsystem_token_to_the_search() {
    let mut c = console_with(vec![msg(10, "is.olibuijr.notes", "one"), msg(20, "is.olibuijr.books", "two")]);
    c.selected = Some(1);
    c.show_subsystem();
    assert_eq!(c.search.to_string(), "subsystem:is.olibuijr.books");
    assert_eq!(c.visible, vec![1]);
    c.selected = Some(0);
    c.show_subsystem();
    assert_eq!(c.search.to_string(), "subsystem:is.olibuijr.books subsystem:is.olibuijr.notes");
    assert!(c.visible.is_empty(), "two subsystems never match one message");
}

#[test]
fn show_only_this_subsystem_does_nothing_for_a_message_without_one() {
    let mut c = console_with(vec![msg(10, "", "claimed")]);
    c.selected = Some(0);
    c.show_subsystem();
    assert!(c.search.to_string().is_empty());
    assert_eq!(c.visible, vec![0]);
}

#[test]
fn the_security_events_view_lists_every_event_and_the_search_filters_by_category() {
    let mut c = Console::new(1200.0, 780.0, Vec::new());
    c.open_view(View::Security);
    assert_eq!(c.visible.len(), 5, "Info is off by default, so only the five Authorize rows show");
    c.levels.info = true;
    c.rebuild();
    assert_eq!(c.visible.len(), 13, "one row per valid (kind, phase) pair");
    assert_eq!(c.footer_text(), "13 messages");
    assert_eq!(c.status, crate::security::NOTE);
    c.focus_search();
    assert!(c.text("category:Socket"));
    assert_eq!(c.visible.len(), 6, "bind, connect and suspend-resume, each with two phases");
    assert_eq!(c.footer_text(), "6 of 13 messages");
}

//! Headless tests for the notification token table (`bifrost_core::notify_center`) and for the logging model
//! (`bifrost_core::log`: levels, privacy redaction and the journal wire format). They run on the host with no shell,
//! no display and no clock: each assertion reads the state the table or the encoder keeps.

use bifrost_core::log::{encode_datagram, private, redact, Level};
use bifrost_core::notify_center::{NotifyCenter, MAX_QUEUED_SIGNALS, MAX_SIGNAL, SYSTEM_EVENTS};

#[test]
fn log_levels_map_to_syslog_priorities_and_back() {
    assert_eq!(Level::Debug.priority(), 7);
    assert_eq!(Level::Info.priority(), 6);
    assert_eq!(Level::Notice.priority(), 5);
    assert_eq!(Level::Error.priority(), 3);
    assert_eq!(Level::Fault.priority(), 2);
    assert_eq!(Level::from_priority(3), Level::Error);
    assert_eq!(Level::from_priority(4), Level::Notice);
    assert_eq!(Level::from_priority(0), Level::Fault);
    assert_eq!(Level::from_priority(9), Level::Debug);
    assert!(Level::Debug < Level::Fault, "the levels are ordered by severity");
}

#[test]
fn log_level_names_parse_case_insensitively_and_default_is_notice() {
    assert_eq!(Level::Error.name(), "error");
    assert_eq!(Level::parse(" FAULT "), Some(Level::Fault));
    assert_eq!(Level::parse("default"), Some(Level::Notice));
    assert_eq!(Level::parse("verbose"), None);
}

#[test]
fn private_values_are_redacted_unless_the_subsystem_shows_them() {
    let msg = format!("opened {}", private("/home/u/notes.txt"));
    assert_eq!(redact(&msg, false), "opened <private>");
    assert_eq!(redact(&msg, true), "opened /home/u/notes.txt");
    assert_eq!(redact("plain text", false), "plain text", "unmarked text is never redacted");
    assert!(!redact(&msg, true).contains('\u{E000}'), "the markers never survive");
}

#[test]
fn a_journal_datagram_writes_key_value_lines() {
    assert_eq!(encode_datagram(&[("MESSAGE", "hi"), ("PRIORITY", "6")]), b"MESSAGE=hi\nPRIORITY=6\n".to_vec());
}

#[test]
fn a_multi_line_journal_value_is_length_prefixed() {
    let mut want = b"MESSAGE\n".to_vec();
    want.extend_from_slice(&3u64.to_le_bytes());
    want.extend_from_slice(b"a\nb\n");
    assert_eq!(encode_datagram(&[("MESSAGE", "a\nb")]), want);
}

#[test]
fn post_marks_every_live_token_on_the_name_and_counts_them() {
    let mut c = NotifyCenter::new();
    let a = c.register("com.example.a");
    let b = c.register("com.example.a");
    let other = c.register("com.example.b");
    assert_ne!(a, 0, "zero is never handed out");
    assert_eq!(c.post("com.example.a"), 2);
    assert!(c.check(a));
    assert!(c.check(b));
    assert!(!c.check(other));
    assert_eq!(c.post("com.example.none"), 0);
}

#[test]
fn check_clears_the_pending_mark() {
    let mut c = NotifyCenter::new();
    let t = c.register("n");
    assert!(!c.check(t), "a fresh token has no mark");
    c.post("n");
    c.post("n");
    assert!(c.check(t), "two posts leave one mark");
    assert!(!c.check(t), "the mark is cleared by the first check");
}

#[test]
fn cancelled_tokens_stop_receiving_posts() {
    let mut c = NotifyCenter::new();
    let t = c.register("n");
    assert!(c.is_valid(t));
    assert!(c.cancel(t));
    assert!(!c.is_valid(t));
    assert!(!c.cancel(t), "a second cancel reports nothing removed");
    assert_eq!(c.post("n"), 0);
    assert!(!c.check(t));
}

#[test]
fn suspended_tokens_miss_posts_and_resume_restarts_delivery() {
    let mut c = NotifyCenter::new();
    let t = c.register("n");
    assert!(c.suspend(t));
    assert_eq!(c.post("n"), 0, "a suspended token is skipped");
    assert!(!c.check(t), "posts while suspended are not kept");
    assert!(c.resume(t));
    assert_eq!(c.post("n"), 1);
    assert!(c.check(t));
    assert!(!c.suspend(999), "an unknown token cannot be suspended");
}

#[test]
fn signal_tokens_queue_their_signal_once_per_post_in_order() {
    let mut c = NotifyCenter::new();
    let s = c.register_signal("refresh", 10).expect("10 is in range");
    let u = c.register_signal("refresh", 12).expect("12 is in range");
    assert!(c.is_valid(s) && c.is_valid(u));
    assert_eq!(c.post("refresh"), 2);
    assert_eq!(c.post("refresh"), 2);
    assert_eq!(c.take_signals(), vec![10, 12, 10, 12]);
    assert!(c.take_signals().is_empty(), "taking clears the queue");
}

#[test]
fn signal_numbers_must_be_between_one_and_the_maximum() {
    let mut c = NotifyCenter::new();
    assert!(c.register_signal("n", 0).is_err());
    assert!(c.register_signal("n", -1).is_err());
    assert!(c.register_signal("n", MAX_SIGNAL + 1).is_err());
    assert!(c.register_signal("n", MAX_SIGNAL).is_ok());
    assert!(c.register_signal("n", 1).is_ok());
}

#[test]
fn an_unread_signal_queue_is_capped_and_keeps_the_newest_signals() {
    let mut c = NotifyCenter::new();
    c.register_signal("burst", 7).unwrap();
    let posts = MAX_QUEUED_SIGNALS + 5;
    for _ in 0..posts {
        assert_eq!(c.post("burst"), 1);
    }
    let queued = c.take_signals();
    assert_eq!(queued.len(), MAX_QUEUED_SIGNALS);
    assert!(queued.iter().all(|&s| s == 7));
}

#[test]
fn a_poll_token_does_not_queue_signals() {
    let mut c = NotifyCenter::new();
    let t = c.register("quiet");
    c.post("quiet");
    assert!(c.take_signals().is_empty());
    assert!(c.check(t));
}

#[test]
fn name_state_defaults_to_zero_and_keeps_the_last_value_set() {
    let mut c = NotifyCenter::new();
    assert_eq!(c.state("counter"), 0);
    c.set_state("counter", 42);
    assert_eq!(c.state("counter"), 42);
    c.set_state("counter", u64::MAX);
    assert_eq!(c.state("counter"), u64::MAX, "the full 64-bit range is stored");
    assert_eq!(c.state("other"), 0);
}

#[test]
fn system_event_names_are_the_system_namespace_and_include_the_clock_and_volume_events() {
    assert_eq!(SYSTEM_EVENTS.len(), 11);
    assert!(SYSTEM_EVENTS.iter().all(|n| n.starts_with("system.")));
    assert!(SYSTEM_EVENTS.contains(&"system.clock.set"));
    assert!(SYSTEM_EVENTS.contains(&"system.volume.mount"));
    assert!(SYSTEM_EVENTS.contains(&"system.volume.unmount"));
}

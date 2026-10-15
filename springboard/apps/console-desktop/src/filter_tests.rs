//! Parity sdk-complete-04: the Console filter path from parsed records to the rows the window shows. Journal lines
//! and coredump rows are parsed, kept in a buffer, and filtered with the search and the toolbar toggles, as the
//! window does. No system log is read.

use crate::model::{self, Buffer, Levels, LogEntry, Query};

const CRASHES: &str = r#"[{"time":1760100000000000,"pid":77,"sig":11,"corefile":"present","exe":"/usr/bin/bifrost-notes"},{"time":1760100001000000,"pid":78,"sig":15,"corefile":"missing","exe":"/usr/bin/bifrost-calendar"}]"#;

fn registry() -> Vec<bifrost_core::log::Spec> {
    bifrost_core::log::registry_from_manifests(&["id = \"is.o.notes.desktop\"\n[[log.subsystem]]\nid = \"is.o.notes\"\n"])
}

/// A journal line written by a process at `exe`, with the given journald `PRIORITY`.
fn journal(exe: &str, priority: &str, subsystem: &str, message: &str) -> String {
    format!(
        r#"{{"__REALTIME_TIMESTAMP":"1760100000000000","_PID":"9","_EXE":"{exe}","PRIORITY":"{priority}","SYSLOG_IDENTIFIER":"notes","BIFROST_SUBSYSTEM":"{subsystem}","BIFROST_CATEGORY":"sync","MESSAGE":"{message}"}}"#
    )
}

const OWNED: &str = "/var/lib/springboard/apps/is.o.notes.desktop/1/n";

fn parsed(line: &str) -> LogEntry {
    model::parse_journal_line_in(line, &registry()).expect("a journal object")
}

/// The messages of the entries the search keeps, oldest first.
fn matching(buf: &Buffer, search: &str) -> Vec<String> {
    let q = Query::parse(search);
    buf.iter().filter(|(_, e)| q.matches(e)).map(|(_, e)| e.message.clone()).collect()
}

#[test]
fn a_search_over_journal_lines_keeps_only_the_entries_that_match_every_token() {
    let mut buf = Buffer::new(10);
    buf.push(parsed(&journal(OWNED, "3", "is.o.notes", "sync failed: timeout")));
    buf.push(parsed(&journal(OWNED, "7", "is.o.notes", "sync cache warm")));
    buf.push(parsed(&journal(OWNED, "6", "is.o.notes", "sync done")));
    assert_eq!(matching(&buf, "type:error"), vec!["sync failed: timeout"]);
    assert_eq!(matching(&buf, "subsystem:is.o.notes message:done"), vec!["sync done"]);
    assert_eq!(matching(&buf, "process:notes timeout"), vec!["sync failed: timeout"]);
    assert!(matching(&buf, "process:calendar").is_empty());
}

#[test]
fn the_toolbar_toggles_hide_info_and_debug_entries_from_a_search_that_matches_them() {
    let mut buf = Buffer::new(10);
    buf.push(parsed(&journal(OWNED, "3", "is.o.notes", "sync failed")));
    buf.push(parsed(&journal(OWNED, "7", "is.o.notes", "sync cache warm")));
    buf.push(parsed(&journal(OWNED, "6", "is.o.notes", "sync done")));
    let shown = |levels: Levels| -> Vec<String> {
        let q = Query::parse("sync");
        buf.iter().filter(|(_, e)| levels.shows(e.level) && q.matches(e)).map(|(_, e)| e.message.clone()).collect()
    };
    assert_eq!(shown(Levels { info: false, debug: false }), vec!["sync failed"]);
    assert_eq!(shown(Levels { info: true, debug: false }), vec!["sync failed", "sync done"]);
    assert_eq!(shown(Levels { info: true, debug: true }), vec!["sync failed", "sync cache warm", "sync done"]);
}

#[test]
fn a_claimed_subsystem_is_not_found_by_a_subsystem_search() {
    let mut buf = Buffer::new(10);
    buf.push(parsed(&journal("/var/lib/springboard/apps/is.o.evil/1/e", "3", "is.o.notes", "forged entry")));
    assert!(matching(&buf, "subsystem:is.o.notes").is_empty(), "the subsystem was cleared on read");
    assert_eq!(matching(&buf, "message:claimed"), vec!["[claimed subsystem is.o.notes] forged entry"]);
}

#[test]
fn coredump_rows_filter_by_type_process_and_subsystem() {
    let mut buf = Buffer::new(10);
    for row in model::parse_coredump_list(CRASHES).expect("a coredump list") {
        buf.push(row);
    }
    assert_eq!(matching(&buf, "type:fault"), vec!["/usr/bin/bifrost-notes terminated by SIGSEGV"]);
    assert_eq!(matching(&buf, "process:calendar"), vec!["/usr/bin/bifrost-calendar terminated by SIGTERM"]);
    assert_eq!(matching(&buf, "subsystem:systemd-coredump type:error").len(), 1);
    assert_eq!(matching(&buf, "category:present"), vec!["/usr/bin/bifrost-notes terminated by SIGSEGV"]);
}

#[test]
fn a_search_over_a_full_buffer_reports_the_sequence_numbers_still_held() {
    let mut buf = Buffer::new(2);
    for message in ["sync a", "sync b", "sync c"] {
        buf.push(parsed(&journal(OWNED, "6", "is.o.notes", message)));
    }
    let q = Query::parse("sync");
    let held: Vec<u64> = buf.iter().filter(|(_, e)| q.matches(e)).map(|(seq, _)| seq).collect();
    assert_eq!(held, vec![1, 2], "the oldest entry left the buffer");
    assert_eq!(buf.get(2).map(|e| e.message.as_str()), Some("sync c"));
    assert!(buf.get(0).is_none());
}

//! The model behind a Versions browser: the list of a document's saved versions (from [`RevisionStore`]), the one
//! selected, what changed from the current text, a preview and the Restore command. A window draws it with the list
//! and toolbar kits; nothing here draws or touches files.
//!
//! - Rows are newest first, each with a date label ("Just now", "12 minutes ago", "Today at 14:03", "Yesterday at 09:30",
//!   "Mar 4 at 16:20", "Mar 4, 2025 at 16:20") computed from a caller-supplied clock and UTC offset, so the output does not
//!   depend on this machine's time zone.
//! - [`VersionsBrowser::summary`] says how the selected version differs from the current content: added and removed lines
//!   (a longest-common-subsequence line diff, bounded so a huge file cannot stall the window; past the bound it counts
//!   lines present on only one side) or "Binary" for content that is not UTF-8.
//! - [`VersionsBrowser::restore`] returns the bytes to write back and records them as the newest revision, as
//!   [`RevisionStore::restore`] does, so the version that was replaced stays in the list.
//!
//! Gaps: the shell has no Versions window yet (the document controller exposes this model through
//! `DocumentController::open_versions`), and a version cannot be named or locked.

use crate::calendar_system::civil_from_days;
use crate::revisions::{Revision, RevisionStore};
use std::collections::HashMap;

/// Lines compared with the exact diff; a larger file is summarised by line counts only.
const DIFF_LINE_LIMIT: usize = 1500;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// How one version differs from the current content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub added: usize,
    pub removed: usize,
    /// either side is not UTF-8 text, so lines are not compared
    pub binary: bool,
}

impl Change {
    /// The version holds exactly the current content.
    pub fn is_same(&self) -> bool {
        !self.binary && self.added == 0 && self.removed == 0
    }
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionRow {
    pub seq: u64,
    pub saved_at: i64,
    /// the date text shown
    pub label: String,
    pub size: usize,
    /// what restoring this version would change, counted from this version to the current content
    pub change: Change,
    /// the version holds exactly the current content
    pub is_current: bool,
}

/// The selected version's content, shaped for a preview pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Text { lines: Vec<String>, truncated: bool },
    Binary { bytes: usize },
}

/// The Versions browser state for one document.
#[derive(Clone, Debug)]
pub struct VersionsBrowser {
    doc: String,
    rows: Vec<VersionRow>,
    selected: Option<usize>,
    contents: HashMap<u64, Vec<u8>>,
}

fn two(n: i64) -> String {
    format!("{n:02}")
}

/// The date text of a version saved at `saved_at`, seen at `now`; both are Unix seconds, `utc_offset` the viewer's
/// offset from UTC in seconds.
pub fn date_label(saved_at: i64, now: i64, utc_offset: i64) -> String {
    let age = now - saved_at;
    if age < 60 {
        return "Just now".into();
    }
    if age < 3600 {
        let m = age / 60;
        return if m == 1 { "1 minute ago".into() } else { format!("{m} minutes ago") };
    }
    let local = saved_at + utc_offset;
    let (day, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let today = (now + utc_offset).div_euclid(86_400);
    let clock = format!("{}:{}", two(secs / 3600), two(secs % 3600 / 60));
    if day == today {
        return format!("Today at {clock}");
    }
    if day == today - 1 {
        return format!("Yesterday at {clock}");
    }
    let (y, m, d) = civil_from_days(day);
    let (ty, _, _) = civil_from_days(today);
    let month = MONTHS[(m as usize).clamp(1, 12) - 1];
    if y == ty {
        format!("{month} {d} at {clock}")
    } else {
        format!("{month} {d}, {y} at {clock}")
    }
}

fn lines_of(bytes: &[u8]) -> Option<Vec<&str>> {
    std::str::from_utf8(bytes).ok().map(|t| t.lines().collect())
}

/// Added and removed lines going from `from` to `to`.
pub fn line_change(from: &[u8], to: &[u8]) -> Change {
    let (Some(a), Some(b)) = (lines_of(from), lines_of(to)) else {
        return Change { binary: from != to, ..Change::default() };
    };
    // trim the common head and tail first: most edits touch a small part of the file
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let common = if a.len() <= DIFF_LINE_LIMIT && b.len() <= DIFF_LINE_LIMIT { lcs_len(a, b) } else { shared_lines(a, b) };
    Change { added: b.len() - common, removed: a.len() - common, binary: false }
}

fn lcs_len(a: &[&str], b: &[&str]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { cur[j].max(prev[j + 1]) };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Lines that appear on both sides, counted with multiplicity; the fallback for files too large for the exact diff.
fn shared_lines(a: &[&str], b: &[&str]) -> usize {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for l in a {
        *counts.entry(l).or_default() += 1;
    }
    let mut shared = 0;
    for l in b {
        if let Some(c) = counts.get_mut(l) {
            if *c > 0 {
                *c -= 1;
                shared += 1;
            }
        }
    }
    shared
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {one}s")
    }
}

impl VersionsBrowser {
    /// The browser for document `doc` of `store`, comparing every version with `current` (the content in the window).
    /// The newest version that differs from the current content starts selected, or the newest when none differs.
    pub fn open(store: &RevisionStore, doc: &str, current: &[u8], now: i64, utc_offset: i64) -> VersionsBrowser {
        let list: Vec<&Revision> = store.list(doc);
        let rows: Vec<VersionRow> = list
            .iter()
            .map(|r| {
                let change = line_change(&r.bytes, current);
                VersionRow { seq: r.seq, saved_at: r.saved_at, label: date_label(r.saved_at, now, utc_offset), size: r.bytes.len(), is_current: change.is_same(), change }
            })
            .collect();
        let selected = rows.iter().position(|r| !r.is_current).or(if rows.is_empty() { None } else { Some(0) });
        let contents = list.iter().map(|r| (r.seq, r.bytes.clone())).collect();
        VersionsBrowser { doc: doc.to_string(), rows, selected, contents }
    }

    pub fn rows(&self) -> &[VersionRow] {
        &self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn selected(&self) -> Option<&VersionRow> {
        self.selected.map(|i| &self.rows[i])
    }

    /// Selects the version with this sequence number; `false` when the list has none.
    pub fn select(&mut self, seq: u64) -> bool {
        match self.rows.iter().position(|r| r.seq == seq) {
            Some(i) => {
                self.selected = Some(i);
                true
            }
            None => false,
        }
    }

    /// Down arrow: the next older version. Stays on the oldest.
    pub fn select_older(&mut self) {
        if let Some(i) = self.selected {
            self.selected = Some((i + 1).min(self.rows.len() - 1));
        }
    }

    /// Up arrow: the next newer version. Stays on the newest.
    pub fn select_newer(&mut self) {
        if let Some(i) = self.selected {
            self.selected = Some(i.saturating_sub(1));
        }
    }

    /// The selected version's content for the preview pane, at most `max_lines` lines.
    pub fn preview(&self, max_lines: usize) -> Option<Preview> {
        let row = self.selected()?;
        let bytes = self.contents.get(&row.seq)?;
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                let all: Vec<&str> = text.lines().collect();
                Some(Preview::Text { lines: all.iter().take(max_lines).map(|l| l.to_string()).collect(), truncated: all.len() > max_lines })
            }
            Err(_) => Some(Preview::Binary { bytes: bytes.len() }),
        }
    }

    /// One line saying what the selected version differs by, for the caption under the preview.
    pub fn summary(&self) -> Option<String> {
        let row = self.selected()?;
        Some(if row.change.binary {
            format!("Binary content, {} bytes", row.size)
        } else if row.is_current {
            "Same as the current version".to_string()
        } else {
            format!("Restoring adds {} and removes {}", plural(row.change.removed, "line"), plural(row.change.added, "line"))
        })
    }

    /// Whether the Restore button is enabled: a version is selected and it differs from the current content.
    pub fn can_restore(&self) -> bool {
        self.selected().is_some_and(|r| !r.is_current)
    }

    /// Restore: the bytes of the selected version to write back. The store records them as the newest revision.
    /// `None` when nothing is selected or the selected version is the current content.
    pub fn restore(&self, store: &mut RevisionStore, now: i64) -> Option<Vec<u8>> {
        if !self.can_restore() {
            return None;
        }
        store.restore(&self.doc, self.selected()?.seq, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2025-03-04 12:00:00 UTC
    const NOON: i64 = 1_741_089_600;

    fn store() -> RevisionStore {
        let mut s = RevisionStore::default();
        s.record("d", NOON - 3 * 86_400, b"one\ntwo\n");
        s.record("d", NOON - 86_400 - 3600, b"one\ntwo\nthree\n");
        s.record("d", NOON - 120, b"one\n2\nthree\nfour\n");
        s
    }

    #[test]
    fn dates_read_as_a_person_would_say_them() {
        let now = NOON;
        assert_eq!(date_label(now - 10, now, 0), "Just now");
        assert_eq!(date_label(now - 60, now, 0), "1 minute ago");
        assert_eq!(date_label(now - 12 * 60, now, 0), "12 minutes ago");
        assert_eq!(date_label(now - 2 * 3600, now, 0), "Today at 10:00");
        assert_eq!(date_label(now - 20 * 3600, now, 0), "Yesterday at 16:00");
        assert_eq!(date_label(now - 3 * 86_400, now, 0), "Mar 1 at 12:00");
        assert_eq!(date_label(now - 400 * 86_400, now, 0), "Jan 29, 2024 at 12:00");
    }

    #[test]
    fn the_utc_offset_moves_the_day_boundary() {
        // 23:30 UTC on Mar 3 is 00:30 on Mar 4 at +1h: today for a viewer at +1h, yesterday for one at UTC
        let saved = NOON - 12 * 3600 - 1800;
        assert_eq!(date_label(saved, NOON, 3600), "Today at 00:30");
        assert_eq!(date_label(saved, NOON, 0), "Yesterday at 23:30");
    }

    #[test]
    fn line_changes_count_added_and_removed_lines() {
        assert_eq!(line_change(b"a\nb\nc\n", b"a\nb\nc\n"), Change::default());
        assert_eq!(line_change(b"a\nb\nc\n", b"a\nx\nc\nd\n"), Change { added: 2, removed: 1, binary: false });
        assert_eq!(line_change(b"", b"a\nb\n"), Change { added: 2, removed: 0, binary: false });
        assert_eq!(line_change(b"a\nb\n", b""), Change { added: 0, removed: 2, binary: false });
        assert!(line_change(&[0xff, 0xfe], b"a").binary);
        assert!(line_change(&[0xff], &[0xff]).is_same());
        // a moved line is one removal and one addition
        assert_eq!(line_change(b"a\nb\nc\n", b"b\nc\na\n"), Change { added: 1, removed: 1, binary: false });
    }

    #[test]
    fn a_very_large_file_uses_the_bounded_count() {
        let a: String = (0..3000).map(|i| format!("line {i}\n")).collect();
        let mut b = a.clone();
        b.push_str("extra\n");
        b = b.replacen("line 10\n", "changed\n", 1);
        let c = line_change(a.as_bytes(), b.as_bytes());
        assert_eq!(c, Change { added: 2, removed: 1, binary: false });
    }

    #[test]
    fn rows_are_newest_first_and_the_first_differing_one_is_selected() {
        let s = store();
        let b = VersionsBrowser::open(&s, "d", b"one\n2\nthree\nfour\n", NOON, 0);
        let labels: Vec<&str> = b.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["2 minutes ago", "Yesterday at 11:00", "Mar 1 at 12:00"]);
        assert!(b.rows()[0].is_current);
        assert_eq!(b.selected().unwrap().seq, b.rows()[1].seq, "the newest version equal to the content is skipped");
        assert_eq!(b.summary().unwrap(), "Restoring adds 1 line and removes 2 lines");
        assert!(b.can_restore());
    }

    #[test]
    fn arrows_walk_the_list_and_stop_at_the_ends() {
        let s = store();
        let mut b = VersionsBrowser::open(&s, "d", b"x", NOON, 0);
        assert_eq!(b.selected().unwrap().seq, b.rows()[0].seq);
        b.select_newer();
        assert_eq!(b.selected().unwrap().seq, b.rows()[0].seq);
        b.select_older();
        b.select_older();
        b.select_older();
        assert_eq!(b.selected().unwrap().seq, b.rows()[2].seq);
        let first = b.rows()[0].seq;
        assert!(b.select(first));
        assert!(!b.select(999));
    }

    #[test]
    fn preview_shows_the_leading_lines_or_says_binary() {
        let s = store();
        let mut b = VersionsBrowser::open(&s, "d", b"x", NOON, 0);
        assert_eq!(b.preview(2), Some(Preview::Text { lines: vec!["one".into(), "2".into()], truncated: true }));
        assert_eq!(b.preview(10), Some(Preview::Text { lines: vec!["one".into(), "2".into(), "three".into(), "four".into()], truncated: false }));
        let mut s2 = RevisionStore::default();
        s2.record("p", 0, &[0xff, 0x00, 0xfe]);
        b = VersionsBrowser::open(&s2, "p", b"text", NOON, 0);
        assert_eq!(b.preview(5), Some(Preview::Binary { bytes: 3 }));
        assert_eq!(b.summary().unwrap(), "Binary content, 3 bytes");
    }

    #[test]
    fn restore_returns_the_bytes_and_keeps_the_replaced_version() {
        let mut s = store();
        let mut b = VersionsBrowser::open(&s, "d", b"one\n2\nthree\nfour\n", NOON, 0);
        let oldest = b.rows()[2].seq;
        assert!(b.select(oldest));
        let bytes = b.restore(&mut s, NOON).unwrap();
        assert_eq!(bytes, b"one\ntwo\n");
        let list = s.list("d");
        assert_eq!(list.len(), 4, "the restored bytes differ from the newest, so they are a new revision");
        assert_eq!(list[0].bytes, b"one\ntwo\n");
        assert_eq!(list[1].bytes, b"one\n2\nthree\nfour\n", "the version that was replaced is still listed");
    }

    #[test]
    fn the_current_version_cannot_be_restored_and_an_empty_store_has_nothing() {
        let mut s = store();
        let mut b = VersionsBrowser::open(&s, "d", b"one\n2\nthree\nfour\n", NOON, 0);
        let newest = b.rows()[0].seq;
        b.select(newest);
        assert!(!b.can_restore());
        assert_eq!(b.restore(&mut s, NOON), None);
        assert_eq!(b.summary().unwrap(), "Same as the current version");
        let empty = VersionsBrowser::open(&RevisionStore::default(), "none", b"", NOON, 0);
        assert!(empty.is_empty() && empty.selected().is_none() && empty.preview(3).is_none() && empty.summary().is_none());
        let mut e = empty;
        e.select_older();
        e.select_newer();
    }
}

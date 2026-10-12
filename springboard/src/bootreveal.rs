//! Reveal once (docs/boot-architecture.md, "Reveal once"): the window server holds the boot frame until the wallpaper,
//! the menu bar and the Dock have each presented their first complete frame, or until a 4 s ceiling, and then fades
//! once. Pure logic (no I/O): the caller reads the marks file and passes its text in. Shared by `bifrost-wm` (included
//! by path in `wm/src/boot.rs`) and the shell's labwc-path KMS boot fade (`bare/kms.rs`). Owner: task 09.

/// The three desktop parts whose first complete frame gates the reveal; each writes its mark when presented. The
/// wallpaper is drawn into the desktop surface (its first commit, `bare/wayland.rs`, `mark_first_frame`); the menu bar
/// and the Dock are chrome layer surfaces (ids 0 and 2), marked on their own first commit (`chrome_part`).
pub const DESKTOP_PARTS: [&str; 3] = ["wallpaper-first-frame", "menu-bar-first-frame", "dock-first-frame"];

/// The mark for a chrome layer surface's first committed frame: the menu bar (id 0) and the Dock (id 2). Other chrome
/// (menus, panels, the keyboard-exclusive surface) is not a desktop part and writes no mark.
pub fn chrome_part(id: u32) -> Option<&'static str> {
    match id {
        0 => Some("menu-bar-first-frame"),
        2 => Some("dock-first-frame"),
        _ => None,
    }
}

/// Longest the boot frame is held waiting for the desktop, from the moment the gate starts.
pub const READY_CEILING_MS: f64 = 4000.0;

/// How often the marks file is re-read while holding.
pub const MARKS_POLL_MS: f64 = 100.0;

/// Which desktop parts have presented, read from the marks file text (lines `"<ms> <event>"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Parts {
    seen: [bool; 3],
}

impl Parts {
    /// Notes every desktop-part event in `marks`; other events are ignored.
    pub fn note_marks(&mut self, marks: &str) {
        for line in marks.lines() {
            if let Some(event) = line.split_whitespace().nth(1) {
                if let Some(i) = DESKTOP_PARTS.iter().position(|p| *p == event) {
                    self.seen[i] = true;
                }
            }
        }
    }

    /// How many of the desktop parts have presented.
    pub fn count(&self) -> usize {
        self.seen.iter().filter(|s| **s).count()
    }

    pub fn complete(&self) -> bool {
        self.count() == DESKTOP_PARTS.len()
    }
}

/// The reveal gate. Holds from `new` until every desktop part has presented or the ceiling has passed, then records
/// `ready_at` once and stays ready. Times are milliseconds on one clock (the caller's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reveal {
    held_since: f64,
    last_poll: f64,
    parts: Parts,
    ready_at: Option<f64>,
}

impl Reveal {
    pub fn new(now: f64) -> Reveal {
        Reveal { held_since: now, last_poll: f64::NEG_INFINITY, parts: Parts::default(), ready_at: None }
    }

    /// Advances the gate at `now`. `marks` yields the marks file text and is called only when a re-read is due
    /// (every `MARKS_POLL_MS`). Returns `Some(ready_at)` once the desktop is ready (`desktop-ready`), `None` while held.
    pub fn poll(&mut self, now: f64, marks: impl FnOnce() -> String) -> Option<f64> {
        if self.ready_at.is_none() {
            if now - self.last_poll >= MARKS_POLL_MS {
                self.last_poll = now;
                self.parts.note_marks(&marks());
            }
            if self.parts.complete() || now - self.held_since >= READY_CEILING_MS {
                self.ready_at = Some(now);
            }
        }
        self.ready_at
    }

    pub fn ready_at(&self) -> Option<f64> {
        self.ready_at
    }

    pub fn parts(&self) -> Parts {
        self.parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ALL: &str = "10.0 wallpaper-first-frame\n11.0 menu-bar-first-frame\n12.0 dock-first-frame\n";

    #[test]
    fn one_part_is_not_enough() {
        let mut p = Parts::default();
        p.note_marks("10.0 wallpaper-first-frame\n");
        assert_eq!(p.count(), 1);
        assert!(!p.complete());
    }

    #[test]
    fn all_three_parts_complete_the_desktop() {
        let mut p = Parts::default();
        p.note_marks(ALL);
        assert_eq!(p.count(), 3);
        assert!(p.complete());
    }

    #[test]
    fn unrelated_marks_and_malformed_lines_are_ignored() {
        let mut p = Parts::default();
        p.note_marks("1.0 supervisor-start\n2.0 desktop-first-frame\nnonsense\n\nwallpaper-first-frame\n");
        assert_eq!(p.count(), 0, "only a line with the exact part name in the event column counts");
    }

    #[test]
    fn parts_accumulate_across_reads() {
        let mut p = Parts::default();
        p.note_marks("10.0 dock-first-frame\n");
        p.note_marks("11.0 menu-bar-first-frame\n");
        assert_eq!(p.count(), 2);
        p.note_marks("12.0 wallpaper-first-frame\n");
        assert!(p.complete());
    }

    #[test]
    fn the_gate_holds_until_every_part_is_marked() {
        let mut r = Reveal::new(0.0);
        assert_eq!(r.poll(0.0, String::new), None);
        assert_eq!(r.poll(100.0, || "10.0 wallpaper-first-frame\n".into()), None);
        assert_eq!(r.poll(200.0, || ALL.into()), Some(200.0), "ready at the poll that sees the last part");
    }

    #[test]
    fn ready_at_is_recorded_once_and_sticks() {
        let mut r = Reveal::new(0.0);
        assert_eq!(r.poll(200.0, || ALL.into()), Some(200.0));
        assert_eq!(r.poll(300.0, String::new), Some(200.0), "later polls keep the first time");
        assert_eq!(r.ready_at(), Some(200.0));
    }

    #[test]
    fn the_ceiling_releases_the_frame_after_4_s() {
        let mut r = Reveal::new(1000.0);
        assert_eq!(r.poll(4999.0, String::new), None, "4 s from the start is 5000");
        assert_eq!(r.poll(5000.0, String::new), Some(5000.0));
    }

    #[test]
    fn the_ceiling_counts_from_the_gate_start_not_from_boot() {
        let mut r = Reveal::new(12_000.0);
        assert_eq!(r.poll(15_999.0, String::new), None);
        assert_eq!(r.poll(16_000.0, String::new), Some(16_000.0));
    }

    #[test]
    fn the_marks_file_is_read_at_most_every_100_ms() {
        let reads = Cell::new(0);
        let mut r = Reveal::new(0.0);
        let read = || {
            reads.set(reads.get() + 1);
            String::new()
        };
        r.poll(0.0, read);
        r.poll(50.0, read);
        r.poll(99.0, read);
        assert_eq!(reads.get(), 1, "only the first poll read the file");
        r.poll(100.0, read);
        assert_eq!(reads.get(), 2, "the next read is due at 100 ms");
    }

    #[test]
    fn only_the_menu_bar_and_the_dock_are_chrome_parts() {
        assert_eq!(chrome_part(0), Some("menu-bar-first-frame"));
        assert_eq!(chrome_part(2), Some("dock-first-frame"));
        assert_eq!(chrome_part(1), None, "a menu or panel surface is not a desktop part");
        assert_eq!(chrome_part(3), None, "the keyboard-exclusive surface writes no mark");
    }

    #[test]
    fn no_reads_after_ready() {
        let reads = Cell::new(0);
        let mut r = Reveal::new(0.0);
        r.poll(0.0, || {
            reads.set(reads.get() + 1);
            ALL.into()
        });
        assert_eq!(reads.get(), 1);
        r.poll(500.0, || {
            reads.set(reads.get() + 1);
            String::new()
        });
        assert_eq!(reads.get(), 1, "a ready gate stops reading the marks");
    }
}

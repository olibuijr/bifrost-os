//! The boot milestone log: every mark is milliseconds since the kernel started (CLOCK_BOOTTIME), one clock for all
//! steps, read by `bootweave` and the boot benchmark. The boot frame itself (paint, fade, hand-off) is
//! `bootweave-frame` (docs/boot-architecture.md, "One frame").

pub const MARKS: &str = "/run/bifrost/boot-marks";

/// Milliseconds since the kernel started, including suspend (the clock systemd-analyze also counts from).
pub fn boottime_ms() -> f64 {
    let mut ts = libc_timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, writable timespec for the duration of the call; CLOCK_BOOTTIME (7) is a valid clock id.
    unsafe { clock_gettime(7 /* CLOCK_BOOTTIME */, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}
#[repr(C)]
struct libc_timespec {
    tv_sec: i64,
    tv_nsec: i64,
}
extern "C" {
    fn clock_gettime(clk: i32, ts: *mut libc_timespec) -> i32;
}

/// The milestones the boot code writes, in boot order (docs/boot-architecture.md, "Budgets"). `loader-start` and
/// `kernel-start` are not written here: the loader cannot write `/run`, and kernel start is the clock's origin.
pub const CONTRACT_MARKS: &[&str] = &[
    "bootweave-start",
    "display-handoff-start",
    "compositor-ready",
    "desktop-first-frame",
    "desktop-ready",
    "desktop-first-ui",
    "desktop-faded",
    "session-exit",
    "shutdown-start",
];

/// Event names are lowercase words joined by dashes or underscores, so they are safe as file fields.
pub fn valid_event(event: &str) -> bool {
    !event.is_empty() && event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Appends `ms event` to the boot milestone log; the first mark of an event wins (a restart does not overwrite it).
pub fn mark(event: &str) {
    use std::io::Write;
    if !valid_event(event) {
        return;
    }
    let _ = std::fs::create_dir_all("/run/bifrost");
    let seen = std::fs::read_to_string(MARKS).unwrap_or_default();
    if seen.lines().any(|l| l.split_whitespace().nth(1) == Some(event)) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(MARKS) {
        let _ = writeln!(f, "{:.1} {event}", boottime_ms());
    }
}

/// Set once the shell has committed its first desktop frame in this process (see `note_first_desktop_frame`).
static FIRST_DESKTOP_FRAME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Records that the first desktop frame was committed. True only for the call that records it, so the caller can
/// write the boot marks once.
pub fn note_first_desktop_frame() -> bool {
    !FIRST_DESKTOP_FRAME.swap(true, std::sync::atomic::Ordering::Relaxed)
}

/// Whether this process has committed its first desktop frame. Work that is not needed for the first frame (login
/// items, restored windows) waits for this.
pub fn first_desktop_frame_presented() -> bool {
    FIRST_DESKTOP_FRAME.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_marks_are_unique_valid_event_names() {
        for (i, a) in CONTRACT_MARKS.iter().enumerate() {
            assert!(valid_event(a), "{a}");
            assert!(!CONTRACT_MARKS[i + 1..].contains(a), "duplicate {a}");
        }
        assert_eq!(CONTRACT_MARKS.len(), 9);
    }
    #[test]
    fn event_names_reject_paths_spaces_and_empty_names() {
        assert!(valid_event("desktop-faded"));
        assert!(valid_event("shutdown_start"));
        assert!(!valid_event(""));
        assert!(!valid_event("a b"));
        assert!(!valid_event("../boot"));
        assert!(!valid_event("line\nbreak"));
    }
    #[test]
    fn mark_names_are_plain_words() {
        assert!(valid_event("desktop-first-ui"));
        assert!(valid_event("installer_faded"));
        assert!(!valid_event("bad name"));
        assert!(!valid_event("a/b"));
    }

    #[test]
    fn the_first_desktop_frame_is_noted_once_and_then_reported_presented() {
        // only this test touches the process-wide flag, so the order of the checks is fixed here
        let was = first_desktop_frame_presented();
        let first = note_first_desktop_frame();
        assert_eq!(first, !was);
        assert!(first_desktop_frame_presented());
        assert!(!note_first_desktop_frame());
    }
}

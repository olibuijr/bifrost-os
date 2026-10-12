//! Reveal once (docs/boot-architecture.md, "Reveal once"). The compositor owns the screen: it keeps the boot frame on
//! screen from its first scanout until the desktop is ready (the wallpaper, menu bar and Dock have each presented a
//! complete frame, or the 4 s ceiling passed), then cross-fades once (450 ms) and marks `desktop-faded`. The gate is
//! `bootreveal::Reveal` (shared with the KMS path); this file adds the fade and the marks I/O.

// the boot marks are the shell's file (the boot log is shared with the shell); the frame is bootweave-frame
#[allow(dead_code)]
#[path = "../../src/bootsplash.rs"]
mod bootsplash;

// the reveal gate, shared with the shell's KMS boot fade
#[allow(dead_code)]
#[path = "../../src/bootreveal.rs"]
pub mod bootreveal;

pub use bootreveal::Reveal;
pub use bootsplash::mark;

/// How long the boot frame takes to fade into the desktop (ms).
pub const FADE_MS: f64 = bootweave_frame::FADE_MS as f64;

/// The marks file text the gate reads (empty until the shell writes its first mark).
pub fn read_marks_file() -> String {
    std::fs::read_to_string(bootsplash::MARKS).unwrap_or_default()
}

/// The supervisor's progress record (contract C3): a `bootweave_frame::timing::Handoff` encoding, written atomically
/// whenever its bar moves.
pub const PROGRESS_FILE: &str = "/run/bifrost/bootweave/progress";

/// The bar position (0..=1) and appearance the supervisor last showed, from the progress record's bytes.
pub fn decode_progress(bytes: &[u8]) -> Option<(f32, bootweave_frame::Appearance)> {
    let h = bootweave_frame::timing::Handoff::decode(bytes)?;
    Some(((h.shown_permille as f32 / 1000.0).clamp(0.0, 1.0), h.timing.appearance))
}

/// The held boot frame: the mark alone (no bar), in the appearance the supervisor published, else Dark. `_last` is
/// kept for callers; there is no bar to hold back.
pub fn held_frame(w: u32, h: u32, record: Option<&[u8]>, _last: f32) -> bootweave_frame::Frame {
    let mut f = bootweave_frame::Frame::boot(w, h);
    if let Some((_, a)) = record.and_then(decode_progress) {
        f.appearance = a;
    }
    f
}

/// Opacity of the loader's frame over the desktop. `since_ready` is the time since the desktop became ready
/// (None until then: the frame stays fully opaque). None once the fade is over.
pub fn splash_alpha(since_ready: Option<f64>) -> Option<f32> {
    let Some(t) = since_ready else { return Some(1.0) };
    if t >= FADE_MS {
        return None;
    }
    // the same curve as the hand-off in bootweave-frame (smoothstep)
    Some(bootweave_frame::fade_weight((t / FADE_MS) as f32))
}

/// Written by `bifrost-bootweave shutdown` (`docs/boot-architecture.md`, "Symmetric shutdown") before systemd stops the
/// session. Its presence starts the shutdown fade; it lives on /run, so a reboot clears it.
pub const SHUTDOWN_REQUEST: &str = "/run/bifrost/bootweave/shutdown";
/// Written here once the boot frame covers the desktop, so the stopping supervisor can go on. It lives in the account's
/// runtime directory: the window server runs as that account, while `/run/bifrost/bootweave` belongs to root.
pub const SHUTDOWN_FADED: &str = "/run/bifrost/user/shutdown-faded";
/// How long the desktop takes to fade back to the boot frame at shutdown (the boot's own fade length).
pub const SHUTDOWN_FADE_MS: f64 = FADE_MS;

/// Opacity of the boot frame over the desktop during a shutdown. `since_request` is the time since the request was
/// first seen (None when there is no request). It eases from 0 to 1 over `SHUTDOWN_FADE_MS` and then holds at 1, so
/// the boot frame stays on screen until power-off. Separate from `splash_alpha`, which is the boot's fade-in.
pub fn shutdown_alpha(since_request: Option<f64>) -> Option<f32> {
    let t = since_request?;
    let e = (t / SHUTDOWN_FADE_MS).clamp(0.0, 1.0);
    Some((e * e * (3.0 - 2.0 * e)) as f32)
}

/// True once the shutdown fade has reached the boot frame (the moment to write `SHUTDOWN_FADED`).
pub fn shutdown_faded(since_request: Option<f64>) -> bool {
    since_request.is_some_and(|t| t >= SHUTDOWN_FADE_MS)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_held_frame_takes_the_supervisors_appearance_and_has_no_bar() {
        use bootweave_frame::{timing::{BootTiming, Handoff}, Appearance};
        let timing = BootTiming { loader_ms: 100, kernel_ms: 200, userspace_ms: 300, samples: 1, appearance: Appearance::Light };
        let rec = Handoff { timing, loader_ms: 10, shown_permille: 420 }.encode();
        let f = super::held_frame(64, 32, Some(&rec), 0.0);
        assert_eq!(f.progress, None);
        assert_eq!(f.appearance, Appearance::Light);
        // no record: the loader's frame
        assert_eq!(super::held_frame(64, 32, None, 0.0).progress, None);
        assert_eq!(super::held_frame(64, 32, Some(&[1, 2]), 0.0).appearance, Appearance::Dark);
    }

    use super::*;

    #[test]
    fn the_loader_frame_holds_until_the_shell_is_ready() {
        assert_eq!(splash_alpha(None), Some(1.0));
    }

    #[test]
    fn it_fades_with_the_loaders_easing_and_then_goes_away() {
        assert_eq!(splash_alpha(Some(0.0)), Some(1.0));
        let mid = splash_alpha(Some(FADE_MS / 2.0)).unwrap();
        assert!((mid - 0.5).abs() < 1e-3, "smoothstep is symmetric: {mid}");
        let early = splash_alpha(Some(FADE_MS / 4.0)).unwrap();
        assert!(early > 0.75, "eased: slow at the start ({early})");
        assert_eq!(splash_alpha(Some(FADE_MS)), None);
        assert_eq!(splash_alpha(Some(FADE_MS * 3.0)), None);
    }

    #[test]
    fn the_frame_is_held_until_the_last_desktop_part_then_fades_once() {
        let marks = "1.0 wallpaper-first-frame\n2.0 menu-bar-first-frame\n";
        let mut r = Reveal::new(0.0);
        assert_eq!(r.poll(100.0, || marks.into()), None, "two of three parts: still held");
        assert_eq!(splash_alpha(r.poll(100.0, || marks.into()).map(|t| 100.0 - t)), Some(1.0));
        let all = "1.0 wallpaper-first-frame\n2.0 menu-bar-first-frame\n3.0 dock-first-frame\n";
        let ready = r.poll(300.0, || all.into());
        assert_eq!(ready, Some(300.0), "ready at the poll that sees the Dock");
        assert_eq!(splash_alpha(ready.map(|t| 300.0 - t)), Some(1.0), "no fade before the first ready frame");
        let half = splash_alpha(ready.map(|t| 300.0 + FADE_MS / 2.0 - t)).unwrap();
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        assert_eq!(splash_alpha(ready.map(|t| 300.0 + FADE_MS - t)), None, "gone after one 450 ms fade");
    }

    #[test]
    fn a_stalled_desktop_is_revealed_by_the_ceiling() {
        let mut r = Reveal::new(0.0);
        assert_eq!(r.poll(3999.0, String::new), None);
        assert_eq!(r.poll(4000.0, String::new), Some(4000.0));
    }

    #[test]
    fn no_shutdown_request_means_no_shutdown_overlay() {
        assert_eq!(shutdown_alpha(None), None);
        assert!(!shutdown_faded(None));
    }

    #[test]
    fn the_shutdown_fade_starts_at_zero_and_eases_to_the_boot_frame() {
        assert_eq!(shutdown_alpha(Some(0.0)), Some(0.0));
        let mid = shutdown_alpha(Some(SHUTDOWN_FADE_MS / 2.0)).unwrap();
        assert!((mid - 0.5).abs() < 1e-3, "smoothstep is symmetric: {mid}");
        let early = shutdown_alpha(Some(SHUTDOWN_FADE_MS / 4.0)).unwrap();
        assert!(early < 0.25, "eased: slow at the start ({early})");
    }

    #[test]
    fn the_boot_frame_holds_at_full_opacity_until_power_off() {
        assert_eq!(shutdown_alpha(Some(SHUTDOWN_FADE_MS)), Some(1.0));
        assert_eq!(shutdown_alpha(Some(SHUTDOWN_FADE_MS * 10.0)), Some(1.0));
    }

    #[test]
    fn the_fade_is_reported_done_exactly_at_its_length() {
        assert!(!shutdown_faded(Some(SHUTDOWN_FADE_MS - 1.0)));
        assert!(shutdown_faded(Some(SHUTDOWN_FADE_MS)));
        assert!(shutdown_faded(Some(SHUTDOWN_FADE_MS + 100.0)));
    }

    #[test]
    fn the_request_and_the_faded_marker_live_where_their_writers_can_write() {
        // the request is written by root (bootweave), the marker by the window server's account
        assert!(SHUTDOWN_REQUEST.starts_with("/run/bifrost/bootweave/"));
        assert!(SHUTDOWN_FADED.starts_with("/run/bifrost/user/"));
        assert_ne!(SHUTDOWN_REQUEST, SHUTDOWN_FADED);
    }
}

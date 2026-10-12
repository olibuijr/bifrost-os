//! Reveal once (docs/boot-architecture.md, "Reveal once"). The compositor owns the screen: it keeps the boot frame on
//! screen from its first scanout until the desktop is ready (the wallpaper, menu bar and Dock have each presented a
//! complete frame, or the 4 s ceiling passed), then cross-fades once (450 ms) and marks `desktop-faded`. The gate is
//! `bootreveal::Reveal` (shared with the KMS path); this file adds the fade and the marks I/O.

// the shell's file, so the loader frame has one source; the CPU `fade` is the shell's, the GPU blends here
#[allow(dead_code)]
#[path = "../../src/bootsplash.rs"]
mod bootsplash;

// the reveal gate, shared with the shell's KMS boot fade
#[allow(dead_code)]
#[path = "../../src/bootreveal.rs"]
pub mod bootreveal;

pub use bootreveal::Reveal;
pub use bootsplash::{mark, paint, FADE_MS};

/// The marks file text the gate reads (empty until the shell writes its first mark).
pub fn read_marks_file() -> String {
    std::fs::read_to_string(bootsplash::MARKS).unwrap_or_default()
}

/// Opacity of the loader's frame over the desktop. `since_ready` is the time since the desktop became ready
/// (None until then: the frame stays fully opaque). None once the fade is over.
pub fn splash_alpha(since_ready: Option<f64>) -> Option<f32> {
    let Some(t) = since_ready else { return Some(1.0) };
    if t >= FADE_MS {
        return None;
    }
    // the loader's own curve (`bootsplash::fade`): smoothstep
    let e = (t / FADE_MS).clamp(0.0, 1.0);
    Some((1.0 - e * e * (3.0 - 2.0 * e)) as f32)
}

#[cfg(test)]
mod tests {
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
}

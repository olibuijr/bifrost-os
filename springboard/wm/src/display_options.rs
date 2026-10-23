//! Settings > Accessibility > Display for the window server: Reduce Transparency and Increase Contrast. The shell writes
//! them as `key=0|1` lines to `/run/bifrost/user/accessibility` (`bifrost_render::appearance::DISPLAY_FILE`, written on
//! every change); the window server reads the file again at most every `POLL` while it composes. Under Reduce
//! Transparency (which Increase Contrast implies, as on the reference desktop) every material it backs, app sidebars
//! and title bars included, is a flat opaque plate instead of a live blur.

use std::time::{Duration, Instant};

/// The file the shell writes (`BIFROST_WM_A11Y_FILE` overrides it, for tests and nested sessions).
pub const FILE: &str = "/run/bifrost/user/accessibility";
/// How often the file is read again at most.
pub const POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DisplayOptions {
    pub reduce_transparency: bool,
    pub increase_contrast: bool,
}

/// The shell's `key=0|1` lines; missing or unknown keys read as off, and Increase Contrast implies Reduce Transparency.
pub fn parse(text: &str) -> DisplayOptions {
    let mut o = DisplayOptions::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let on = v.trim() == "1";
        match k.trim() {
            "reduce_transparency" => o.reduce_transparency = on,
            "increase_contrast" => o.increase_contrast = on,
            _ => {}
        }
    }
    o.reduce_transparency |= o.increase_contrast;
    o
}

/// The options now and when the file was last read.
#[derive(Debug, Default)]
pub struct DisplayPoll {
    pub now: DisplayOptions,
    read_at: Option<Instant>,
}

impl DisplayPoll {
    /// Reads the file again when `POLL` has passed since the last read. True when the options changed (a frame is due).
    /// A missing file is all off.
    pub fn poll(&mut self, at: Instant) -> bool {
        if self.read_at.is_some_and(|r| at.duration_since(r) < POLL) {
            return false;
        }
        self.read_at = Some(at);
        let path = std::env::var("BIFROST_WM_A11Y_FILE").unwrap_or_else(|_| FILE.to_string());
        let next = std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default();
        std::mem::replace(&mut self.now, next) != next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_s_lines_parse_and_contrast_implies_reduce() {
        assert_eq!(parse(""), DisplayOptions::default());
        let o = parse("reduce_transparency=1\nincrease_contrast=0\ndifferentiate_without_color=1\n");
        assert!(o.reduce_transparency && !o.increase_contrast);
        let o = parse("reduce_transparency=0\nincrease_contrast=1\n");
        assert!(o.reduce_transparency && o.increase_contrast, "Increase Contrast turns Reduce Transparency on");
        assert_eq!(parse("garbage\nreduce_transparency=yes"), DisplayOptions::default());
    }

    #[test]
    fn the_file_is_read_at_most_every_poll_interval() {
        let dir = std::env::temp_dir().join(format!("bifrost-wm-a11y-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("accessibility");
        std::fs::write(&file, "reduce_transparency=1\n").unwrap();
        std::env::set_var("BIFROST_WM_A11Y_FILE", &file);
        let mut p = DisplayPoll::default();
        let t0 = Instant::now();
        assert!(p.poll(t0), "the first read turns it on");
        assert!(p.now.reduce_transparency);
        std::fs::write(&file, "reduce_transparency=0\n").unwrap();
        assert!(!p.poll(t0 + POLL / 2), "not read again before the interval");
        assert!(p.poll(t0 + POLL), "read again after it");
        assert!(!p.now.reduce_transparency);
        assert!(!p.poll(t0 + POLL * 3), "unchanged: no frame");
        std::env::remove_var("BIFROST_WM_A11Y_FILE");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

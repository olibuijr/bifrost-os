//! The Dock's Downloads stack: a Dock entry just left of the Trash that shows the newest files of the account's
//! Downloads folder as a fanned stack, and opens the folder in Files on a click.
//!
//! The folder is listed on a timer (every `REFRESH_S` seconds, at most `MAX_SCAN` entries read), never per frame, so a
//! large folder cannot stall drawing. Hidden files are left out.
use super::desk::{Desk, Rect};
use crate::gfx::{rgba, Surface};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The Dock list entry of the Downloads stack. Minimized windows take the entries from `DOCK_MIN_BASE` (usize::MAX - 1)
/// down to `usize::MAX - 4096`, so this one sits just below them and `dock_min_index` does not read it as a window.
pub(super) const DOCK_STACK: usize = usize::MAX - 4097;
/// How many of the newest files the stack fans out.
pub(super) const SHOWN: usize = 3;
/// Seconds between two listings of the Downloads folder.
const REFRESH_S: f64 = 10.0;
/// Entries read from the folder per listing at most.
const MAX_SCAN: usize = 2048;

/// The account's home folder: the installed account's home, else `$HOME`, else `/root` (the shell runs as root).
pub(super) fn home_dir() -> PathBuf {
    #[cfg(feature = "desktop-wayland")]
    if let Ok(account) = crate::bare::app_policy::Account::installed() {
        return account.home;
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/root".into())
}

/// The account's Downloads folder.
pub(super) fn downloads_dir() -> PathBuf {
    home_dir().join("Downloads")
}

/// The names of the newest `limit` visible entries of `dir`, newest first (ties by name). An unreadable folder is empty.
pub(super) fn recent_files(dir: &Path, limit: usize) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(SystemTime, String)> = read
        .take(MAX_SCAN)
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let when = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            Some((when, name))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().take(limit).map(|(_, name)| name).collect()
}

/// Offsets (px) of `n` fanned tiles from the front one (index 0, at the origin) to the back one: each further tile
/// steps right by `step` and up by half of it.
pub(super) fn fan_offsets(n: usize, step: i32) -> Vec<(i32, i32)> {
    (0..n as i32).map(|i| (i * step, -(i * step) / 2)).collect()
}

impl Desk {
    /// Re-lists the Downloads folder when the last listing is `REFRESH_S` old. Returns whether the listed names changed.
    pub(super) fn stack_refresh(&mut self, now: f64) -> bool {
        if self.stack_listed.is_some_and(|at| now - at < REFRESH_S) {
            return false;
        }
        self.stack_listed = Some(now);
        let next = recent_files(&downloads_dir(), SHOWN);
        if next == self.stack_files {
            return false;
        }
        self.stack_files = next;
        true
    }
}

/// Draws the Downloads stack in the Dock slot `r`: a blue folder with up to `SHOWN` pale file tiles fanned out behind
/// its front flap. An empty folder shows the flap alone.
pub(super) fn draw_stack(s: &mut Surface, r: &Rect, files: &[String]) {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    let (back, front, tile) = (rgba(88, 150, 232, 255), rgba(116, 172, 244, 255), rgba(246, 247, 250, 255));
    let tiles = files.len().min(SHOWN);
    let (tw, th) = (w * 0.52, h * 0.52);
    let (x0, y0) = (x + w * 0.22, y + h * 0.14);
    let step = (w * 0.09).round() as i32;
    // the back of the folder, then the tiles from the back forward, then the front flap over their feet
    s.fill_rrect(x, y + h * 0.08, w, h * 0.9, w * 0.16, back);
    for (dx, dy) in fan_offsets(tiles, step).into_iter().rev() {
        s.fill_rrect(x0 + dx as f32, y0 + dy as f32, tw, th, tw * 0.12, tile);
    }
    s.fill_rrect(x, y + h * 0.36, w, h * 0.62, w * 0.16, front);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_entry_is_not_a_minimized_window() {
        assert_eq!(crate::shell::desktop_dock::dock_min_index(DOCK_STACK), None);
        assert_ne!(DOCK_STACK, crate::shell::desk::DOCK_TRASH);
    }

    #[test]
    fn fan_steps_up_and_right_from_the_front_tile() {
        assert_eq!(fan_offsets(0, 10), Vec::<(i32, i32)>::new());
        assert_eq!(fan_offsets(3, 10), vec![(0, 0), (10, -5), (20, -10)]);
    }

    #[test]
    fn recent_files_are_newest_first_and_skip_hidden_ones() {
        let dir = std::env::temp_dir().join(format!("bifrost-stack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let new = std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000);
        std::fs::write(dir.join("old.pdf"), b"a").unwrap();
        std::fs::write(dir.join("new.zip"), b"b").unwrap();
        std::fs::write(dir.join(".hidden"), b"c").unwrap();
        std::fs::File::options().write(true).open(dir.join("old.pdf")).unwrap().set_modified(old).unwrap();
        std::fs::File::options().write(true).open(dir.join("new.zip")).unwrap().set_modified(new).unwrap();
        assert_eq!(recent_files(&dir, 5), vec!["new.zip".to_string(), "old.pdf".to_string()]);
        assert_eq!(recent_files(&dir, 1), vec!["new.zip".to_string()]);
        assert!(recent_files(&dir.join("missing"), 3).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

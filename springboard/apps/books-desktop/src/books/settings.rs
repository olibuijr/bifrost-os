//! The Settings window (Books > Settings). General: the library folders, with Add Folder and Remove for the folders the
//! user added, and Open Books In (single page or continuous scroll for a book's first opening). Reading: the text size,
//! the theme and Justify Text. The reading choices are the same preferences the reader's Display panel edits, so the
//! two always agree. Library folders and the open mode are kept in the state file beside the reading state.
//!
//! This file holds the line model, its geometry and the taps; `draw.rs` paints it and `mod.rs` routes the events.

use super::store::Theme;
use super::{nav_bottom_pt, Books, Screen, Target};
use springboard::shell::ui::Ui;
use std::path::PathBuf;

/// The widest the settings column gets, in points; on a wider window it is centred.
const COLUMN_MAX_PT: f32 = 640.0;
const HEADER_PT: f32 = 40.0;
const ROW_PT: f32 = 52.0;
const CHOICE_ROW_PT: f32 = 52.0;
const THEME_ROW_PT: f32 = 60.0;
const NOTE_PT: f32 = 48.0;
/// Space between two rows, and the extra space above a group's header.
const LINE_GAP_PT: f32 = 6.0;
const GROUP_GAP_PT: f32 = 18.0;
/// Edge of a control inside its row, in points.
const PAD_PT: f32 = 8.0;
/// The side of a stepper button, in points, and the width of Remove and Done.
const STEP_PT: f32 = 44.0;
const REMOVE_PT: f32 = 96.0;
const DONE_PT: f32 = 96.0;

/// The footnote under Reading: the reflow engine does not hyphenate, so the setting is not offered.
pub(super) const HYPHENATION_NOTE: &str = "Hyphenation is not available: the reflow engine does not hyphenate text yet.";

/// One line of the Settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Line {
    Header(&'static str),
    /// a library folder, by its index in `Books::folder_rows`
    Folder(usize),
    AddFolder,
    OpenIn,
    TextSize,
    Theme,
    Justify,
    Note(&'static str),
}

/// The lines of the window for a library with `folders` folders.
pub(super) fn lines(folders: usize) -> Vec<Line> {
    let mut out = vec![Line::Header("General")];
    out.extend((0..folders).map(Line::Folder));
    out.extend([Line::AddFolder, Line::Header("Open Books In"), Line::OpenIn, Line::Header("Reading"), Line::TextSize, Line::Theme, Line::Justify, Line::Note(HYPHENATION_NOTE)]);
    out
}

/// The height of a line in points.
pub(super) fn height_pt(line: Line) -> f32 {
    match line {
        Line::Header(_) => HEADER_PT,
        Line::Folder(_) | Line::AddFolder | Line::TextSize | Line::Justify => ROW_PT,
        Line::OpenIn => CHOICE_ROW_PT,
        Line::Theme => THEME_ROW_PT,
        Line::Note(_) => NOTE_PT,
    }
}

/// Where each line sits in a window `w` px wide, starting at `top` px: a rect (x, y, w, h) in px per line, in order.
pub(super) fn geometry(lines: &[Line], ui: &Ui, w: f32, top: f32) -> Vec<(Line, (f32, f32, f32, f32))> {
    let column = (w - 2.0 * ui.pt(PAD_PT * 2.0)).min(ui.pt(COLUMN_MAX_PT)).max(0.0);
    let x = (w - column) / 2.0;
    let mut y = top;
    let mut out = Vec::with_capacity(lines.len());
    for (i, &line) in lines.iter().enumerate() {
        if i > 0 && matches!(line, Line::Header(_)) {
            y += ui.pt(GROUP_GAP_PT);
        }
        let h = ui.pt(height_pt(line));
        out.push((line, (x, y, column, h)));
        y += h + ui.pt(LINE_GAP_PT);
    }
    out
}

/// The top of the first line, below the window's bar.
pub(super) fn content_top(ui: &Ui) -> f32 {
    ui.pt(nav_bottom_pt()) + ui.pt(PAD_PT * 2.0)
}

impl Books {
    /// Settings > Books: opens the window over the library.
    pub fn open_settings(&mut self) {
        self.screen = Screen::Settings;
        self.dirty = true;
    }

    /// Done: back to the library.
    pub(super) fn close_settings(&mut self) {
        self.screen = Screen::Library;
        self.dirty = true;
    }

    /// The folders the library scans: the built-in ones first (they cannot be removed), then the folders the user added.
    pub(super) fn folder_rows(&self) -> Vec<(PathBuf, Option<usize>)> {
        let mut out: Vec<(PathBuf, Option<usize>)> = self.dirs.iter().map(|d| (d.clone(), None)).collect();
        out.extend(self.store.folders.iter().enumerate().filter(|(_, d)| !self.dirs.contains(*d)).map(|(i, d)| (d.clone(), Some(i))));
        out
    }

    /// The scanned folders: the built-in ones and the user's, for `store::library`.
    pub(super) fn library_dirs(&self) -> Vec<PathBuf> {
        self.folder_rows().into_iter().map(|(d, _)| d).collect()
    }

    /// Every tap target of the window with its rect in px.
    pub(super) fn settings_targets(&self, ui: &Ui) -> Vec<(Target, (f32, f32, f32, f32))> {
        let w = self.w as f32;
        let rows = self.folder_rows();
        let mut out = vec![];
        // the Done button sits in the bar
        let bar = ui.pt(nav_bottom_pt());
        out.push((Target::SettingsDone, (w - ui.pt(PAD_PT * 2.0) - ui.pt(DONE_PT), ui.pt(PAD_PT), ui.pt(DONE_PT), bar - ui.pt(PAD_PT * 2.0))));
        for (line, (x, y, cw, h)) in geometry(&lines(rows.len()), ui, w, content_top(ui)) {
            match line {
                Line::Folder(k) => {
                    if matches!(rows.get(k), Some((_, Some(_)))) {
                        out.push((Target::RemoveFolder(k), (x + cw - ui.pt(REMOVE_PT), y, ui.pt(REMOVE_PT), h)));
                    }
                }
                Line::AddFolder => out.push((Target::AddFolder, (x, y, cw, h))),
                Line::OpenIn => {
                    let half = (cw - ui.pt(PAD_PT)) / 2.0;
                    out.push((Target::OpenMode(false), (x, y, half, h)));
                    out.push((Target::OpenMode(true), (x + half + ui.pt(PAD_PT), y, half, h)));
                }
                Line::TextSize => {
                    let step = ui.pt(STEP_PT);
                    let mid = (h - ui.pt(PAD_PT * 2.0)) / 2.0;
                    out.push((Target::SizeDown, (x + cw - 2.0 * step - ui.pt(PAD_PT), y + mid, step, h - ui.pt(PAD_PT * 2.0))));
                    out.push((Target::SizeUp, (x + cw - step, y + mid, step, h - ui.pt(PAD_PT * 2.0))));
                }
                Line::Theme => {
                    let tw = (cw - 2.0 * ui.pt(PAD_PT)) / Theme::ALL.len() as f32;
                    for i in 0..Theme::ALL.len() {
                        out.push((Target::ThemeButton(i), (x + i as f32 * (tw + ui.pt(PAD_PT)), y, tw, h - ui.pt(PAD_PT))));
                    }
                }
                Line::Justify => out.push((Target::Justify, (x, y, cw, h))),
                Line::Header(_) | Line::Note(_) => {}
            }
        }
        out
    }

    /// What a tap on the window lands on.
    pub(super) fn settings_hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        self.settings_targets(ui).into_iter().find(|&(_, rect)| super::within(x, y, rect)).map_or(Target::PanelBody, |(t, _)| t)
    }

    /// Settings: the open mode. `continuous` is what a book opens in from now on; the open book is not changed.
    pub(super) fn set_open_mode(&mut self, continuous: bool) {
        if self.store.open_continuous != continuous {
            self.store.open_continuous = continuous;
            self.save_now();
        }
        self.dirty = true;
    }

    /// Settings: Remove on a folder the user added. The library is rescanned without it.
    pub(super) fn remove_folder(&mut self, k: usize) {
        let Some((_, Some(user))) = self.folder_rows().get(k).cloned() else { return };
        if user < self.store.folders.len() {
            self.store.folders.remove(user);
            self.save_now();
            self.rescan();
        }
        self.dirty = true;
    }

    /// Settings: Add Folder asks the app to show the folder panel. The app calls `set_folder_request` with the request.
    pub fn take_folder_panel(&mut self) -> bool {
        std::mem::take(&mut self.wants_folder_panel)
    }

    pub fn set_folder_request(&mut self, request: u32) {
        self.folder_request = Some(request);
    }

    /// Whether `request` is the Add Folder panel's answer; clears the request either way.
    pub fn take_folder_request(&mut self, request: u32) -> bool {
        if self.folder_request == Some(request) {
            self.folder_request = None;
            true
        } else {
            false
        }
    }

    /// The folder the panel chose, added to the library. Linux gives the folder's path through its file descriptor; a
    /// folder whose path cannot be read is refused with a message.
    pub fn add_folder_file(&mut self, file: &std::fs::File) -> Result<(), String> {
        use std::os::fd::AsRawFd;
        let path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|_| "The folder could not be found on disk.".to_string())?;
        if !path.is_dir() {
            return Err("Only folders can be added to the library.".to_string());
        }
        if !self.dirs.contains(&path) && !self.store.folders.contains(&path) {
            self.store.folders.push(path);
            self.save_now();
            self.rescan();
        }
        self.dirty = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_lists_each_folder_then_the_groups() {
        let l = lines(2);
        assert_eq!(l[0], Line::Header("General"));
        assert_eq!(l[1], Line::Folder(0));
        assert_eq!(l[2], Line::Folder(1));
        assert_eq!(l[3], Line::AddFolder);
        assert!(l.contains(&Line::OpenIn));
        assert_eq!(l.iter().filter(|x| matches!(x, Line::Header(_))).count(), 3);
        assert!(matches!(l.last(), Some(Line::Note(_))));
    }

    #[test]
    fn a_long_folder_list_adds_lines_without_moving_the_reading_group() {
        let few = lines(1).len();
        let many = lines(4).len();
        assert_eq!(many - few, 3);
    }

    #[test]
    fn height_of_each_line_is_positive() {
        for l in lines(1) {
            assert!(height_pt(l) > 0.0, "{l:?}");
        }
    }
}

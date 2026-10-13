//! The long-press context menu: which actions an item (or the empty
//! background) offers, where the menu sits next to the touch, and what a
//! tap on it hits. Drawing lives with the app; this is the model.

use crate::shell::ui::Ui;

/// Width of the menu in points (bifrost 15 uses 250).
pub const WIDTH_PT: f32 = 250.0;
pub const ROW_PT: f32 = 44.0;
pub const RADIUS_PT: f32 = 13.0;
/// Gap between sections (destructive actions sit in their own section).
pub const GAP_PT: f32 = 8.0;
const MARGIN_PT: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Rename,
    MoveTo,
    Copy,
    Share,
    /// the share sheet (Copy, Save to Files, Send Nearby); files only
    SendNearby,
    OpenInTerminal,
    GetInfo,
    Delete,
    /// compress the item into a new .zip next to it
    Compress,
    /// expand an archive into a new folder next to it
    Extract,
    NewFolder,
    Paste,
    /// Recently Deleted: put the item back in a folder of your choice
    Restore,
    /// cycles the item's colour tag: Red, Orange, ... Gray, none
    Tag,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Rename => "Rename",
            Action::MoveTo => "Move to…",
            Action::Copy => "Copy",
            Action::Share => "Share",
            Action::SendNearby => "Send Nearby…",
            Action::OpenInTerminal => "Open in Terminal",
            Action::GetInfo => "Get Info",
            Action::Delete => "Delete",
            Action::Compress => "Compress",
            Action::Extract => "Expand",
            Action::NewFolder => "New Folder",
            Action::Paste => "Paste",
            Action::Restore => "Put Back…",
            Action::Tag => "Tag",
        }
    }

    /// Drawn in red, in a section of its own.
    pub fn destructive(self) -> bool {
        self == Action::Delete
    }

    /// The SF-style glyph drawn at the row's right edge.
    pub fn glyph(self) -> Glyph {
        match self {
            Action::Rename => Glyph::Pencil,
            Action::MoveTo => Glyph::Folder,
            Action::Copy => Glyph::Doc,
            Action::Share => Glyph::Share,
            Action::SendNearby => Glyph::Share,
            Action::OpenInTerminal => Glyph::Terminal,
            Action::GetInfo => Glyph::Info,
            Action::Delete => Glyph::Trash,
            Action::Compress | Action::Extract => Glyph::Doc,
            Action::NewFolder => Glyph::FolderPlus,
            Action::Paste => Glyph::Clipboard,
            Action::Restore => Glyph::Folder,
            Action::Tag => Glyph::Doc,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Pencil,
    Folder,
    Doc,
    Share,
    Terminal,
    Info,
    Trash,
    FolderPlus,
    Clipboard,
}

/// What the menu was opened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// an entry of the listing, by index
    Entry(usize),
    /// the folder itself (a long press on empty space)
    Background,
}

/// The actions for a subject; `clipboard` says whether Paste applies.
pub fn actions(subject: Subject, clipboard: bool) -> Vec<Action> {
    match subject {
        Subject::Entry(_) => vec![Action::Rename, Action::MoveTo, Action::Copy, Action::Share, Action::Compress, Action::Tag, Action::GetInfo, Action::Delete],
        Subject::Background => {
            let mut v = vec![Action::NewFolder];
            if clipboard {
                v.push(Action::Paste);
            }
            v
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub subject: Subject,
    pub items: Vec<Action>,
    /// where the finger was, in px
    pub anchor: (f32, f32),
    /// the Share submenu is showing
    pub sharing: bool,
}

/// The actions of an item in Recently Deleted.
pub fn trash_actions() -> Vec<Action> {
    vec![Action::Restore, Action::Delete]
}

impl Menu {
    pub fn new(subject: Subject, clipboard: bool, anchor: (f32, f32)) -> Self {
        Menu { subject, items: actions(subject, clipboard), anchor, sharing: false }
    }

    /// Swaps the items for the Share sheet's targets.
    pub fn show_share(&mut self, file: bool) {
        self.items = if file { vec![Action::SendNearby, Action::OpenInTerminal] } else { vec![Action::OpenInTerminal] };
        self.sharing = true;
    }

    /// Row tops relative to the menu's top, in px (sections are separated by a gap).
    fn row_tops(&self, ui: &Ui) -> Vec<f32> {
        let mut y = 0.0;
        let mut out = vec![];
        for (i, a) in self.items.iter().enumerate() {
            if i > 0 && a.destructive() != self.items[i - 1].destructive() {
                y += ui.pt(GAP_PT);
            }
            out.push(y);
            y += ui.pt(ROW_PT);
        }
        out
    }

    /// Total height in px.
    pub fn height(&self, ui: &Ui) -> f32 {
        self.row_tops(ui).last().map_or(0.0, |y| y + ui.pt(ROW_PT))
    }

    /// The menu's box (x, y, w, h) in px: just below and right of the
    /// touch, pulled back inside the screen (`w` x `h` px, `bottom` is the
    /// lowest usable y).
    pub fn rect(&self, ui: &Ui, w: i32, bottom: f32) -> (f32, f32, f32, f32) {
        let (mw, mh, m) = (ui.pt(WIDTH_PT), self.height(ui), ui.pt(MARGIN_PT));
        let x = (self.anchor.0 + ui.pt(8.0)).min(w as f32 - m - mw).max(m);
        let y = (self.anchor.1 + ui.pt(8.0)).min(bottom - m - mh).max(ui.pt(crate::scene::STATUS_PT) + m);
        (x, y, mw, mh)
    }

    /// The rectangles of the sections (runs of rows without a gap), in px.
    pub fn sections(&self, ui: &Ui, w: i32, bottom: f32) -> Vec<(f32, f32, f32, f32)> {
        let (x, y, mw, _) = self.rect(ui, w, bottom);
        let tops = self.row_tops(ui);
        let mut out: Vec<(f32, f32, f32, f32)> = vec![];
        for (i, top) in tops.iter().enumerate() {
            let starts = i == 0 || self.items[i].destructive() != self.items[i - 1].destructive();
            if starts {
                out.push((x, y + top, mw, ui.pt(ROW_PT)));
            } else if let Some(last) = out.last_mut() {
                last.3 += ui.pt(ROW_PT);
            }
        }
        out
    }

    /// Each item's row (x, y, w, h) in px.
    pub fn rows(&self, ui: &Ui, w: i32, bottom: f32) -> Vec<(Action, (f32, f32, f32, f32))> {
        let (x, y, mw, _) = self.rect(ui, w, bottom);
        self.items.iter().zip(self.row_tops(ui)).map(|(&a, top)| (a, (x, y + top, mw, ui.pt(ROW_PT)))).collect()
    }

    /// The action under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, bottom: f32, x: f32, y: f32) -> Option<Action> {
        self.rows(ui, w, bottom).into_iter().find(|(_, (rx, ry, rw, rh))| x >= *rx && x < rx + rw && y >= *ry && y < ry + rh).map(|(a, _)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_depend_on_the_subject() {
        let e = actions(Subject::Entry(3), false);
        assert_eq!(e.first(), Some(&Action::Rename));
        assert_eq!(e.last(), Some(&Action::Delete));
        assert!(e.contains(&Action::Share) && e.contains(&Action::MoveTo) && e.contains(&Action::Copy));
        assert_eq!(actions(Subject::Background, false), vec![Action::NewFolder]);
        assert_eq!(actions(Subject::Background, true), vec![Action::NewFolder, Action::Paste]);
        assert!(Action::Delete.destructive() && !Action::Rename.destructive());
        assert_eq!(Action::MoveTo.label(), "Move to…");
        assert_eq!(Action::OpenInTerminal.glyph(), Glyph::Terminal);
    }

    #[test]
    fn layout_hits_and_clamps() {
        let ui = crate::shell::tests::ui();
        let bottom = 1920.0 - ui.pt(28.0);
        let mut m = Menu::new(Subject::Entry(0), false, (100.0, 500.0));
        let (x, y, w, h) = m.rect(&ui, 1200, bottom);
        assert_eq!((x, y, w), (100.0 + ui.pt(8.0), 500.0 + ui.pt(8.0), ui.pt(WIDTH_PT)));
        assert_eq!(h, ui.pt(8.0 * ROW_PT + GAP_PT), "eight rows with one section gap");
        let sections = m.sections(&ui, 1200, bottom);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].3, ui.pt(ROW_PT), "Delete alone in the second section");
        assert_eq!(m.hit(&ui, 1200, bottom, x + 10.0, y + 10.0), Some(Action::Rename));
        assert_eq!(m.hit(&ui, 1200, bottom, x + 10.0, y + h - 10.0), Some(Action::Delete));
        assert_eq!(m.hit(&ui, 1200, bottom, x + 10.0, y + ui.pt(7.0 * ROW_PT) + 2.0), None, "the gap hits nothing");
        assert_eq!(m.hit(&ui, 1200, bottom, x - 5.0, y + 10.0), None);
        // near the bottom-right corner the menu is pulled back on screen
        let m2 = Menu::new(Subject::Entry(0), false, (1190.0, 1900.0));
        let (x2, y2, _, h2) = m2.rect(&ui, 1200, bottom);
        assert!(x2 + ui.pt(WIDTH_PT) <= 1200.0 - ui.pt(16.0) + 0.01);
        assert!(y2 + h2 <= bottom - ui.pt(16.0) + 0.01);
        m.show_share(false);
        assert!(m.sharing);
        assert_eq!(m.items, vec![Action::OpenInTerminal]);
        assert_eq!(m.height(&ui), ui.pt(ROW_PT));
        m.show_share(true);
        assert_eq!(m.items, vec![Action::SendNearby, Action::OpenInTerminal]);
        let bg = Menu::new(Subject::Background, true, (0.0, 0.0));
        assert_eq!(bg.rows(&ui, 1200, bottom).len(), 2);
        assert!(bg.rect(&ui, 1200, bottom).1 >= ui.pt(20.0 + 16.0), "never under the status bar");
    }
}

//! Text entry for Photos: one line typed into a card with the on-screen keyboard under it (a physical keyboard types
//! into the same field). The toolbar search, an album name, a photo's title, caption, keywords, place and capture time,
//! a keyword's new name and the folder to import from all use it. The card and its keyboard are drawn by `Photos`; this
//! file holds the state and the geometry, so they can be tested without a screen.

use crate::shell::keyboard::{Keyboard, Out};
use crate::shell::textfield::TextField;
use crate::shell::ui::{home_zone, Ui};
use bifrost_proto::ReturnKey;

/// What the typed text is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// the toolbar search; the grid follows the text as it is typed
    Search,
    /// the user album in this slot
    RenameAlbum(u8),
    Title,
    Caption,
    Keywords,
    Place,
    /// the capture time of the open photo
    TakenAt,
    /// a library keyword, to be renamed (an empty name deletes it)
    RenameKeyword(String),
    /// a keyword to add to every selected photo
    AddKeyword,
    /// the folder to copy pictures from
    ImportFolder,
    /// the folder of the library to show (created when missing)
    SwitchLibrary,
    /// the name of a new smart album, which keeps the search in force as its rules (slot of the album)
    SmartName(u8),
    /// the rules of the smart album in this slot
    SmartRules(u8),
}

impl Purpose {
    /// The card's title.
    pub fn heading(&self) -> &'static str {
        match self {
            Purpose::Search => "Search",
            Purpose::RenameAlbum(_) => "Rename Album",
            Purpose::Title => "Title",
            Purpose::Caption => "Caption",
            Purpose::Keywords => "Keywords",
            Purpose::Place => "Location",
            Purpose::TakenAt => "Date and Time",
            Purpose::RenameKeyword(_) => "Rename Keyword",
            Purpose::AddKeyword => "Add Keyword",
            Purpose::ImportFolder => "Import",
            Purpose::SwitchLibrary => "Open Library",
            Purpose::SmartName(_) => "New Smart Album",
            Purpose::SmartRules(_) => "Smart Album Rules",
        }
    }

    /// The grey text in an empty field.
    pub fn placeholder(&self) -> &'static str {
        match self {
            Purpose::Search => "Photos, albums, dates, keywords",
            Purpose::RenameAlbum(_) => "Album name",
            Purpose::Title => "Add a title",
            Purpose::Caption => "Add a caption",
            Purpose::Keywords => "Separate keywords with commas",
            Purpose::Place => "Add a place",
            Purpose::TakenAt => "2026-10-04 14:30",
            Purpose::RenameKeyword(_) => "New name",
            Purpose::AddKeyword => "Keyword",
            Purpose::ImportFolder => "/path/to/folder",
            Purpose::SwitchLibrary => "/path/to/library",
            Purpose::SmartName(_) => "Album name",
            Purpose::SmartRules(_) => "#beach is:favorite kind:png",
        }
    }

    /// The line under the field that says what it understands.
    pub fn hint(&self) -> &'static str {
        match self {
            Purpose::Search => "Words match names, albums, dates and keywords. #beach, is:favorite, is:edited, kind:png, rating:3.",
            Purpose::RenameAlbum(_) => "An empty name keeps the old one.",
            Purpose::Title | Purpose::Caption | Purpose::Place => "Saved beside the photo. The picture file is not changed.",
            Purpose::Keywords => "Saved beside the photo. The picture file is not changed.",
            Purpose::TakenAt => "A date, a time or both. Empty restores the time of the file.",
            Purpose::RenameKeyword(_) => "Photos with the new name already keep one copy. An empty name deletes the keyword.",
            Purpose::AddKeyword => "Added to every selected photo.",
            Purpose::ImportFolder => "Pictures are copied into Pictures; the originals stay where they are.",
            Purpose::SwitchLibrary => "Shows the pictures in this folder. A folder that does not exist is created.",
            Purpose::SmartName(_) => "The album keeps the rules of your search and lists every photo that matches them.",
            Purpose::SmartRules(_) => "Words match names, albums, dates and keywords. #beach, is:favorite, is:edited, kind:png, rating:3.",
        }
    }

    /// Longest text in characters.
    pub fn max(&self) -> usize {
        match self {
            Purpose::Caption => super::meta::MAX_CAPTION,
            Purpose::Search | Purpose::ImportFolder | Purpose::SwitchLibrary | Purpose::SmartRules(_) => 200,
            Purpose::Keywords => 400,
            Purpose::TakenAt => 40,
            _ => super::meta::MAX_TEXT,
        }
    }

    /// Does the grid follow the text while it is typed.
    pub fn live(&self) -> bool {
        matches!(self, Purpose::Search)
    }
}

/// What a key did to the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Typing,
    /// Return was pressed: use the text
    Commit,
    /// Escape was pressed: drop it
    Cancel,
}

/// The card on screen.
pub struct Entry {
    pub purpose: Purpose,
    pub field: TextField,
    pub keyboard: Keyboard,
}

const CARD_W_PT: f32 = 520.0;
const CARD_TOP_PT: f32 = 60.0;
const BAR_PT: f32 = 48.0;
const FIELD_H_PT: f32 = 40.0;
const HINT_PT: f32 = 44.0;
const MARGIN_PT: f32 = 16.0;

impl Entry {
    /// Opens the card for `purpose` with `initial` in the field. The on-screen keyboard slides in at `t` unless a
    /// physical keyboard does the typing (`soft_keyboard` false). `w` is the screen width in px.
    pub fn open(ui: &Ui, w: i32, purpose: Purpose, initial: &str, soft_keyboard: bool, t: f64) -> Entry {
        let mut field = TextField::new(ui, purpose.placeholder(), 17.0, false, purpose.max());
        field.set_text(ui, initial);
        field.focused = true;
        let mut keyboard = Keyboard::new_hidden(w as f32, ui.s);
        keyboard.set_return_key(if purpose.live() { ReturnKey::Search } else { ReturnKey::Done });
        if soft_keyboard {
            keyboard.set_shown(true, t);
        }
        Entry { purpose, field, keyboard }
    }

    /// Applies a key typed on either keyboard.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) -> Outcome {
        match out {
            Out::Hide => {
                self.keyboard.set_shown(false, t);
                self.field.focused = false;
                Outcome::Typing
            }
            Out::Named("Escape") => Outcome::Cancel,
            out => {
                self.field.focused = true;
                if self.field.edit(ui, &out, t) {
                    Outcome::Commit
                } else {
                    Outcome::Typing
                }
            }
        }
    }

    /// Top of the keyboard in px while it shows, otherwise the bottom of the usable screen.
    pub fn keyboard_top(&self, ui: &Ui, h: i32) -> f32 {
        let docked = if self.keyboard.shown() { self.keyboard.h() } else { 0.0 };
        h as f32 - home_zone(ui) - docked
    }

    /// The card (x, y, w, h) on a screen `w` px wide.
    pub fn card(&self, ui: &Ui, w: i32, status: f32) -> (f32, f32, f32, f32) {
        let width = ui.pt(CARD_W_PT).min(w as f32 - 2.0 * ui.pt(MARGIN_PT));
        ((w as f32 - width) / 2.0, status + ui.pt(CARD_TOP_PT), width, ui.pt(BAR_PT + FIELD_H_PT + HINT_PT + 8.0))
    }

    /// The Cancel button's box.
    pub fn cancel_rect(&self, ui: &Ui, w: i32, status: f32) -> (f32, f32, f32, f32) {
        let (x, y, _, _) = self.card(ui, w, status);
        (x, y, ui.pt(96.0), ui.pt(BAR_PT))
    }

    /// The Done button's box.
    pub fn done_rect(&self, ui: &Ui, w: i32, status: f32) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = self.card(ui, w, status);
        (x + cw - ui.pt(96.0), y, ui.pt(96.0), ui.pt(BAR_PT))
    }

    /// The rounded box behind the text.
    pub fn field_rect(&self, ui: &Ui, w: i32, status: f32) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = self.card(ui, w, status);
        (x + ui.pt(MARGIN_PT), y + ui.pt(BAR_PT), cw - 2.0 * ui.pt(MARGIN_PT), ui.pt(FIELD_H_PT))
    }

    /// The box the text is drawn and touched in.
    pub fn field_frame(&self, ui: &Ui, w: i32, status: f32) -> (f32, f32, f32, f32) {
        let (x, y, fw, fh) = self.field_rect(ui, w, status);
        (x + ui.pt(12.0), y, fw - ui.pt(24.0), fh)
    }

    /// The hint line's box under the field.
    pub fn hint_rect(&self, ui: &Ui, w: i32, status: f32) -> (f32, f32, f32, f32) {
        let (x, y, fw, fh) = self.field_rect(ui, w, status);
        (x, y + fh, fw, ui.pt(HINT_PT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    const EVERY: [Purpose; 13] = [
        Purpose::Search,
        Purpose::RenameAlbum(0),
        Purpose::Title,
        Purpose::Caption,
        Purpose::Keywords,
        Purpose::Place,
        Purpose::TakenAt,
        Purpose::AddKeyword,
        Purpose::ImportFolder,
        Purpose::SwitchLibrary,
        Purpose::SmartName(0),
        Purpose::SmartRules(0),
        Purpose::RenameKeyword(String::new()),
    ];

    #[test]
    fn every_purpose_has_words_for_the_card() {
        for p in EVERY {
            assert!(!p.heading().is_empty() && !p.placeholder().is_empty() && !p.hint().is_empty(), "{p:?}");
            assert!(p.max() >= 40, "{p:?}");
        }
        assert!(Purpose::Search.live());
        assert!(!Purpose::Title.live() && !Purpose::ImportFolder.live());
    }

    #[test]
    fn typing_edits_the_field_return_commits_and_escape_cancels() {
        let ui = ui();
        let mut e = Entry::open(&ui, 1200, Purpose::Title, "Sea", true, 0.0);
        assert_eq!(e.field.text(), "Sea");
        assert_eq!(e.key(&ui, Out::Text("s".into()), 1.0), Outcome::Typing);
        assert_eq!(e.field.text(), "Seas");
        assert_eq!(e.key(&ui, Out::Backspace, 2.0), Outcome::Typing);
        assert_eq!(e.field.text(), "Sea");
        assert_eq!(e.key(&ui, Out::Enter, 3.0), Outcome::Commit);
        assert_eq!(e.key(&ui, Out::Named("Escape"), 4.0), Outcome::Cancel);
        assert_eq!(e.key(&ui, Out::Hide, 5.0), Outcome::Typing, "hiding the keyboard keeps the card");
        assert_eq!(e.field.text(), "Sea");
    }

    #[test]
    fn the_field_keeps_to_the_limit_of_its_purpose() {
        let ui = ui();
        let e = Entry::open(&ui, 1200, Purpose::Title, &"x".repeat(500), true, 0.0);
        assert_eq!(e.field.len(), Purpose::Title.max());
    }

    #[test]
    fn the_card_sits_under_the_status_bar_inside_the_screen() {
        let ui = ui();
        let e = Entry::open(&ui, 1200, Purpose::Search, "", true, 0.0);
        let status = ui.pt(20.0);
        let (x, y, w, h) = e.card(&ui, 1200, status);
        assert!(x >= 0.0 && x + w <= 1200.0 && y > status);
        let field = e.field_rect(&ui, 1200, status);
        assert!(field.1 >= y && field.1 + field.3 <= y + h);
        let (c, d) = (e.cancel_rect(&ui, 1200, status), e.done_rect(&ui, 1200, status));
        assert!(c.0 + c.2 <= d.0, "Cancel and Done do not overlap");
        assert!(e.hint_rect(&ui, 1200, status).1 >= field.1 + field.3);
        // a narrow screen shrinks the card to its margins
        let (nx, _, nw, _) = e.card(&ui, 400, status);
        assert!(nx >= ui.pt(MARGIN_PT) - 0.5 && nx + nw <= 400.0);
    }
}

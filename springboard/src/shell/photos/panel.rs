//! The list panel Photos uses for everything that is a list of choices: Library and Tools, Memories, Places,
//! Duplicates, Keywords, Folders, Smart Albums and their editor, Filter By, Slideshow Options, Export, Settings,
//! Libraries, the details of a photo and the welcome tips. A panel is a card with a title bar (Back and Done) over
//! a scrolling list of rows; a tap on a row hands back the `Act` the row carries. The app builds the rows from its
//! state (`commands.rs`), so this file knows nothing about photos.

use super::rules::KindFilter;
use super::{flat, Album, BLUE, RED, SHEET};
use crate::deildu::scroll::Scroller;
use crate::gfx::{rgba, Surface};
use crate::layout::in_rect as within;
use crate::shell::cache::TextCache;
use crate::shell::ui::{Ui, Weight, SECONDARY, SEPARATOR, WHITE};
use crate::shell::Event;

const CARD_W_PT: f32 = 460.0;
const TITLE_PT: f32 = 44.0;
const ROW_PT: f32 = 48.0;
const HEADER_PT: f32 = 34.0;
const SIDE_PT: f32 = 16.0;
const MARGIN_PT: f32 = 12.0;
/// The width of the trailing button of a row.
const TRAIL_W_PT: f32 = 76.0;

/// The panels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelKind {
    /// Library and Tools: the entry to everything below
    Library,
    Memories,
    Places,
    Duplicates,
    Keywords,
    /// one keyword: rename, delete, apply to the selection
    Keyword(String),
    Folders,
    Folder(u8),
    SmartAlbums,
    SmartEditor(u8),
    FilterBy,
    SlideshowOptions,
    Export,
    Settings,
    Libraries,
    /// the details of the photo in the viewer
    Details,
    /// a message with an OK row (an import report, an export report)
    Notice(String),
    /// the first-launch tips
    Tour,
}

/// What a typed answer is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    RenameAlbum(u8),
    NewFolder,
    RenameFolder(u8),
    NewSmart,
    RenameSmart(u8),
    SmartKeyword(u8),
    SmartFilename(u8),
    SmartYear(u8),
    Title,
    Caption,
    Keywords,
    DateTime,
    Place,
    NewKeyword,
    RenameKeyword(String),
    Search,
    DateFilter,
    LibraryPath,
}

/// A setting a row changes by tapping it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    SlideSeconds,
    SlideTransition,
    SlideMotion,
    ExportKind,
    ExportQuality,
    ExportSize,
    ExportMetadata,
    ExportNaming,
    ShowMemories,
    ShowHiddenAlbum,
    AspectGrid,
    Histogram,
    Kind(KindFilter),
    SmartMatch(u8),
    SmartFavourite(u8),
    SmartEdited(u8),
    SmartRating(u8),
    SmartRecent(u8),
    SmartType(u8),
    HideLocation,
    FolderMember { folder: u8, album: u8 },
}

/// Something a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// show another panel (Back returns to this one)
    Open(PanelKind),
    /// close the panel and show an album
    OpenAlbum(Album),
    /// ask for a line of text
    Ask(Purpose),
    Change(Setting),
    ExportNow,
    MergeDuplicates,
    DeleteSmart(u8),
    DeleteFolder(u8),
    DeleteKeyword(String),
    /// give a keyword to the photos the app acts on (the selection, else the open photo)
    KeywordToTargets(String),
    KeywordFromTargets(String),
    CopyLocation,
    ClearDateFilter,
    ClearSearch,
    PlayAlbum(Album),
    ChooseLibrary(usize),
    /// run a menu command
    Run(super::Cmd),
    EndTour,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Accent,
    Danger,
    Dim,
}

/// One row of a panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub tone: Tone,
    pub act: Option<Act>,
    /// a small button at the right end that does its own act
    pub trailing: Option<(String, Act)>,
    pub checked: bool,
    /// a section title: smaller, not tappable
    pub header: bool,
}

impl Row {
    pub fn new(label: &str, act: Act) -> Row {
        Row { label: label.to_string(), value: String::new(), tone: Tone::Normal, act: Some(act), trailing: None, checked: false, header: false }
    }

    /// A line of text with no action.
    pub fn info(label: &str) -> Row {
        Row { label: label.to_string(), value: String::new(), tone: Tone::Dim, act: None, trailing: None, checked: false, header: false }
    }

    pub fn header(label: &str) -> Row {
        Row { header: true, ..Row::info(label) }
    }

    pub fn value(mut self, v: &str) -> Row {
        self.value = v.to_string();
        self
    }

    pub fn tone(mut self, t: Tone) -> Row {
        self.tone = t;
        self
    }

    pub fn checked(mut self, on: bool) -> Row {
        self.checked = on;
        self
    }

    pub fn trailing(mut self, label: &str, act: Act) -> Row {
        self.trailing = Some((label.to_string(), act));
        self
    }

    fn height_pt(&self) -> f32 {
        if self.header { HEADER_PT } else { ROW_PT }
    }
}

/// Where a touch landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Back,
    Done,
    Row(usize),
    Trailing(usize),
    /// the title bar or a row with no action
    Card,
    Outside,
}

/// What a touch did to the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    None,
    Close,
    Back,
    Act(Act),
}

pub struct Panel {
    pub kind: PanelKind,
    pub title: String,
    pub rows: Vec<Row>,
    /// a Back button is shown (the panel was opened from another)
    pub back: bool,
    w: i32,
    h: i32,
    scroll: Scroller,
    pressed: Option<Press>,
    scrolling: bool,
    text: TextCache,
}

/// `text` cut to `max` characters with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}

impl Panel {
    pub fn new(ui: &Ui, w: i32, h: i32, kind: PanelKind, title: &str, rows: Vec<Row>, back: bool, top: f32, bottom: f32) -> Panel {
        let mut p = Panel { kind, title: title.to_string(), rows, back, w, h, scroll: Scroller::new(), pressed: None, scrolling: false, text: TextCache::default() };
        p.fit(ui, top, bottom);
        p
    }

    /// Replaces the rows, keeping the scroll position (the list changed under the finger of a toggle).
    pub fn set_rows(&mut self, ui: &Ui, rows: Vec<Row>, top: f32, bottom: f32) {
        self.rows = rows;
        self.fit(ui, top, bottom);
    }

    fn total(&self, ui: &Ui) -> f32 {
        self.rows.iter().map(|r| ui.pt(r.height_pt())).sum()
    }

    /// The card (x, y, w, h) between `top` and `bottom`.
    pub fn card(&self, ui: &Ui, top: f32, bottom: f32) -> (f32, f32, f32, f32) {
        let width = ui.pt(CARD_W_PT).min(self.w as f32 - 2.0 * ui.pt(MARGIN_PT));
        let avail = (bottom - top - 2.0 * ui.pt(MARGIN_PT)).max(ui.pt(TITLE_PT + ROW_PT));
        let height = (ui.pt(TITLE_PT) + self.total(ui)).min(avail);
        let y = (top + ui.pt(MARGIN_PT)).max(top + (bottom - top - height) / 2.0 - ui.pt(24.0)).min(bottom - height - ui.pt(MARGIN_PT)).max(top);
        ((self.w as f32 - width) / 2.0, y, width, height)
    }

    fn fit(&mut self, ui: &Ui, top: f32, bottom: f32) {
        let (_, _, _, ch) = self.card(ui, top, bottom);
        self.scroll.set_max(self.total(ui) - (ch - ui.pt(TITLE_PT)));
    }

    pub fn hit(&self, ui: &Ui, top: f32, bottom: f32, x: f32, y: f32) -> Press {
        let (cx, cy, cw, ch) = self.card(ui, top, bottom);
        if !within(x, y, (cx, cy, cw, ch)) {
            return Press::Outside;
        }
        if y < cy + ui.pt(TITLE_PT) {
            if x >= cx + cw - ui.pt(90.0) {
                return Press::Done;
            }
            if self.back && x < cx + ui.pt(90.0) {
                return Press::Back;
            }
            return Press::Card;
        }
        let mut ry = cy + ui.pt(TITLE_PT) - self.scroll.offset();
        for (i, r) in self.rows.iter().enumerate() {
            let rh = ui.pt(r.height_pt());
            if y >= ry && y < ry + rh {
                if r.header {
                    return Press::Card;
                }
                if r.trailing.is_some() && x >= cx + cw - ui.pt(TRAIL_W_PT) {
                    return Press::Trailing(i);
                }
                return if r.act.is_some() { Press::Row(i) } else { Press::Card };
            }
            ry += rh;
        }
        Press::Card
    }

    fn act_of(&self, press: Press) -> Option<Act> {
        match press {
            Press::Row(i) => self.rows.get(i).and_then(|r| r.act.clone()),
            Press::Trailing(i) => self.rows.get(i).and_then(|r| r.trailing.clone()).map(|t| t.1),
            _ => None,
        }
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, top: f32, bottom: f32, t: f64) -> PanelEvent {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return PanelEvent::None,
        };
        let (_, cy, _, _) = self.card(ui, top, bottom);
        match ev {
            Event::Down(..) => {
                let press = self.hit(ui, top, bottom, x, y);
                self.pressed = Some(press);
                if y >= cy + ui.pt(TITLE_PT) && press != Press::Outside {
                    self.scroll.down(y, t);
                    self.scrolling = true;
                }
            }
            Event::Move(..) => {
                if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                }
            }
            _ => {
                let pressed = self.pressed.take();
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                if !tap {
                    return PanelEvent::None;
                }
                let now = self.hit(ui, top, bottom, x, y);
                if pressed != Some(now) {
                    return PanelEvent::None;
                }
                return match now {
                    Press::Outside | Press::Done => PanelEvent::Close,
                    Press::Back => PanelEvent::Back,
                    Press::Card => PanelEvent::None,
                    p => self.act_of(p).map_or(PanelEvent::None, PanelEvent::Act),
                };
            }
        }
        PanelEvent::None
    }

    /// Advances the scroll; true when the screen changed.
    pub fn tick(&mut self, t: f64) -> bool {
        self.scroll.tick(t)
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, top: f32, bottom: f32) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 110));
        let (x, y, w, h) = self.card(ui, top, bottom);
        s.fill_rrect(x, y, w, h, flat(ui), SHEET);
        let bar = ui.pt(TITLE_PT);
        let mid = y + bar / 2.0;
        let pressed = self.pressed;
        let tint = |p: Press, c: u32| if pressed == Some(p) { crate::gfx::scale(c, 140) } else { c };
        let title = clip(&self.title, 30);
        let img = self.text.get(ui, &title, 17.0, Weight::Semibold, WHITE);
        s.blit(img, (x + (w - img.w as f32) / 2.0) as i32, (mid - img.h as f32 / 2.0) as i32, 255, 255);
        let done = self.text.get(ui, "Done", 17.0, Weight::Semibold, tint(Press::Done, BLUE));
        s.blit(done, (x + w - ui.pt(SIDE_PT) - done.w as f32) as i32, (mid - done.h as f32 / 2.0) as i32, 255, 255);
        if self.back {
            let back = self.text.get(ui, "\u{2039} Back", 17.0, Weight::Regular, tint(Press::Back, BLUE));
            s.blit(back, (x + ui.pt(SIDE_PT)) as i32, (mid - back.h as f32 / 2.0) as i32, 255, 255);
        }
        let list_top = y + bar;
        let off = self.scroll.offset();
        let mut band = s.rows(list_top as usize, (y + h) as usize);
        let mut ry = list_top - off;
        let rows = self.rows.clone();
        let max_label = ((w - ui.pt(2.0 * SIDE_PT)) / ui.pt(8.5)) as usize;
        for (i, r) in rows.iter().enumerate() {
            let rh = ui.pt(r.height_pt());
            if ry + rh < list_top || ry > y + h {
                ry += rh;
                continue;
            }
            let cy = ry + rh / 2.0;
            if r.header {
                let img = self.text.get(ui, &clip(&r.label.to_uppercase(), max_label), 12.0, Weight::Semibold, SECONDARY);
                band.blit(img, (x + ui.pt(SIDE_PT)) as i32, (ry + rh * 0.62 - img.h as f32 / 2.0) as i32, 255, 255);
                ry += rh;
                continue;
            }
            if matches!(pressed, Some(Press::Row(p)) if p == i) {
                band.fill_rrect(x, ry, w, rh, 0.0, rgba(255, 255, 255, 20));
            }
            band.fill_rect((x + ui.pt(SIDE_PT)) as usize, ry as usize, (w - ui.pt(2.0 * SIDE_PT)) as usize, 1, SEPARATOR);
            let colour = match r.tone {
                Tone::Normal => WHITE,
                Tone::Accent => BLUE,
                Tone::Danger => RED,
                Tone::Dim => SECONDARY,
            };
            let trail_w = if r.trailing.is_some() { ui.pt(TRAIL_W_PT) } else { 0.0 };
            let mut lx = x + ui.pt(SIDE_PT);
            if r.checked {
                let img = self.text.get(ui, "\u{2713}", 17.0, Weight::Semibold, BLUE);
                band.blit(img, lx as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
                lx += ui.pt(24.0);
            }
            let label_chars = if r.value.is_empty() { max_label } else { max_label * 6 / 10 };
            let img = self.text.get(ui, &clip(&r.label, label_chars), 17.0, Weight::Regular, colour);
            band.blit(img, lx as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
            if !r.value.is_empty() {
                let img = self.text.get(ui, &clip(&r.value, max_label * 4 / 10), 15.0, Weight::Regular, SECONDARY);
                band.blit(img, (x + w - ui.pt(SIDE_PT) - trail_w - img.w as f32) as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
            }
            if let Some((label, _)) = &r.trailing {
                let c = if matches!(pressed, Some(Press::Trailing(p)) if p == i) { crate::gfx::scale(BLUE, 140) } else { BLUE };
                let img = self.text.get(ui, label, 15.0, Weight::Regular, c);
                band.blit(img, (x + w - ui.pt(SIDE_PT) - img.w as f32) as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
            }
            ry += rh;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn panel(back: bool) -> (Panel, Ui) {
        let ui = ui();
        let rows = vec![
            Row::header("Albums"),
            Row::new("Trip", Act::OpenAlbum(Album::User(0))).value("12").trailing("Edit", Act::Open(PanelKind::SmartEditor(0))),
            Row::info("Nothing else"),
            Row::new("Close", Act::Close),
        ];
        let p = Panel::new(&ui, 1200, 1920, PanelKind::Library, "Library", rows, back, 100.0, 1800.0);
        (p, ui)
    }

    fn tap(p: &mut Panel, ui: &Ui, x: f32, y: f32) -> PanelEvent {
        p.touch(ui, Event::Down(x, y), 100.0, 1800.0, 1.0);
        p.touch(ui, Event::Up(x, y), 100.0, 1800.0, 1.04)
    }

    #[test]
    fn rows_are_hit_by_position_and_headers_do_nothing() {
        let (p, ui) = panel(true);
        let (cx, cy, cw, _) = p.card(&ui, 100.0, 1800.0);
        let first = cy + ui.pt(TITLE_PT);
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + 50.0, first + 5.0), Press::Card, "a header is not a button");
        let row1 = first + ui.pt(HEADER_PT);
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + 50.0, row1 + 5.0), Press::Row(1));
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + cw - 10.0, row1 + 5.0), Press::Trailing(1));
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + 50.0, row1 + ui.pt(ROW_PT) + 5.0), Press::Card, "an info row has no action");
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + 50.0, row1 + ui.pt(2.0 * ROW_PT) + 5.0), Press::Row(3));
        assert_eq!(p.hit(&ui, 100.0, 1800.0, 2.0, 2.0), Press::Outside);
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + cw - 10.0, cy + 5.0), Press::Done);
        assert_eq!(p.hit(&ui, 100.0, 1800.0, cx + 10.0, cy + 5.0), Press::Back);
    }

    #[test]
    fn a_tap_returns_the_act_of_the_row_or_its_trailing_button() {
        let (mut p, ui) = panel(false);
        let (cx, cy, cw, _) = p.card(&ui, 100.0, 1800.0);
        let row1 = cy + ui.pt(TITLE_PT) + ui.pt(HEADER_PT) + 5.0;
        assert_eq!(tap(&mut p, &ui, cx + 50.0, row1), PanelEvent::Act(Act::OpenAlbum(Album::User(0))));
        assert_eq!(tap(&mut p, &ui, cx + cw - 10.0, row1), PanelEvent::Act(Act::Open(PanelKind::SmartEditor(0))));
        assert_eq!(tap(&mut p, &ui, cx + cw - 10.0, cy + 5.0), PanelEvent::Close, "Done closes");
        assert_eq!(tap(&mut p, &ui, 3.0, 3.0), PanelEvent::Close, "a tap outside closes");
        assert_eq!(tap(&mut p, &ui, cx + 10.0, cy + 5.0), PanelEvent::None, "no Back button on the first panel");
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("a very long name", 6), "a ver\u{2026}");
    }
}

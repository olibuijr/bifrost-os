//! Edit > Emoji & Symbols (Control+Command+Space): the character viewer, a floating panel with the emoji catalogue in tabs
//! and pages (`crate::emoji::CATEGORIES`, the same catalogue the on-screen keyboard's emoji page uses). A pick types the
//! emoji into the Search field when that is open, and otherwise copies it to the clipboard. The panel stays up for more
//! picks. It takes every touch while it is up; Close or Escape hides it. Pure geometry and paging live here; the shell
//! wiring is in `Shell::char_viewer_*`.
//!
//! Not yet: a search field over the catalogue, recent emoji, and the symbols tabs of the reference viewer (the catalogue
//! holds emoji only).

use super::keyboard::Out;
use super::ui::{Ui, Weight, CARD_BG, FILL_GRAY, LABEL, LINK, MAC_BODY_PT, MAC_TITLE3_PT, SEPARATOR, WHITE};
use super::{Drawn, Effect, Event, Shell};
use crate::emoji::CATEGORIES;
use crate::gfx::Surface;

/// Emoji per row and rows per page: one page is a 10 by 5 grid.
const COLS: usize = 10;
const ROWS: usize = 5;
pub const PER_PAGE: usize = COLS * ROWS;
/// Cell and glyph sizes, the side padding, the title, tab and footer rows, and the buttons, in points.
const CELL_PT: f32 = 36.0;
const GLYPH_PT: f32 = 26.0;
const TAB_GLYPH_PT: f32 = 16.0;
const PAD_PT: f32 = 16.0;
const TITLE_PT: f32 = 40.0;
const TAB_PT: f32 = 34.0;
const FOOT_PT: f32 = 32.0;
const DONE_W_PT: f32 = 56.0;
const PAGE_BTN_W_PT: f32 = 84.0;
const RADIUS_PT: f32 = 13.0;
const TAB_RADIUS_PT: f32 = 9.0;
/// Top of the panel as a share of the screen height (the Search card sits near the top, the panel below it).
const TOP_SHARE: f32 = 0.2;

/// x, y, w, h in px.
type Rect = (f32, f32, f32, f32);

/// What a touch on the panel lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Done,
    Tab(usize),
    /// index into the page's emoji
    Emoji(usize),
    Previous,
    Next,
    /// the panel itself, where no control is
    Panel,
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The viewer's state: the open tab and the page within it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharacterViewer {
    tab: usize,
    page: usize,
}

impl CharacterViewer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tab(&self) -> usize {
        self.tab
    }

    pub fn page(&self) -> usize {
        self.page
    }

    /// The emoji of the open tab, in display order.
    pub fn tab_items(&self) -> &'static [&'static str] {
        match CATEGORIES.get(self.tab) {
            Some(c) => c.emoji,
            None => &[],
        }
    }

    /// The pages in the open tab (at least one, so an empty tab still shows a page).
    pub fn pages(&self) -> usize {
        let n = self.tab_items().len();
        ((n + PER_PAGE - 1) / PER_PAGE).max(1)
    }

    /// The emoji on the open page.
    pub fn page_items(&self) -> &'static [&'static str] {
        let all = self.tab_items();
        let start = (self.page * PER_PAGE).min(all.len());
        let end = (start + PER_PAGE).min(all.len());
        &all[start..end]
    }

    /// Opens tab `tab` on its first page. A tab past the catalogue is ignored.
    pub fn select_tab(&mut self, tab: usize) {
        if tab < CATEGORIES.len() {
            self.tab = tab;
            self.page = 0;
        }
    }

    /// Turns the page forward or back. False at either end, where nothing changes.
    pub fn turn(&mut self, forward: bool) -> bool {
        if forward && self.page + 1 < self.pages() {
            self.page += 1;
            true
        } else if !forward && self.page > 0 {
            self.page -= 1;
            true
        } else {
            false
        }
    }

    fn panel(ui: &Ui, w: i32, h: i32) -> Rect {
        let pw = ui.pt(COLS as f32 * CELL_PT + 2.0 * PAD_PT);
        let ph = ui.pt(TITLE_PT + TAB_PT + ROWS as f32 * CELL_PT + FOOT_PT);
        ((w as f32 - pw) / 2.0, h as f32 * TOP_SHARE, pw, ph)
    }

    fn tab_rect(ui: &Ui, w: i32, h: i32, i: usize) -> Rect {
        let (x, y, _, _) = Self::panel(ui, w, h);
        let tw = ui.pt(COLS as f32 * CELL_PT) / CATEGORIES.len() as f32;
        (x + ui.pt(PAD_PT) + tw * i as f32, y + ui.pt(TITLE_PT), tw, ui.pt(TAB_PT))
    }

    fn cell_rect(ui: &Ui, w: i32, h: i32, i: usize) -> Rect {
        let (x, y, _, _) = Self::panel(ui, w, h);
        let cell = ui.pt(CELL_PT);
        let (col, row) = (i % COLS, i / COLS);
        (x + ui.pt(PAD_PT) + cell * col as f32, y + ui.pt(TITLE_PT + TAB_PT) + cell * row as f32, cell, cell)
    }

    fn foot_y(ui: &Ui, w: i32, h: i32) -> f32 {
        let (_, y, _, _) = Self::panel(ui, w, h);
        y + ui.pt(TITLE_PT + TAB_PT + ROWS as f32 * CELL_PT)
    }

    fn previous_rect(ui: &Ui, w: i32, h: i32) -> Rect {
        let (x, _, _, _) = Self::panel(ui, w, h);
        (x + ui.pt(PAD_PT), Self::foot_y(ui, w, h), ui.pt(PAGE_BTN_W_PT), ui.pt(FOOT_PT))
    }

    fn next_rect(ui: &Ui, w: i32, h: i32) -> Rect {
        let (x, _, pw, _) = Self::panel(ui, w, h);
        (x + pw - ui.pt(PAD_PT + PAGE_BTN_W_PT), Self::foot_y(ui, w, h), ui.pt(PAGE_BTN_W_PT), ui.pt(FOOT_PT))
    }

    fn done_rect(ui: &Ui, w: i32, h: i32) -> Rect {
        let (x, y, pw, _) = Self::panel(ui, w, h);
        (x + pw - ui.pt(PAD_PT + DONE_W_PT), y, ui.pt(DONE_W_PT), ui.pt(TITLE_PT))
    }

    /// What a touch at (x, y) lands on; None when it is off the panel.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Hit> {
        if !inside(Self::panel(ui, w, h), x, y) {
            return None;
        }
        if inside(Self::done_rect(ui, w, h), x, y) {
            return Some(Hit::Done);
        }
        if let Some(i) = (0..CATEGORIES.len()).find(|&i| inside(Self::tab_rect(ui, w, h, i), x, y)) {
            return Some(Hit::Tab(i));
        }
        if let Some(i) = (0..self.page_items().len()).find(|&i| inside(Self::cell_rect(ui, w, h, i), x, y)) {
            return Some(Hit::Emoji(i));
        }
        if self.page > 0 && inside(Self::previous_rect(ui, w, h), x, y) {
            return Some(Hit::Previous);
        }
        if self.page + 1 < self.pages() && inside(Self::next_rect(ui, w, h), x, y) {
            return Some(Hit::Next);
        }
        Some(Hit::Panel)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let (x, y, pw, ph) = Self::panel(ui, w, h);
        s.fill_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), 1.0, SEPARATOR);

        let title = ui.text("Emoji & Symbols", MAC_TITLE3_PT, Weight::Semibold, WHITE);
        s.blit_colored(&title, (x + ui.pt(PAD_PT)) as i32, (y + (ui.pt(TITLE_PT) - title.h as f32) / 2.0) as i32, LABEL.now(), 255);
        let (dx, dy, dw, dh) = Self::done_rect(ui, w, h);
        let done = ui.text("Done", MAC_BODY_PT, Weight::Semibold, WHITE);
        s.blit_colored(&done, (dx + (dw - done.w as f32) / 2.0) as i32, (dy + (dh - done.h as f32) / 2.0) as i32, LINK, 255);

        for i in 0..CATEGORIES.len() {
            let (tx, ty, tw, th) = Self::tab_rect(ui, w, h, i);
            if i == self.tab() {
                s.fill_rrect(tx + 1.0, ty + 1.0, tw - 2.0, th - 2.0, ui.pt(TAB_RADIUS_PT), FILL_GRAY);
            }
            if let Some(img) = crate::emoji::render(CATEGORIES[i].icon, ui.pt(TAB_GLYPH_PT)) {
                s.blit(&img, (tx + (tw - img.w as f32) / 2.0) as i32, (ty + (th - img.h as f32) / 2.0) as i32, 255, 255);
            }
        }

        for (i, e) in self.page_items().iter().enumerate() {
            let (cx, cy, cw, ch) = Self::cell_rect(ui, w, h, i);
            if let Some(img) = crate::emoji::render(e, ui.pt(GLYPH_PT)) {
                s.blit(&img, (cx + (cw - img.w as f32) / 2.0) as i32, (cy + (ch - img.h as f32) / 2.0) as i32, 255, 255);
            }
        }

        if self.page > 0 {
            let (bx, by, bw, bh) = Self::previous_rect(ui, w, h);
            let label = ui.text("Previous", MAC_BODY_PT, Weight::Regular, WHITE);
            s.blit_colored(&label, (bx + (bw - label.w as f32) / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32, LINK, 255);
        }
        if self.page + 1 < self.pages() {
            let (bx, by, bw, bh) = Self::next_rect(ui, w, h);
            let label = ui.text("Next", MAC_BODY_PT, Weight::Regular, WHITE);
            s.blit_colored(&label, (bx + (bw - label.w as f32) / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32, LINK, 255);
        }
        let (fy, fh) = (Self::foot_y(ui, w, h), ui.pt(FOOT_PT));
        let page = ui.text(&format!("Page {} of {}", self.page() + 1, self.pages()), MAC_BODY_PT, Weight::Regular, WHITE);
        s.blit_colored(&page, (x + (pw - page.w as f32) / 2.0) as i32, (fy + (fh - page.h as f32) / 2.0) as i32, LABEL.now(), 255);
    }
}

impl Shell {
    /// Edit > Emoji & Symbols: shows the viewer on its first tab.
    pub(super) fn char_viewer_open(&mut self) {
        self.char_viewer = Some(CharacterViewer::new());
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Escape hides the viewer. True when it was up (the key is then used up).
    pub(super) fn char_viewer_escape(&mut self) -> bool {
        if self.char_viewer.take().is_none() {
            return false;
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        true
    }

    /// A touch while the viewer is up: a release on a control acts, a touch off the panel does nothing. Every touch is
    /// used up by the viewer.
    pub(super) fn char_viewer_event(&mut self, ev: Event) -> Vec<Effect> {
        let Event::Up(x, y) = ev else { return vec![] };
        let Some(viewer) = self.char_viewer.as_ref() else { return vec![] };
        let picked = match viewer.hit(&self.ui, self.w, self.h, x, y) {
            None | Some(Hit::Panel) => None,
            Some(Hit::Done) => return self.char_viewer_done(),
            Some(Hit::Tab(i)) => {
                if let Some(v) = self.char_viewer.as_mut() {
                    v.select_tab(i);
                }
                None
            }
            Some(Hit::Previous) => {
                if let Some(v) = self.char_viewer.as_mut() {
                    v.turn(false);
                }
                None
            }
            Some(Hit::Next) => {
                if let Some(v) = self.char_viewer.as_mut() {
                    v.turn(true);
                }
                None
            }
            Some(Hit::Emoji(i)) => viewer.page_items().get(i).map(|e| (*e).to_string()),
        };
        self.drawn = Drawn::Nothing;
        match picked {
            Some(emoji) => self.char_viewer_pick(&emoji),
            None => vec![],
        }
    }

    fn char_viewer_done(&mut self) -> Vec<Effect> {
        self.char_viewer = None;
        self.drawn = Drawn::Nothing;
        vec![]
    }

    /// A pick: typed into the open Search field, or else copied to the clipboard. The viewer stays up.
    fn char_viewer_pick(&mut self, emoji: &str) -> Vec<Effect> {
        if self.quicksearch.is_shown() {
            let t = self.last_input;
            return self.quicksearch_key(Out::Text(emoji.to_string()), t);
        }
        crate::shell::clipboard::set_text(emoji);
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_tab_is_the_first_category_on_one_page_or_more() {
        let v = CharacterViewer::new();
        assert_eq!(v.tab(), 0);
        assert_eq!(v.page(), 0);
        assert_eq!(v.tab_items(), CATEGORIES[0].emoji);
        assert!(v.pages() >= 1);
        assert_eq!(v.page_items().len(), v.tab_items().len().min(PER_PAGE));
    }

    #[test]
    fn selecting_a_tab_resets_to_its_first_page_and_ignores_a_tab_past_the_end() {
        let mut v = CharacterViewer::new();
        v.select_tab(2);
        assert_eq!(v.tab(), 2);
        v.select_tab(CATEGORIES.len());
        assert_eq!(v.tab(), 2, "a tab past the catalogue is ignored");
    }

    #[test]
    fn paging_stops_at_both_ends() {
        let mut v = CharacterViewer::new();
        assert!(!v.turn(false), "no page before the first");
        let pages = v.pages();
        for _ in 1..pages {
            assert!(v.turn(true));
        }
        assert_eq!(v.page(), pages - 1);
        assert!(!v.turn(true), "no page past the last");
        // every emoji of the tab is on exactly one page
        let total: usize = (0..pages).map(|p| {
            let mut w = CharacterViewer::new();
            w.select_tab(v.tab());
            for _ in 0..p {
                w.turn(true);
            }
            w.page_items().len()
        }).sum();
        assert_eq!(total, v.tab_items().len());
    }

    #[test]
    fn every_category_has_emoji_and_an_icon() {
        for c in CATEGORIES.iter() {
            assert!(!c.emoji.is_empty());
            assert!(!c.icon.is_empty());
        }
    }
}

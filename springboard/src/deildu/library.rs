//! The library screen: category tabs and a search field on top, then a
//! scrolling page with a featured hero, a poster grid and page controls.

use super::art::Cache;
use super::site::{self, Item, CATEGORIES};
use super::scroll::Scroller;
use super::{within, Target, BLUE, FIELD, SEGMENT_ON};
use crate::gfx::{rgba, Image, Surface};
use crate::shell::ui::{centred, Ui, Weight, CELL, SECONDARY, TRACK, WHITE};

pub const MARGIN_PT: f32 = 20.0;
pub const HEADER_PT: f32 = 84.0;
const SEG_W_PT: f32 = 88.0;
const BAR_H_PT: f32 = 36.0;
const BAR_Y_PT: f32 = 36.0;
const HERO_H_PT: f32 = 320.0;
const HERO_TOP_PT: f32 = 8.0;
pub const RADIUS_PT: f32 = 12.0;
const COLS: usize = 4;
const GAP_PT: f32 = 14.0;
const SECTION_PT: f32 = 56.0;
const CAPTION_PT: f32 = 44.0;
const PAGER_PT: f32 = 60.0;
const POSTER_RATIO: f32 = 1.5;
/// 16:10 landscape cards for streaming sources (site thumbs are 800x450).
pub const LANDSCAPE_RATIO: f32 = 10.0 / 16.0;
/// Tabs: the three library categories plus streaming Sources.
pub const SOURCE_TAB: usize = 3;
pub const TABS: usize = 4;
const SOURCE_LABEL: &str = "Sources";
/// art cache kinds: grid poster, library hero, detail poster, source card,
/// source detail banner
pub const POSTER: u8 = 0;
pub const HERO: u8 = 1;
pub const DETAIL: u8 = 2;
pub const SOURCE: u8 = 3;
pub const SOURCE_DETAIL: u8 = 4;
/// Filter chips row in source mode (provider cycles separately).
pub const CHIPS: [&str; 4] = super::sources::FILTERS;
const CHIP_H_PT: f32 = 32.0;
const CHIP_ROW_PT: f32 = 44.0;

/// A listing row with its rendered labels.
pub struct Card {
    pub item: Item,
    /// None while the torrent is still being checked
    pub compat: Option<bool>,
    pub title: Image,
    info: Image,
    /// large title for the hero and the poster placeholder
    big: Image,
}

pub struct Library {
    pub category: usize,
    pub page: u32,
    pub query: String,
    pub cards: Vec<Card>,
    pub next: bool,
    pub loading: bool,
    /// a message shown instead of the grid
    pub status: Option<String>,
    pub scroll: Scroller,
    /// search field focused (keyboard shown)
    pub searching: bool,
    /// selected duration chip in source mode (mirrors sources::State::filter)
    pub chip: usize,
    tabs: Vec<Image>,
    tabs_dim: Vec<Image>,
    placeholder: Image,
    query_img: Option<Image>,
    section: Image,
    status_img: Option<Image>,
    page_img: Image,
    prev: Image,
    next_img: Image,
    play: Image,
    clear: Image,
    chips: Vec<Image>,
    chips_dim: Vec<Image>,
    provider_img: Image,
}

impl Library {
    pub fn source_mode(&self) -> bool {
        self.category == SOURCE_TAB
    }

    /// Art height ratio for grid cards: landscape on Sources.
    fn ratio(&self) -> f32 {
        if self.source_mode() { LANDSCAPE_RATIO } else { POSTER_RATIO }
    }

    pub fn new(ui: &Ui) -> Self {
        let mut names: Vec<&str> = CATEGORIES.iter().map(|c| c.0).collect();
        names.push(SOURCE_LABEL);
        let mut l = Library {
            category: 0,
            page: 0,
            query: String::new(),
            cards: vec![],
            next: false,
            loading: false,
            status: None,
            scroll: Scroller::new(),
            searching: false,
            chip: 0,
            tabs: names.iter().map(|c| ui.text(c, 15.0, Weight::Semibold, WHITE)).collect(),
            tabs_dim: names.iter().map(|c| ui.text(c, 15.0, Weight::Medium, SECONDARY)).collect(),
            placeholder: ui.text("Search", 17.0, Weight::Regular, SECONDARY),
            query_img: None,
            section: ui.text("", 20.0, Weight::Semibold, WHITE),
            status_img: None,
            page_img: ui.text("Page 1", 15.0, Weight::Regular, SECONDARY),
            prev: ui.text("‹ Previous", 17.0, Weight::Regular, BLUE),
            next_img: ui.text("Next ›", 17.0, Weight::Regular, BLUE),
            play: ui.text("Play", 17.0, Weight::Semibold, super::BLACK),
            clear: ui.text("×", 20.0, Weight::Semibold, WHITE),
            chips: CHIPS.iter().map(|c| ui.text(c, 14.0, Weight::Semibold, WHITE)).collect(),
            chips_dim: CHIPS.iter().map(|c| ui.text(c, 14.0, Weight::Medium, SECONDARY)).collect(),
            provider_img: ui.text("", 15.0, Weight::Semibold, WHITE),
        };
        l.refresh_labels(ui);
        l
    }

    /// Cards visible in the grid: confirmed compatible ones, in listing order.
    pub fn visible(&self) -> Vec<usize> {
        self.cards.iter().enumerate().filter(|(_, c)| c.compat == Some(true)).map(|(i, _)| i).collect()
    }

    /// The featured card (first visible) and the rest for the grid.
    pub fn split(&self) -> (Option<usize>, Vec<usize>) {
        let mut v = self.visible();
        if v.is_empty() {
            (None, v)
        } else {
            let hero = v.remove(0);
            (Some(hero), v)
        }
    }

    pub fn checking(&self) -> usize {
        self.cards.iter().filter(|c| c.compat.is_none()).count()
    }

    pub fn set_status(&mut self, ui: &Ui, s: Option<&str>) {
        self.status = s.map(str::to_string);
        self.status_img = s.map(|s| ui.text(s, 17.0, Weight::Regular, SECONDARY));
    }

    pub fn set_query(&mut self, ui: &Ui, q: &str) {
        self.query = q.to_string();
        self.query_img = (!q.is_empty()).then(|| ui.text(&ui.fit(q, 17.0, ui.pt(300.0)), 17.0, Weight::Regular, WHITE));
    }

    /// Replaces the page's items; compatibility starts from the name hint.
    pub fn set_items(&mut self, ui: &Ui, w: i32, items: Vec<Item>, next: bool, known: &dyn Fn(&str) -> Option<bool>) {
        let cw = Self::card_w(ui, w);
        self.cards = items
            .into_iter()
            .map(|item| {
                let compat = known(&item.id).or_else(|| site::name_hint(&item.title));
                let title = site::clean_title(&item.title);
                Card {
                    compat,
                    title: ui.text(&ui.fit(&title, 14.0, cw), 14.0, Weight::Medium, WHITE),
                    info: ui.text(&ui.fit(&item.info, 12.0, cw), 12.0, Weight::Regular, SECONDARY),
                    big: ui.text(&ui.fit(&title, 28.0, w as f32 - 2.0 * ui.pt(MARGIN_PT) - ui.pt(40.0)), 28.0, Weight::Bold, WHITE),
                    item,
                }
            })
            .collect();
        self.next = next;
        self.scroll.jump_top();
        self.refresh_labels(ui);
    }

    pub fn set_compat(&mut self, ui: &Ui, id: &str, ok: bool) {
        if let Some(c) = self.cards.iter_mut().find(|c| c.item.id == id) {
            c.compat = Some(ok);
        }
        self.refresh_labels(ui);
    }

    /// Replaces the page's streaming items; playability starts unknown and
    /// the enrich pass confirms it card by card.
    pub fn set_source_items(&mut self, ui: &Ui, w: i32, items: Vec<Item>, next: bool) {
        let cw = Self::card_w(ui, w);
        self.cards = items
            .into_iter()
            .map(|item| {
                let title = site::clean_title(&item.title);
                Card {
                    compat: None,
                    title: ui.text(&ui.fit(&title, 14.0, cw), 14.0, Weight::Medium, WHITE),
                    info: ui.text(&ui.fit(&item.info, 12.0, cw), 12.0, Weight::Regular, SECONDARY),
                    big: ui.text(&ui.fit(&title, 28.0, w as f32 - 2.0 * ui.pt(MARGIN_PT) - ui.pt(40.0)), 28.0, Weight::Bold, WHITE),
                    item,
                }
            })
            .collect();
        self.next = next;
        self.scroll.jump_top();
        self.refresh_labels(ui);
    }

    pub fn set_provider_label(&mut self, ui: &Ui, name: &str) {
        self.provider_img = ui.text(name, 15.0, Weight::Semibold, WHITE);
    }

    fn refresh_labels(&mut self, ui: &Ui) {
        let name = if self.source_mode() { SOURCE_LABEL } else { CATEGORIES[self.category].0 };
        let mut title = if self.query.is_empty() { format!("Latest {}", name.to_lowercase()) } else { format!("Results for “{}”", self.query) };
        let checking = self.checking();
        if checking > 0 {
            title.push_str(&format!(" · checking {checking}"));
        }
        self.section = ui.text(&ui.fit(&title, 20.0, ui.pt(700.0)), 20.0, Weight::Semibold, WHITE);
        self.page_img = ui.text(&format!("Page {}", self.page + 1), 15.0, Weight::Regular, SECONDARY);
    }

    // --- geometry (px) ---

    pub fn seg_rect(ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        (ui.pt(MARGIN_PT) + ui.pt(SEG_W_PT) * i as f32, ui.pt(BAR_Y_PT), ui.pt(SEG_W_PT), ui.pt(BAR_H_PT))
    }

    pub fn search_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let x = ui.pt(MARGIN_PT) + ui.pt(SEG_W_PT) * TABS as f32 + ui.pt(20.0);
        (x, ui.pt(BAR_Y_PT), w as f32 - x - ui.pt(MARGIN_PT), ui.pt(BAR_H_PT))
    }

    /// Top of the chips row: just under the hero, above the section title.
    fn chip_y(ui: &Ui) -> f32 {
        ui.pt(HERO_TOP_PT + HERO_H_PT + 8.0)
    }

    /// Filter chips row (source mode only), content-top relative.
    pub fn chip_rect(ui: &Ui, _w: i32, i: usize) -> (f32, f32, f32, f32) {
        let widths: [f32; 4] = [64.0, 110.0, 84.0, 96.0];
        let mut x = ui.pt(MARGIN_PT);
        for k in 0..i {
            x += ui.pt(widths[k]) + ui.pt(8.0);
        }
        (x, Self::chip_y(ui), ui.pt(widths[i]), ui.pt(CHIP_H_PT))
    }

    /// Provider switch button at the right of the chips row.
    pub fn provider_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        (w as f32 - ui.pt(MARGIN_PT) - ui.pt(150.0), Self::chip_y(ui), ui.pt(150.0), ui.pt(CHIP_H_PT))
    }

    pub fn card_w(ui: &Ui, w: i32) -> f32 {
        (w as f32 - 2.0 * ui.pt(MARGIN_PT) - ui.pt(GAP_PT) * (COLS - 1) as f32) / COLS as f32
    }

    /// Hero rect relative to the content's top.
    pub fn hero_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        (ui.pt(MARGIN_PT), ui.pt(HERO_TOP_PT), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(HERO_H_PT))
    }

    /// The hero's Play button, content-relative.
    pub fn hero_play_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let (x, y, _, h) = Self::hero_rect(ui, w);
        (x + ui.pt(24.0), y + h - ui.pt(24.0) - ui.pt(40.0), ui.pt(96.0), ui.pt(40.0))
    }

    fn grid_top(&self, ui: &Ui) -> f32 {
        ui.pt(HERO_TOP_PT + HERO_H_PT + SECTION_PT) + if self.source_mode() { ui.pt(CHIP_ROW_PT) } else { 0.0 }
    }

    /// Row pitch of the grid in px.
    pub fn row_h(&self, ui: &Ui, w: i32) -> f32 {
        Self::card_w(ui, w) * self.ratio() + ui.pt(CAPTION_PT)
    }

    /// Grid cell `i` (0-based within the grid), content-relative; the poster is the top part.
    pub fn cell_rect(&self, ui: &Ui, w: i32, i: usize) -> (f32, f32, f32, f32) {
        let cw = Self::card_w(ui, w);
        let (c, r) = ((i % COLS) as f32, (i / COLS) as f32);
        (ui.pt(MARGIN_PT) + c * (cw + ui.pt(GAP_PT)), self.grid_top(ui) + r * self.row_h(ui, w), cw, self.row_h(ui, w))
    }

    pub fn poster_size(&self, ui: &Ui, w: i32) -> (usize, usize) {
        let cw = Self::card_w(ui, w);
        (cw.round() as usize, (cw * self.ratio()).round() as usize)
    }

    pub fn hero_size(ui: &Ui, w: i32) -> (usize, usize) {
        let (_, _, hw, hh) = Self::hero_rect(ui, w);
        (hw.round() as usize, hh.round() as usize)
    }

    fn pager_top(&self, ui: &Ui, w: i32) -> f32 {
        let n = self.split().1.len();
        self.grid_top(ui) + n.div_ceil(COLS) as f32 * self.row_h(ui, w) + ui.pt(8.0)
    }

    /// Height of the scrolling content.
    pub fn content_h(&self, ui: &Ui, w: i32) -> f32 {
        if self.visible().is_empty() {
            return 0.0;
        }
        self.pager_top(ui, w) + ui.pt(PAGER_PT + 16.0)
    }

    fn prev_rect(&self, ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        (ui.pt(MARGIN_PT), self.pager_top(ui, w), ui.pt(140.0), ui.pt(PAGER_PT))
    }

    fn next_rect(&self, ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        (w as f32 - ui.pt(MARGIN_PT) - ui.pt(140.0), self.pager_top(ui, w), ui.pt(140.0), ui.pt(PAGER_PT))
    }

    pub fn content_top(ui: &Ui) -> f32 {
        ui.pt(HEADER_PT)
    }

    /// Keeps the scroller's range in step with the content and viewport.
    pub fn fit_scroll(&mut self, ui: &Ui, w: i32, bottom: f32) {
        let max = self.content_h(ui, w) - (bottom - Self::content_top(ui));
        self.scroll.set_max(max);
    }

    /// Cards whose art is wanted now: the hero and the grid cells on screen.
    pub fn on_screen(&self, ui: &Ui, w: i32, bottom: f32) -> Vec<(usize, u8)> {
        let (hero, grid) = self.split();
        let mut out: Vec<(usize, u8)> = hero.map(|h| (h, HERO)).into_iter().collect();
        let (top, view_h) = (self.scroll.offset(), bottom - Self::content_top(ui));
        let kind = if self.source_mode() { SOURCE } else { POSTER };
        for (gi, &ci) in grid.iter().enumerate() {
            let (_, y, _, h) = self.cell_rect(ui, w, gi);
            if y + h > top && y < top + view_h {
                out.push((ci, kind));
            }
        }
        out
    }

    pub fn hit(&self, ui: &Ui, w: i32, bottom: f32, x: f32, y: f32) -> Option<Target> {
        if let Some(i) = (0..TABS).find(|&i| within(x, y, Self::seg_rect(ui, i))) {
            return Some(Target::Tab(i));
        }
        let sr = Self::search_rect(ui, w);
        if within(x, y, sr) {
            let clear = (sr.0 + sr.2 - ui.pt(44.0), sr.1, ui.pt(44.0), sr.3);
            return Some(if !self.query.is_empty() && within(x, y, clear) { Target::Clear } else { Target::Search });
        }
        if y < Self::content_top(ui) || y >= bottom {
            return None;
        }
        if self.source_mode() {
            // Chips row scrolls with the content, just under the header.
            let cy = y - Self::content_top(ui) + self.scroll.offset();
            if within(x, cy, Self::provider_rect(ui, w)) {
                return Some(Target::Provider);
            }
            for i in 0..CHIPS.len() {
                if within(x, cy, Self::chip_rect(ui, w, i)) {
                    return Some(Target::Chip(i));
                }
            }
        }
        let cy = y - Self::content_top(ui) + self.scroll.offset();
        let (hero, grid) = self.split();
        if hero.is_some() {
            if within(x, cy, Self::hero_play_rect(ui, w)) {
                return Some(Target::HeroPlay);
            }
            if within(x, cy, Self::hero_rect(ui, w)) {
                return Some(Target::Hero);
            }
        }
        for (gi, &ci) in grid.iter().enumerate() {
            if within(x, cy, self.cell_rect(ui, w, gi)) {
                return Some(Target::Card(ci));
            }
        }
        if hero.is_some() {
            if self.page > 0 && within(x, cy, self.prev_rect(ui, w)) {
                return Some(Target::Prev);
            }
            if self.next && within(x, cy, self.next_rect(ui, w)) {
                return Some(Target::Next);
            }
        }
        Some(Target::Content)
    }

    // --- drawing ---

    fn draw_header(&self, s: &mut Surface, ui: &Ui, w: i32, pressed: Option<Target>) {
        let (x0, y, sw, sh) = Self::seg_rect(ui, 0);
        s.fill_rrect(x0, y, sw * TABS as f32, sh, ui.pt(9.0), TRACK);
        for i in 0..TABS {
            let (x, y, sw, sh) = Self::seg_rect(ui, i);
            let on = i == self.category;
            if on {
                s.fill_rrect(x + ui.pt(2.0), y + ui.pt(2.0), sw - ui.pt(4.0), sh - ui.pt(4.0), ui.pt(7.0), SEGMENT_ON);
            }
            let img = if on || pressed == Some(Target::Tab(i)) { &self.tabs[i] } else { &self.tabs_dim[i] };
            s.blit(img, (x + (sw - img.w as f32) / 2.0) as i32, (y + (sh - img.h as f32) / 2.0) as i32, 255, 255);
        }
        let (x, y, fw, fh) = Self::search_rect(ui, w);
        s.fill_rrect(x, y, fw, fh, ui.pt(10.0), FIELD);
        let mid = y + fh / 2.0;
        // magnifier: a stroked circle with a handle
        let r = ui.pt(7.0);
        let cx = x + ui.pt(12.0) + r;
        s.stroke_rrect(cx - r, mid - r - ui.pt(1.0), 2.0 * r, 2.0 * r, r, ui.pt(1.8), SECONDARY);
        for k in 0..(ui.pt(6.0) as i32) {
            let d = k as f32;
            s.fill_rrect(cx + r * 0.7 + d, mid - ui.pt(1.0) + r * 0.7 + d, ui.pt(1.8), ui.pt(1.8), ui.pt(0.9), SECONDARY);
        }
        let tx = cx + r + ui.pt(10.0);
        match &self.query_img {
            Some(q) => {
                s.blit(q, tx as i32, (mid - q.h as f32 / 2.0) as i32, 255, 255);
                let cl = &self.clear;
                s.blit(cl, (x + fw - ui.pt(22.0) - cl.w as f32 / 2.0) as i32, (mid - cl.h as f32 / 2.0) as i32, 255, 255);
            }
            None => s.blit(&self.placeholder, tx as i32, (mid - self.placeholder.h as f32 / 2.0) as i32, 255, 255),
        }
        if self.searching {
            let qx = tx + self.query_img.as_ref().map_or(0.0, |q| q.w as f32 - ui.pt(2.0));
            s.fill_rrect(qx, mid - ui.pt(11.0), ui.pt(2.0), ui.pt(22.0), ui.pt(1.0), BLUE);
        }
    }

    /// Darkens the lower part of the hero so the title reads over the art.
    /// Black over the black page outside the rounded corners is invisible.
    fn gradient(s: &mut Surface, x: f32, y: f32, w: f32, h: f32) {
        let rows = (h * 0.55) as i32;
        for i in 0..rows {
            let a = (i as f32 / rows as f32).powf(1.5) * 230.0;
            s.fill_rrect(x, y + h - rows as f32 + i as f32, w, 1.0, 0.0, rgba(0, 0, 0, a as u8));
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, bottom: f32, art: &mut Cache, pressed: Option<Target>) {
        let top = Self::content_top(ui);
        let (hero, grid) = self.split();
        {
            let mut band = s.band(top as usize);
            let off = self.scroll.offset();
            let cy = |y: f32| y - off + top;
            if let Some(hi) = hero {
                let card = &self.cards[hi];
                let (x, y, hw, hh) = Self::hero_rect(ui, w);
                let (y, r) = (cy(y), ui.pt(RADIUS_PT));
                match art.get(&card.item.id, HERO) {
                    Some(img) => band.blit(img, x as i32, y as i32, 255, 255),
                    None => band.fill_rrect(x, y, hw, hh, r, CELL),
                }
                Self::gradient(&mut band, x, y, hw, hh);
                let (px, py, pw, ph) = Self::hero_play_rect(ui, w);
                let py = cy(py);
                band.fill_rrect(px, py, pw, ph, ui.pt(8.0), if pressed == Some(Target::HeroPlay) { SECONDARY } else { WHITE });
                band.blit(&self.play, (px + (pw - self.play.w as f32) / 2.0) as i32, (py + (ph - self.play.h as f32) / 2.0) as i32, 255, 255);
                let info_y = py - ui.pt(12.0) - card.info.h as f32;
                let big_y = info_y - ui.pt(4.0) - card.big.h as f32;
                band.blit(&card.big, (x + ui.pt(24.0)) as i32, big_y as i32, 255, 255);
                band.blit(&card.info, (x + ui.pt(24.0)) as i32, info_y as i32, 255, 255);
                if pressed == Some(Target::Hero) {
                    band.fill_rrect(x, y, hw, hh, r, rgba(255, 255, 255, 40));
                }
                let sec = &self.section;
                band.blit(sec, x as i32, (cy(self.grid_top(ui)) - ui.pt(12.0) - sec.h as f32) as i32, 255, 255);
            }
            let (pw, ph) = self.poster_size(ui, w);
            let kind = if self.source_mode() { SOURCE } else { POSTER };
            if self.source_mode() {
                for i in 0..CHIPS.len() {
                    let (x, y, cw, ch) = Self::chip_rect(ui, w, i);
                    let y = cy(y);
                    let on = i == self.chip;
                    band.fill_rrect(x, y, cw, ch, ui.pt(8.0), if on { SEGMENT_ON } else { TRACK });
                    let img = if on { &self.chips[i] } else { &self.chips_dim[i] };
                    band.blit(img, (x + (cw - img.w as f32) / 2.0) as i32, (y + (ch - img.h as f32) / 2.0) as i32, 255, 255);
                }
                let (x, y, pw2, ph2) = Self::provider_rect(ui, w);
                let y = cy(y);
                band.fill_rrect(x, y, pw2, ph2, ui.pt(8.0), TRACK);
                band.blit(&self.provider_img, (x + (pw2 - self.provider_img.w as f32) / 2.0) as i32, (y + (ph2 - self.provider_img.h as f32) / 2.0) as i32, 255, 255);
            }
            for (gi, &ci) in grid.iter().enumerate() {
                let card = &self.cards[ci];
                let (x, y, cw, _) = self.cell_rect(ui, w, gi);
                let y = cy(y);
                if y + ph as f32 + ui.pt(CAPTION_PT) < top || y > bottom {
                    continue;
                }
                let tint = if pressed == Some(Target::Card(ci)) { 150 } else { 255 };
                match art.get(&card.item.id, kind) {
                    Some(img) => band.blit(img, x as i32, y as i32, tint, 255),
                    None => {
                        band.fill_rrect(x, y, pw as f32, ph as f32, ui.pt(10.0), if tint == 255 { CELL } else { TRACK });
                        let t = &card.title;
                        band.blit(t, (x + (cw - t.w as f32) / 2.0) as i32, (y + ph as f32 / 2.0 - t.h as f32 / 2.0) as i32, 255, 255);
                    }
                }
                band.blit(&card.title, x as i32, (y + ph as f32 + ui.pt(6.0)) as i32, 255, 255);
                band.blit(&card.info, x as i32, (y + ph as f32 + ui.pt(6.0) + card.title.h as f32) as i32, 255, 255);
            }
            if hero.is_some() {
                let (px, py, pw, ph) = self.prev_rect(ui, w);
                let mid = cy(py) + ph / 2.0;
                if self.page > 0 {
                    band.blit(&self.prev, (px + ui.pt(4.0)) as i32, (mid - self.prev.h as f32 / 2.0) as i32, 255, if pressed == Some(Target::Prev) { 120 } else { 255 });
                }
                if self.next {
                    let (nx, _, nw, _) = self.next_rect(ui, w);
                    band.blit(&self.next_img, (nx + nw - ui.pt(4.0) - self.next_img.w as f32) as i32, (mid - self.next_img.h as f32 / 2.0) as i32, 255, if pressed == Some(Target::Next) { 120 } else { 255 });
                }
                centred(&mut band, &self.page_img, w / 2, (mid - self.page_img.h as f32 / 2.0) as i32);
                let _ = pw;
            } else if let Some(st) = &self.status_img {
                centred(&mut band, st, w / 2, (top + ui.pt(120.0)) as i32);
            }
        }
        s.fill_rect(0, bottom as usize, w as usize, (s.h as f32 - bottom).max(0.0) as usize, super::BLACK);
        s.fill_rect(0, 0, w as usize, top as usize, super::BLACK);
        self.draw_header(s, ui, w, pressed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn item(id: &str, title: &str) -> Item {
        Item { id: id.into(), title: title.into(), url: format!("https://www.deildu.net/details.php?id={id}"), info: "1 GB · 3 seeders".into(), torrent_url: String::new() }
    }

    fn lib(ui: &Ui, n: usize) -> Library {
        let mut l = Library::new(ui);
        let items = (0..n).map(|i| item(&i.to_string(), &format!("Film.{i}.1080p.x264.MP4-G"))).collect();
        l.set_items(ui, 1200, items, true, &|_| None);
        l
    }

    #[test]
    fn header_geometry_fits_the_width() {
        let ui = ui();
        let l = lib(&ui, 1);
        let (x, _, w, _) = Library::search_rect(&ui, 1200);
        assert!(x + w <= 1200.0 - ui.pt(MARGIN_PT) + 0.5);
        let (sx, _, sw, _) = Library::seg_rect(&ui, 2);
        assert!(sx + sw < x);
        let cw = Library::card_w(&ui, 1200);
        let (lx, _, lw, _) = l.cell_rect(&ui, 1200, 3);
        assert!((lx + lw - (1200.0 - ui.pt(MARGIN_PT))).abs() < 1.0, "last column ends at the margin");
        assert_eq!(l.poster_size(&ui, 1200), (cw.round() as usize, (cw * 1.5).round() as usize));
    }

    #[test]
    fn name_hints_and_checks_split_hero_and_grid() {
        let ui = ui();
        let mut l = Library::new(&ui);
        let items = vec![item("1", "A.x265"), item("2", "B.x264.MP4"), item("3", "C.x264"), item("4", "D.x264.MP4")];
        l.set_items(&ui, 1200, items, false, &|id| (id == "3").then_some(true));
        assert_eq!(l.visible(), vec![1, 2, 3]);
        assert_eq!(l.checking(), 0);
        assert_eq!(l.split(), (Some(1), vec![2, 3]));
        l.set_items(&ui, 1200, vec![item("5", "E.x264")], false, &|_| None);
        assert_eq!(l.checking(), 1);
        assert!(l.visible().is_empty());
        assert_eq!(l.content_h(&ui, 1200), 0.0);
        l.set_compat(&ui, "5", true);
        assert_eq!(l.split(), (Some(0), vec![]));
    }

    #[test]
    fn hits_cover_tabs_search_hero_cards_and_pager() {
        let ui = ui();
        let mut l = lib(&ui, 10);
        l.page = 1;
        let bottom = 1920.0 - ui.pt(28.0);
        let c = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (x, y) = c(Library::seg_rect(&ui, 1));
        assert_eq!(l.hit(&ui, 1200, bottom, x, y), Some(Target::Tab(1)));
        let sr = Library::search_rect(&ui, 1200);
        assert_eq!(l.hit(&ui, 1200, bottom, sr.0 + 10.0, sr.1 + 10.0), Some(Target::Search));
        l.set_query(&ui, "x");
        assert_eq!(l.hit(&ui, 1200, bottom, sr.0 + sr.2 - 10.0, sr.1 + 10.0), Some(Target::Clear));
        let top = Library::content_top(&ui);
        let (hx, hy) = c(Library::hero_rect(&ui, 1200));
        assert_eq!(l.hit(&ui, 1200, bottom, hx, hy + top), Some(Target::Hero));
        let (px, py) = c(Library::hero_play_rect(&ui, 1200));
        assert_eq!(l.hit(&ui, 1200, bottom, px, py + top), Some(Target::HeroPlay));
        let (cx, cy) = c(l.cell_rect(&ui, 1200, 0));
        assert!(cy + top < bottom);
        assert_eq!(l.hit(&ui, 1200, bottom, cx, cy + top), Some(Target::Card(1)), "grid cell 0 is the second visible card");
        // scroll down to the pager
        let content = l.content_h(&ui, 1200);
        l.fit_scroll(&ui, 1200, bottom);
        l.scroll.down(900.0, 0.0);
        l.scroll.moved(900.0 - (content - (bottom - top)), 100.0);
        l.scroll.up(900.0 - (content - (bottom - top)), 1000.0);
        let (ppx, ppy) = c(l.prev_rect(&ui, 1200));
        assert_eq!(l.hit(&ui, 1200, bottom, ppx, ppy - l.scroll.offset() + top), Some(Target::Prev));
        let (nx, ny) = c(l.next_rect(&ui, 1200));
        assert_eq!(l.hit(&ui, 1200, bottom, nx, ny - l.scroll.offset() + top), Some(Target::Next));
        assert_eq!(l.hit(&ui, 1200, bottom, 600.0, 1919.0), None, "home zone");
    }

    #[test]
    fn on_screen_follows_the_scroll_offset() {
        let ui = ui();
        let mut l = lib(&ui, 30);
        let bottom = 1920.0 - ui.pt(28.0);
        l.fit_scroll(&ui, 1200, bottom);
        let first = l.on_screen(&ui, 1200, bottom);
        assert_eq!(first[0], (0, HERO));
        assert!(first.contains(&(1, POSTER)) && !first.contains(&(29, POSTER)));
        l.scroll.down(1800.0, 0.0);
        l.scroll.moved(100.0, 100.0);
        l.scroll.moved(-1500.0, 200.0);
        l.scroll.up(-1500.0, 1200.0);
        let later = l.on_screen(&ui, 1200, bottom);
        assert!(later.contains(&(29, POSTER)) && !later.contains(&(1, POSTER)));
        assert!(l.content_h(&ui, 1200) > 1920.0);
    }

    #[test]
    fn source_mode_uses_landscape_cards_and_chips() {
        let ui = ui();
        let mut l = Library::new(&ui);
        l.category = SOURCE_TAB;
        assert!(l.source_mode());
        let items = vec![item("s1", "Sample one"), item("s2", "Sample two")];
        l.set_source_items(&ui, 1200, items, true);
        assert_eq!(l.cards.len(), 2);
        assert!(l.visible().is_empty(), "playability unknown until enrich");
        let cw = Library::card_w(&ui, 1200);
        assert_eq!(l.poster_size(&ui, 1200), (cw.round() as usize, (cw * LANDSCAPE_RATIO).round() as usize));
        assert!(l.row_h(&ui, 1200) < Library::card_w(&ui, 1200) * POSTER_RATIO + ui.pt(CAPTION_PT));
        let bottom = 1920.0 - ui.pt(28.0);
        let top = Library::content_top(&ui);
        let c = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (px, py) = c(Library::provider_rect(&ui, 1200));
        assert_eq!(l.hit(&ui, 1200, bottom, px, py + top), Some(Target::Provider));
        let (hx, hy) = c(Library::chip_rect(&ui, 1200, 2));
        assert_eq!(l.hit(&ui, 1200, bottom, hx, hy + top), Some(Target::Chip(2)));
        l.set_compat(&ui, "s1", true);
        assert_eq!(l.visible(), vec![0]);
        // chips sit between the hero and the grid, overlapping neither
        let (_, hy, _, hh) = Library::hero_rect(&ui, 1200);
        let (_, chy, _, chh) = Library::chip_rect(&ui, 1200, 0);
        assert!(chy >= hy + hh, "chips below the hero");
        let (_, gy, _, _) = l.cell_rect(&ui, 1200, 0);
        assert!(chy + chh < gy, "chips above the grid");
    }

    #[test]
    fn draws_with_and_without_items() {        let ui = ui();
        let mut buf = vec![0u32; 1200 * 1920];
        let mut art = Cache::new(1 << 20);
        let bottom = 1920.0 - ui.pt(28.0);
        let mut l = Library::new(&ui);
        l.set_status(&ui, Some("Loading…"));
        l.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, bottom, &mut art, None);
        let l = lib(&ui, 9);
        art.insert("0", HERO, Image { w: 10, h: 10, px: vec![WHITE; 100] });
        l.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, bottom, &mut art, Some(Target::Card(1)));
        let (x, y, _, _) = Library::hero_rect(&ui, 1200);
        assert_eq!(buf[(y + Library::content_top(&ui)) as usize * 1200 + x as usize + 2], WHITE, "hero art drawn");
    }
}

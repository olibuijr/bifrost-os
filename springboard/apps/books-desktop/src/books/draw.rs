//! Books drawing: the library grid, the page, the reader's bars and the
//! contents / bookmarks / search / display panels.

use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{MAC_CALLOUT_PT, MAC_HEADLINE_PT};
use bifrost_app::widgets;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE3_PT};
use super::layout::{Kind, LINE_HEIGHT};
use super::store::{Theme, SIZES};
use super::*;
use springboard::gfx::pack;
use bifrost_app::controls::prefs_layout::m;
use springboard::shell::cache::TextCache;
use springboard::shell::ui::{centred, Weight, CARD_BG, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, SEPARATOR_C, TRACK_C, WHITE};
use bifrost_app::kit::ui::{AVATAR_FG, BLACK, DESTRUCTIVE, SYSTEM_ORANGE, SYSTEM_YELLOW, TINT};
use bifrost_app::kit::ui::{READER_DARK_BG, READER_DARK_DIM, READER_DARK_INK, READER_DARK_SEP, READER_SEPIA_ACCENT, READER_SEPIA_BG, READER_SEPIA_DIM, READER_SEPIA_INK, READER_SEPIA_RAISED, READER_SEPIA_SEP, READER_WHITE_ACCENT, READER_WHITE_DIM, READER_WHITE_INK, READER_WHITE_RAISED, READER_WHITE_SEP};

const BLUE: u32 = TINT;
const ORANGE: u32 = SYSTEM_ORANGE;
const RED: u32 = DESTRUCTIVE;
/// Tabs, swatches, size buttons and the search field: the small tile radius of `prefs_layout::m`.
const RADIUS_PT: f32 = m::ICON_RADIUS;
/// Cards (the loading box, the error banner): the grouped-card radius.
const CARD_R_PT: f32 = m::CARD_RADIUS;

/// The colours of a reading theme.
#[derive(Clone, Copy, Debug)]
pub(super) struct Palette {
    pub bg: u32,
    pub fg: u32,
    pub dim: u32,
    pub sep: u32,
    pub raised: u32,
    pub accent: u32,
}

pub(super) fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::White => Palette { bg: WHITE, fg: READER_WHITE_INK, dim: READER_WHITE_DIM, sep: READER_WHITE_SEP, raised: READER_WHITE_RAISED, accent: READER_WHITE_ACCENT },
        Theme::Sepia => Palette { bg: READER_SEPIA_BG, fg: READER_SEPIA_INK, dim: READER_SEPIA_DIM, sep: READER_SEPIA_SEP, raised: READER_SEPIA_RAISED, accent: READER_SEPIA_ACCENT },
        Theme::Dark => Palette { bg: READER_DARK_BG, fg: READER_DARK_INK, dim: READER_DARK_DIM, sep: READER_DARK_SEP, raised: AVATAR_FG, accent: TINT },
    }
}

/// Draws `text` with its top-left at `(x, y)`; returns its size.
#[allow(clippy::too_many_arguments)]
fn put(labels: &mut TextCache, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) -> (usize, usize) {
    let img = labels.get(ui, text, pt, weight, color);
    let size = (img.w, img.h);
    s.blit(img, x as i32, y as i32, 255, 255);
    size
}

/// Draws `text` centred on `cx` and vertically on `cy`.
#[allow(clippy::too_many_arguments)]
fn put_mid(labels: &mut TextCache, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, cx: f32, cy: f32) {
    let img = labels.get(ui, text, pt, weight, color);
    let y = (cy - img.h as f32 / 2.0) as i32;
    centred(s, img, cx as i32, y);
}

/// Draws `text` ending at `right`, vertically centred on `cy`.
#[allow(clippy::too_many_arguments)]
fn put_right(labels: &mut TextCache, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, right: f32, cy: f32) {
    let img = labels.get(ui, text, pt, weight, color);
    s.blit(img, (right - img.w as f32) as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
}

/// The darkening on a cover's spine edge: a translucent overlay, not a background.
fn spine_shade() -> u32 {
    fade(BLACK, 56)
}

/// A muted cover colour derived from the title.
fn cover_colour(title: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in title.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    const SET: [(u8, u8, u8); 6] = [(142, 68, 73), (66, 103, 130), (88, 120, 84), (128, 98, 60), (96, 78, 124), (60, 60, 68)];
    let (r, g, b) = SET[(h % SET.len() as u32) as usize];
    pack(r, g, b, 255)
}

impl Books {
    // --- library ---

    pub(super) fn draw_library(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (self.w as usize, self.h as usize);
        s.fill_rect(0, 0, w, h, DETAIL_BG.now());
        let side = self.side(ui);
        let m = side + ui.pt(MARGIN_PT);
        let mid = side + (self.w as f32 - side) / 2.0;
        let top = self.grid_top(ui);
        let bottom = self.h as f32 - home_zone(ui);
        let (cols, tw, ch, _, _) = self.grid(ui);
        if self.shelf.is_empty() {
            let empty_y = top + (bottom - top) * 0.2;
            put_mid(&mut self.labels, s, ui, "No Books", MAC_TITLE1_PT, Weight::Bold, LABEL_2.now(), mid, empty_y);
            put_mid(&mut self.labels, s, ui, "Put .epub or .pdf files in Files or Downloads.", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), mid, empty_y + ui.pt(SPACE_HUGE));
        } else if self.visible().is_empty() {
            let title = if self.library_search.is_empty() { format!("No {}", self.filter.name()) } else { "No Results".to_string() };
            put_mid(&mut self.labels, s, ui, &title, MAC_TITLE1_PT, Weight::Bold, LABEL_2.now(), mid, top + (bottom - top) * 0.2);
        }
        let mut content = s.rows(top as usize, bottom as usize);
        let n = self.reading_count(ui);
        if n > 0 {
            let base = -self.shelf_scroll.offset();
            put(&mut self.labels, &mut content, ui, "Reading Now", MAC_TITLE3_PT, Weight::Bold, LABEL.now(), m, base + ui.pt(SPACE_XS));
            let lib = base + self.library_offset(ui) - ui.pt(SPACE_HUGE);
            put(&mut self.labels, &mut content, ui, "Library", MAC_TITLE3_PT, Weight::Bold, LABEL.now(), m, lib + ui.pt(SPACE_XS));
        }
        for (k, i) in self.visible().into_iter().enumerate() {
            let (cx, cy, _, _) = self.cover_rect(ui, k);
            if cy + ch + ui.pt(64.0) < top || cy > bottom {
                continue;
            }
            self.draw_cover(&mut content, ui, i, (cx, cy, tw, ch));
            let path = self.shelf[i].path.clone();
            let progress = self.store.entry(&path).map_or(0, |e| e.progress);
            let title = self.shelf[i].title.clone();
            let lines = ui.wrap(&title, MAC_FOOTNOTE_PT, Weight::Medium, tw);
            for (k, line) in lines.iter().take(2).enumerate() {
                let line = ui.fit(line, MAC_FOOTNOTE_PT, tw);
                put(&mut self.labels, &mut content, ui, &line, MAC_FOOTNOTE_PT, Weight::Medium, LABEL.now(), cx, cy + ch + ui.pt(8.0 + 17.0 * k as f32));
            }
            let sub = if progress == 0 { "New".to_string() } else { format!("{}%", progress / 10) };
            put(&mut self.labels, &mut content, ui, &sub, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), cx, cy + ch + ui.pt(44.0));
        }
        let _ = cols;
        drop(content);
        if side > 0.0 {
            widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), -(bifrost_app::layout::UNIFIED_BAR_PT + 4.0), None);
            s.fill_rect(side as usize - 1, 0, 1, h, SEPARATOR_C.now());
        }
        self.draw_toolbar(s, ui);
        if let Some(r) = self.library_search_rect(ui) {
            let mut field = bifrost_app::controls::TextField::new();
            field.push_str(self.library_search.text());
            widgets::search_field(s, ui, &field, r, "Search", self.library_search.focused);
        }
        if self.loading.is_some() {
            let (bw, bh) = (self.w as f32 * 0.25, ui.pt(SPACE_XXXL * 2.0));
            let (bx, by) = ((self.w as f32 - bw) / 2.0, self.h as f32 / 2.0 - bh / 2.0);
            s.fill_rrect(bx, by, bw, bh, ui.pt(CARD_R_PT), CARD_BG.now());
            put_mid(&mut self.labels, s, ui, "Opening…", MAC_BODY_PT, Weight::Medium, LABEL.now(), self.w as f32 / 2.0, by + bh / 2.0);
        } else if let Some(e) = self.error.clone() {
            let text = ui.fit(&e, MAC_BODY_PT, self.w as f32 - ui.pt(60.0));
            let (bh, by) = (ui.pt(48.0), self.h as f32 - home_zone(ui) - ui.pt(70.0));
            s.fill_rrect(ui.pt(SPACE_L), by, self.w as f32 - ui.pt(SPACE_XXXL), bh, ui.pt(CARD_R_PT), CARD_BG.now());
            put_mid(&mut self.labels, s, ui, &text, MAC_BODY_PT, Weight::Regular, RED, self.w as f32 / 2.0, by + bh / 2.0);
        }
    }

    fn draw_cover(&mut self, s: &mut Surface, ui: &Ui, i: usize, (x, y, w, h): (f32, f32, f32, f32)) {
        let (wi, hi) = (w.round() as usize, h.round() as usize);
        if self.shelf[i].tile.is_none() {
            if let Some(img) = &self.shelf[i].image {
                if img.w > 0 && img.h > 0 {
                    let f = (wi as f32 / img.w as f32).max(hi as f32 / img.h as f32);
                    let scaled = img.resized(((img.w as f32 * f).round() as usize).max(wi), ((img.h as f32 * f).round() as usize).max(hi));
                    let (ox, oy) = ((scaled.w - wi) / 2, (scaled.h - hi) / 2);
                    self.shelf[i].tile = Some(scaled.crop(ox, oy, wi, hi));
                }
            }
        }
        if let Some(tile) = &self.shelf[i].tile {
            s.blit(tile, x as i32, y as i32, 255, 255);
            s.stroke_rrect(x, y, w, h, 0.0, 1.0, fade(WHITE, 36));
            return;
        }
        let title = self.shelf[i].title.clone();
        let author = self.shelf[i].author.clone();
        s.fill_rrect(x, y, w, h, ui.pt(SPACE_XXS), cover_colour(&title));
        s.fill_rrect(x, y, ui.pt(SPACE_S), h, 0.0, spine_shade());
        let pad = ui.pt(SPACE_L);
        let lines = ui.wrap(&title, MAC_BODY_PT, Weight::Bold, w - pad * 2.0 - ui.pt(SPACE_S));
        for (k, line) in lines.iter().take(5).enumerate() {
            put(&mut self.labels, s, ui, line, MAC_BODY_PT, Weight::Bold, WHITE, x + pad + ui.pt(SPACE_S), y + pad + ui.pt(SPACE_XXL) * k as f32);
        }
        if !author.is_empty() {
            let a = ui.fit(&author, MAC_CAPTION1_PT, w - pad * 2.0 - ui.pt(SPACE_S));
            put(&mut self.labels, s, ui, &a, MAC_CAPTION1_PT, Weight::Regular, fade(WHITE, 200), x + pad + ui.pt(SPACE_S), y + h - pad - ui.pt(SPACE_L));
        }
    }

    // --- reader ---

    pub(super) fn draw_reader(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let theme = self.store.settings.theme;
        let pal = palette(theme);
        s.fill_rect(0, 0, self.w as usize, self.h as usize, pal.bg);
        let cur = self.reader.as_ref().map_or(0, |r| r.page);
        match self.slide {
            Some((old, dir, t0)) => {
                let p = ((t - t0) / super::SLIDE_MS).clamp(0.0, 1.0) as f32;
                let e = 1.0 - (1.0 - p) * (1.0 - p) * (1.0 - p);
                let w = self.w as f32;
                self.draw_page(s, ui, &pal, old, -dir as f32 * w * e);
                self.draw_page(s, ui, &pal, cur, dir as f32 * w * (1.0 - e));
            }
            None => self.draw_page(s, ui, &pal, cur, 0.0),
        }
        let (chrome, panel) = match &self.reader {
            Some(r) => (r.chrome, r.panel),
            None => return,
        };
        if panel.is_some() {
            self.draw_panel(s, ui, &pal, t);
        } else if chrome {
            self.draw_bottom(s, ui);
        }
        self.draw_toolbar(s, ui);
    }

    fn draw_page(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette, page_no: usize, dx: f32) {
        let m = self.metrics(ui);
        let marked = self.reader.as_ref().is_some_and(|_r| !self.page_marks(page_no).is_empty());
        let Some(r) = self.reader.as_mut() else { return };
        let (left, top) = (ui.pt(PAGE_MARGIN_PT) + dx, ui.pt(page_top_pt()));
        let body_pt = Self::pt_of(ui, m.body);
        let Some(page) = r.layout.pages.get(page_no) else { return };
        for line in &page.lines {
            match line.kind {
                Kind::Image { w, h } => {
                    if let Some(super::engine::Block::Image(img)) = r.book.blocks.get(line.block) {
                        let key = (line.block, w as u32);
                        let scaled = r.images.entry(key).or_insert_with(|| img.resized((w.round() as usize).max(1), (h.round() as usize).max(1)));
                        s.blit(scaled, (left + (m.width - w) / 2.0) as i32, (top + line.y) as i32, 255, 255);
                    }
                }
                kind => {
                    let (pt, weight) = match kind {
                        Kind::Heading(l) => (body_pt * layout::heading_scale(l), Weight::Bold),
                        _ => (body_pt, Weight::Regular),
                    };
                    let lh = ui.pt(pt) * ui.text_scale * LINE_HEIGHT;
                    if let Some((block, off, len)) = r.highlight {
                        if block == line.block {
                            let text = r.book.blocks.get(block).and_then(|b| b.text()).unwrap_or("");
                            let (a, b) = layout::line_cols(text, line.offset, &line.text, off, off + len);
                            if a < b {
                                let prefix: String = line.text.chars().take(a).collect();
                                let word: String = line.text.chars().skip(a).take(b - a).collect();
                                let x0 = left + ui.measure(&prefix, pt, weight);
                                let x1 = x0 + ui.measure(&word, pt, weight);
                                s.fill_rrect(x0, top + line.y, x1 - x0, lh, 0.0, fade(SYSTEM_YELLOW, 110));
                            }
                        }
                    }
                    let img = self.labels.get(ui, &line.text, pt, weight, pal.fg);
                    let y = top + line.y + (lh - img.h as f32) / 2.0;
                    s.blit(img, left as i32, y as i32, 255, 255);
                }
            }
        }
        let total = r.layout.pages.len();
        let label = format!("{} / {}", page_no + 1, total);
        put_mid(&mut self.labels, s, ui, &label, MAC_FOOTNOTE_PT, Weight::Regular, pal.dim, self.w as f32 / 2.0 + dx, self.h as f32 - ui.pt(FOOTER_PT) + ui.pt(SPACE_XXL));
        if marked {
            s.fill_rrect(self.w as f32 - ui.pt(64.0) + dx, ui.pt(super::nav_bottom_pt()), ui.pt(SPACE_XL), ui.pt(38.0), 0.0, ORANGE);
        }
    }

    /// The unified toolbar band with the bar, and the title in its slot.
    fn draw_toolbar(&mut self, s: &mut Surface, ui: &Ui) {
        let k = ui.pt(1.0);
        let side = self.side(ui) / k;
        widgets::toolbar_band(s, ui, Rect { x: side, y: 0.0, w: self.w as f32 / k - side, h: super::nav_bottom_pt() });
        let placed = widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), None);
        if let Some(r) = placed.rect_of(super::BAR_TITLE) {
            let title = ui.fit(&self.title(), MAC_HEADLINE_PT, ui.pt(r.w));
            let centre = self.screen_is_reader();
            let im = self.labels.get(ui, &title, MAC_HEADLINE_PT, Weight::Bold, LABEL.now());
            let x = if centre { ui.pt(r.x + r.w / 2.0) - im.w as f32 / 2.0 } else { ui.pt(r.x) };
            s.blit(im, x as i32, (ui.pt(r.y + r.h / 2.0) as i32) - im.h as i32 / 2, 255, 255);
        }
    }

    fn draw_bottom(&mut self, s: &mut Surface, ui: &Ui) {
        let (bx, by, bw, bh) = self.bottom_bar_rect(ui);
        s.fill_rect(bx as usize, by as usize, bw as usize, bh as usize + 1, CARD_BG.now());
        s.fill_rect(0, by as usize, self.w as usize, 1, SEPARATOR_C.now());
        let Some(r) = &self.reader else { return };
        let m = ui.pt(MARGIN_PT);
        let page = r.page;
        let total = r.layout.pages.len();
        let block = r.layout.pos_of(page).block;
        let chapter = r.book.chapter_at(block).map(|e| e.title.clone()).unwrap_or_else(|| r.book.title.clone());
        let label = format!("{} / {}", page + 1, total);
        let label_w = ui.measure(&label, MAC_BODY_PT, Weight::Regular);
        let chapter = ui.fit(&chapter, MAC_BODY_PT, self.w as f32 - 2.0 * m - label_w - ui.pt(SPACE_L));
        put(&mut self.labels, s, ui, &chapter, MAC_BODY_PT, Weight::Medium, LABEL.now(), m, by + ui.pt(SPACE_M));
        put_right(&mut self.labels, s, ui, &label, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), self.w as f32 - m, by + ui.pt(SPACE_XXL));
        let (sx, sy, sw, sh) = self.slider_rect(ui);
        let (ty, th) = (sy + sh / 2.0 - ui.pt(SPACE_XXS), ui.pt(SPACE_XS));
        s.fill_rrect(sx, ty, sw, th, ui.pt(SPACE_XXS), TRACK_C.now());
        let f = if total > 1 { page as f32 / (total - 1) as f32 } else { 0.0 };
        s.fill_rrect(sx, ty, sw * f, th, ui.pt(SPACE_XXS), BLUE);
        s.fill_rrect(sx + sw * f - ui.pt(SPACE_S), sy + sh / 2.0 - ui.pt(SPACE_M), ui.pt(SPACE_M), ui.pt(SPACE_XXL), ui.pt(SPACE_XS), DETAIL_BG.now());
    }

    fn draw_panel(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette, t: f64) {
        let (px, py, pw, ph) = self.panel_rect(ui);
        s.fill_rect(px as usize, py as usize, pw as usize, ph as usize, pal.bg);
        s.fill_rect((px + pw) as usize - 1, py as usize, 1, ph as usize, pal.sep);
        let Some(panel) = self.reader.as_ref().and_then(|r| r.panel) else { return };
        match panel {
            Panel::Display => self.draw_display(s, ui, pal),
            Panel::Contents | Panel::Bookmarks => {
                for (p, (x, y, w, h)) in self.tab_rects(ui) {
                    let on = p == panel;
                    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), if on { pal.raised } else { pal.bg });
                    let name = if p == Panel::Contents { "Contents" } else { "Bookmarks" };
                    put_mid(&mut self.labels, s, ui, name, MAC_BODY_PT, if on { Weight::Semibold } else { Weight::Regular }, if on { pal.fg } else { pal.dim }, x + w / 2.0, y + h / 2.0);
                }
                self.draw_rows(s, ui, pal, panel);
            }
            Panel::Search => {
                let (fx, fy, fw, fh) = self.search_field_rect(ui);
                s.fill_rrect(fx, fy, fw, fh, ui.pt(RADIUS_PT), FIELD_BG.now());
                if let Some(r) = &self.reader {
                    r.search.draw(s, ui, (fx + ui.pt(SPACE_M), fy, fw - ui.pt(SPACE_XXL), fh), t);
                }
                self.draw_rows(s, ui, pal, panel);
            }
        }
    }

    fn draw_rows(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette, panel: Panel) {
        let (_, py, pw, ph) = self.panel_rect(ui);
        let top = self.list_top(ui);
        let rows = self.panel_rows();
        let Some(r) = &self.reader else { return };
        if rows == 0 {
            let (title, hint) = match panel {
                Panel::Contents => ("No Contents", "This book has no table of contents."),
                Panel::Bookmarks => ("No Bookmarks", "Tap Bookmark to mark a page."),
                _ if !r.searched => ("Search the Book", "Type a word and press return."),
                _ => ("No Results", ""),
            };
            let y = top + ui.pt(90.0);
            put_mid(&mut self.labels, s, ui, title, MAC_TITLE3_PT, Weight::Bold, pal.dim, pw / 2.0, y);
            put_mid(&mut self.labels, s, ui, hint, MAC_BODY_PT, Weight::Regular, pal.dim, pw / 2.0, y + ui.pt(30.0));
            return;
        }
        let bottom = (py + ph) as usize;
        let mut c = s.rows(top as usize, bottom);
        let block_now = r.layout.pos_of(r.page).block;
        let current = r.book.toc.iter().rposition(|e| e.block <= block_now);
        let marks: Vec<Pos> = self.store.entry(&r.path).map_or(vec![], |e| e.bookmarks.clone());
        for i in 0..rows {
            let (rx, ry, rw, rh) = self.row_rect(ui, i);
            if ry + rh < top || ry > bottom as f32 {
                continue;
            }
            let (left, page_no, weight) = match panel {
                Panel::Contents => {
                    let e = &r.book.toc[i];
                    (e.title.clone(), r.layout.page_of(Pos { block: e.block, offset: 0 }) + 1, if current == Some(i) { Weight::Bold } else { Weight::Regular })
                }
                Panel::Bookmarks => {
                    let p = marks[i];
                    let text = r.book.blocks.get(p.block).and_then(|b| b.text()).map(|t| t.chars().skip(p.offset).take(80).collect::<String>()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Picture".to_string());
                    (text, r.layout.page_of(p) + 1, Weight::Regular)
                }
                _ => (r.hits[i].snippet.clone(), r.layout.page_of(Pos { block: r.hits[i].block, offset: r.hits[i].offset }) + 1, Weight::Regular),
            };
            let indent = if panel == Panel::Contents { ui.pt(SPACE_XL) * r.book.toc[i].level.min(4) as f32 } else { 0.0 };
            let right_pad = if panel == Panel::Bookmarks { pw * 0.5 } else { ui.pt(70.0) };
            let text = ui.fit(&left, MAC_CALLOUT_PT, rw - indent - right_pad);
            put(&mut self.labels, &mut c, ui, &text, MAC_CALLOUT_PT, weight, pal.fg, rx + indent, ry + rh / 2.0 - ui.pt(SPACE_M));
            let label = format!("{page_no}");
            let edge = if panel == Panel::Bookmarks { rx + rw - ui.pt(90.0) } else { rx + rw };
            put_right(&mut self.labels, &mut c, ui, &label, MAC_BODY_PT, Weight::Regular, pal.dim, edge, ry + rh / 2.0);
            if panel == Panel::Bookmarks {
                put_right(&mut self.labels, &mut c, ui, "Remove", MAC_BODY_PT, Weight::Regular, RED, rx + rw, ry + rh / 2.0);
            }
            c.fill_rrect(rx, ry + rh - 1.0, rw, 1.0, 0.0, pal.sep);
        }
    }

    fn draw_display(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette) {
        let settings = self.store.settings;
        let (_, py, pw, _) = self.panel_rect(ui);
        for (target, (x, y, w, h)) in self.display_buttons(ui) {
            match target {
                Target::SizeDown | Target::SizeUp => {
                    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), pal.raised);
                    let small = target == Target::SizeDown;
                    let at_limit = if small { settings.size == 0 } else { settings.size + 1 >= SIZES.len() };
                    let colour = if at_limit { pal.dim } else { pal.fg };
                    put_mid(&mut self.labels, s, ui, "A", if small { MAC_BODY_PT } else { MAC_TITLE1_PT }, Weight::Medium, colour, x + w / 2.0, y + h / 2.0);
                }
                Target::ThemeButton(i) => {
                    let theme = Theme::ALL[i];
                    let p = palette(theme);
                    let on = theme == settings.theme;
                    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), if on { pal.accent } else { pal.sep });
                    s.fill_rrect(x + 2.0, y + 2.0, w - 4.0, h - 4.0, ui.pt(RADIUS_PT), p.bg);
                    put_mid(&mut self.labels, s, ui, "Aa", MAC_TITLE1_PT, Weight::Medium, p.fg, x + w / 2.0, y + h / 2.0 - ui.pt(SPACE_M));
                    put_mid(&mut self.labels, s, ui, theme.name(), MAC_FOOTNOTE_PT, Weight::Regular, p.dim, x + w / 2.0, y + h - ui.pt(SPACE_XL));
                }
                _ => {}
            }
        }
        let size = format!("Text size {}", SIZES[settings.size.min(SIZES.len() - 1)] as i32);
        // midway between the size buttons and the swatch row
        let size_y = py + ui.pt(SPACE_XXL) + ui.pt(56.0) + ui.pt(SPACE_XL + SPACE_XS);
        put_mid(&mut self.labels, s, ui, &size, MAC_BODY_PT, Weight::Regular, pal.dim, pw / 2.0, size_y);
    }
}

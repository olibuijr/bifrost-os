//! Books drawing: the library grid, the page, the reader's bars and the
//! contents / bookmarks / search / display panels.

use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{MAC_CALLOUT_PT, MAC_HEADLINE_PT};
use bifrost_app::widgets;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MAC_TITLE3_PT};
use super::layout::Kind;
use super::settings::{self, Line};
use super::store::{Theme, SIZES};
use super::*;
use springboard::gfx::pack;
use bifrost_app::controls::prefs_layout::m;
use springboard::shell::cache::TextCache;
use springboard::shell::ui::{centred, Weight, CARD_BG, CONTENT_BG, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, SEPARATOR_C, TRACK_C, WHITE};
use bifrost_app::kit::ui::{LINK, AVATAR_FG, BLACK, DESTRUCTIVE, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_YELLOW, accent_now};
use bifrost_app::kit::ui::{READER_DARK_BG, READER_DARK_DIM, READER_DARK_INK, READER_DARK_SEP, READER_SEPIA_ACCENT, READER_SEPIA_BG, READER_SEPIA_DIM, READER_SEPIA_INK, READER_SEPIA_RAISED, READER_SEPIA_SEP, READER_WHITE_ACCENT, READER_WHITE_DIM, READER_WHITE_INK, READER_WHITE_RAISED, READER_WHITE_SEP};

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
        Theme::Dark => Palette { bg: READER_DARK_BG, fg: READER_DARK_INK, dim: READER_DARK_DIM, sep: READER_DARK_SEP, raised: AVATAR_FG, accent: accent_now() },
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

/// Pixels from the left edge of a line to its character column `col`, with `extra` px added at each space before it.
fn col_x(ui: &Ui, text: &str, col: usize, pt: f32, weight: Weight, extra: f32) -> f32 {
    let prefix: String = text.chars().take(col).collect();
    ui.measure(&prefix, pt, weight) + extra * prefix.matches(' ').count() as f32
}

/// The colour of a highlight style (an index into `store::HIGHLIGHT_STYLES`); the underline has its own.
fn highlight_colour(style: u8) -> u32 {
    match style {
        1 => SYSTEM_GREEN,
        2 => SYSTEM_BLUE,
        3 => SYSTEM_PINK,
        4 => SYSTEM_PURPLE,
        _ => SYSTEM_YELLOW,
    }
}

/// Marks one run of text: a tint behind it, or a line under it for the underline style. `y` is the top of the line and
/// `lh` its height.
/// A choice button of the Settings window: the chosen one is filled in the tint and labelled white.
fn settings_choice(labels: &mut TextCache, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), name: &str, on: bool) {
    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), if on { accent_now() } else { FIELD_BG.now() });
    put_mid(labels, s, ui, name, MAC_BODY_PT, if on { Weight::Semibold } else { Weight::Regular }, if on { WHITE } else { LABEL.now() }, x + w / 2.0, y + h / 2.0);
}

fn paint_highlight(s: &mut Surface, ui: &Ui, style: u8, x0: f32, x1: f32, y: f32, lh: f32) {
    if style == super::store::UNDERLINE {
        let thick = ui.pt(SPACE_XXS).max(1.0);
        s.fill_rrect(x0, y + lh - thick, x1 - x0, thick, 0.0, RED);
    } else {
        s.fill_rrect(x0, y, x1 - x0, lh, 0.0, fade(highlight_colour(style), 110));
    }
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
        // the main content of the window: white in Light, the dark content grey in Dark (not the black of DETAIL_BG)
        s.fill_rect(0, 0, w, h, CONTENT_BG.now());
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
            let title = if self.library_search.is_empty() { self.empty_title() } else { "No Results".to_string() };
            let empty_y = top + (bottom - top) * 0.2;
            put_mid(&mut self.labels, s, ui, &title, MAC_TITLE1_PT, Weight::Bold, LABEL_2.now(), mid, empty_y);
            let hint = match self.filter {
                Filter::Collection(_) if self.library_search.is_empty() => "Right-click a book and choose Add to this collection.",
                Filter::WantToRead if self.library_search.is_empty() => "Right-click a book and choose Add to Want to Read.",
                _ => "",
            };
            put_mid(&mut self.labels, s, ui, hint, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), mid, empty_y + ui.pt(SPACE_HUGE));
        }
        let mut content = s.rows(top as usize, bottom as usize);
        let n = self.reading_count(ui);
        if n > 0 {
            let base = top - self.shelf_scroll.offset();
            put(&mut self.labels, &mut content, ui, "Reading Now", MAC_TITLE3_PT, Weight::Bold, LABEL.now(), m, base + ui.pt(SPACE_XS));
            // the daily reading goal sits on the same line, at the trailing edge
            if let Some(goal) = self.goal_text() {
                put_right(&mut self.labels, &mut content, ui, &goal, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 - ui.pt(MARGIN_PT), base + ui.pt(SPACE_XS) + ui.pt(SPACE_XL));
            }
            let lib = base + self.library_offset(ui) - ui.pt(SPACE_HUGE);
            put(&mut self.labels, &mut content, ui, "Library", MAC_TITLE3_PT, Weight::Bold, LABEL.now(), m, lib + ui.pt(SPACE_XS));
        }
        let list = self.list_view();
        for (k, i) in self.visible().into_iter().enumerate() {
            if list {
                let row = self.list_row_rect(ui, k);
                if row.1 + row.3 < top || row.1 > bottom {
                    continue;
                }
                self.draw_list_row(&mut content, ui, i, row);
                continue;
            }
            let (cx, cy, _, _) = self.cover_rect(ui, k);
            if cy + ch + ui.pt(64.0) < top || cy > bottom {
                continue;
            }
            self.draw_cover(&mut content, ui, i, (cx, cy, tw, ch));
            let path = self.shelf[i].path.clone();
            let (progress, status) = self.store.entry(&path).map_or((0, Status::None), |e| (e.progress, e.status));
            let title = self.shelf[i].title.clone();
            let lines = ui.wrap(&title, MAC_FOOTNOTE_PT, Weight::Medium, tw);
            for (k, line) in lines.iter().take(2).enumerate() {
                let line = ui.fit(line, MAC_FOOTNOTE_PT, tw);
                put(&mut self.labels, &mut content, ui, &line, MAC_FOOTNOTE_PT, Weight::Medium, LABEL.now(), cx, cy + ch + ui.pt(8.0 + 17.0 * k as f32));
            }
            let sub = store::caption(status, progress);
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
        if self.shelf[i].tile.as_ref().is_none_or(|t| t.w != wi || t.h != hi) {
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

    /// One book of the library as a list: a small cover, the title and author, and where the book stands.
    fn draw_list_row(&mut self, s: &mut Surface, ui: &Ui, i: usize, (x, y, w, h): (f32, f32, f32, f32)) {
        let pad = ui.pt(SPACE_S);
        let (cw, ch) = ((h - 2.0 * pad) * 2.0 / 3.0, h - 2.0 * pad);
        if self.shelf[i].image.is_some() {
            self.draw_cover(s, ui, i, (x, y + pad, cw, ch));
        } else {
            let colour = cover_colour(&self.shelf[i].title);
            s.fill_rrect(x, y + pad, cw, ch, ui.pt(SPACE_XXS), colour);
            s.fill_rrect(x, y + pad, ui.pt(SPACE_XXS), ch, 0.0, spine_shade());
        }
        let path = self.shelf[i].path.clone();
        let (progress, status) = self.store.entry(&path).map_or((0, Status::None), |e| (e.progress, e.status));
        let caption = store::caption(status, progress);
        let caption_w = ui.measure(&caption, MAC_CALLOUT_PT, Weight::Regular);
        let text_x = x + cw + ui.pt(SPACE_L);
        let room = (x + w - caption_w - ui.pt(SPACE_XL) - text_x).max(ui.pt(SPACE_HUGE));
        let title = ui.fit(&self.shelf[i].title, MAC_BODY_PT, room);
        put(&mut self.labels, s, ui, &title, MAC_BODY_PT, Weight::Medium, LABEL.now(), text_x, y + h / 2.0 - ui.pt(SPACE_XL));
        let author = self.shelf[i].author.clone();
        let kind = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) { "PDF" } else { "Book" };
        let sub = if author.is_empty() { kind.to_string() } else { author };
        let sub = ui.fit(&sub, MAC_SUBHEAD_PT, room);
        put(&mut self.labels, s, ui, &sub, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), text_x, y + h / 2.0 + ui.pt(SPACE_XXS));
        put_right(&mut self.labels, s, ui, &caption, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x + w, y + h / 2.0);
        s.fill_rect(text_x as usize, (y + h - 1.0) as usize, (x + w - text_x).max(0.0) as usize, 1, SEPARATOR_C.now());
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
                self.draw_page(s, ui, &pal, old, -dir as f32 * w * e, 0.0);
                self.draw_page(s, ui, &pal, cur, dir as f32 * w * (1.0 - e), 0.0);
            }
            None if self.continuous() => {
                // View > Continuous Scroll: the pages one below the other, the ones in view drawn at their scroll position
                let pitch = self.page_pitch(ui);
                let offset = self.page_scroll.offset();
                let pages = self.reader.as_ref().map_or(0, |r| r.layout.pages.len());
                for page in 0..pages {
                    let dy = page as f32 * pitch - offset;
                    if dy < self.h as f32 && dy + pitch > 0.0 {
                        self.draw_page(s, ui, &pal, page, 0.0, dy);
                    }
                }
            }
            None => {
                self.draw_page(s, ui, &pal, cur, 0.0, 0.0);
                // View > Two Pages: the page after the current one sits in the right column, after the gutter
                if self.two_pages() && self.reader.as_ref().is_some_and(|r| cur + 1 < r.layout.pages.len()) {
                    let column = self.metrics(ui).width + ui.pt(PAGE_GUTTER_PT);
                    self.draw_page(s, ui, &pal, cur + 1, column, 0.0);
                }
            }
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
        self.draw_banner(s, ui);
        self.draw_toolbar(s, ui);
    }

    /// A one-line card at the top of the page: the notice of Look Up or Translate, or the Read Aloud note while a
    /// chapter is read. A tap dismisses a notice.
    fn draw_banner(&mut self, s: &mut Surface, ui: &Ui) {
        let reading = self.reader.as_ref().is_some_and(|r| r.narration.is_some());
        let text = match (&self.notice, reading) {
            (Some(n), _) => n.clone(),
            (None, true) => narrate::READ_ALOUD_NOTE.to_string(),
            (None, false) => return,
        };
        let pad_x = ui.pt(SPACE_XL);
        let pad_y = ui.pt(SPACE_S);
        let w = ui.measure(&text, MAC_CALLOUT_PT, Weight::Regular) + pad_x * 2.0;
        let h = ui.pt(MAC_CALLOUT_PT) * 2.0 + pad_y * 2.0;
        let x = (self.w as f32 - w) / 2.0;
        let y = ui.pt(super::nav_bottom_pt()) + pad_y;
        s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        put_mid(&mut self.labels, s, ui, &text, MAC_CALLOUT_PT, Weight::Regular, LABEL.now(), self.w as f32 / 2.0, y + h / 2.0);
    }

    /// Draws page `page_no` with its column shifted `dx` px to the right (a slide, the second page of a spread) and `dy` px
    /// down (continuous scrolling).
    fn draw_page(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette, page_no: usize, dx: f32, dy: f32) {
        // the body text is drawn in the chosen face, the same one the pages were laid out with
        let face = self.body_face(ui);
        let ui: &Ui = face.as_deref().unwrap_or(ui);
        let m = self.metrics(ui);
        let marked = self.reader.as_ref().is_some_and(|_r| !self.page_marks(page_no).is_empty());
        let justify = self.store.settings.justify;
        // a PDF book's saved highlights are highlight annotations, drawn under the text; other books draw them as spans
        let marks = self.pdf_highlight_marks(ui, page_no);
        // the saved highlights of this book with their colours, drawn under the text like the find match
        let spans: Vec<(super::store::Span, u8)> = if marks.is_some() {
            Vec::new()
        } else {
            self.reader.as_ref().and_then(|r| self.store.entry(&r.path)).map_or_else(Vec::new, |e| {
                e.highlights.iter().map(|sp| (*sp, e.styles.iter().find(|(x, _)| x == sp).map_or(0, |(_, style)| *style))).collect()
            })
        };
        let Some(r) = self.reader.as_mut() else { return };
        let (left, top) = (ui.pt(self.store.settings.margin_pt()) + dx, ui.pt(page_top_pt()) + dy);
        let body_pt = Self::pt_of(ui, m.body);
        let Some(page) = r.layout.pages.get(page_no) else { return };
        if let Some(marks) = &marks {
            for (mark, style) in marks.iter().filter(|(a, _)| a.should_display()) {
                let b = mark.bounds();
                paint_highlight(s, ui, *style, left + b.x, left + b.x + b.w, top + b.y, b.h);
            }
        }
        // the text picked by Select Page on this page: character ranges of the page text (see `page_text`)
        let selected: Vec<std::ops::Range<usize>> = r.selection.as_ref().map_or_else(Vec::new, |sel| {
            sel.ranges().iter().filter(|(p, _)| *p == page_no).map(|(_, range)| range.clone()).collect()
        });
        // where each line starts in the page text; pictures take no characters
        let mut starts = Vec::with_capacity(page.lines.len());
        let mut at = 0usize;
        for line in &page.lines {
            starts.push(at);
            if !matches!(line.kind, Kind::Image { .. }) {
                at += line.text.chars().count() + 1;
            }
        }
        for (li, line) in page.lines.iter().enumerate() {
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
                    let lh = ui.pt(pt) * ui.text_scale * m.line_height;
                    let block_text = r.book.blocks.get(line.block).and_then(|b| b.text()).unwrap_or("");
                    // a justified line gives each of its spaces a little more width; every tint below follows it
                    let extra = super::line_extra(ui, justify, block_text, line, pt, m.width);
                    // the Select Page tint, over the characters of this line that the selection covers
                    let line_start = starts[li];
                    let line_end = line_start + line.text.chars().count();
                    for range in &selected {
                        let (a, b) = (range.start.max(line_start), range.end.min(line_end));
                        if a < b {
                            let x0 = left + col_x(ui, &line.text, a - line_start, pt, weight, extra);
                            let x1 = left + col_x(ui, &line.text, b - line_start, pt, weight, extra);
                            s.fill_rrect(x0, top + line.y, x1 - x0, lh, 0.0, fade(accent_now(), 90));
                        }
                    }
                    if let Some((block, off, len)) = r.highlight {
                        if block == line.block {
                            let (a, b) = layout::line_cols(block_text, line.offset, &line.text, off, off + len);
                            if a < b {
                                let x0 = left + col_x(ui, &line.text, a, pt, weight, extra);
                                let x1 = left + col_x(ui, &line.text, b, pt, weight, extra);
                                s.fill_rrect(x0, top + line.y, x1 - x0, lh, 0.0, fade(SYSTEM_YELLOW, 110));
                            }
                        }
                    }
                    // the sentence being read aloud: the one tint, lighter than the yellow of a highlight
                    if let Some(span) = r.narration.as_ref().and_then(|n| n.current()) {
                        if span.block == line.block {
                            let (a, b) = layout::line_cols(block_text, line.offset, &line.text, span.offset, span.offset + span.len);
                            if a < b {
                                let x0 = left + col_x(ui, &line.text, a, pt, weight, extra);
                                let x1 = left + col_x(ui, &line.text, b, pt, weight, extra);
                                s.fill_rrect(x0, top + line.y, x1 - x0, lh, 0.0, fade(accent_now(), 70));
                            }
                        }
                    }
                    for (sp, style) in spans.iter().filter(|(sp, _)| sp.block == line.block) {
                        let (a, b) = layout::line_cols(block_text, line.offset, &line.text, sp.offset, sp.offset + sp.len);
                        if a < b {
                            let x0 = left + col_x(ui, &line.text, a, pt, weight, extra);
                            let x1 = left + col_x(ui, &line.text, b, pt, weight, extra);
                            paint_highlight(s, ui, *style, x0, x1, top + line.y, lh);
                        }
                    }
                    if extra > 0.0 {
                        // set word by word, each at its place in the stretched line
                        let mut col = 0usize;
                        for word in line.text.split(' ') {
                            if !word.is_empty() {
                                let x = left + col_x(ui, &line.text, col, pt, weight, extra);
                                let img = self.labels.get(ui, word, pt, weight, pal.fg);
                                let y = top + line.y + (lh - img.h as f32) / 2.0;
                                s.blit(img, x as i32, y as i32, 255, 255);
                            }
                            col += word.chars().count() + 1;
                        }
                    } else {
                        let img = self.labels.get(ui, &line.text, pt, weight, pal.fg);
                        let y = top + line.y + (lh - img.h as f32) / 2.0;
                        s.blit(img, left as i32, y as i32, 255, 255);
                    }
                }
            }
        }
        let total = r.layout.pages.len();
        let label = format!("{} / {}", page_no + 1, total);
        put_mid(&mut self.labels, s, ui, &label, MAC_FOOTNOTE_PT, Weight::Regular, pal.dim, self.w as f32 / 2.0 + dx, top + m.height + ui.pt(SPACE_XXL));
        if marked {
            // the ribbon sits at the trailing edge of this page's column (a spread has two)
            let right = if self.two_pages() { left + m.width } else { self.w as f32 + dx };
            s.fill_rrect(right - ui.pt(64.0), ui.pt(super::nav_bottom_pt()) + dy, ui.pt(SPACE_XL), ui.pt(38.0), 0.0, ORANGE);
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
        s.fill_rrect(sx, ty, sw * f, th, ui.pt(SPACE_XXS), accent_now());
        s.fill_rrect(sx + sw * f - ui.pt(SPACE_S), sy + sh / 2.0 - ui.pt(SPACE_M), ui.pt(SPACE_M), ui.pt(SPACE_XXL), ui.pt(SPACE_XS), DETAIL_BG.now());
    }

    fn draw_panel(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette, t: f64) {
        let (px, py, pw, ph) = self.panel_rect(ui);
        s.fill_rect(px as usize, py as usize, pw as usize, ph as usize, pal.bg);
        s.fill_rect((px + pw) as usize - 1, py as usize, 1, ph as usize, pal.sep);
        let Some(panel) = self.reader.as_ref().and_then(|r| r.panel) else { return };
        match panel {
            Panel::Display => self.draw_display(s, ui, pal),
            Panel::Contents | Panel::Bookmarks | Panel::Pages | Panel::Notes => {
                for (p, (x, y, w, h)) in self.tab_rects(ui) {
                    let on = p == panel;
                    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), if on { pal.raised } else { pal.bg });
                    let name = match p {
                        Panel::Contents => "Contents",
                        Panel::Bookmarks => "Bookmarks",
                        Panel::Pages => "Pages",
                        _ => "Notes",
                    };
                    put_mid(&mut self.labels, s, ui, name, MAC_BODY_PT, if on { Weight::Semibold } else { Weight::Regular }, if on { pal.fg } else { pal.dim }, x + w / 2.0, y + h / 2.0);
                }
                if panel == Panel::Notes {
                    let (fx, fy, fw, fh) = self.note_field_rect(ui);
                    s.fill_rrect(fx, fy, fw, fh, ui.pt(RADIUS_PT), FIELD_BG.now());
                    if let Some(r) = &self.reader {
                        r.note.draw(s, ui, (fx + ui.pt(SPACE_M), fy, fw - ui.pt(SPACE_XXL), fh), t);
                    }
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
                Panel::Pages => ("No Pages", "This book has no pages."),
                Panel::Notes => ("No Notes", "Choose Add Note in the Find menu to write a note on a highlight."),
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
        let notes = self.notes_list();
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
                Panel::Pages => {
                    // the text thumbnail: the first line of text on the page
                    let text = r.layout.pages[i].lines.iter().find(|l| !l.text.trim().is_empty()).map_or_else(|| "Picture".to_string(), |l| l.text.chars().take(80).collect());
                    (text, i + 1, if i == r.page { Weight::Bold } else { Weight::Regular })
                }
                Panel::Notes => {
                    let (span, text) = &notes[i];
                    let weight = if r.note_target == Some(*span) { Weight::Bold } else { Weight::Regular };
                    (text.clone(), r.layout.page_of(Pos { block: span.block, offset: span.offset }) + 1, weight)
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

    /// A choice button of the Display panel: the chosen one has the accent outline.
    fn draw_choice(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette, (x, y, w, h): (f32, f32, f32, f32), name: &str, on: bool) {
        s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), if on { pal.accent } else { pal.sep });
        s.fill_rrect(x + 2.0, y + 2.0, w - 4.0, h - 4.0, ui.pt(RADIUS_PT), pal.raised);
        put_mid(&mut self.labels, s, ui, name, MAC_BODY_PT, if on { Weight::Semibold } else { Weight::Regular }, if on { pal.fg } else { pal.dim }, x + w / 2.0, y + h / 2.0);
    }

    fn draw_display(&mut self, s: &mut Surface, ui: &Ui, pal: &Palette) {
        let settings = self.store.settings;
        let (_, py, pw, _) = self.panel_rect(ui);
        // a PDF keeps the font its document sets, so the font row is off for it
        let applies = self.reader.as_ref().is_some_and(|r| store::family_applies(&r.path));
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
                Target::Spacing(i) => self.draw_choice(s, ui, pal, (x, y, w, h), store::SPACINGS[i].0, i == settings.spacing),
                Target::Margin(i) => self.draw_choice(s, ui, pal, (x, y, w, h), store::MARGINS[i].0, i == settings.margin),
                Target::Justify => self.draw_choice(s, ui, pal, (x, y, w, h), "Justify Text", settings.justify),
                Target::FontPrev | Target::FontNext => {
                    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), pal.raised);
                    let colour = if applies { pal.fg } else { pal.dim };
                    let glyph = if target == Target::FontPrev { "<" } else { ">" };
                    put_mid(&mut self.labels, s, ui, glyph, MAC_BODY_PT, Weight::Medium, colour, x + w / 2.0, y + h / 2.0);
                }
                _ => {}
            }
        }
        // names above the rows of choices
        let m = ui.pt(MARGIN_PT);
        let y0 = py + ui.pt(SPACE_XXL);
        for (name, top) in [("Line Spacing", SPACING_ROW_PT), ("Margins", MARGIN_ROW_PT)] {
            put(&mut self.labels, s, ui, name, MAC_CAPTION1_PT, Weight::Regular, pal.dim, m, y0 + ui.pt(top) - ui.pt(SPACE_XL));
        }
        let size = format!("Text size {}", SIZES[settings.size.min(SIZES.len() - 1)] as i32);
        // midway between the size buttons and the swatch row
        let size_y = py + ui.pt(SPACE_XXL) + ui.pt(56.0) + ui.pt(SPACE_XL + SPACE_XS);
        put_mid(&mut self.labels, s, ui, &size, MAC_BODY_PT, Weight::Regular, pal.dim, pw / 2.0, size_y);
        // the family the body text is set in, between the two steps; a PDF names the font its pages keep
        let fy = py + ui.pt(SPACE_XXL) + ui.pt(FAMILY_ROW_PT);
        let name = if self.store.family.is_empty() { "System Font".to_string() } else { self.store.family.clone() };
        let name_colour = if applies { pal.fg } else { pal.dim };
        put_mid(&mut self.labels, s, ui, &name, MAC_BODY_PT, Weight::Regular, name_colour, pw / 2.0, fy + ui.pt(CHOICE_PT) / 2.0);
        if !applies {
            put_mid(&mut self.labels, s, ui, "PDF pages keep the font the document sets.", MAC_FOOTNOTE_PT, Weight::Regular, pal.dim, pw / 2.0, fy + ui.pt(CHOICE_PT + SPACE_M));
        }
    }

    /// Books > Settings: the bar with Done, then the General and Reading groups, laid out by `settings.rs`.
    pub(super) fn draw_settings(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, bar) = (self.w as f32, ui.pt(nav_bottom_pt()));
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
        put_mid(&mut self.labels, s, ui, "Settings", MAC_TITLE3_PT, Weight::Bold, LABEL.now(), w / 2.0, bar / 2.0);
        let rows = self.folder_rows();
        let prefs = self.store.settings;
        let continuous = self.store.open_continuous;
        let targets = self.settings_targets(ui);
        let at = |t: Target| targets.iter().find(|(x, _)| *x == t).map(|(_, r)| *r);
        for (line, (x, y, cw, lh)) in settings::geometry(&settings::lines(rows.len()), ui, w, settings::content_top(ui)) {
            let mid = y + lh / 2.0;
            if !matches!(line, Line::Header(_) | Line::Note(_)) {
                s.fill_rrect(x, y, cw, lh, ui.pt(CARD_R_PT), CARD_BG.now());
            }
            match line {
                Line::Header(name) => {
                    put(&mut self.labels, s, ui, &name.to_uppercase(), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), x + ui.pt(SPACE_S), mid - ui.pt(MAC_CAPTION1_PT));
                }
                Line::Folder(k) => {
                    let (name, user) = match rows.get(k) {
                        Some((p, user)) => (p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()), *user),
                        None => (String::new(), None),
                    };
                    put(&mut self.labels, s, ui, &name, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), mid - ui.pt(MAC_BODY_PT));
                    match (user, at(Target::RemoveFolder(k))) {
                        (Some(_), Some((rx, ry, rw, rh))) => put_mid(&mut self.labels, s, ui, "Remove", MAC_BODY_PT, Weight::Regular, LINK, rx + rw / 2.0, ry + rh / 2.0),
                        _ => put_right(&mut self.labels, s, ui, "Built In", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + cw - ui.pt(SPACE_L), mid),
                    }
                }
                Line::AddFolder => {
                    put(&mut self.labels, s, ui, "Add Folder...", MAC_BODY_PT, Weight::Regular, LINK, x + ui.pt(SPACE_L), mid - ui.pt(MAC_BODY_PT));
                }
                Line::OpenIn => {
                    for (t, name, on) in [(Target::OpenMode(false), "Single Page", !continuous), (Target::OpenMode(true), "Continuous Scroll", continuous)] {
                        if let Some(r) = at(t) {
                            settings_choice(&mut self.labels, s, ui, r, name, on);
                        }
                    }
                }
                Line::TextSize => {
                    put(&mut self.labels, s, ui, "Text Size", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), mid - ui.pt(MAC_BODY_PT));
                    let size = SIZES[prefs.size.min(SIZES.len() - 1)] as i32;
                    if let Some((sx, _, _, _)) = at(Target::SizeDown) {
                        put_right(&mut self.labels, s, ui, &format!("{size} pt"), MAC_BODY_PT, Weight::Regular, LABEL_2.now(), sx - ui.pt(SPACE_M), mid);
                    }
                    for (t, small) in [(Target::SizeDown, true), (Target::SizeUp, false)] {
                        let Some((bx, by, bw, bh)) = at(t) else { continue };
                        let at_limit = if small { prefs.size == 0 } else { prefs.size + 1 >= SIZES.len() };
                        s.fill_rrect(bx, by, bw, bh, ui.pt(RADIUS_PT), FIELD_BG.now());
                        let colour = if at_limit { LABEL_2.now() } else { LABEL.now() };
                        put_mid(&mut self.labels, s, ui, "A", if small { MAC_BODY_PT } else { MAC_TITLE1_PT }, Weight::Medium, colour, bx + bw / 2.0, by + bh / 2.0);
                    }
                }
                Line::Theme => {
                    for (i, theme) in Theme::ALL.iter().enumerate() {
                        if let Some(r) = at(Target::ThemeButton(i)) {
                            settings_choice(&mut self.labels, s, ui, r, theme.name(), prefs.theme == *theme);
                        }
                    }
                }
                Line::Justify => {
                    put(&mut self.labels, s, ui, "Justify Text", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), mid - ui.pt(MAC_BODY_PT));
                    put_right(&mut self.labels, s, ui, if prefs.justify { "On" } else { "Off" }, MAC_BODY_PT, Weight::Medium, LINK, x + cw - ui.pt(SPACE_L), mid);
                }
                Line::Note(text) => {
                    put(&mut self.labels, s, ui, text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + ui.pt(SPACE_S), y + ui.pt(SPACE_XS));
                }
            }
        }
        if let Some((dx, dy, dw, dh)) = at(Target::SettingsDone) {
            put_mid(&mut self.labels, s, ui, "Done", MAC_BODY_PT, Weight::Semibold, LINK, dx + dw / 2.0, dy + dh / 2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::appearance::{accent, set_accent, Accent};

    /// The accent is process-wide: tests that change it run one at a time.
    static ACCENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn the_dark_reader_accent_follows_the_chosen_accent_not_system_blue() {
        let _g = ACCENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep = accent();
        set_accent(Accent::Green);
        let (pal, green) = (palette(Theme::Dark), accent_now());
        set_accent(keep);
        assert_eq!(pal.accent, green);
        assert_ne!(pal.accent, bifrost_app::kit::ui::TINT);
    }

    #[test]
    fn the_white_and_sepia_reader_themes_keep_their_own_accent() {
        assert_eq!(palette(Theme::White).accent, READER_WHITE_ACCENT);
        assert_eq!(palette(Theme::Sepia).accent, READER_SEPIA_ACCENT);
    }
}

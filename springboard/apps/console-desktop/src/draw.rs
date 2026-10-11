//! Paints the Console window: the sidebar, the toolbar band, the table, the detail pane and the footer. The geometry
//! is `Console::geo`, which the hit tests share, so a tap lands on what is drawn. Colours are the kit's semantic
//! tokens (`Pair`s resolved with `now()`), so Light and Dark follow the appearance.

use crate::console::{Col, Console, Geo, B_SEARCH, FOOT_PT, HEAD_PT, ROW_PT, SIDE_PT, SIDE_SCROLL};
use crate::model::Level;
use bifrost_app::controls::Rect;
use bifrost_app::kit::appearance::Pair;
use bifrost_app::kit::ui::{CARD_BG, DETAIL_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, SELECTION, SEPARATOR_C, WHITE};
use bifrost_app::kit::{colors, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// Microseconds in a second, and in a millisecond (journal stamps are microseconds).
const US_PER_SEC: i64 = 1_000_000;
const US_PER_MS: i64 = 1_000;
/// Text inset in a table cell, and the Type column's inset (its level dot comes first).
const PAD: f32 = 8.0;
const TYPE_PAD: f32 = 16.0;
/// Detail pane inset and the width of its field names.
const PANE_PAD: f32 = 16.0;
const KEY_W: f32 = 84.0;
/// Lines of message and extra detail the pane shows before it stops.
const MAX_LINES: usize = 60;

/// The colour of a level's dot and label: Fault and Error are the kit's semantic red and orange, Default is
/// secondary, Info and Debug tertiary.
fn level_color(level: Level) -> Pair {
    match level {
        Level::Fault => colors::SYSTEM_RED,
        Level::Error => colors::SYSTEM_ORANGE,
        Level::Notice => LABEL_2,
        Level::Info | Level::Debug => LABEL_3,
    }
}

/// A filled rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, color: u32) {
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w.max(0.0)), ui.pt(h.max(0.0)), 0.0, color);
}

/// A round dot of diameter `d` at a point.
fn dot(s: &mut Surface, ui: &Ui, x: f32, y: f32, d: f32, color: u32) {
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(d), ui.pt(d), ui.pt(d) / 2.0, color);
}

/// Text at a point, vertically centred on `mid`, cut to `max` points.
#[allow(clippy::too_many_arguments)]
fn text(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// The width of a label in points.
fn width(ui: &Ui, label: &str, pt: f32, weight: Weight) -> f32 {
    ui.measure(label, pt, weight) / ui.pt(1.0)
}

/// The text cut into lines no wider than `max` points at `pt`, breaking at spaces.
fn wrap(ui: &Ui, body: &str, pt: f32, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in body.lines() {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if cur.is_empty() || width(ui, &next, pt, Weight::Regular) <= max {
                cur = next;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        lines.push(cur);
    }
    lines
}

/// The machine's offset from UTC at `secs`, applied to a Unix time (0 when no zone database is readable).
fn local(secs: i64) -> i64 {
    if bifrost_app::testing::pinned() {
        return secs; // UTC under the testing kit, so a snapshot does not depend on the machine's zone
    }
    secs + springboard::bare::timezone::offset_at(secs).unwrap_or(0) as i64
}

/// "14:03:22.123" in the machine's zone; empty when the entry has no date.
pub fn clock(date_us: i64) -> String {
    if date_us <= 0 {
        return String::new();
    }
    let (h, m, s, ..) = springboard::bare::timezone::civil(local(date_us.div_euclid(US_PER_SEC)));
    format!("{h:02}:{m:02}:{s:02}.{:03}", date_us.rem_euclid(US_PER_SEC) / US_PER_MS)
}

/// "2026-10-10 14:03:22" in the machine's zone.
pub fn full_date(date_us: i64) -> String {
    if date_us <= 0 {
        return "Unknown".into();
    }
    let l = local(date_us.div_euclid(US_PER_SEC));
    let (y, mo, d) = springboard::bare::timezone::civil_from_days(l.div_euclid(86_400));
    let (h, m, s, ..) = springboard::bare::timezone::civil(l);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// A value with control characters turned into spaces, so one message is one row.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

impl Console {
    /// Paints the whole window. Also keeps the scroll range and, while streaming, the newest row in view.
    pub fn draw(&mut self, ui: &Ui, s: &mut Surface) {
        let g = self.geo(ui);
        let max = self.fit_scroll(ui);
        self.set_stick_bottom(max);
        fill(s, ui, 0.0, 0.0, g.w, g.h, DETAIL_BG.now());
        self.draw_sidebar(ui, s);
        self.draw_rows(ui, s, &g);
        if let Some(x) = g.pane_x {
            self.draw_pane(ui, s, &g, x);
        }
        self.draw_chrome(ui, s, &g);
    }

    fn draw_sidebar(&self, ui: &Ui, s: &mut Surface) {
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDE_PT, h: self.h / ui.pt(1.0) };
        widgets::source_list(s, ui, bounds, &self.sidebar(), SIDE_SCROLL, None);
        s.fill_rect(ui.pt(SIDE_PT) as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
    }

    /// The table rows that are on screen (the header and footer cover the overflow).
    fn draw_rows(&mut self, ui: &Ui, s: &mut Surface, g: &Geo) {
        let table_w = g.table_right - g.table_x;
        if self.visible.is_empty() {
            let msg = self.empty_message();
            let x = g.table_x + ((table_w - width(ui, msg, MAC_TITLE3_PT, Weight::Regular)) / 2.0).max(PAD);
            let mid = (g.table_top + g.table_bottom) / 2.0;
            text(&mut self.labels, s, ui, msg, MAC_TITLE3_PT, Weight::Regular, LABEL_3.now(), x, mid, table_w - 2.0 * PAD);
            if !self.status.is_empty() {
                let status = self.status.clone();
                text(&mut self.labels, s, ui, &status, MAC_SUBHEAD_PT, Weight::Regular, colors::SYSTEM_ORANGE.now(), g.table_x + PAD, mid + 22.0, table_w - 2.0 * PAD);
            }
            return;
        }
        let (first, last) = self.row_range(ui);
        let offset = self.scroll.offset();
        for i in first..last {
            let seq = self.visible[i];
            let Some(e) = self.buffer.get(seq) else { continue };
            let y = g.table_top + i as f32 * ROW_PT - offset;
            let mid = y + ROW_PT / 2.0;
            let selected = self.selected == Some(seq);
            if selected {
                fill(s, ui, g.table_x, y, table_w, ROW_PT, SELECTION.now());
            }
            let ink = if selected { WHITE } else { LABEL.now() };
            let quiet = if selected { WHITE } else { LABEL_2.now() };
            let process = if e.process.is_empty() { "-" } else { e.process.as_str() };
            let message = one_line(&e.message);
            let stamp = clock(e.date);
            for &(col, x, w) in &g.cols {
                match col {
                    Col::Type => {
                        let c = if selected { WHITE } else { level_color(e.level).now() };
                        dot(s, ui, x + 4.0, mid - 3.5, 7.0, c);
                        text(&mut self.labels, s, ui, e.level.label(), MAC_SUBHEAD_PT, Weight::Regular, c, x + TYPE_PAD, mid, w - TYPE_PAD);
                    }
                    Col::Time => text(&mut self.labels, s, ui, &stamp, MAC_SUBHEAD_PT, Weight::Regular, quiet, x + PAD, mid, w - 2.0 * PAD),
                    Col::Process => text(&mut self.labels, s, ui, process, MAC_SUBHEAD_PT, Weight::Regular, ink, x + PAD, mid, w - 2.0 * PAD),
                    Col::Subsystem => text(&mut self.labels, s, ui, &e.subsystem, MAC_SUBHEAD_PT, Weight::Regular, quiet, x + PAD, mid, w - 2.0 * PAD),
                    Col::Category => text(&mut self.labels, s, ui, &e.category, MAC_SUBHEAD_PT, Weight::Regular, quiet, x + PAD, mid, w - 2.0 * PAD),
                    Col::Message => text(&mut self.labels, s, ui, &message, MAC_SUBHEAD_PT, Weight::Regular, ink, x + PAD, mid, w - 2.0 * PAD),
                }
            }
        }
    }

    /// The detail pane for the selected entry: its level, process, the fields, the message and any extra detail.
    fn draw_pane(&mut self, ui: &Ui, s: &mut Surface, g: &Geo, x: f32) {
        let Some(e) = self.selected_entry().cloned() else { return };
        let detail = self.selected_detail().map(|t| t.to_string());
        let w = g.w - x;
        fill(s, ui, x, UNIFIED_BAR_PT, w, g.h - UNIFIED_BAR_PT, CARD_BG.now());
        fill(s, ui, x, UNIFIED_BAR_PT, 1.0, g.h - UNIFIED_BAR_PT, SEPARATOR_C.now());
        let x0 = x + PANE_PAD;
        let inner = w - 2.0 * PANE_PAD;
        let bottom = g.h - FOOT_PT;
        let mut y = UNIFIED_BAR_PT + 16.0;

        let level = level_color(e.level).now();
        dot(s, ui, x0, y + 4.0, 9.0, level);
        text(&mut self.labels, s, ui, e.level.label(), MAC_BODY_PT, Weight::Semibold, level, x0 + 16.0, y + 8.5, inner - 16.0);
        y += 28.0;
        let title = if e.process.is_empty() { "Unknown process" } else { e.process.as_str() };
        text(&mut self.labels, s, ui, title, MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), x0, y + 9.0, inner);
        y += 30.0;

        let pid = if e.pid > 0 { e.pid.to_string() } else { "-".to_string() };
        let subsystem = if e.subsystem.is_empty() { "-".to_string() } else { e.subsystem.clone() };
        let category = if e.category.is_empty() { "-".to_string() } else { e.category.clone() };
        let fields = [("Date", full_date(e.date)), ("PID", pid), ("Subsystem", subsystem), ("Category", category)];
        for (key, value) in fields {
            text(&mut self.labels, s, ui, key, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), x0, y + 8.0, KEY_W - 8.0);
            text(&mut self.labels, s, ui, &value, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), x0 + KEY_W, y + 8.0, inner - KEY_W);
            y += 20.0;
        }
        y += 12.0;

        // (line, is a section heading)
        let mut body: Vec<(String, bool)> = Vec::new();
        if !e.message.is_empty() {
            body.push(("Message".into(), true));
            body.extend(wrap(ui, &e.message, MAC_SUBHEAD_PT, inner).into_iter().take(MAX_LINES).map(|l| (l, false)));
        }
        if let Some(extra) = detail {
            body.push((String::new(), false));
            body.push(("Details".into(), true));
            body.extend(extra.lines().take(MAX_LINES).map(|l| (l.to_string(), false)));
        }
        for (line, is_heading) in body {
            if y + 15.0 > bottom {
                break;
            }
            if is_heading {
                text(&mut self.labels, s, ui, &line, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), x0, y + 7.5, inner);
            } else if !line.is_empty() {
                text(&mut self.labels, s, ui, &line, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), x0, y + 7.5, inner);
            }
            y += 15.0;
        }
    }

    /// The toolbar band, the table header, the footer and the toolbar's items and search field.
    fn draw_chrome(&mut self, ui: &Ui, s: &mut Surface, g: &Geo) {
        let x0 = SIDE_PT;
        widgets::toolbar_band(s, ui, Rect { x: x0, y: 0.0, w: g.w - x0, h: UNIFIED_BAR_PT });
        let (bar, layout) = self.bar_layout(ui);
        widgets::toolbar(s, ui, self.band(ui), &bar, None);
        if let Some(r) = layout.rect_of(B_SEARCH) {
            widgets::search_field(s, ui, &self.search, Rect { x: r.x, y: r.y + 4.0, w: r.w, h: r.h - 8.0 }, "Search", self.search_focused);
        }

        let table_w = g.table_right - g.table_x;
        fill(s, ui, g.table_x, UNIFIED_BAR_PT, table_w, HEAD_PT, DETAIL_BG.now());
        for &(col, x, w) in &g.cols {
            let (title, pad) = match col {
                Col::Type => ("Type", TYPE_PAD),
                Col::Time => ("Time", PAD),
                Col::Process => ("Process", PAD),
                Col::Subsystem => ("Subsystem", PAD),
                Col::Category => ("Category", PAD),
                Col::Message => ("Message", PAD),
            };
            text(&mut self.labels, s, ui, title, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), x + pad, UNIFIED_BAR_PT + HEAD_PT / 2.0, w - 2.0 * PAD);
        }
        fill(s, ui, g.table_x, UNIFIED_BAR_PT + HEAD_PT - 1.0, table_w, 1.0, SEPARATOR_C.now());

        let fy = g.h - FOOT_PT;
        fill(s, ui, x0, fy, g.w - x0, FOOT_PT, DETAIL_BG.now());
        fill(s, ui, x0, fy, g.w - x0, 1.0, SEPARATOR_C.now());
        let footer = self.footer_text();
        text(&mut self.labels, s, ui, &footer, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x0 + 12.0, fy + FOOT_PT / 2.0, g.w - x0 - 24.0);
        if !self.status.is_empty() && !self.visible.is_empty() {
            let status = self.status.clone();
            let max = (g.w - x0) * 0.5;
            text(&mut self.labels, s, ui, &status, MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_ORANGE.now(), g.w - 12.0 - max, fy + FOOT_PT / 2.0, max);
        }
    }
}

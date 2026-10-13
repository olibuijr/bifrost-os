//! Reminders drawing: bifrost grouped look on black, bifrost corner radii.
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_TITLE2_PT};
use bifrost_app::kit::ui::{MAC_TITLE3_PT};
use springboard::shell::glyphs::{disc, either, paint, ring, segment};
use bifrost_app::kit::ui::{Weight, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, WHITE};
use bifrost_app::kit::ui::{CARD_BG, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LINK, SEPARATOR_C, TRACK_C};

/// Points: the empty-state message's top, the time wheel's colon and hour columns, the Undo label's room.
const EMPTY_TOP_PT: f32 = 100.0;
const COLON_X_PT: f32 = 130.0;
const HOUR_X_PT: f32 = 196.0;
const UNDO_LABEL_ROOM_PT: f32 = 200.0;
use super::*;
use bifrost_app::kit::Surface;
use bifrost_app::controls::tips::TipSection;

const BLUE: u32 = bifrost_app::kit::ui::SYSTEM_BLUE;
const RED: u32 = bifrost_app::kit::ui::SYSTEM_RED;
const ORANGE: u32 = bifrost_app::kit::ui::SYSTEM_ORANGE;
const GREEN: u32 = bifrost_app::kit::ui::SYSTEM_GREEN;
const PURPLE: u32 = bifrost_app::kit::ui::SYSTEM_PURPLE;
const PINK: u32 = bifrost_app::kit::ui::SYSTEM_PINK;
const TEAL: u32 = bifrost_app::kit::ui::SYSTEM_TEAL;
const BROWN: u32 = bifrost_app::kit::ui::SYSTEM_BROWN;
/// List colours, indexed by `ListDef::color`.
pub(super) const COLORS: [u32; 8] = [BLUE, RED, ORANGE, GREEN, PURPLE, PINK, TEAL, BROWN];
/// List glyphs, indexed by `ListDef::icon`.
pub(super) const ICONS: [&str; 6] = ["≡", "•", "▦", "✓", "!", "+"];
/// Corner radii in points: grouped cards and panels (`prefs_layout::m::CARD_RADIUS`), colour swatches
/// (`ICON_RADIUS`). Drawn through `ui.pt`, so they scale with the window.
const CARD_R: f32 = bifrost_app::controls::prefs_layout::m::CARD_RADIUS;
const SWATCH_R: f32 = bifrost_app::controls::prefs_layout::m::ICON_RADIUS;
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn date_label(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);
    format!("{} {} {}", date, MONTHS[month as usize - 1], year)
}
fn list_color(data: &Data, id: u64) -> u32 {
    data.lists
        .iter()
        .find(|l| l.id == id)
        .map_or(BLUE, |l| COLORS[l.color as usize % COLORS.len()])
}

impl Reminders {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, x: f32, y: f32) {
        let label = self.labels.get(ui, text, size, weight, color);
        s.blit(label, x as i32, y as i32, 255, 255);
    }
    #[allow(clippy::too_many_arguments)]
    fn center(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, x: f32, y: f32) {
        let label = self.labels.get(ui, text, size, weight, color);
        s.blit(label, (x - label.w as f32 / 2.0) as i32, (y - label.h as f32 / 2.0) as i32, 255, 255);
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn right(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, x_right: f32, y: f32) {
        let w = ui.measure(text, size, weight);
        self.text(s, ui, text, size, weight, color, x_right - w, y);
    }
    fn flag(&mut self, s: &mut Surface, (x, y): (f32, f32), size: f32, color: u32) {
        s.fill_rrect(x, y, (size * 0.12).max(1.5), size, 0.0, color);
        s.fill_rrect(x, y, size * 0.8, size * 0.55, 0.0, color);
    }
    fn chevron(&mut self, s: &mut Surface, (cx, cy): (f32, f32), size: f32, down: bool, color: u32) {
        let (a, b, c) = if down {
            ((cx - size, cy - size / 2.0), (cx, cy + size / 2.0), (cx + size, cy - size / 2.0))
        } else {
            ((cx - size / 2.0, cy - size), (cx + size / 2.0, cy), (cx - size / 2.0, cy + size))
        };
        paint(
            s,
            (cx - size - 3.0, cy - size - 3.0, 2.0 * size + 6.0, 2.0 * size + 6.0),
            color,
            either(segment(a, b, 2.5), segment(b, c, 2.5)),
        );
    }
    fn toggle(&mut self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), on: bool) {
        let (tw, th) = (ui.pt(51.0), ui.pt(31.0));
        let (tx, ty) = (x + w - tw - ui.pt(SPACE_L), y + (h - th) / 2.0);
        s.fill_rrect(tx, ty, tw, th, 1.0e4, if on { LINK } else { TRACK_C.now() });
        let knob = th - ui.pt(SPACE_XS);
        let kx = if on { tx + tw - knob - ui.pt(SPACE_XXS) } else { tx + ui.pt(SPACE_XXS) };
        // compliance-allow: COL-03 the switch knob is white in both schemes (the NativeSwitch knob), not a background
        s.fill_rrect(kx, ty + ui.pt(SPACE_XXS), knob, knob, 1.0e4, WHITE);
    }
    fn cell(&mut self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32)) {
        s.fill_rrect(x, y, w, h, ui.pt(CARD_R), CARD_BG.now());
    }
    /// A tappable row: label left, value right.
    fn value_row(&mut self, s: &mut Surface, ui: &Ui, rect: (f32, f32, f32, f32), label: &str, value: &str, color: u32) {
        self.cell(s, ui, rect);
        self.text(s, ui, label, MAC_BODY_PT, Weight::Regular, LABEL.now(), rect.0 + ui.pt(SPACE_L), rect.1 + ui.pt(SPACE_L));
        self.right(s, ui, value, MAC_BODY_PT, Weight::Regular, color, rect.0 + rect.2 - ui.pt(SPACE_L), rect.1 + ui.pt(SPACE_L));
    }

    /// A smart tile's glyph: a white disc with the tile's mark in its colour (Today shows the day of the month).
    pub(super) fn smart_glyph(&mut self, c: &mut Surface, ui: &Ui, smart: Smart, (cx, cy): (f32, f32), color: u32, today: i64) {
        let r = ui.pt(SPACE_M);
        paint(c, (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), WHITE, disc((cx, cy), r));
        match smart {
            Smart::Flagged => self.flag(c, (cx - ui.pt(SPACE_XS), cy - ui.pt(SPACE_S)), ui.pt(SPACE_L), color),
            Smart::Today => {
                let day = springboard::shell::clock::tz::civil_from_days(today).2;
                self.center(c, ui, &day.to_string(), MAC_FOOTNOTE_PT, Weight::Bold, color, cx, cy);
            }
            Smart::Scheduled => {
                // a calendar page: a dot grid in the tile's colour
                for k in 0..6 {
                    let (dx, dy) = ((k % 3) as f32 - 1.0, (k / 3) as f32 - 0.5);
                    c.fill_rrect(cx + ui.pt(4.5 * dx) - ui.pt(1.5), cy + ui.pt(4.5 * dy) - ui.pt(1.5), ui.pt(3.0), ui.pt(3.0), 1.0e4, color);
                }
            }
            Smart::All => {
                // a tray: three stacked bars
                for k in 0..3 {
                    c.fill_rrect(cx - ui.pt(SPACE_S - 2.0), cy - ui.pt(SPACE_XS + 1.0) + ui.pt(SPACE_XS) * k as f32, ui.pt(SPACE_L - 4.0), ui.pt(SPACE_XXS), 1.0e4, color);
                }
            }
            Smart::Completed => self.center(c, ui, "✓", MAC_BODY_PT, Weight::Bold, color, cx, cy),
        }
    }

    /// A pinned list tile's glyph: a white disc with the list's icon in the tile's colour.
    pub(super) fn list_glyph(&mut self, c: &mut Surface, ui: &Ui, list: u64, (cx, cy): (f32, f32), color: u32) {
        let r = ui.pt(SPACE_M);
        paint(c, (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), WHITE, disc((cx, cy), r));
        let icon = self.data.lists.iter().find(|l| l.id == list).map_or(0, |l| l.icon as usize);
        self.center(c, ui, ICONS[icon % ICONS.len()], MAC_BODY_PT, Weight::Bold, color, cx, cy);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_item(&mut self, c: &mut Surface, ui: &Ui, r: &Reminder, x: f32, y: f32, w: f32, depth: u8, shift: f32, last: bool) {
        let h = ui.pt(ITEM_H);
        let indent = ui.pt(30.0) * depth as f32;
        let accent = list_color(&self.data, r.list);
        c.fill_rrect(x, y, w + shift, h, 0.0, if self.selected == Some(r.id) { CARD_BG.now() } else { bifrost_app::kit::ui::CONTENT_BG.now() });
        let (cx, cy, rad) = (x + ui.pt(26.0) + indent, y + h / 2.0, ui.pt(SPACE_M));
        paint(c, (cx - rad - 2.0, cy - rad - 2.0, 2.0 * rad + 4.0, 2.0 * rad + 4.0), if r.completed { BLUE } else { accent }, ring((cx, cy), rad, ui.pt(1.5)));
        if r.completed {
            paint(c, (cx - rad, cy - rad, 2.0 * rad, 2.0 * rad), BLUE, disc((cx, cy), rad - ui.pt(SPACE_XS)));
        }
        let tx = x + ui.pt(52.0) + indent;
        let right_edge = x + w + shift - ui.pt(SPACE_L);
        let kids = depth == 0 && model::has_children(&self.data.items, r.id);
        let mut sub = String::new();
        if let Some(due) = r.due {
            sub = if due == self.today {
                "Today".into()
            } else if due == self.today + 1 {
                "Tomorrow".into()
            } else {
                date_label(due)
            };
            if let Some(t) = r.time {
                sub = format!("{sub}, {}", model::clock_label(t));
            }
            if let Some(rep) = r.repeat {
                sub = format!("{sub} · {}", rep.label());
            }
        } else if let Some(line) = r.notes.lines().next().filter(|l| !l.is_empty()) {
            sub = line.to_string();
        }
        // right-hand decorations, laid out right to left
        let mut rx = right_edge;
        if kids {
            let collapsed = self.collapsed.contains(&r.id);
            self.chevron(c, (rx - ui.pt(SPACE_M), cy), ui.pt(SPACE_XS), !collapsed, LABEL_2.now());
            rx -= ui.pt(SPACE_XXXL);
            let (done, total) = model::progress(&self.data.items, r.id);
            let p = format!("{done}/{total}");
            let pw = ui.measure(&p, MAC_SUBHEAD_PT, Weight::Regular);
            self.text(c, ui, &p, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), rx - pw, cy - ui.pt(SPACE_M));
            rx -= pw + ui.pt(SPACE_M);
        }
        if r.flagged {
            self.flag(c, (rx - ui.pt(SPACE_L), cy - ui.pt(SPACE_S)), ui.pt(SPACE_XL), ORANGE);
            rx -= ui.pt(SPACE_XXXL);
        }
        let mut title_x = tx;
        let ty = y + ui.pt(if sub.is_empty() { 15.0 } else { 6.0 });
        if r.priority > 0 {
            let marks = "!".repeat(r.priority as usize);
            self.text(c, ui, &marks, MAC_BODY_PT, Weight::Bold, accent, tx, ty);
            title_x += ui.measure(&marks, MAC_BODY_PT, Weight::Bold) + ui.pt(SPACE_S);
        }
        let title = ui.fit(&clean(&r.title), MAC_BODY_PT, (rx - title_x - ui.pt(SPACE_S)).max(ui.pt(SPACE_HUGE)));
        self.text(c, ui, &title, MAC_BODY_PT, Weight::Regular, if r.completed { LABEL_2.now() } else { LABEL.now() }, title_x, ty);
        if !sub.is_empty() {
            let overdue = r.due.is_some_and(|d| d < self.today) && !r.completed;
            let sub = ui.fit(&sub, MAC_FOOTNOTE_PT, (right_edge - tx - ui.pt(SPACE_S)).max(ui.pt(SPACE_HUGE)));
            self.text(c, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, if overdue { RED } else { LABEL_2.now() }, tx, y + ui.pt(26.0));
        }
        if !last {
            c.fill_rrect(tx, y + h - 1.0, w - (tx - x), 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    fn draw_items(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let wpx = self.w as f32;
        let (title, color) = match self.view {
            View::Smart(sm) => (sm.label().to_string(), BLUE),
            View::Tag(key) => (crate::model::tag_name(&self.data.items, key).map_or(String::new(), |n| format!("#{n}")), BLUE),
            View::Custom(id) => self
                .data
                .smarts
                .iter()
                .find(|m| m.id == id)
                .map_or((String::new(), BLUE), |m| (m.name.clone(), COLORS[m.color as usize % COLORS.len()])),
            View::List(id) => (
                self.data.lists.iter().find(|l| l.id == id).map_or(String::new(), |l| l.name.clone()),
                list_color(&self.data, id),
            ),
        };
        // the large title in the list's colour with the open count at the right (Golden Gate Reminders)
        let title = if self.searching { format!("Results for \u{201c}{}\u{201d}", self.title.text().trim()) } else { title };
        let open = self.rows.iter().filter(|r| matches!(r, Row::Item { idx, .. } if !self.data.items[*idx].completed)).count();
        let title = ui.fit(&title, MAC_LARGE_TITLE_PT, wpx - ui.pt(SPACE_HUGE + 60.0));
        // compliance-allow: LAY-07 the list title is tinted and has the open count at its right, which widgets::large_title cannot draw
        self.text(s, ui, &title, MAC_LARGE_TITLE_PT, Weight::Bold, color, ui.pt(SPACE_XL), ui.pt(self.top_pt + 62.0));
        self.right(s, ui, &open.to_string(), MAC_LARGE_TITLE_PT, Weight::Regular, color, wpx - ui.pt(SPACE_XL), ui.pt(self.top_pt + 62.0));
        let _ = t;
        let (top, bottom) = (self.top_px(ui), self.bottom(ui));
        let mut c = s.rows(top as usize, bottom.max(top) as usize);
        let rows = self.rows.clone();
        let dragged = self.drag.as_ref().map(|d| d.id);
        let mut floating: Option<(Reminder, u8)> = None;
        for (i, row) in rows.iter().enumerate() {
            let (x, y, w, h) = self.row_rect(ui, i);
            if y + h < top || y >= bottom {
                continue;
            }
            match row {
                Row::Header(label) => {
                    self.text(&mut c, ui, label, MAC_BODY_PT, Weight::Bold, LABEL.now(), x + ui.pt(SPACE_XS), y + ui.pt(SPACE_L));
                }
                Row::Section { name, .. } => {
                    // a section header: the name in the list's colour over a rule, and the number of reminders in it
                    let accent = match self.view {
                        View::List(id) => list_color(&self.data, id),
                        _ => LABEL.now(),
                    };
                    let name = ui.fit(name, MAC_BODY_PT, w - ui.pt(SPACE_HUGE));
                    self.text(&mut c, ui, &name, MAC_BODY_PT, Weight::Bold, accent, x + ui.pt(SPACE_XS), y + ui.pt(SPACE_L));
                    c.fill_rrect(x, y + h - 1.0, w, 1.0, 0.0, SEPARATOR_C.now());
                }
                Row::Item { idx, depth } => {
                    let r = self.data.items[*idx].clone();
                    if Some(r.id) == dragged {
                        c.fill_rrect(x, y, w, h, 0.0, SEPARATOR_C.now());
                        floating = Some((r, *depth));
                        continue;
                    }
                    let last = !matches!(rows.get(i + 1), Some(Row::Item { .. }));
                    let swipe = self.swipe.as_ref().filter(|sw| sw.id == r.id).map(|sw| sw.x);
                    if let Some(lead) = swipe.filter(|sx| *sx > 0.0) {
                        // Swiped right: green reveal on the leading edge; the row content moves with it.
                        c.fill_rrect(x, y, lead, h, 0.0, GREEN);
                        if lead > ui.pt(48.0) {
                            self.center(&mut c, ui, "Complete", MAC_SUBHEAD_PT, Weight::Semibold, WHITE, x + lead / 2.0, y + h / 2.0);
                        }
                        self.draw_item(&mut c, ui, &r, x + lead, y, w - lead, *depth, 0.0, last);
                        continue;
                    }
                    if let Some(sx) = swipe.filter(|sx| *sx < 0.0) {
                        let half = -sx / 2.0;
                        let right = x + w;
                        c.fill_rrect(right + sx, y, half, h, 0.0, ORANGE);
                        c.fill_rrect(right + sx + half, y, half, h, 0.0, DESTRUCTIVE);
                        if half > ui.pt(48.0) {
                            let flag_label = if r.flagged { "Unflag" } else { "Flag" };
                            self.center(&mut c, ui, flag_label, MAC_SUBHEAD_PT, Weight::Semibold, WHITE, right + sx + half / 2.0, y + h / 2.0);
                            self.center(&mut c, ui, "Delete", MAC_SUBHEAD_PT, Weight::Semibold, WHITE, right + sx + half * 1.5, y + h / 2.0);
                        }
                    }
                    self.draw_item(&mut c, ui, &r, x, y, w, *depth, swipe.unwrap_or(0.0), last);
                }
            }
        }
        let drag_y = self.drag.as_ref().map(|d| d.y);
        if let (Some(dy), Some((r, _))) = (drag_y, floating) {
            let (x, _, w, h) = self.row_rect(ui, 0);
            let y = (dy - h / 2.0).clamp(top, (bottom - h).max(top));
            self.draw_item(&mut c, ui, &r, x, y, w, 0, 0.0, true);
            c.stroke_rrect(x, y, w, h, 0.0, 1.5, BLUE);
        }
        if self.rows.is_empty() {
            let msg = if self.searching { "No Results" } else { "No Reminders" };
            self.center(&mut c, ui, msg, MAC_TITLE3_PT, Weight::Semibold, LABEL_2.now(), wpx / 2.0, top + ui.pt(EMPTY_TOP_PT));
        }
    }

    fn draw_options(&mut self, s: &mut Surface, ui: &Ui) {
        let layout = self.options_layout(ui);
        if let (Some(first), Some(last)) = (layout.first(), layout.last()) {
            let (x, y) = (first.3 .0, first.3 .1);
            s.fill_rrect(x - 1.0, y - 1.0, first.3 .2 + 2.0, last.3 .1 + last.3 .3 - y + 2.0, ui.pt(CARD_R), SEPARATOR_C.now());
            s.fill_rrect(x, y, first.3 .2, last.3 .1 + last.3 .3 - y, ui.pt(CARD_R), CARD_BG.now());
        }
        for (target, label, on, rect) in layout {
            match target {
                None => self.text(s, ui, &label, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), rect.0 + ui.pt(SPACE_L), rect.1 + ui.pt(SPACE_M)),
                Some(Target::OptCompleted) => {
                    self.text(s, ui, &label, MAC_BODY_PT, Weight::Regular, LABEL.now(), rect.0 + ui.pt(SPACE_L), rect.1 + ui.pt(SPACE_L));
                    self.toggle(s, ui, rect, on);
                }
                Some(_) => {
                    self.text(s, ui, &label, MAC_BODY_PT, Weight::Regular, LABEL.now(), rect.0 + ui.pt(SPACE_L), rect.1 + ui.pt(SPACE_M));
                    if on {
                        self.right(s, ui, "✓", MAC_BODY_PT, Weight::Bold, BLUE, rect.0 + rect.2 - ui.pt(SPACE_L), rect.1 + ui.pt(SPACE_M));
                    }
                }
            }
        }
    }

    fn draw_calendar(&mut self, s: &mut Surface, ui: &Ui, (cx, cy, cw, ch): (f32, f32, f32, f32), selected: i64, month: i64) {
        self.cell(s, ui, (cx, cy, cw, ch));
        let (year, month_no, _) = civil_from_days(month);
        self.center(s, ui, &format!("{} {}", MONTHS[month_no as usize - 1], year), MAC_TITLE2_PT, Weight::Semibold, LABEL.now(), cx + cw / 2.0, cy + ui.pt(SPACE_XXL));
        self.center(s, ui, "‹", MAC_LARGE_TITLE_PT, Weight::Regular, LINK, cx + ui.pt(28.0), cy + ui.pt(SPACE_XXL));
        self.center(s, ui, "›", MAC_LARGE_TITLE_PT, Weight::Regular, LINK, cx + cw - ui.pt(28.0), cy + ui.pt(SPACE_XXL));
        for (i, label) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
            self.center(s, ui, label, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), cx + cw / 7.0 * (i as f32 + 0.5), cy + ui.pt(59.0));
        }
        let (first, count, start) = self.calendar_days();
        for day in 0..count {
            let index = start + day as usize;
            let dx = cx + cw / 7.0 * ((index % 7) as f32 + 0.5);
            let dy = cy + ui.pt(90.0 + 36.0 * (index / 7) as f32);
            let value = first + day as i64;
            if selected == value {
                s.fill_rrect(dx - ui.pt(SPACE_L), dy - ui.pt(SPACE_L), ui.pt(SPACE_XXXL), ui.pt(SPACE_XXXL), 1.0e4, BLUE);
            }
            let color = if value == selected { WHITE } else if value == self.today { BLUE } else { LABEL.now() };
            self.center(s, ui, &(day + 1).to_string(), MAC_BODY_PT, Weight::Regular, color, dx, dy);
        }
    }

    fn draw_form(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        // Cancel, the heading and Done are in the toolbar (`chrome`)
        let (top, bottom) = (self.top_px(ui), self.view_bottom(ui));
        let mut c = s.rows(top as usize, bottom.max(top) as usize);
        let rows = self.form_rows().0;
        for (target, ry, rh) in rows {
            let rect = self.form_rect(ui, ry, rh);
            if rect.1 + rect.3 < top || rect.1 >= bottom {
                continue;
            }
            self.draw_form_row(&mut c, ui, target, rect, t);
        }
    }

    /// The first-use alert tip: a card with the tip's title and message (`TipView`, Mini style). The first time
    /// it is drawn it is recorded as shown.
    fn draw_tip(&mut self, c: &mut Surface, ui: &Ui, rect: (f32, f32, f32, f32)) {
        let (x, y, _, _) = rect;
        let now = now_seconds();
        let sections = self.tip_sections(now);
        if !sections.is_empty() && !self.tip_marked {
            self.tip_marked = true;
            if let Some(store) = &mut self.tips {
                let _ = store.mark_shown(AlertHint.id(), now);
            }
        }
        self.cell(c, ui, rect);
        let mut line_y = y + ui.pt(SPACE_M);
        for section in &sections {
            match section {
                TipSection::Title(title) => {
                    self.text(c, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), x + ui.pt(SPACE_L), line_y);
                    line_y += ui.pt(MAC_BODY_PT) + ui.pt(SPACE_XS);
                }
                TipSection::Message(message) => {
                    self.text(c, ui, message, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + ui.pt(SPACE_L), line_y);
                }
                TipSection::Actions(_) => {}
            }
        }
    }

    fn draw_form_row(&mut self, c: &mut Surface, ui: &Ui, target: Target, rect: (f32, f32, f32, f32), t: f64) {
        let (x, y, w, h) = rect;
        let draft = self.editor.as_ref().map(|e| e.draft.clone());
        match target {
            Target::Field(f) => {
                self.cell(c, ui, rect);
                // the kit's field look follows Light and Dark; the shell field keeps the text and caret
                let field = self.field(f);
                let mut kit = bifrost_app::controls::TextField::new();
                kit.push_str(field.text());
                kit.home();
                for _ in 0..field.text().get(..field.caret()).map_or(field.caret(), |p| p.chars().count()) {
                    kit.move_caret(true);
                }
                let k = ui.pt(1.0);
                let r = bifrost_app::controls::Rect { x: x / k + SPACE_M, y: y / k + SPACE_S + 2.0, w: w / k - 2.0 * SPACE_M, h: h / k - 2.0 * (SPACE_S + 2.0) };
                let hint = match f { Field::Title => self.title_hint, Field::Notes => "Notes", Field::Url => "URL" };
                bifrost_app::widgets::text_field(c, ui, &kit, r, hint, field.focused);
                let _ = t;
            }
            Target::Tip => self.draw_tip(c, ui, rect),
            Target::Due => {
                self.cell(c, ui, rect);
                self.text(c, ui, "Date", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                let on = draft.as_ref().is_some_and(|d| d.due.is_some());
                self.toggle(c, ui, rect, on);
            }
            Target::Calendar => {
                let month = self.editor.as_ref().map(|e| e.month);
                if let (Some(d), Some(month)) = (&draft, month) {
                    let selected = d.due.unwrap_or(self.today);
                    self.draw_calendar(c, ui, rect, selected, month);
                }
            }
            Target::Time => {
                self.cell(c, ui, rect);
                self.text(c, ui, "Time", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                let on = draft.as_ref().is_some_and(|d| d.time.is_some());
                self.toggle(c, ui, rect, on);
            }
            Target::TimeStep => {
                self.cell(c, ui, rect);
                let time = draft.as_ref().and_then(|d| d.time).unwrap_or(0);
                self.text(c, ui, "At", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                let right = x + w;
                let cy = y + h / 2.0;
                for (centre, glyph) in [(24.0, "›"), (104.0, "‹"), (156.0, "›"), (236.0, "‹")] {
                    self.center(c, ui, glyph, MAC_LARGE_TITLE_PT, Weight::Regular, LINK, right - ui.pt(centre), cy);
                }
                self.center(c, ui, &format!("{:02}", time % 60), MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), right - ui.pt(64.0), cy);
                self.center(c, ui, ":", MAC_TITLE3_PT, Weight::Semibold, LABEL_2.now(), right - ui.pt(COLON_X_PT), cy);
                self.center(c, ui, &format!("{:02}", time / 60), MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), right - ui.pt(HOUR_X_PT), cy);
            }
            Target::Repeat => {
                let v = draft.as_ref().and_then(|d| d.repeat).map_or("Never".to_string(), |r| r.label());
                self.value_row(c, ui, rect, "Repeat", &v, LABEL_2.now());
            }
            Target::Early => {
                let v = draft.as_ref().and_then(|d| d.early).map_or("None".to_string(), model::early_label);
                self.value_row(c, ui, rect, "Early Reminder", &v, LABEL_2.now());
            }
            Target::Flag => {
                self.cell(c, ui, rect);
                self.text(c, ui, "Flag", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                let on = draft.as_ref().is_some_and(|d| d.flagged);
                self.toggle(c, ui, rect, on);
            }
            Target::Priority => {
                let p = draft.as_ref().map_or(0, |d| d.priority);
                self.value_row(c, ui, rect, "Priority", model::priority_label(p), LABEL_2.now());
            }
            Target::MoveList => {
                let id = draft.as_ref().map_or(0, |d| d.list);
                let name = self.data.lists.iter().find(|l| l.id == id).map_or(String::new(), |l| l.name.clone());
                self.value_row(c, ui, rect, "List", &name, list_color(&self.data, id));
            }
            Target::Indent => {
                let sub = draft.as_ref().is_some_and(|d| d.parent.is_some());
                self.cell(c, ui, rect);
                self.text(c, ui, if sub { "Make Regular Reminder" } else { "Make Subtask" }, MAC_BODY_PT, Weight::Regular, LINK, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
            }
            Target::Duplicate => {
                self.cell(c, ui, rect);
                self.center(c, ui, "Duplicate Reminder", MAC_BODY_PT, Weight::Regular, LINK, x + w / 2.0, y + h / 2.0);
            }
            Target::Rule(row) => {
                let rules = self.list_edit.as_ref().map(|e| e.rules).unwrap_or_default();
                let value = match row {
                    RuleRow::Due => rules.due.label().to_string(),
                    RuleRow::Time => rules.time.label().to_string(),
                    RuleRow::Priority => model::priority_rule_label(rules.priority).to_string(),
                    RuleRow::Flagged => rules.flagged.label().to_string(),
                    RuleRow::Tag => rules.tag.map_or("Any".to_string(), |key| model::tag_name(&self.data.items, key).map_or("Unused Tag".to_string(), |n| format!("#{n}"))),
                    RuleRow::List => rules.list.map_or("Any".to_string(), |l| self.data.lists.iter().find(|x| x.id == l).map_or("Unknown List".to_string(), |x| x.name.clone())),
                };
                self.value_row(c, ui, rect, row.label(), &value, LABEL_2.now());
            }
            Target::Setting(row) => match row {
                SettingRow::DefaultList => {
                    let value = self
                        .settings
                        .default_list
                        .and_then(|id| self.data.lists.iter().find(|l| l.id == id))
                        .map_or("Automatic".to_string(), |l| l.name.clone());
                    self.value_row(c, ui, rect, "New Reminders Go To", &value, LABEL_2.now());
                }
                SettingRow::AllDayAlert => {
                    let value = model::clock_label(self.settings.all_day_min);
                    self.value_row(c, ui, rect, "All-Day Reminders Alert At", &value, LABEL_2.now());
                }
                SettingRow::Badge => {
                    self.cell(c, ui, rect);
                    self.text(c, ui, "Badge Due Reminders on the Dock", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                    let on = self.settings.badge;
                    self.toggle(c, ui, rect, on);
                }
            },
            Target::PinList => {
                self.cell(c, ui, rect);
                self.text(c, ui, "Pin in Sidebar", MAC_BODY_PT, Weight::Regular, LABEL.now(), x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
                let on = self.list_edit.as_ref().is_some_and(|e| e.pinned);
                self.toggle(c, ui, rect, on);
            }
            Target::ListGroup => {
                let group = self.list_edit.as_ref().and_then(|e| e.group);
                let name = group.and_then(|g| self.data.groups.iter().find(|x| x.id == g)).map_or("None".to_string(), |g| g.name.clone());
                self.value_row(c, ui, rect, "Group", &name, LABEL_2.now());
            }
            Target::Delete => {
                self.cell(c, ui, rect);
                let label = match (self.editor.is_some(), self.list_edit.as_ref().map(|e| e.mode)) {
                    (true, _) => "Delete Reminder",
                    (false, Some(EditMode::Section(_))) => "Delete Section",
                    (false, Some(EditMode::Group)) => "Delete Group",
                    (false, Some(EditMode::Smart)) => "Delete Smart List",
                    (false, _) => "Delete List",
                };
                self.center(c, ui, label, MAC_BODY_PT, Weight::Regular, DESTRUCTIVE, x + w / 2.0, y + h / 2.0);
            }
            Target::Colors => {
                self.cell(c, ui, rect);
                let sel = self.list_edit.as_ref().map_or(0, |e| e.color) as usize;
                let cell_w = w / COLORS.len() as f32;
                for (i, col) in COLORS.iter().enumerate() {
                    let (cx, cy) = (x + cell_w * (i as f32 + 0.5), y + h / 2.0);
                    let sz = ui.pt(SPACE_XXXL);
                    if i == sel {
                        c.fill_rrect(cx - sz / 2.0 - 4.0, cy - sz / 2.0 - 4.0, sz + 8.0, sz + 8.0, ui.pt(SWATCH_R + 2.0), LABEL.now());
                    }
                    c.fill_rrect(cx - sz / 2.0, cy - sz / 2.0, sz, sz, ui.pt(SWATCH_R), *col);
                }
            }
            Target::Icons => {
                self.cell(c, ui, rect);
                let (sel, col) = self.list_edit.as_ref().map_or((0, 0), |e| (e.icon as usize, e.color as usize));
                let cell_w = w / ICONS.len() as f32;
                for (i, glyph) in ICONS.iter().enumerate() {
                    let (cx, cy) = (x + cell_w * (i as f32 + 0.5), y + h / 2.0);
                    let sz = ui.pt(36.0);
                    c.fill_rrect(cx - sz / 2.0, cy - sz / 2.0, sz, sz, ui.pt(SWATCH_R), if i == sel { COLORS[col % COLORS.len()] } else { FIELD_BG.now() });
                    self.center(c, ui, glyph, MAC_TITLE2_PT, Weight::Bold, if i == sel { WHITE } else { LABEL.now() }, cx, cy);
                }
            }
            Target::ListUp | Target::ListDown => {
                self.cell(c, ui, rect);
                let label = if target == Target::ListUp { "Move Up" } else { "Move Down" };
                self.text(c, ui, label, MAC_BODY_PT, Weight::Regular, LINK, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L));
            }
            _ => {}
        }
    }
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.layout_columns(ui);
        self.fit_scroll(ui);
        self.background(s);
        {
            let mut col = self.column(s);
            let mut content = col.rows(ui.pt(self.top_pt) as usize, (self.h as f32 - home_zone(ui)) as usize);
            match self.screen {
                Screen::Items => self.draw_items(&mut content, ui, t),
                Screen::Editor | Screen::ListEditor => self.draw_form(&mut content, ui, t),
            }
            if self.options_open && matches!(self.screen, Screen::Items) {
                self.draw_options(&mut content, ui);
            }
            let wpx = self.w as f32;
            if let (Some(bar), true) = (&self.undo, matches!(self.screen, Screen::Items)) {
                let label = bar.label.clone();
                let y = self.bottom(ui);
                content.fill_rrect(ui.pt(SPACE_XL), y + ui.pt(SPACE_XS), wpx - ui.pt(SPACE_HUGE), ui.pt(44.0), ui.pt(CARD_R), CARD_BG.now());
                content.stroke_rrect(ui.pt(SPACE_XL), y + ui.pt(SPACE_XS), wpx - ui.pt(SPACE_HUGE), ui.pt(44.0), ui.pt(CARD_R), 1.0, SEPARATOR_C.now());
                let label = ui.fit(&label, MAC_SUBHEAD_PT, wpx - ui.pt(UNDO_LABEL_ROOM_PT));
                self.text(&mut content, ui, &label, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), ui.pt(36.0), y + ui.pt(SPACE_L));
                self.right(&mut content, ui, "Undo", MAC_BODY_PT, Weight::Semibold, LINK, wpx - ui.pt(36.0), y + ui.pt(SPACE_L));
            }
            if let Some(notice) = self.notice.clone() {
                let y = if matches!(self.screen, Screen::Editor | Screen::ListEditor) { ui.pt(self.top_pt + 58.0) } else { self.bottom(ui) - ui.pt(SPACE_L) };
                self.center(&mut content, ui, &notice, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), wpx / 2.0, y);
            }
            if self.error {
                let msg = if self.load_error { "Could not read reminders. Existing file preserved." } else { "Could not save reminders. Try again." };
                let y = if matches!(self.screen, Screen::Editor | Screen::ListEditor) { ui.pt(self.top_pt + 58.0) } else { self.bottom(ui) - ui.pt(SPACE_L) };
                self.center(&mut content, ui, msg, MAC_FOOTNOTE_PT, Weight::Regular, RED, wpx / 2.0, y);
            }
        }
        self.draw_chrome(s, ui);
        self.dirty = false;
    }
}

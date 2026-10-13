//! The Messages window's drawing and hit-testing. Every size is in points and goes through `ui.pt`. The room list is the
//! sidebar; the timeline and the composer are the main pane; the sign-in form is a sheet over both. [`Layout`] holds the
//! geometry for the painter and the hit test alike, so a tap lands on what is drawn.

use crate::model::{day_label, day_of, preview, starts_day, typing_text, FIELD_LABELS, PASSWORD_FIELD};
use crate::protocol::Body;
use crate::window::MessagesWindow;
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT, SEPARATOR_C, SPACE_M, SPACE_S, WHITE};
use bifrost_app::kit::{colors, Surface, TextCache, Ui, Weight};

pub const HEADER_PT: f32 = 52.0;
const SIDE_MIN_PT: f32 = 220.0;
const SIDE_MAX_PT: f32 = 320.0;
const SIDE_SHARE: f32 = 0.3;
const ROW_PT: f32 = 62.0;
const COMPOSER_PT: f32 = 58.0;
const BUTTON_H_PT: f32 = 28.0;
const IMAGE_W_PT: f32 = 76.0;
const SEND_W_PT: f32 = 64.0;
const BUBBLE_PAD_PT: f32 = 10.0;
const BUBBLE_LINE_PT: f32 = 18.0;
const BUBBLE_GAP_PT: f32 = 8.0;
const FOOT_LINE_PT: f32 = 15.0;
/// Widest a bubble gets, as a share of the main pane.
const BUBBLE_SHARE: f32 = 0.62;
const RADIUS_PT: f32 = 12.0;
const BADGE_PT: f32 = 16.0;
const SHEET_W_PT: f32 = 420.0;
const SHEET_HEAD_PT: f32 = 44.0;
const SHEET_FIELD_PT: f32 = 58.0;
const SHEET_LABEL_PT: f32 = 16.0;
const SHEET_BOX_PT: f32 = 30.0;
const SHEET_FOOT_PT: f32 = 68.0;
const SHEET_BUTTON_W_PT: f32 = 120.0;
const PANEL_RADIUS_PT: f32 = 12.0;

/// A rectangle in pixels: x, y, width, height.
pub type Rect = [f32; 4];

pub fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// What a tap landed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A conversation in the list, by room id.
    Room(String),
    Image,
    Send,
    /// A field of the sign-in sheet.
    Field(usize),
    SignIn,
    /// Inside the sign-in sheet, on nothing that acts.
    Panel,
    Nothing,
}

/// The pixel geometry of the window for one size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    /// The width of the room list.
    pub side: f32,
    pub header: f32,
    pub row: f32,
    pub composer: f32,
    pub pad: f32,
    pub gap: f32,
    pub rule: f32,
    pub radius: f32,
    pub button_h: f32,
    pub image_w: f32,
    pub send_w: f32,
    pub bubble_pad: f32,
    pub line: f32,
    pub foot: f32,
    pub bubble_gap: f32,
}

/// The sign-in sheet's rectangles: the panel, each field's label line and box, the sign-in button and the error line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetGeometry {
    pub panel: Rect,
    pub labels: [f32; 3],
    pub boxes: [Rect; 3],
    pub button: Rect,
    pub error_y: f32,
    pub error_x: f32,
}

impl Layout {
    pub fn new(ui: &Ui, w: f32, h: f32) -> Layout {
        let pt = ui.pt(1.0);
        let width_pt = (w / pt).max(0.0);
        let side = (width_pt * SIDE_SHARE).clamp(SIDE_MIN_PT, SIDE_MAX_PT).min(width_pt) * pt;
        Layout {
            w,
            h,
            side,
            header: ui.pt(HEADER_PT),
            row: ui.pt(ROW_PT),
            composer: ui.pt(COMPOSER_PT),
            pad: ui.pt(SPACE_M),
            gap: ui.pt(SPACE_S),
            rule: pt.max(1.0),
            radius: ui.pt(RADIUS_PT),
            button_h: ui.pt(BUTTON_H_PT),
            image_w: ui.pt(IMAGE_W_PT),
            send_w: ui.pt(SEND_W_PT),
            bubble_pad: ui.pt(BUBBLE_PAD_PT),
            line: ui.pt(BUBBLE_LINE_PT),
            foot: ui.pt(FOOT_LINE_PT),
            bubble_gap: ui.pt(BUBBLE_GAP_PT),
        }
    }

    /// The row of the conversation at `index`, moved up by the list's scroll.
    pub fn room_row(&self, index: usize, scroll: f32) -> Rect {
        [0.0, self.header + index as f32 * self.row - scroll, self.side, self.row]
    }

    pub fn sidebar(&self) -> Rect {
        [0.0, self.header, self.side, (self.h - self.header).max(0.0)]
    }

    /// The timeline: the main pane between the header and the composer.
    pub fn timeline(&self) -> Rect {
        [self.side, self.header, (self.w - self.side).max(0.0), (self.h - self.header - self.composer).max(0.0)]
    }

    pub fn send_button(&self) -> Rect {
        let x = self.w - self.pad - self.send_w;
        [x, self.h - self.composer / 2.0 - self.button_h / 2.0, self.send_w, self.button_h]
    }

    pub fn image_button(&self) -> Rect {
        let s = self.send_button();
        [s[0] - self.gap - self.image_w, s[1], self.image_w, self.button_h]
    }

    pub fn composer_field(&self) -> Rect {
        let b = self.image_button();
        let x = self.side + self.pad;
        [x, b[1], (b[0] - self.gap - x).max(0.0), self.button_h]
    }

    /// The sign-in sheet, centred over the window (below the header when the window is short).
    pub fn sheet(&self, ui: &Ui) -> SheetGeometry {
        let pw = ui.pt(SHEET_W_PT).min(self.w - 2.0 * self.pad).max(0.0);
        let head = ui.pt(SHEET_HEAD_PT);
        let field = ui.pt(SHEET_FIELD_PT);
        let label = ui.pt(SHEET_LABEL_PT);
        let box_h = ui.pt(SHEET_BOX_PT);
        let ph = head + self.pad + 3.0 * field + ui.pt(SHEET_FOOT_PT);
        let px = ((self.w - pw) / 2.0).max(0.0);
        let py = ((self.h - ph) / 2.0).max(self.header + self.pad);
        let labels = [0usize, 1, 2].map(|i| py + head + self.pad + i as f32 * field);
        let boxes = [0usize, 1, 2].map(|i| [px + self.pad, labels[i] + label, (pw - 2.0 * self.pad).max(0.0), box_h]);
        let bw = ui.pt(SHEET_BUTTON_W_PT);
        let button = [px + pw - self.pad - bw, py + ph - self.pad - self.button_h, bw, self.button_h];
        SheetGeometry {
            panel: [px, py, pw, ph],
            labels,
            boxes,
            button,
            error_y: button[1] + (self.button_h - self.foot) / 2.0,
            error_x: px + self.pad,
        }
    }
}

impl MessagesWindow {
    /// What a tap at (`x`, `y`) lands on. The sheet covers the window while it is open.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Hit {
        let lay = Layout::new(ui, self.w as f32, self.h as f32);
        if self.model.sign_in.is_some() {
            let sheet = lay.sheet(ui);
            if let Some(i) = (0..3).find(|&i| inside(sheet.boxes[i], x, y)) {
                return Hit::Field(i);
            }
            if inside(sheet.button, x, y) {
                return Hit::SignIn;
            }
            return if inside(sheet.panel, x, y) { Hit::Panel } else { Hit::Nothing };
        }
        if inside(lay.image_button(), x, y) {
            return Hit::Image;
        }
        if inside(lay.send_button(), x, y) {
            return Hit::Send;
        }
        if inside(lay.sidebar(), x, y) {
            let index = ((y - lay.header + self.room_scroll.offset()) / lay.row).floor();
            if index >= 0.0 {
                if let Some(room) = self.model.rooms.get(index as usize) {
                    return Hit::Room(room.id.clone());
                }
            }
        }
        Hit::Nothing
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        let lay = Layout::new(ui, self.w as f32, self.h as f32);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        self.draw_sidebar(s, ui, &lay);
        self.draw_timeline(s, ui, &lay);
        self.draw_header(s, ui, &lay);
        self.draw_composer(s, ui, &lay);
        if self.model.sign_in.is_some() {
            self.draw_sheet(s, ui, &lay);
        }
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let side = lay.sidebar();
        s.fill_rect(px(side[0]), px(side[1]), px(side[2]), px(side[3]), CARD_BG.now());
        s.fill_rect(px(lay.side), 0, px(lay.rule), px(lay.h), SEPARATOR_C.now());
        put_mid(s, &mut self.labels, ui, "Conversations", MAC_BODY_PT, Weight::Semibold, LABEL.now(), lay.pad, lay.header / 2.0);
        let rows = self.model.rooms.len() as f32 * lay.row;
        self.room_scroll.set_max((rows - side[3]).max(0.0));
        let scroll = self.room_scroll.offset();
        if self.model.rooms.is_empty() {
            let hint = if self.model.signed_in() { "No conversations yet" } else { "Sign in to see your conversations" };
            put(s, &mut self.labels, ui, hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), lay.pad, lay.header + lay.pad);
        }
        let inset = lay.gap;
        let text_w = (lay.side - 2.0 * lay.pad).max(0.0);
        for (i, room) in self.model.rooms.iter().enumerate() {
            let r = lay.room_row(i, scroll);
            if r[1] + r[3] < lay.header || r[1] > lay.h {
                continue;
            }
            let selected = self.model.selected.as_deref() == Some(room.id.as_str());
            if selected {
                s.fill_rrect(r[0] + inset, r[1] + inset / 2.0, r[2] - 2.0 * inset, r[3] - inset, lay.radius, colors::SYSTEM_BLUE.now());
            }
            let (c_name, c_sub) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
            let badge = if room.unread > 0 { Some(badge_text(room.unread)) } else { None };
            let badge_w = badge.as_ref().map_or(0.0, |b| ui.measure(b, MAC_FOOTNOTE_PT, Weight::Semibold) + lay.pad);
            let name_w = (text_w - badge_w).max(0.0);
            let name = fit(ui, &room.name, MAC_BODY_PT, Weight::Semibold, name_w);
            put(s, &mut self.labels, ui, &name, MAC_BODY_PT, Weight::Semibold, c_name, r[0] + lay.pad, r[1] + lay.row * 0.16);
            let sub = fit(ui, &room.preview(), MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, c_sub, r[0] + lay.pad, r[1] + lay.row * 0.56);
            if let Some(b) = badge {
                let bw = ui.measure(&b, MAC_FOOTNOTE_PT, Weight::Semibold) + lay.pad;
                let bx = r[0] + r[2] - lay.pad - bw;
                let by = r[1] + (lay.row - ui.pt(BADGE_PT)) / 2.0;
                let (fill, text) = if selected { (WHITE, colors::SYSTEM_BLUE.now()) } else { (colors::SYSTEM_BLUE.now(), WHITE) };
                s.fill_rrect(bx, by, bw, ui.pt(BADGE_PT), ui.pt(BADGE_PT) / 2.0, fill);
                put_centred(s, &mut self.labels, ui, &b, MAC_FOOTNOTE_PT, Weight::Semibold, text, bx + bw / 2.0, by + ui.pt(BADGE_PT) / 2.0);
            }
            if !selected {
                s.fill_rect(px(r[0] + lay.pad), px(r[1] + r[3] - lay.rule), px((r[2] - lay.pad).max(0.0)), px(lay.rule), SEPARATOR_C.now());
            }
        }
    }

    fn draw_header(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let x0 = lay.side;
        let width = (lay.w - x0).max(0.0);
        s.fill_rect(px(x0), 0, px(width), px(lay.header), CONTENT_BG.now());
        s.fill_rect(px(x0), px(lay.header - lay.rule), px(width), px(lay.rule), SEPARATOR_C.now());
        let title = match self.model.selected_room() {
            Some(room) => room.name.clone(),
            None => String::new(),
        };
        let title = fit(ui, &title, MAC_TITLE1_PT, Weight::Semibold, (width - 2.0 * lay.pad).max(0.0));
        put_mid(s, &mut self.labels, ui, &title, MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x0 + lay.pad, lay.header / 2.0);
        let status = self.model.status.clone();
        if !status.is_empty() {
            let shown = fit(ui, &status, MAC_FOOTNOTE_PT, Weight::Regular, (width / 2.0).max(0.0));
            let sw = ui.measure(&shown, MAC_FOOTNOTE_PT, Weight::Regular);
            put_mid(s, &mut self.labels, ui, &shown, MAC_FOOTNOTE_PT, Weight::Regular, DESTRUCTIVE, lay.w - lay.pad - sw, lay.header / 2.0);
        }
    }

    fn draw_timeline(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let area = lay.timeline();
        s.fill_rect(px(area[0]), px(area[1]), px(area[2]), px(area[3]), CONTENT_BG.now());
        let rows = self.timeline_rows(ui, lay);
        let typing = self.model.selected_room().is_some_and(|r| !r.typing.is_empty());
        let typing_h = if typing { typing_height(lay) } else { 0.0 };
        let content: f32 = rows.iter().map(|r| r.height + lay.bubble_gap).sum::<f32>() + 2.0 * lay.pad + typing_h;
        let max = (content - area[3]).max(0.0);
        self.timeline_scroll.set_max(max);
        if self.stick_bottom {
            self.timeline_scroll.jump_to(max);
        }
        let Some(room) = self.model.selected_room() else {
            let hint = if self.model.signed_in() { "Choose a conversation" } else { "Sign in to Matrix to start" };
            put_centred(s, &mut self.labels, ui, hint, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), area[0] + area[2] / 2.0, area[1] + area[3] / 2.0);
            return;
        };
        if room.timeline.is_empty() {
            put_centred(s, &mut self.labels, ui, "No messages yet", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), area[0] + area[2] / 2.0, area[1] + area[3] / 2.0);
            return;
        }
        let base = if content < area[3] { area[1] + (area[3] - content) } else { area[1] };
        let mut y = base + lay.pad - self.timeline_scroll.offset();
        let bubble_max = area[2] * BUBBLE_SHARE;
        for row in &rows {
            let top = y;
            y += row.height + lay.bubble_gap;
            if y < area[1] || top > area[1] + area[3] {
                continue;
            }
            if let Some(day) = &row.day {
                let cy = top + lay.gap + lay.foot / 2.0;
                put_centred(s, &mut self.labels, ui, day, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), area[0] + area[2] / 2.0, cy);
            }
            let right = area[0] + area[2] - lay.pad;
            let bw = (row.widest + 2.0 * lay.bubble_pad).min(bubble_max);
            let x = if row.own { right - bw } else { area[0] + lay.pad };
            let mut by = top + row.head;
            if let Some(name) = &row.sender {
                put(s, &mut self.labels, ui, name, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), x, by);
                by += lay.foot;
            }
            let (fill, ink) = if row.own { (colors::SYSTEM_BLUE.now(), WHITE) } else { (FIELD_BG.now(), LABEL.now()) };
            s.fill_rrect(x, by, bw, row.bubble_h, lay.radius, fill);
            for (i, line) in row.lines.iter().enumerate() {
                let ly = by + lay.bubble_pad + i as f32 * lay.line;
                put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, ink, x + lay.bubble_pad, ly);
            }
        }
        if let Some(typing) = typing_text(&room.typing) {
            let ty = area[1] + area[3] - typing_height(lay) + lay.gap;
            put(s, &mut self.labels, ui, &typing, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), area[0] + lay.pad, ty);
        }
    }

    /// The bubbles of the selected room, wrapped for the timeline width, with a day heading where a day starts. Built
    /// first so the scroll range is known.
    fn timeline_rows(&self, ui: &Ui, lay: &Layout) -> Vec<TimelineRow> {
        let Some(room) = self.model.selected_room() else { return Vec::new() };
        let area_w = (lay.w - lay.side).max(0.0);
        let text_w = (area_w * BUBBLE_SHARE - 2.0 * lay.bubble_pad).max(0.0);
        let today = day_of(now_ms());
        let mut prev = None;
        room.timeline
            .iter()
            .map(|m| {
                let own = m.sender == self.model.user_id;
                let day = starts_day(prev, m.ts).then(|| day_label(day_of(m.ts), today));
                prev = Some(m.ts);
                let head = if day.is_some() { day_height(lay) } else { 0.0 };
                let lines = wrap(ui, &full_text(&m.body), text_w, MAC_BODY_PT, Weight::Regular);
                let widest = lines.iter().map(|l| ui.measure(l, MAC_BODY_PT, Weight::Regular)).fold(0.0, f32::max);
                let sender = if own { None } else { Some(fit(ui, crate::protocol::display_name(&m.sender), MAC_FOOTNOTE_PT, Weight::Semibold, text_w)) };
                let sender_h = if sender.is_some() { lay.foot } else { 0.0 };
                let bubble_h = lines.len() as f32 * lay.line + 2.0 * lay.bubble_pad;
                TimelineRow { own, day, head, sender, lines, widest, bubble_h, height: head + sender_h + bubble_h }
            })
            .collect()
    }

    fn draw_composer(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let y0 = lay.h - lay.composer;
        let x0 = lay.side;
        s.fill_rect(px(x0), px(y0), px((lay.w - x0).max(0.0)), px(lay.composer), CONTENT_BG.now());
        s.fill_rect(px(x0), px(y0), px((lay.w - x0).max(0.0)), px(lay.rule), SEPARATOR_C.now());
        let field = lay.composer_field();
        let can_send = self.model.selected.is_some() && !self.model.composer.trim().is_empty();
        s.fill_rrect(field[0], field[1], field[2], field[3], lay.radius, FIELD_BG.now());
        let text_w = (field[2] - 2.0 * lay.gap).max(0.0);
        let cy = field[1] + field[3] / 2.0;
        if self.model.composer.is_empty() {
            let hint = if self.model.selected.is_some() { "Message" } else { "Choose a conversation first" };
            put_mid(s, &mut self.labels, ui, hint, MAC_BODY_PT, Weight::Regular, LABEL_3.now(), field[0] + lay.gap, cy);
        } else {
            let shown = fit_tail(ui, &self.model.composer, MAC_BODY_PT, text_w);
            let tw = ui.measure(&shown, MAC_BODY_PT, Weight::Regular);
            put_mid(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), field[0] + lay.gap, cy);
            let caret = lay.rule.max(1.0);
            s.fill_rect(px(field[0] + lay.gap + tw), px(cy - ui.pt(MAC_BODY_PT) / 2.0), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
        }
        let image = lay.image_button();
        s.fill_rrect(image[0], image[1], image[2], image[3], lay.radius, FIELD_BG.now());
        put_centred(s, &mut self.labels, ui, "Image", MAC_BODY_PT, Weight::Regular, LABEL.now(), image[0] + image[2] / 2.0, image[1] + image[3] / 2.0);
        let send = lay.send_button();
        let (fill, ink) = if can_send { (colors::SYSTEM_BLUE.now(), WHITE) } else { (FIELD_BG.now(), LABEL_3.now()) };
        s.fill_rrect(send[0], send[1], send[2], send[3], lay.radius, fill);
        put_centred(s, &mut self.labels, ui, "Send", MAC_BODY_PT, Weight::Semibold, ink, send[0] + send[2] / 2.0, send[1] + send[3] / 2.0);
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let Some(sign) = self.model.sign_in.clone() else { return };
        let sheet = lay.sheet(ui);
        let [px_, py, pw, ph] = sheet.panel;
        let radius = ui.pt(PANEL_RADIUS_PT);
        s.fill_rrect(px_ - lay.rule, py - lay.rule, pw + 2.0 * lay.rule, ph + 2.0 * lay.rule, radius + lay.rule, SEPARATOR_C.now());
        s.fill_rrect(px_, py, pw, ph, radius, CARD_BG.now());
        put_centred(s, &mut self.labels, ui, "Sign in to Matrix", MAC_BODY_PT, Weight::Semibold, LABEL.now(), px_ + pw / 2.0, py + ui.pt(SHEET_HEAD_PT) / 2.0);
        for i in 0..3 {
            let b = sheet.boxes[i];
            if sign.focus == i {
                let rule = lay.rule.max(1.0);
                s.fill_rrect(b[0] - rule, b[1] - rule, b[2] + 2.0 * rule, b[3] + 2.0 * rule, lay.radius, colors::SYSTEM_BLUE.now());
            }
            s.fill_rrect(b[0], b[1], b[2], b[3], lay.radius, FIELD_BG.now());
            put(s, &mut self.labels, ui, FIELD_LABELS[i], MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), b[0], sheet.labels[i]);
            let value = if i == PASSWORD_FIELD { "\u{2022}".repeat(sign.values[i].chars().count()) } else { sign.values[i].clone() };
            let shown = fit_tail(ui, &value, MAC_BODY_PT, b[2] - 2.0 * lay.gap);
            let vy = b[1] + b[3] / 2.0;
            put_mid(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), b[0] + lay.gap, vy);
            if sign.focus == i {
                let tw = ui.measure(&shown, MAC_BODY_PT, Weight::Regular);
                s.fill_rect(px(b[0] + lay.gap + tw), px(vy - ui.pt(MAC_BODY_PT) / 2.0), px(lay.rule.max(1.0)), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
            }
        }
        let message = if sign.busy { "Signing in..." } else { sign.error.as_str() };
        let ink = if sign.busy { LABEL_2.now() } else { DESTRUCTIVE };
        let shown = fit(ui, message, MAC_FOOTNOTE_PT, Weight::Regular, (sheet.button[0] - sheet.error_x - lay.gap).max(0.0));
        put_mid(s, &mut self.labels, ui, &shown, MAC_FOOTNOTE_PT, Weight::Regular, ink, sheet.error_x, sheet.error_y + lay.foot / 2.0);
        let b = sheet.button;
        let fill = if sign.busy { FIELD_BG.now() } else { colors::SYSTEM_BLUE.now() };
        let ink = if sign.busy { LABEL_3.now() } else { WHITE };
        s.fill_rrect(b[0], b[1], b[2], b[3], lay.radius, fill);
        put_centred(s, &mut self.labels, ui, "Sign In", MAC_BODY_PT, Weight::Semibold, ink, b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    }
}

/// One conversation bubble, measured and wrapped. `day` is the heading of the day the message starts, with `head` the
/// height that heading takes above the bubble.
struct TimelineRow {
    own: bool,
    day: Option<String>,
    head: f32,
    sender: Option<String>,
    lines: Vec<String>,
    widest: f32,
    bubble_h: f32,
    height: f32,
}

/// The height of a day heading: its line and the space around it.
fn day_height(lay: &Layout) -> f32 {
    lay.foot + 2.0 * lay.gap
}

/// The height the typing line takes at the foot of the timeline.
fn typing_height(lay: &Layout) -> f32 {
    lay.foot + 2.0 * lay.gap
}

/// The wall clock in milliseconds since the epoch (0 when the clock is before it).
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The text of a message as the bubble shows it: the words of a text, or the preview of anything else.
fn full_text(body: &Body) -> String {
    match body {
        Body::Text(s) | Body::Notice(s) | Body::Emote(s) => s.clone(),
        other => preview(other),
    }
}

fn badge_text(unread: u64) -> String {
    if unread > 99 {
        "99+".to_string()
    } else {
        unread.to_string()
    }
}

fn px(v: f32) -> usize {
    v.max(0.0).round() as usize
}

/// Wraps `text` to `width` pixels, one entry per line. Blank lines of the text are kept.
pub fn wrap(ui: &Ui, text: &str, width: f32, pt: f32, weight: Weight) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if line.is_empty() || ui.measure(&candidate, pt, weight) <= width {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    lines
}

/// `text` cut to fit `max` pixels, with `...` where it is cut.
pub fn fit(ui: &Ui, text: &str, pt: f32, weight: Weight, max: f32) -> String {
    if ui.measure(text, pt, weight) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "...";
        if ui.measure(&candidate, pt, weight) <= max {
            return candidate;
        }
    }
    String::new()
}

/// The end of `text` that fits `max` pixels (a field shows what was typed last).
pub fn fit_tail(ui: &Ui, text: &str, pt: f32, max: f32) -> String {
    if ui.measure(text, pt, Weight::Regular) <= max {
        return text.to_string();
    }
    let mut start = 0;
    let chars: Vec<char> = text.chars().collect();
    while start < chars.len() {
        let candidate: String = chars[start..].iter().collect();
        if ui.measure(&candidate, pt, Weight::Regular) <= max {
            return candidate;
        }
        start += 1;
    }
    String::new()
}

// text helpers --------------------------------------------------------------------------------------------------------

fn put(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, x: f32, y: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    s.blit(img, x as i32, y as i32, 255, 255);
}

/// Text centred vertically on `cy`, its left edge at `x`.
#[allow(clippy::too_many_arguments)]
fn put_mid(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, x: f32, cy: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    let top = cy - img.h as f32 / 2.0;
    s.blit(img, x as i32, top as i32, 255, 255);
}

/// Text centred on (`cx`, `cy`).
#[allow(clippy::too_many_arguments)]
fn put_centred(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, cx: f32, cy: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    let (x, y) = (cx - img.w as f32 / 2.0, cy - img.h as f32 / 2.0);
    s.blit(img, x as i32, y as i32, 255, 255);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(w: f32, h: f32) -> Layout {
        // a fixed scale without a font: the geometry only needs the point size
        Layout {
            w,
            h,
            side: 0.0,
            header: HEADER_PT,
            row: ROW_PT,
            composer: COMPOSER_PT,
            pad: 12.0,
            gap: 8.0,
            rule: 1.0,
            radius: RADIUS_PT,
            button_h: BUTTON_H_PT,
            image_w: IMAGE_W_PT,
            send_w: SEND_W_PT,
            bubble_pad: BUBBLE_PAD_PT,
            line: BUBBLE_LINE_PT,
            foot: FOOT_LINE_PT,
            bubble_gap: BUBBLE_GAP_PT,
        }
    }

    #[test]
    fn rectangles_include_their_left_and_top_edges_only() {
        assert!(inside([10.0, 10.0, 5.0, 5.0], 10.0, 10.0));
        assert!(!inside([10.0, 10.0, 5.0, 5.0], 15.0, 10.0));
        assert!(!inside([10.0, 10.0, 5.0, 5.0], 9.9, 12.0));
    }

    #[test]
    fn the_composer_sits_at_the_bottom_and_the_timeline_fills_the_rest() {
        let mut lay = layout(900.0, 600.0);
        lay.side = 240.0;
        let t = lay.timeline();
        assert_eq!(t[0], 240.0);
        assert_eq!(t[1], HEADER_PT);
        assert_eq!(t[1] + t[3], 600.0 - COMPOSER_PT, "the timeline stops above the composer");
        let send = lay.send_button();
        assert!(send[0] + send[2] <= 900.0 - 12.0 + 0.01, "the send button keeps the pad from the edge");
        let field = lay.composer_field();
        assert!(field[0] >= 240.0 && field[0] + field[2] <= lay.image_button()[0], "the field stops before Image");
    }

    #[test]
    fn a_room_row_moves_with_the_scroll() {
        let lay = layout(900.0, 600.0);
        let r0 = lay.room_row(0, 0.0);
        let r2 = lay.room_row(2, 0.0);
        assert_eq!(r2[1] - r0[1], 2.0 * ROW_PT);
        assert_eq!(lay.room_row(2, 10.0)[1], r2[1] - 10.0);
    }

    #[test]
    fn badges_past_ninety_nine_read_99_plus() {
        assert_eq!(badge_text(3), "3");
        assert_eq!(badge_text(120), "99+");
    }

    #[test]
    fn a_picture_message_is_described_and_a_text_keeps_its_lines() {
        assert_eq!(full_text(&Body::Text("a\nb".into())), "a\nb");
        assert_eq!(full_text(&Body::Image { name: "x.png".into(), mxc: String::new() }), "Sent an image: x.png");
    }

    #[test]
    fn pixels_clamp_at_zero() {
        assert_eq!(px(-3.0), 0);
        assert_eq!(px(2.6), 3);
    }
}

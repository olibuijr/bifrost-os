//! The Mail window's drawing and hit-testing. Every size is in points and goes through `ui.pt`; the three panes come from
//! [`crate::layout`], the message list and the reader scroll, and the sheets are panels centred over the window. The
//! layout functions here are shared by the painter and the hit test, so a tap lands on what is drawn.

use crate::flags::Flag;
use crate::layout;
use crate::state::{Mail, Sheet};
use crate::window::{MailWindow, Region};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT, SEPARATOR_C, SPACE_M, SPACE_S, WHITE};
use bifrost_app::kit::{colors, Surface, TextCache, Ui, Weight};

pub const TOOLBAR_PT: f32 = 52.0;
const BUTTON_H_PT: f32 = 28.0;
const BUTTON_W_PT: f32 = 96.0;
const SIDE_HEAD_PT: f32 = 30.0;
const SIDE_ROW_PT: f32 = 26.0;
const ROW_PT: f32 = 58.0;
const READER_PAD_PT: f32 = 20.0;
const BODY_LINE_PT: f32 = 18.0;
const FOOT_LINE_PT: f32 = 15.0;
const TITLE_LINE_PT: f32 = 28.0;
const SHEET_W_PT: f32 = 440.0;
const SHEET_HEAD_PT: f32 = 44.0;
const SHEET_FOOT_PT: f32 = 60.0;
const SHEET_PAD_PT: f32 = 16.0;
const FIELD_PT: f32 = 46.0;
const MESSAGE_PT: f32 = 170.0;
const DOT_PT: f32 = 8.0;
const RADIUS_PT: f32 = 8.0;
const PANEL_RADIUS_PT: f32 = 12.0;

/// A rectangle in pixels: x, y, width, height.
pub type Rect = [f32; 4];

pub fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// The widths of the sidebar, the message list and the reader, in pixels. They fill the width of the window.
pub fn columns(ui: &Ui, w: f32) -> [f32; 3] {
    let pt = ui.pt(1.0);
    let [a, b, c] = layout::panes(w / pt);
    [a * pt, b * pt, c * pt]
}

/// Which pane a point is in. The toolbar, and anything left of the list or right of it, is chrome.
pub fn pane_at(columns: [f32; 3], top: f32, x: f32, y: f32) -> Region {
    let [side, list, _] = columns;
    if y < top {
        Region::Chrome
    } else if x >= side && x < side + list {
        Region::List
    } else if x >= side + list {
        Region::Reader
    } else {
        Region::Chrome
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Button {
    Refresh,
    Compose,
    Reply,
    Archive,
    Trash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Side {
    Account(usize),
    Mailbox(String),
    Add,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(Button),
    Side(Side),
    Message(u32),
    Field(usize),
    Cancel,
    Primary,
    None,
}

/// The toolbar buttons and their rectangles, from the right edge inwards: Compose, then Reply, Archive and Trash when a
/// message is open, then Refresh.
fn toolbar_items(ui: &Ui, w: f32, open: bool) -> Vec<(Button, &'static str, Rect)> {
    let h = ui.pt(BUTTON_H_PT);
    let y = (ui.pt(TOOLBAR_PT) - h) / 2.0;
    let pad = ui.pt(SPACE_M);
    let mut order: Vec<(Button, &'static str)> = vec![(Button::Compose, "Compose")];
    if open {
        order.push((Button::Reply, "Reply"));
        order.push((Button::Archive, "Archive"));
        order.push((Button::Trash, "Trash"));
    }
    order.push((Button::Refresh, "Refresh"));
    let mut right = w - pad;
    let mut items = Vec::new();
    for (button, label) in order {
        let width = ui.measure(label, MAC_BODY_PT, Weight::Regular) + 2.0 * pad;
        items.push((button, label, [right - width, y, width, h]));
        right -= width + ui.pt(SPACE_S);
    }
    items
}

/// The sidebar rows top to bottom: each account, the mailboxes of the shown account under it, then Add Account.
fn side_items(mail: &Mail, ui: &Ui, width: f32, top: f32) -> Vec<(Side, Rect)> {
    let head = ui.pt(SIDE_HEAD_PT);
    let row = ui.pt(SIDE_ROW_PT);
    let mut y = top + ui.pt(SPACE_S);
    let mut items = Vec::new();
    for index in 0..mail.accounts.len() {
        items.push((Side::Account(index), [0.0, y, width, head]));
        y += head;
        if index == mail.account {
            for name in &mail.mailboxes {
                items.push((Side::Mailbox(name.clone()), [0.0, y, width, row]));
                y += row;
            }
        }
    }
    items.push((Side::Add, [0.0, y, width, row]));
    items
}

/// The sheet's panel, field rectangles and buttons. Compose's message field is taller than the others.
pub struct SheetLayout {
    pub panel: Rect,
    pub fields: Vec<Rect>,
    pub cancel: Rect,
    pub primary: Rect,
}

pub fn sheet_layout(ui: &Ui, w: f32, h: f32, sheet: &Sheet) -> SheetLayout {
    let form = match sheet {
        Sheet::Setup(form) => form,
        Sheet::Compose(compose) => &compose.form,
    };
    let pad = ui.pt(SHEET_PAD_PT);
    let head = ui.pt(SHEET_HEAD_PT);
    let foot = ui.pt(SHEET_FOOT_PT);
    let heights: Vec<f32> = form.fields.iter().map(|f| if f.label == "Message" { ui.pt(MESSAGE_PT) } else { ui.pt(FIELD_PT) }).collect();
    let body: f32 = heights.iter().sum();
    let pw = ui.pt(SHEET_W_PT).min(w - 2.0 * ui.pt(SPACE_M)).max(0.0);
    let ph = head + body + foot;
    let px = ((w - pw) / 2.0).max(0.0);
    let py = ((h - ph) / 2.0).max(ui.pt(TOOLBAR_PT) + pad);
    let mut y = py + head;
    let mut fields = Vec::with_capacity(heights.len());
    for hh in heights {
        fields.push([px + pad, y, pw - 2.0 * pad, hh]);
        y += hh;
    }
    let bh = ui.pt(BUTTON_H_PT);
    let bw = ui.pt(BUTTON_W_PT);
    let by = py + ph - pad - bh;
    let primary = [px + pw - pad - bw, by, bw, bh];
    let cancel = [primary[0] - ui.pt(SPACE_S) - bw, by, bw, bh];
    SheetLayout { panel: [px, py, pw, ph], fields, cancel, primary }
}

/// The wrapped text of the open message, kept while the message and the width stay the same.
pub struct Laid {
    pub uid: u32,
    pub width: f32,
    pub lines: Vec<String>,
}

/// Greedy word wrap at `width` pixels. Blank lines in the text stay as blank lines. A word wider than the line is
/// left whole on its own line.
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
fn fit(ui: &Ui, text: &str, pt: f32, weight: Weight, max: f32) -> String {
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

/// The name in a `Name <address>` value, or the address when there is no name.
fn display_name(from: &str) -> &str {
    let name = from.split('<').next().unwrap_or("").trim().trim_matches('"').trim();
    if name.is_empty() {
        from.trim()
    } else {
        name
    }
}

/// The day and month of a Date header (`Tue, 13 Oct 2026 ...` gives `13 Oct`), or the header as it is.
fn short_date(date: &str) -> String {
    let parts: Vec<&str> = date.split_whitespace().collect();
    if parts.len() >= 3 {
        format!("{} {}", parts[1], parts[2])
    } else {
        date.to_string()
    }
}

fn mailbox_title(name: &str) -> &str {
    if name.eq_ignore_ascii_case("INBOX") {
        "Inbox"
    } else {
        name
    }
}

// text helpers --------------------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
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

fn px(v: f32) -> usize {
    v.max(0.0) as usize
}

impl MailWindow {
    /// The control under a point, or [`Hit::None`]. While a sheet is open it takes every tap.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Hit {
        let (w, h) = (self.w as f32, self.h as f32);
        if let Some(sheet) = &self.mail.sheet {
            let l = sheet_layout(ui, w, h, sheet);
            if inside(l.primary, x, y) {
                return Hit::Primary;
            }
            if inside(l.cancel, x, y) {
                return Hit::Cancel;
            }
            return l.fields.iter().position(|r| inside(*r, x, y)).map_or(Hit::None, Hit::Field);
        }
        let top = ui.pt(TOOLBAR_PT);
        if y < top {
            let open = self.mail.open.is_some();
            return toolbar_items(ui, w, open).into_iter().find(|(_, _, r)| inside(*r, x, y)).map_or(Hit::None, |(b, _, _)| Hit::Button(b));
        }
        let [side, list, _] = columns(ui, w);
        if x < side {
            return side_items(&self.mail, ui, side, top).into_iter().find(|(_, r)| inside(*r, x, y)).map_or(Hit::None, |(s, _)| Hit::Side(s));
        }
        if x < side + list {
            let row = ui.pt(ROW_PT);
            let index = ((y - top + self.list_scroll.offset()) / row).floor();
            if index >= 0.0 && (index as usize) < self.mail.rows.len() {
                return Hit::Message(self.mail.rows[index as usize].uid);
            }
        }
        Hit::None
    }

    /// Paints the window: the sidebar, the message list, the reader, the toolbar, and any sheet on top.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (s.w as f32, s.h as f32);
        let [side, list, view] = columns(ui, w);
        let top = ui.pt(TOOLBAR_PT);
        let rule = (ui.pt(1.0) as usize).max(1);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        s.fill_rect(0, px(top), px(side), px(h - top), CARD_BG.now());
        self.draw_sidebar(s, ui, side, top);
        self.draw_list(s, ui, side, list, top, h);
        self.draw_reader(s, ui, side + list, view, top, h);
        s.fill_rect(px(side), px(top), rule, px(h - top), SEPARATOR_C.now());
        s.fill_rect(px(side + list), px(top), rule, px(h - top), SEPARATOR_C.now());
        self.draw_toolbar(s, ui, w, top);
        if let Some(sheet) = self.mail.sheet.clone() {
            self.draw_sheet(s, ui, &sheet, w, h);
        }
    }

    fn draw_toolbar(&mut self, s: &mut Surface, ui: &Ui, w: f32, top: f32) {
        s.fill_rect(0, 0, s.w, px(top), CONTENT_BG.now());
        s.fill_rect(0, px(top) - 1, s.w, 1, SEPARATOR_C.now());
        let pad = ui.pt(SPACE_M);
        let title = if self.mail.accounts.is_empty() { "Mail" } else { mailbox_title(&self.mail.mailbox) };
        put_mid(s, &mut self.labels, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), pad, top / 2.0);
        for (button, label, r) in toolbar_items(ui, w, self.mail.open.is_some()) {
            let (fill, color) = if button == Button::Compose { (colors::SYSTEM_BLUE.now(), WHITE) } else { (FIELD_BG.now(), LABEL.now()) };
            s.fill_rrect(r[0], r[1], r[2], r[3], ui.pt(RADIUS_PT), fill);
            put_centred(s, &mut self.labels, ui, label, MAC_BODY_PT, Weight::Regular, color, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        }
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui, side: f32, top: f32) {
        let pad = ui.pt(SPACE_M);
        let inset = ui.pt(SPACE_S);
        for (item, r) in side_items(&self.mail, ui, side, top) {
            let cy = r[1] + r[3] / 2.0;
            match item {
                Side::Account(index) => {
                    let name = self.mail.accounts.get(index).map_or("", |a| a.name.as_str());
                    put_mid(s, &mut self.labels, ui, name, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), pad, cy);
                }
                Side::Mailbox(name) => {
                    let selected = name == self.mail.mailbox;
                    if selected {
                        s.fill_rrect(r[0] + inset, r[1], r[2] - 2.0 * inset, r[3], ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
                    }
                    let color = if selected { WHITE } else { LABEL.now() };
                    put_mid(s, &mut self.labels, ui, mailbox_title(&name), MAC_BODY_PT, Weight::Regular, color, pad + inset, cy);
                }
                Side::Add => {
                    put_mid(s, &mut self.labels, ui, "Add Account...", MAC_BODY_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), pad, cy);
                }
            }
        }
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui, x0: f32, width: f32, top: f32, h: f32) {
        let row = ui.pt(ROW_PT);
        let pad = ui.pt(SPACE_M);
        let inset = ui.pt(SPACE_S);
        let dot = ui.pt(DOT_PT);
        let area = (h - top).max(0.0);
        self.list_scroll.set_max((self.mail.rows.len() as f32 * row - area).max(0.0));
        let offset = self.list_scroll.offset();
        s.fill_rect(px(x0), px(top), px(width), px(area), CONTENT_BG.now());
        let text_x = x0 + pad + dot;
        let right = x0 + width - pad;
        let open = self.mail.open;
        for (i, r) in self.mail.rows.iter().enumerate() {
            let y = top + i as f32 * row - offset;
            if y + row < top || y > h {
                continue;
            }
            let selected = open == Some(r.uid);
            if selected {
                s.fill_rrect(x0 + inset, y + inset / 2.0, width - 2.0 * inset, row - inset, ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
            } else if !r.flags.contains(Flag::Seen) {
                s.fill_rrect(x0 + pad, y + row * 0.3 - dot / 2.0, dot, dot, dot / 2.0, colors::SYSTEM_BLUE.now());
            }
            let (c_name, c_sub, c_date) = if selected { (WHITE, WHITE, WHITE) } else { (LABEL.now(), LABEL.now(), LABEL_2.now()) };
            let date = short_date(&r.summary.date);
            let date_w = ui.measure(&date, MAC_FOOTNOTE_PT, Weight::Regular);
            let name_max = (right - text_x - date_w - inset).max(0.0);
            let name = fit(ui, display_name(&r.summary.from), MAC_BODY_PT, Weight::Semibold, name_max);
            put(s, &mut self.labels, ui, &name, MAC_BODY_PT, Weight::Semibold, c_name, text_x, y + row * 0.14);
            put(s, &mut self.labels, ui, &date, MAC_FOOTNOTE_PT, Weight::Regular, c_date, right - date_w, y + row * 0.16);
            let subject = if r.summary.subject.is_empty() { "(no subject)" } else { r.summary.subject.as_str() };
            let subject = fit(ui, subject, MAC_BODY_PT, Weight::Regular, (right - text_x).max(0.0));
            put(s, &mut self.labels, ui, &subject, MAC_BODY_PT, Weight::Regular, c_sub, text_x, y + row * 0.5);
            if !selected {
                s.fill_rect(px(text_x), px(y + row - 1.0), px(right - text_x), 1, SEPARATOR_C.now());
            }
        }

        let footer = ui.pt(FOOT_LINE_PT) + 2.0 * pad;
        let footer_top = top + area - footer;
        let centre_x = x0 + width / 2.0;
        if self.mail.rows.is_empty() {
            let message = if self.mail.accounts.is_empty() {
                "Add an account to start"
            } else if self.busy() {
                "Loading..."
            } else {
                "No messages"
            };
            put_centred(s, &mut self.labels, ui, message, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), centre_x, top + area / 2.0);
        }
        s.fill_rect(px(x0), px(footer_top), px(width), px(footer), CONTENT_BG.now());
        let note = if !self.mail.status.is_empty() {
            self.mail.status.clone()
        } else if self.busy() {
            "Checking mail...".to_string()
        } else {
            String::new()
        };
        let note = fit(ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, (width - 2.0 * pad).max(0.0));
        put_mid(s, &mut self.labels, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x0 + pad, footer_top + footer / 2.0);
    }

    fn draw_reader(&mut self, s: &mut Surface, ui: &Ui, x0: f32, width: f32, top: f32, h: f32) {
        let pad = ui.pt(READER_PAD_PT);
        let area = (h - top).max(0.0);
        let text_w = (width - 2.0 * pad).max(0.0);
        let Some(uid) = self.mail.open else {
            self.reader_scroll.set_max(0.0);
            let message = if self.mail.accounts.is_empty() { "Add an account to read your mail" } else { "Select a message" };
            put_centred(s, &mut self.labels, ui, message, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x0 + width / 2.0, top + area / 2.0);
            return;
        };
        let Some(summary) = self.mail.rows.iter().find(|r| r.uid == uid).map(|r| r.summary.clone()) else {
            self.reader_scroll.set_max(0.0);
            return;
        };
        let loaded = self.mail.bodies.contains_key(&uid);
        if loaded {
            self.ensure_laid(ui, uid, text_w);
        }
        let lines: Vec<String> = match &self.laid {
            Some(laid) if laid.uid == uid => laid.lines.clone(),
            _ => Vec::new(),
        };
        let offset = self.reader_scroll.offset();
        let x = x0 + pad;
        let mut y = top + pad - offset;

        let subject = if summary.subject.is_empty() { "(no subject)" } else { summary.subject.as_str() };
        for line in wrap(ui, subject, text_w, MAC_TITLE1_PT, Weight::Semibold) {
            put(s, &mut self.labels, ui, &line, MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x, y);
            y += ui.pt(TITLE_LINE_PT);
        }
        y += ui.pt(SPACE_S);
        for (label, value) in [("From", summary.from.as_str()), ("To", summary.to.as_str()), ("Date", summary.date.as_str())] {
            if value.is_empty() {
                continue;
            }
            let text = fit(ui, &format!("{label}: {value}"), MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(FOOT_LINE_PT);
        }
        y += ui.pt(SPACE_M);
        s.fill_rect(px(x), px(y), px(text_w), (ui.pt(1.0) as usize).max(1), SEPARATOR_C.now());
        y += ui.pt(SPACE_M);

        if !loaded {
            put(s, &mut self.labels, ui, "Loading...", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(BODY_LINE_PT);
        }
        let line_h = ui.pt(BODY_LINE_PT);
        for line in &lines {
            if y >= top - line_h && y <= h {
                put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, LABEL.now(), x, y);
            }
            y += line_h;
        }
        let content = y - (top + pad - offset) + pad;
        self.reader_scroll.set_max((content - area).max(0.0));
    }

    /// Lays out the open message's text at `width` unless the cache already holds it at that width.
    fn ensure_laid(&mut self, ui: &Ui, uid: u32, width: f32) {
        let fresh = matches!(&self.laid, Some(laid) if laid.uid == uid && (laid.width - width).abs() < 0.5);
        if fresh {
            return;
        }
        let Some(raw) = self.mail.bodies.get(&uid) else { return };
        let text = crate::body::readable(raw);
        let lines = wrap(ui, &text, width, MAC_BODY_PT, Weight::Regular);
        self.laid = Some(Laid { uid, width, lines });
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, sheet: &Sheet, w: f32, h: f32) {
        let l = sheet_layout(ui, w, h, sheet);
        let [px_, py, pw, ph] = l.panel;
        let rule = ui.pt(1.0).max(1.0);
        let radius = ui.pt(PANEL_RADIUS_PT);
        s.fill_rrect(px_ - rule, py - rule, pw + 2.0 * rule, ph + 2.0 * rule, radius + rule, SEPARATOR_C.now());
        s.fill_rrect(px_, py, pw, ph, radius, CARD_BG.now());

        let (title, action, form) = match sheet {
            Sheet::Setup(form) => ("Add Account", "Add Account", form),
            Sheet::Compose(compose) => (if compose.in_reply_to.is_some() { "Reply" } else { "New Message" }, "Send", &compose.form),
        };
        put_centred(s, &mut self.labels, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), px_ + pw / 2.0, py + ui.pt(SHEET_HEAD_PT) / 2.0);

        let pad = ui.pt(SPACE_S);
        let caret = ui.pt(1.0).max(1.0);
        for (i, field) in form.fields.iter().enumerate() {
            let r = l.fields[i];
            let focused = form.focus == i;
            if focused {
                s.fill_rrect(r[0] - rule, r[1] - rule, r[2] + 2.0 * rule, r[3] + 2.0 * rule, ui.pt(RADIUS_PT) + rule, colors::SYSTEM_BLUE.now());
            }
            s.fill_rrect(r[0], r[1], r[2], r[3], ui.pt(RADIUS_PT), FIELD_BG.now());
            let tx = r[0] + pad;
            put(s, &mut self.labels, ui, field.label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), tx, r[1] + ui.pt(5.0));
            let value_y = r[1] + ui.pt(20.0);
            if field.label == "Message" {
                let text_w = r[2] - 2.0 * pad;
                let line_h = ui.pt(BODY_LINE_PT);
                if field.value.is_empty() {
                    put(s, &mut self.labels, ui, field.hint, MAC_BODY_PT, Weight::Regular, LABEL_3.now(), tx, value_y);
                    if focused {
                        s.fill_rect(px(tx), px(value_y), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
                    }
                    continue;
                }
                let lines = wrap(ui, &field.value, text_w, MAC_BODY_PT, Weight::Regular);
                let room = ((r[1] + r[3] - value_y - ui.pt(4.0)) / line_h).floor().max(1.0) as usize;
                let start = lines.len().saturating_sub(room);
                for (k, line) in lines[start..].iter().enumerate() {
                    put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, LABEL.now(), tx, value_y + k as f32 * line_h);
                }
                if focused {
                    let last = lines.last().map_or(0.0, |line| ui.measure(line, MAC_BODY_PT, Weight::Regular));
                    let cy = value_y + (lines.len() - start).saturating_sub(1) as f32 * line_h;
                    s.fill_rect(px(tx + last), px(cy), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
                }
            } else {
                let (shown, color) = if field.value.is_empty() {
                    (field.hint.to_string(), LABEL_3.now())
                } else if field.secret {
                    ("*".repeat(field.value.chars().count()), LABEL.now())
                } else {
                    (field.value.clone(), LABEL.now())
                };
                put(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, color, tx, value_y);
                if focused {
                    let end = if field.value.is_empty() { 0.0 } else { ui.measure(&shown, MAC_BODY_PT, Weight::Regular) };
                    s.fill_rect(px(tx + end), px(value_y), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
                }
            }
        }

        let (cr, pr) = (l.cancel, l.primary);
        s.fill_rrect(cr[0], cr[1], cr[2], cr[3], ui.pt(RADIUS_PT), FIELD_BG.now());
        put_centred(s, &mut self.labels, ui, "Cancel", MAC_BODY_PT, Weight::Regular, LABEL.now(), cr[0] + cr[2] / 2.0, cr[1] + cr[3] / 2.0);
        s.fill_rrect(pr[0], pr[1], pr[2], pr[3], ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
        put_centred(s, &mut self.labels, ui, action, MAC_BODY_PT, Weight::Semibold, WHITE, pr[0] + pr[2] / 2.0, pr[1] + pr[3] / 2.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_regions_follow_the_columns() {
        let cols = [200.0, 300.0, 700.0];
        assert_eq!(pane_at(cols, 50.0, 10.0, 200.0), Region::Chrome);
        assert_eq!(pane_at(cols, 50.0, 210.0, 200.0), Region::List);
        assert_eq!(pane_at(cols, 50.0, 600.0, 200.0), Region::Reader);
        assert_eq!(pane_at(cols, 50.0, 210.0, 20.0), Region::Chrome, "the toolbar is chrome");
    }

    #[test]
    fn rectangles_include_their_left_and_top_edges_only() {
        assert!(inside([10.0, 10.0, 5.0, 5.0], 10.0, 10.0));
        assert!(!inside([10.0, 10.0, 5.0, 5.0], 15.0, 10.0));
        assert!(!inside([10.0, 10.0, 5.0, 5.0], 9.9, 12.0));
    }

    #[test]
    fn display_names_and_dates_are_short() {
        assert_eq!(display_name("Ana O <ana@example.com>"), "Ana O");
        assert_eq!(display_name("\"Bo\" <bo@example.com>"), "Bo");
        assert_eq!(display_name("bo@example.com"), "bo@example.com");
        assert_eq!(short_date("Tue, 13 Oct 2026 09:30:00 +0000"), "13 Oct");
        assert_eq!(short_date("13 Oct"), "13 Oct");
    }

    #[test]
    fn mailbox_titles() {
        assert_eq!(mailbox_title("INBOX"), "Inbox");
        assert_eq!(mailbox_title("Sent"), "Sent");
    }
}

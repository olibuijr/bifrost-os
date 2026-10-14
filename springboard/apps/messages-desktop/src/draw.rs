//! The Messages window's drawing and hit-testing. Every size is in points and goes through `ui.pt`. The room list is the
//! sidebar; the timeline and the composer are the main pane; the sign-in form is a sheet over both. [`Layout`] holds the
//! geometry for the painter and the hit test alike, so a tap lands on what is drawn.

use crate::model::{day_label, day_of, preview, starts_day, typing_text, EmojiGroup, Overlay, EMOJI_CATALOG, INFO_ROWS_MAX, EMOJI_COLUMNS, FIELD_LABELS, PASSWORD_FIELD};
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
const EMOJI_W_PT: f32 = 44.0;
const INFO_W_PT: f32 = 76.0;
const BANNER_PT: f32 = 36.0;
/// Corner radius of the newer controls (flat corners, at most 6 points).
const CORNER_PT: f32 = 6.0;
const CELL_PT: f32 = 40.0;
const MEMBER_ROWS: usize = 8;
/// How many shared picture names the details panel lists.
const SHARED_ROWS: usize = 4;
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
    /// The Attach button.
    Image,
    Send,
    /// The search field of the room list.
    Search,
    /// The New Message button.
    NewMessage,
    /// The Details button of the header.
    Info,
    /// The Emoji button of the composer.
    EmojiButton,
    /// The Cancel button of the reply or edit banner.
    CancelCompose,
    /// A reaction chip under a message: tapping it adds or takes back the user's own reaction.
    Chip { msg: String, key: String },
    /// The quoted line of a reply, by the id of the message it quotes: tapping it scrolls to that message.
    Quote(String),
    /// A button of the sheet or panel that is open.
    Overlay(OverlayAction),
    /// A field of the sign-in sheet.
    Field(usize),
    SignIn,
    /// Inside the sign-in sheet, on nothing that acts.
    Panel,
    Nothing,
}

/// What a button of a sheet or panel does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayAction {
    Ok,
    Cancel,
    Rename,
    SignOut,
    Delete,
    /// The emoji at this index of the picker's choices (`Messages::emoji_choices`).
    Pick(usize),
    /// A button of the picker's group strip: a group, or None for All.
    EmojiGroup(Option<EmojiGroup>),
}

/// A button on a sheet or panel.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayButton {
    pub rect: Rect,
    pub label: String,
    pub action: OverlayAction,
    pub primary: bool,
    pub destructive: bool,
}

/// The sheet or panel over the window: its rectangle, the text field when it has one, the top of its content and
/// its buttons.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub panel: Rect,
    pub field: Option<Rect>,
    pub content_y: f32,
    pub buttons: Vec<OverlayButton>,
}

/// A reaction chip as drawn, kept for the next tap.
#[derive(Clone, Debug, PartialEq)]
pub struct ChipHit {
    pub rect: Rect,
    pub msg: String,
    pub key: String,
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
    pub emoji_w: f32,
    pub info_w: f32,
    /// The height of the reply or edit banner above the composer; 0 when there is none.
    pub banner: f32,
    pub corner: f32,
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
            emoji_w: ui.pt(EMOJI_W_PT),
            info_w: ui.pt(INFO_W_PT),
            banner: 0.0,
            corner: ui.pt(CORNER_PT),
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
        [self.side, self.header, (self.w - self.side).max(0.0), (self.h - self.header - self.composer - self.banner).max(0.0)]
    }

    /// The strip above the composer that names the message being answered or edited.
    pub fn banner_rect(&self) -> Rect {
        [self.side, self.h - self.composer - self.banner, (self.w - self.side).max(0.0), self.banner]
    }

    pub fn banner_cancel(&self) -> Rect {
        let b = self.banner_rect();
        [b[0] + b[2] - self.pad - self.send_w, b[1] + (b[3] - self.button_h) / 2.0, self.send_w, self.button_h]
    }

    /// New Message, at the right of the room list's header.
    pub fn new_button(&self) -> Rect {
        [self.side - self.pad - self.button_h, (self.header - self.button_h) / 2.0, self.button_h, self.button_h]
    }

    pub fn search_field(&self) -> Rect {
        let n = self.new_button();
        [self.pad, n[1], (n[0] - self.gap - self.pad).max(0.0), self.button_h]
    }

    /// Details, at the right of the conversation header.
    pub fn info_button(&self) -> Rect {
        [self.w - self.pad - self.info_w, (self.header - self.button_h) / 2.0, self.info_w, self.button_h]
    }

    pub fn send_button(&self) -> Rect {
        let x = self.w - self.pad - self.send_w;
        [x, self.h - self.composer / 2.0 - self.button_h / 2.0, self.send_w, self.button_h]
    }

    pub fn emoji_button(&self) -> Rect {
        let a = self.attach_button();
        [a[0] - self.gap - self.emoji_w, a[1], self.emoji_w, self.button_h]
    }

    pub fn attach_button(&self) -> Rect {
        let s = self.send_button();
        [s[0] - self.gap - self.image_w, s[1], self.image_w, self.button_h]
    }

    pub fn composer_field(&self) -> Rect {
        let b = self.emoji_button();
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
    /// The geometry for the window as it is now: the banner strip exists while the composer holds a reply or an edit.
    pub fn layout(&self, ui: &Ui) -> Layout {
        let mut lay = Layout::new(ui, self.w as f32, self.h as f32);
        if self.model.banner().is_some() {
            lay.banner = ui.pt(BANNER_PT);
        }
        lay
    }

    /// What a tap at (`x`, `y`) lands on. The sign-in sheet and the other panels cover the window while they are open.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Hit {
        let lay = self.layout(ui);
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
        if let Some(panel) = self.overlay_panel(ui, &lay) {
            if let Some(b) = panel.buttons.iter().find(|b| inside(b.rect, x, y)) {
                return Hit::Overlay(b.action.clone());
            }
            return if inside(panel.panel, x, y) { Hit::Panel } else { Hit::Nothing };
        }
        if inside(lay.new_button(), x, y) {
            return Hit::NewMessage;
        }
        if inside(lay.search_field(), x, y) {
            return Hit::Search;
        }
        if self.model.selected.is_some() && inside(lay.info_button(), x, y) {
            return Hit::Info;
        }
        if lay.banner > 0.0 && inside(lay.banner_cancel(), x, y) {
            return Hit::CancelCompose;
        }
        if inside(lay.emoji_button(), x, y) {
            return Hit::EmojiButton;
        }
        if inside(lay.attach_button(), x, y) {
            return Hit::Image;
        }
        if inside(lay.send_button(), x, y) {
            return Hit::Send;
        }
        if let Some(c) = self.chips.iter().find(|c| inside(c.rect, x, y)) {
            return Hit::Chip { msg: c.msg.clone(), key: c.key.clone() };
        }
        if let Some((_, target)) = self.quotes.iter().find(|(r, _)| inside(*r, x, y)) {
            return Hit::Quote(target.clone());
        }
        if inside(lay.sidebar(), x, y) {
            let index = ((y - lay.header + self.room_scroll.offset()) / lay.row).floor();
            if index >= 0.0 {
                if let Some(room) = self.model.visible_rooms().get(index as usize) {
                    return Hit::Room(room.id.clone());
                }
            }
        }
        Hit::Nothing
    }

    /// The room row under (`x`, `y`) for a context menu.
    pub fn room_at(&self, ui: &Ui, x: f32, y: f32) -> Option<String> {
        let lay = self.layout(ui);
        if !inside(lay.sidebar(), x, y) {
            return None;
        }
        let index = ((y - lay.header + self.room_scroll.offset()) / lay.row).floor();
        if index < 0.0 {
            return None;
        }
        self.model.visible_rooms().get(index as usize).map(|r| r.id.clone())
    }

    /// The message bubble under (`x`, `y`), by event id, from the last frame.
    pub fn message_at(&self, x: f32, y: f32) -> Option<String> {
        self.bubbles.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, id)| id.clone())
    }

    /// The scroll offset that puts the message `id` at the top of the timeline, from the rows as they are laid out now.
    /// `None` when the message is not in the room on screen.
    pub fn message_offset(&self, ui: &Ui, id: &str) -> Option<f32> {
        let lay = self.layout(ui);
        let rows = self.timeline_rows(ui, &lay);
        let heights: Vec<f32> = rows.iter().map(|r| r.height).collect();
        let index = rows.iter().position(|r| r.id == id)?;
        Some(rows_height_before(&heights, lay.bubble_gap, index))
    }

    /// The panel that is open over the window, with its buttons. `None` when no panel is open.
    pub fn overlay_panel(&self, ui: &Ui, lay: &Layout) -> Option<Panel> {
        let head = ui.pt(SHEET_HEAD_PT);
        let pw = ui.pt(SHEET_W_PT).min(lay.w - 2.0 * lay.pad).max(0.0);
        let bw = ui.pt(SHEET_BUTTON_W_PT);
        let place = |ph: f32, pw: f32| -> Rect {
            let px_ = ((lay.w - pw) / 2.0).max(0.0);
            let py = ((lay.h - ph) / 2.0).max(lay.header + lay.pad);
            [px_, py, pw, ph]
        };
        let button = |rect: Rect, label: &str, action: OverlayAction, primary: bool, destructive: bool| OverlayButton {
            rect,
            label: label.to_string(),
            action,
            primary,
            destructive,
        };
        match &self.model.overlay {
            Overlay::None => None,
            Overlay::Prompt(p) => {
                let ph = head + lay.pad + ui.pt(SHEET_FIELD_PT) + ui.pt(SHEET_FOOT_PT);
                let r = place(ph, pw);
                let content_y = r[1] + head + lay.pad;
                let field = [r[0] + lay.pad, content_y + ui.pt(SHEET_LABEL_PT), (pw - 2.0 * lay.pad).max(0.0), ui.pt(SHEET_BOX_PT)];
                let y = r[1] + ph - lay.pad - lay.button_h;
                let ok = [r[0] + pw - lay.pad - bw, y, bw, lay.button_h];
                let cancel = [ok[0] - lay.gap - bw, y, bw, lay.button_h];
                Some(Panel {
                    panel: r,
                    field: Some(field),
                    content_y,
                    buttons: vec![
                        button(cancel, "Cancel", OverlayAction::Cancel, false, false),
                        button(ok, p.ok_label(), OverlayAction::Ok, !p.busy, false),
                    ],
                })
            }
            Overlay::Confirm(_) => {
                let ph = head + lay.pad + 2.0 * lay.line + ui.pt(SHEET_FOOT_PT);
                let r = place(ph, pw);
                let y = r[1] + ph - lay.pad - lay.button_h;
                let ok = [r[0] + pw - lay.pad - bw, y, bw, lay.button_h];
                let cancel = [ok[0] - lay.gap - bw, y, bw, lay.button_h];
                Some(Panel {
                    panel: r,
                    field: None,
                    content_y: r[1] + head + lay.pad,
                    buttons: vec![
                        button(cancel, "Cancel", OverlayAction::Cancel, false, false),
                        button(ok, "Delete", OverlayAction::Delete, true, true),
                    ],
                })
            }
            Overlay::Details => {
                let members = self.model.selected_room().map_or(0, |r| r.members.len());
                let rows = members.min(MEMBER_ROWS) + usize::from(members > MEMBER_ROWS);
                let pictures = self.model.selected_room().map_or(0, |r| r.shared_pictures().len());
                let media = media_rows(pictures);
                let media_h = if media == 0 { 0.0 } else { lay.pad + media as f32 * lay.line };
                let ph = head + lay.pad + lay.button_h + lay.pad + lay.line + rows as f32 * lay.line + media_h + ui.pt(SHEET_FOOT_PT);
                let r = place(ph, pw);
                let content_y = r[1] + head + lay.pad;
                let rename = [r[0] + pw - lay.pad - bw, content_y, bw, lay.button_h];
                let done = [r[0] + pw - lay.pad - bw, r[1] + ph - lay.pad - lay.button_h, bw, lay.button_h];
                Some(Panel {
                    panel: r,
                    field: None,
                    content_y,
                    buttons: vec![
                        button(rename, "Rename", OverlayAction::Rename, false, false),
                        button(done, "Done", OverlayAction::Ok, true, false),
                    ],
                })
            }
            Overlay::Settings => {
                let ph = head + lay.pad + 3.0 * lay.line + lay.pad + lay.button_h + ui.pt(SHEET_FOOT_PT);
                let r = place(ph, pw);
                let content_y = r[1] + head + lay.pad;
                let sign_out = [r[0] + lay.pad, content_y + 3.0 * lay.line + lay.pad, bw, lay.button_h];
                let done = [r[0] + pw - lay.pad - bw, r[1] + ph - lay.pad - lay.button_h, bw, lay.button_h];
                Some(Panel {
                    panel: r,
                    field: None,
                    content_y,
                    buttons: vec![
                        button(sign_out, "Sign Out", OverlayAction::SignOut, false, true),
                        button(done, "Done", OverlayAction::Ok, true, false),
                    ],
                })
            }
            Overlay::Info(kind, id) => {
                let rows = self.model.info_lines(*kind, id).len().clamp(1, INFO_ROWS_MAX);
                let ph = head + lay.pad + rows as f32 * lay.line + ui.pt(SHEET_FOOT_PT);
                let r = place(ph, pw);
                let done = [r[0] + pw - lay.pad - bw, r[1] + ph - lay.pad - lay.button_h, bw, lay.button_h];
                Some(Panel {
                    panel: r,
                    field: None,
                    content_y: r[1] + head + lay.pad,
                    buttons: vec![button(done, "Done", OverlayAction::Ok, true, false)],
                })
            }
            Overlay::Emoji(_) => {
                let cell = ui.pt(CELL_PT);
                // The panel keeps the height of the whole catalogue, so a search does not make it jump.
                let rows = EMOJI_CATALOG.len().div_ceil(EMOJI_COLUMNS);
                let width = (EMOJI_COLUMNS as f32 * cell + 2.0 * lay.pad).min(lay.w);
                let search_h = ui.pt(SHEET_BOX_PT) + lay.pad;
                let strip_h = ui.pt(SHEET_BOX_PT) + lay.pad;
                let ph = head + search_h + strip_h + rows as f32 * cell + lay.pad;
                let r = place(ph, width);
                let field = [r[0] + lay.pad, r[1] + head, (width - 2.0 * lay.pad).max(0.0), ui.pt(SHEET_BOX_PT)];
                let strip_y = r[1] + head + search_h;
                let content_y = strip_y + strip_h;
                let strip_w = (width - 2.0 * lay.pad).max(0.0) / (EmojiGroup::ALL.len() + 1) as f32;
                let groups = std::iter::once((None, "All")).chain(EmojiGroup::ALL.iter().map(|g| (Some(*g), g.label())));
                let mut buttons: Vec<OverlayButton> = groups
                    .enumerate()
                    .map(|(i, (group, label))| {
                        let rect = [r[0] + lay.pad + i as f32 * strip_w, strip_y, strip_w, ui.pt(SHEET_BOX_PT)];
                        button(rect, label, OverlayAction::EmojiGroup(group), self.model.emoji_group == group, false)
                    })
                    .collect();
                buttons.extend(self.model.emoji_choices().iter().enumerate().map(|(i, emoji)| {
                    let rect = [r[0] + lay.pad + (i % EMOJI_COLUMNS) as f32 * cell, content_y + (i / EMOJI_COLUMNS) as f32 * cell, cell, cell];
                    button(rect, emoji.as_str(), OverlayAction::Pick(i), false, false)
                }));
                Some(Panel { panel: r, field: Some(field), content_y, buttons })
            }
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        let lay = self.layout(ui);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        self.bubbles.clear();
        self.chips.clear();
        self.quotes.clear();
        self.draw_sidebar(s, ui, &lay);
        self.draw_timeline(s, ui, &lay);
        self.draw_header(s, ui, &lay);
        self.draw_banner(s, ui, &lay);
        self.draw_composer(s, ui, &lay);
        if self.model.sign_in.is_some() {
            self.draw_sheet(s, ui, &lay);
        } else {
            self.draw_overlay(s, ui, &lay);
        }
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let side = lay.sidebar();
        s.fill_rect(px(side[0]), px(side[1]), px(side[2]), px(side[3]), CARD_BG.now());
        s.fill_rect(px(lay.side), 0, px(lay.rule), px(lay.h), SEPARATOR_C.now());
        let focus = self.model.search_focus;
        let search = lay.search_field();
        let add = lay.new_button();
        let enabled = self.model.signed_in();
        let rooms = self.model.visible_rooms();
        let rows = rooms.len() as f32 * lay.row;
        self.room_scroll.set_max((rows - side[3]).max(0.0));
        let scroll = self.room_scroll.offset();
        if rooms.is_empty() {
            let hint = if !self.model.signed_in() {
                "Sign in to see your conversations"
            } else if self.model.search.trim().is_empty() {
                "No conversations yet"
            } else {
                "No conversation matches"
            };
            put(s, &mut self.labels, ui, hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), lay.pad, lay.header + lay.pad);
        }
        let inset = lay.gap;
        let text_w = (lay.side - 2.0 * lay.pad).max(0.0);
        for (i, room) in rooms.iter().enumerate() {
            let r = lay.room_row(i, scroll);
            if r[1] + r[3] < lay.header || r[1] > lay.h {
                continue;
            }
            let selected = self.model.selected.as_deref() == Some(room.id.as_str());
            if selected {
                s.fill_rrect(r[0] + inset, r[1] + inset / 2.0, r[2] - 2.0 * inset, r[3] - inset, lay.corner, colors::SYSTEM_BLUE.now());
            }
            let (c_name, c_sub) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
            let marked = room.unread == 0 && self.model.is_marked_unread(&room.id) && !self.model.is_muted(&room.id);
            let badge = if room.unread > 0 {
                Some(badge_text(room.unread))
            } else if marked {
                Some("\u{2022}".to_string())
            } else {
                None
            };
            let badge_w = badge.as_ref().map_or(0.0, |b| ui.measure(b, MAC_FOOTNOTE_PT, Weight::Semibold) + lay.pad);
            let tags = room_tags(self.model.is_pinned(&room.id), self.model.is_muted(&room.id));
            let tags_w = if tags.is_empty() { 0.0 } else { ui.measure(&tags, MAC_FOOTNOTE_PT, Weight::Regular) + lay.gap };
            let name_w = (text_w - badge_w - tags_w).max(0.0);
            let title = room.title(&self.model.user_id);
            let name = fit(ui, &title, MAC_BODY_PT, Weight::Semibold, name_w);
            put(s, &mut self.labels, ui, &name, MAC_BODY_PT, Weight::Semibold, c_name, r[0] + lay.pad, r[1] + lay.row * 0.16);
            if !tags.is_empty() {
                let tx = r[0] + r[2] - lay.pad - badge_w - tags_w + lay.gap;
                put(s, &mut self.labels, ui, &tags, MAC_FOOTNOTE_PT, Weight::Regular, c_sub, tx, r[1] + lay.row * 0.18);
            }
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
        // The header strip is drawn last so a scrolled row never shows through it.
        s.fill_rect(px(side[0]), 0, px(side[2]), px(lay.header), CARD_BG.now());
        s.fill_rect(px(side[0]), px(lay.header - lay.rule), px(side[2]), px(lay.rule), SEPARATOR_C.now());
        draw_field(s, &mut self.labels, ui, lay, search, &self.model.search, "Search", focus);
        draw_button(s, &mut self.labels, ui, lay, add, "+", false, false, enabled);
    }

    fn draw_header(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let x0 = lay.side;
        let width = (lay.w - x0).max(0.0);
        s.fill_rect(px(x0), 0, px(width), px(lay.header), CONTENT_BG.now());
        s.fill_rect(px(x0), px(lay.header - lay.rule), px(width), px(lay.rule), SEPARATOR_C.now());
        let title = match self.model.selected_room() {
            Some(room) => room.title(&self.model.user_id),
            None => String::new(),
        };
        let has_room = self.model.selected.is_some();
        let right = if has_room { lay.info_w + lay.gap } else { 0.0 };
        let title = fit(ui, &title, MAC_TITLE1_PT, Weight::Semibold, (width / 2.0 - lay.pad).max(0.0));
        put_mid(s, &mut self.labels, ui, &title, MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x0 + lay.pad, lay.header / 2.0);
        let status = self.model.status.clone();
        if !status.is_empty() {
            let shown = fit(ui, &status, MAC_FOOTNOTE_PT, Weight::Regular, (width / 2.0 - right).max(0.0));
            let sw = ui.measure(&shown, MAC_FOOTNOTE_PT, Weight::Regular);
            put_mid(s, &mut self.labels, ui, &shown, MAC_FOOTNOTE_PT, Weight::Regular, DESTRUCTIVE, lay.w - lay.pad - right - sw, lay.header / 2.0);
        }
        if has_room {
            draw_button(s, &mut self.labels, ui, lay, lay.info_button(), "Details", false, false, true);
        }
    }

    fn draw_banner(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let Some(text) = self.model.banner() else { return };
        let b = lay.banner_rect();
        s.fill_rect(px(b[0]), px(b[1]), px(b[2]), px(b[3]), CARD_BG.now());
        s.fill_rect(px(b[0]), px(b[1]), px(b[2]), px(lay.rule), SEPARATOR_C.now());
        let cancel = lay.banner_cancel();
        let shown = fit(ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, (cancel[0] - b[0] - 2.0 * lay.pad).max(0.0));
        put_mid(s, &mut self.labels, ui, &shown, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), b[0] + lay.pad, b[1] + b[3] / 2.0);
        draw_button(s, &mut self.labels, ui, lay, cancel, "Cancel", false, false, true);
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
        let chip_h = chip_height(lay);
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
            let faint = if row.own { WHITE } else { LABEL_2.now() };
            s.fill_rrect(x, by, bw, row.bubble_h, lay.radius, fill);
            self.bubbles.push(([x, by, bw, row.bubble_h], row.id.clone()));
            let mut ly = by + lay.bubble_pad;
            if let Some(quote) = &row.quote {
                put(s, &mut self.labels, ui, quote, MAC_FOOTNOTE_PT, Weight::Regular, faint, x + lay.bubble_pad, ly);
                if let Some(target) = &row.quote_target {
                    self.quotes.push(([x, ly, bw, lay.foot], target.clone()));
                }
                ly += lay.foot;
            }
            for line in &row.lines {
                put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, ink, x + lay.bubble_pad, ly);
                ly += lay.line;
            }
            if row.edited {
                put(s, &mut self.labels, ui, "Edited", MAC_FOOTNOTE_PT, Weight::Regular, faint, x + lay.bubble_pad, ly);
            }
            let mut cx = x;
            let cy = by + row.bubble_h + lay.gap / 2.0;
            for chip in &row.chips {
                let rect = [cx, cy, chip.width, chip_h];
                let (cf, ci) = if chip.mine { (colors::SYSTEM_BLUE.now(), WHITE) } else { (FIELD_BG.now(), LABEL.now()) };
                s.fill_rrect(rect[0], rect[1], rect[2], rect[3], lay.corner, cf);
                put_centred(s, &mut self.labels, ui, &chip.label, MAC_FOOTNOTE_PT, Weight::Regular, ci, rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
                self.chips.push(ChipHit { rect, msg: row.id.clone(), key: chip.key.clone() });
                cx += chip.width + lay.gap / 2.0;
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
        let chip_pad = 2.0 * lay.gap;
        let mut prev = None;
        room.timeline
            .iter()
            .map(|m| {
                let own = m.sender == self.model.user_id;
                let day = starts_day(prev, m.ts).then(|| day_label(day_of(m.ts), today));
                prev = Some(m.ts);
                let head = if day.is_some() { day_height(lay) } else { 0.0 };
                let lines = wrap(ui, &full_text(&m.body), text_w, MAC_BODY_PT, Weight::Regular);
                let quote = m.reply_to.as_ref().and_then(|id| room.quote(id)).map(|(who, what)| {
                    fit(ui, &format!("{who}: {what}"), MAC_FOOTNOTE_PT, Weight::Regular, text_w)
                });
                let quote_target = quote.as_ref().and(m.reply_to.clone());
                let mut widest = lines.iter().map(|l| ui.measure(l, MAC_BODY_PT, Weight::Regular)).fold(0.0, f32::max);
                if let Some(q) = &quote {
                    widest = widest.max(ui.measure(q, MAC_FOOTNOTE_PT, Weight::Regular));
                }
                if m.edited {
                    widest = widest.max(ui.measure("Edited", MAC_FOOTNOTE_PT, Weight::Regular));
                }
                let sender = if own { None } else { Some(fit(ui, crate::protocol::display_name(&m.sender), MAC_FOOTNOTE_PT, Weight::Semibold, text_w)) };
                let sender_h = if sender.is_some() { lay.foot } else { 0.0 };
                let extra = if quote.is_some() { lay.foot } else { 0.0 } + if m.edited { lay.foot } else { 0.0 };
                let bubble_h = lines.len() as f32 * lay.line + extra + 2.0 * lay.bubble_pad;
                let chips: Vec<ChipView> = m
                    .reactions
                    .iter()
                    .map(|r| {
                        let label = chip_label(&r.key, r.users.len());
                        let width = ui.measure(&label, MAC_FOOTNOTE_PT, Weight::Regular) + chip_pad;
                        ChipView { key: r.key.clone(), label, width, mine: m.reaction_of(&self.model.user_id, &r.key).is_some() }
                    })
                    .collect();
                let chips_h = if chips.is_empty() { 0.0 } else { chip_height(lay) + lay.gap / 2.0 };
                TimelineRow {
                    id: m.id.clone(),
                    own,
                    day,
                    head,
                    sender,
                    quote,
                    quote_target,
                    edited: m.edited,
                    lines,
                    widest,
                    bubble_h,
                    chips,
                    height: head + sender_h + bubble_h + chips_h,
                }
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
        let hint = if self.model.selected.is_some() { "Message" } else { "Choose a conversation first" };
        let focus = !self.model.search_focus && self.model.overlay == Overlay::None && self.model.sign_in.is_none();
        let text = self.model.composer.clone();
        draw_field_tail(s, &mut self.labels, ui, lay, field, &text, hint, focus);
        let on = self.model.selected.is_some();
        draw_button(s, &mut self.labels, ui, lay, lay.emoji_button(), "Emoji", false, false, on);
        draw_button(s, &mut self.labels, ui, lay, lay.attach_button(), "Attach", false, false, on);
        let label = if matches!(self.model.compose, Some(crate::model::Compose::Edit(_))) { "Save" } else { "Send" };
        draw_button(s, &mut self.labels, ui, lay, lay.send_button(), label, can_send, false, can_send);
    }

    /// The panel over the window (New Message, Details, Settings, Delete, Emoji), when one is open.
    fn draw_overlay(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout) {
        let Some(panel) = self.overlay_panel(ui, lay) else { return };
        let [px_, py, pw, ph] = panel.panel;
        let radius = ui.pt(PANEL_RADIUS_PT);
        s.fill_rrect(px_ - lay.rule, py - lay.rule, pw + 2.0 * lay.rule, ph + 2.0 * lay.rule, radius + lay.rule, SEPARATOR_C.now());
        s.fill_rrect(px_, py, pw, ph, radius, CARD_BG.now());
        let title = match &self.model.overlay {
            Overlay::Prompt(p) => p.title(),
            Overlay::Confirm(_) => "Delete Conversation",
            Overlay::Details => "Conversation Details",
            Overlay::Settings => "Settings",
            Overlay::Emoji(_) => "Emoji",
            Overlay::Info(kind, _) => kind.title(),
            Overlay::None => "",
        };
        put_centred(s, &mut self.labels, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), px_ + pw / 2.0, py + ui.pt(SHEET_HEAD_PT) / 2.0);
        let text_x = px_ + lay.pad;
        let text_w = (pw - 2.0 * lay.pad).max(0.0);
        let me = self.model.user_id.clone();
        match self.model.overlay.clone() {
            Overlay::Prompt(p) => {
                put(s, &mut self.labels, ui, p.label(), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, panel.content_y);
                if let Some(f) = panel.field {
                    draw_field_tail(s, &mut self.labels, ui, lay, f, &p.value, "", true);
                }
                let message = if p.busy { "Working..." } else { p.error.as_str() };
                let ink = if p.busy { LABEL_2.now() } else { DESTRUCTIVE };
                let first = panel.buttons.first().map_or(px_ + pw, |b| b.rect[0]);
                let shown = fit(ui, message, MAC_FOOTNOTE_PT, Weight::Regular, (first - text_x - lay.gap).max(0.0));
                let by = py + ph - lay.pad - lay.button_h / 2.0;
                put_mid(s, &mut self.labels, ui, &shown, MAC_FOOTNOTE_PT, Weight::Regular, ink, text_x, by);
            }
            Overlay::Confirm(id) => {
                let name = self.model.rooms.iter().find(|r| r.id == id).map_or(id.clone(), |r| r.title(&me));
                let line1 = fit(ui, &format!("Leave \"{name}\"?"), MAC_BODY_PT, Weight::Regular, text_w);
                put(s, &mut self.labels, ui, &line1, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, panel.content_y);
                put(s, &mut self.labels, ui, "You leave the conversation and it is removed from the list.", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, panel.content_y + lay.line);
            }
            Overlay::Details => {
                if let Some(room) = self.model.selected_room() {
                    let rename_x = panel.buttons.first().map_or(px_ + pw, |b| b.rect[0]);
                    let name = fit(ui, &room.title(&me), MAC_BODY_PT, Weight::Semibold, (rename_x - text_x - lay.gap).max(0.0));
                    put_mid(s, &mut self.labels, ui, &name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), text_x, panel.content_y + lay.button_h / 2.0);
                    let mut y = panel.content_y + lay.button_h + lay.pad;
                    let heading = format!("Members ({})", room.members.len());
                    put(s, &mut self.labels, ui, &heading, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), text_x, y);
                    y += lay.line;
                    for member in room.members.iter().take(MEMBER_ROWS) {
                        let shown = fit(ui, &member.label(), MAC_BODY_PT, Weight::Regular, text_w);
                        put(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, y);
                        y += lay.line;
                    }
                    if room.members.len() > MEMBER_ROWS {
                        let more = format!("and {} more", room.members.len() - MEMBER_ROWS);
                        put(s, &mut self.labels, ui, &more, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, y);
                        y += lay.line;
                    }
                    let pictures = room.shared_pictures();
                    if !pictures.is_empty() {
                        y += lay.pad;
                        let heading = format!("Shared pictures ({})", pictures.len());
                        put(s, &mut self.labels, ui, &heading, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), text_x, y);
                        y += lay.line;
                        for name in pictures.iter().take(SHARED_ROWS) {
                            let shown = fit(ui, name, MAC_BODY_PT, Weight::Regular, text_w);
                            put(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, y);
                            y += lay.line;
                        }
                        if pictures.len() > SHARED_ROWS {
                            let more = format!("and {} more", pictures.len() - SHARED_ROWS);
                            put(s, &mut self.labels, ui, &more, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, y);
                        }
                    }
                }
            }
            Overlay::Settings => {
                let rows = [("Account", me.clone()), ("Homeserver", self.model.homeserver.clone()), ("Device", crate::protocol::DEVICE_NAME.to_string())];
                for (i, (label, value)) in rows.iter().enumerate() {
                    let y = panel.content_y + i as f32 * lay.line;
                    put(s, &mut self.labels, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, y);
                    let shown = fit(ui, value, MAC_BODY_PT, Weight::Regular, text_w * 0.65);
                    let vw = ui.measure(&shown, MAC_BODY_PT, Weight::Regular);
                    put(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x + text_w - vw, y);
                }
            }
            Overlay::Info(kind, id) => {
                let lines = self.model.info_lines(kind, &id);
                if lines.is_empty() {
                    put(s, &mut self.labels, ui, "Nothing to show", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), text_x, panel.content_y);
                }
                for (i, line) in lines.iter().enumerate() {
                    let shown = fit(ui, line, MAC_BODY_PT, Weight::Regular, text_w);
                    put(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, panel.content_y + i as f32 * lay.line);
                }
            }
            Overlay::Emoji(_) => {
                if let Some(f) = panel.field {
                    draw_field_tail(s, &mut self.labels, ui, lay, f, &self.model.emoji_query, "Search emoji", true);
                }
                if !panel.buttons.iter().any(|b| matches!(b.action, OverlayAction::Pick(_))) {
                    put_centred(s, &mut self.labels, ui, "No emoji match", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), px_ + pw / 2.0, panel.content_y + lay.pad);
                }
            }
            Overlay::None => {}
        }
        for b in &panel.buttons {
            if let OverlayAction::Pick(_) = b.action {
                put_centred(s, &mut self.labels, ui, &b.label, MAC_TITLE1_PT, Weight::Regular, LABEL.now(), b.rect[0] + b.rect[2] / 2.0, b.rect[1] + b.rect[3] / 2.0);
            } else {
                draw_button(s, &mut self.labels, ui, lay, b.rect, &b.label, b.primary, b.destructive, true);
            }
        }
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
    id: String,
    own: bool,
    day: Option<String>,
    head: f32,
    sender: Option<String>,
    /// The line quoting the message this one answers.
    quote: Option<String>,
    /// The id of the message the quote line is about, when the line is shown.
    quote_target: Option<String>,
    edited: bool,
    lines: Vec<String>,
    widest: f32,
    bubble_h: f32,
    chips: Vec<ChipView>,
    height: f32,
}

/// One reaction chip under a bubble, measured.
struct ChipView {
    key: String,
    label: String,
    width: f32,
    /// True when the signed-in user gave this reaction.
    mine: bool,
}

/// The height of a reaction chip.
fn chip_height(lay: &Layout) -> f32 {
    lay.foot + lay.gap
}

/// The chip text: the emoji, with the count when more than one person reacted.
pub fn chip_label(key: &str, count: usize) -> String {
    if count > 1 {
        format!("{key} {count}")
    } else {
        key.to_string()
    }
}

/// The small note on a room row for a pinned conversation or one with its alerts hidden.
pub fn room_tags(pinned: bool, muted: bool) -> String {
    match (pinned, muted) {
        (true, true) => "Pinned, alerts hidden".into(),
        (true, false) => "Pinned".into(),
        (false, true) => "Alerts hidden".into(),
        (false, false) => String::new(),
    }
}

/// The height of a day heading: its line and the space around it.
fn day_height(lay: &Layout) -> f32 {
    lay.foot + 2.0 * lay.gap
}

/// The lines the shared-pictures block of the details panel takes for `pictures` pictures: its heading, the names it
/// lists and a "more" line. None when there are no pictures.
pub fn media_rows(pictures: usize) -> usize {
    if pictures == 0 {
        0
    } else {
        1 + pictures.min(SHARED_ROWS) + usize::from(pictures > SHARED_ROWS)
    }
}

/// The height of the timeline rows before row `index`, each followed by `gap`: how far the content has to scroll for
/// that row to sit at the top.
pub fn rows_height_before(heights: &[f32], gap: f32, index: usize) -> f32 {
    heights.iter().take(index).map(|h| h + gap).sum()
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

/// A button: filled blue when `primary`, red text when `destructive`, dimmed when not `enabled`.
#[allow(clippy::too_many_arguments)]
fn draw_button(s: &mut Surface, labels: &mut TextCache, ui: &Ui, lay: &Layout, r: Rect, label: &str, primary: bool, destructive: bool, enabled: bool) {
    let (fill, ink) = if !enabled {
        (FIELD_BG.now(), LABEL_3.now())
    } else if primary {
        (colors::SYSTEM_BLUE.now(), WHITE)
    } else if destructive {
        (FIELD_BG.now(), DESTRUCTIVE)
    } else {
        (FIELD_BG.now(), LABEL.now())
    };
    s.fill_rrect(r[0], r[1], r[2], r[3], lay.corner, fill);
    put_centred(s, labels, ui, label, MAC_BODY_PT, Weight::Regular, ink, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
}

/// A one-line text field showing the end of `text`, a caret when `focused`, and `hint` while empty.
#[allow(clippy::too_many_arguments)]
fn draw_field_tail(s: &mut Surface, labels: &mut TextCache, ui: &Ui, lay: &Layout, r: Rect, text: &str, hint: &str, focused: bool) {
    s.fill_rrect(r[0], r[1], r[2], r[3], lay.corner, FIELD_BG.now());
    let cy = r[1] + r[3] / 2.0;
    if text.is_empty() {
        put_mid(s, labels, ui, hint, MAC_BODY_PT, Weight::Regular, LABEL_3.now(), r[0] + lay.gap, cy);
        if focused {
            s.fill_rect(px(r[0] + lay.gap), px(cy - ui.pt(MAC_BODY_PT) / 2.0), px(lay.rule.max(1.0)), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
        }
        return;
    }
    let shown = fit_tail(ui, text, MAC_BODY_PT, (r[2] - 2.0 * lay.gap).max(0.0));
    let tw = ui.measure(&shown, MAC_BODY_PT, Weight::Regular);
    put_mid(s, labels, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), r[0] + lay.gap, cy);
    if focused {
        s.fill_rect(px(r[0] + lay.gap + tw), px(cy - ui.pt(MAC_BODY_PT) / 2.0), px(lay.rule.max(1.0)), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
    }
}

/// The search field of the room list.
#[allow(clippy::too_many_arguments)]
fn draw_field(s: &mut Surface, labels: &mut TextCache, ui: &Ui, lay: &Layout, r: Rect, text: &str, hint: &str, focused: bool) {
    draw_field_tail(s, labels, ui, lay, r, text, hint, focused);
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
            emoji_w: EMOJI_W_PT,
            info_w: INFO_W_PT,
            banner: 0.0,
            corner: CORNER_PT,
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
        assert!(field[0] >= 240.0 && field[0] + field[2] <= lay.emoji_button()[0], "the field stops before Emoji");
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
    fn the_list_header_holds_the_search_field_and_new_message() {
        let mut lay = layout(900.0, 600.0);
        lay.side = 240.0;
        let (search, add) = (lay.search_field(), lay.new_button());
        assert!(search[0] + search[2] <= add[0], "the field stops before the button");
        assert!(add[0] + add[2] <= 240.0 - lay.pad + 0.01, "the button keeps the pad from the divider");
        let info = lay.info_button();
        assert_eq!(info[0] + info[2], 900.0 - lay.pad);
    }

    #[test]
    fn a_banner_takes_height_from_the_timeline() {
        let mut lay = layout(900.0, 600.0);
        lay.side = 240.0;
        let plain = lay.timeline()[3];
        lay.banner = BANNER_PT;
        assert_eq!(lay.timeline()[3], plain - BANNER_PT);
        let b = lay.banner_rect();
        assert_eq!(b[1] + b[3], 600.0 - COMPOSER_PT, "the banner sits right above the composer");
        let c = lay.banner_cancel();
        assert!(c[0] + c[2] <= b[0] + b[2]);
    }

    #[test]
    fn the_composer_buttons_sit_side_by_side() {
        let mut lay = layout(900.0, 600.0);
        lay.side = 240.0;
        let (emoji, attach, send) = (lay.emoji_button(), lay.attach_button(), lay.send_button());
        assert!(emoji[0] + emoji[2] < attach[0] && attach[0] + attach[2] < send[0]);
    }

    #[test]
    fn chips_show_a_count_past_one_and_rows_show_their_tags() {
        assert_eq!(chip_label("x", 1), "x");
        assert_eq!(chip_label("x", 3), "x 3");
        assert_eq!(room_tags(false, false), "");
        assert_eq!(room_tags(true, false), "Pinned");
        assert_eq!(room_tags(false, true), "Alerts hidden");
        assert!(room_tags(true, true).contains("Pinned"));
    }

    #[test]
    fn a_row_scrolls_to_the_top_past_the_rows_before_it() {
        let heights = [30.0, 50.0, 20.0];
        assert_eq!(rows_height_before(&heights, 8.0, 0), 0.0);
        assert_eq!(rows_height_before(&heights, 8.0, 2), 38.0 + 58.0);
        assert_eq!(rows_height_before(&heights, 8.0, 9), 38.0 + 58.0 + 28.0, "past the end counts every row");
    }

    #[test]
    fn the_shared_pictures_block_has_a_heading_the_names_and_a_more_line() {
        assert_eq!(media_rows(0), 0);
        assert_eq!(media_rows(1), 2);
        assert_eq!(media_rows(SHARED_ROWS), 1 + SHARED_ROWS);
        assert_eq!(media_rows(SHARED_ROWS + 3), 2 + SHARED_ROWS);
    }

    #[test]
    fn pixels_clamp_at_zero() {
        assert_eq!(px(-3.0), 0);
        assert_eq!(px(2.6), 3);
    }
}

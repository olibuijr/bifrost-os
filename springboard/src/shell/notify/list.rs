//! Notification cards and the grouped list they stack into: on the lock
//! screen one card per app with the rest peeking out underneath and a count
//! (a tap expands the stack), in Notification Center every group expanded
//! under its name. An expanded group has Mute and Clear buttons, the lock
//! screen's also Show Less; a Clear All button ends the list. Geometry in
//! bifrost points: 420 pt cards, 80 pt tall, 13 pt corners, a header row with a 20 pt icon, app name and time.

use super::super::ui::{Ui, Weight, GRAY_DIM, SYSTEM_BLUE, SYSTEM_RED, WHITE};
use super::super::notify_page::AppNotif;
use bifrost_proto::NotifAction;
use super::super::AppId;
use super::store::{age_text, Group, Notification};
use crate::gfx::{pack, scale, Image, Surface};

pub const CARD_W_PT: f32 = 420.0;
pub const CARD_H_PT: f32 = 80.0;
pub const RADIUS_PT: f32 = 23.0;
pub const ICON_PT: f32 = 20.0;
const PAD_PT: f32 = 14.0;
/// The attachment thumbnail at the right of a card, and the gap before the text.
const THUMB_PT: f32 = 40.0;
const THUMB_GAP_PT: f32 = 8.0;
const TEXT_X_PT: f32 = PAD_PT;
/// between the cards of an expanded group
const GAP_PT: f32 = 8.0;
pub const GROUP_GAP_PT: f32 = 10.0;
/// the cards peeking out under a stacked group
const PEEK_PT: f32 = 7.0;
const PEEK_INSET_PT: f32 = 10.0;
const MAX_PEEK: usize = 2;
/// the count line under a stacked group of several
const CAPTION_PT: f32 = 22.0;
/// the app name row above an expanded group
pub const HEADER_PT: f32 = 30.0;
const MORE_PT: f32 = 22.0;
/// how many cards a group expands to
pub const EXPANDED: usize = 4;
const BTN_H_PT: f32 = 24.0;
const BTN_RADIUS_PT: f32 = 12.0;
const BTN_GAP_PT: f32 = 6.0;
/// button widths, right to left: Clear, Mute, Show Less
const BTN_W_PT: [f32; 3] = [60.0, 66.0, 84.0];
const CLEAR_ALL_W_PT: f32 = 92.0;
/// a swiped-open card's Options, View and Clear buttons (pt wide, gap between)
const REVEAL_BTN_W_PT: f32 = 72.0;
const REVEAL_GAP_PT: f32 = 6.0;
const REVEAL_FILL: [u32; 3] = [GRAY_DIM, SYSTEM_BLUE, SYSTEM_RED];
const CLEAR_ALL_H_PT: f32 = 28.0;
/// the card material: dark, a little translucent (premultiplied)
pub const CARD: u32 = pack(58, 58, 60, 170);
const BODY: u32 = pack(230, 230, 230, 230);
const TIME: u32 = pack(138, 138, 144, 150);

/// Something a finger can land on in a list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// a card: the group it belongs to and the notification it shows
    Card { group: usize, id: u64, app: AppId },
    /// the group's Clear button
    Clear(usize),
    /// the group's Mute / Unmute button
    Mute(usize),
    /// the lock screen group's Show Less button
    Collapse(usize),
    /// the button under the list that drops everything
    ClearAll,
    /// an action button under a card: the group, the notification (its store id) and the button's index in its category
    Action { group: usize, id: u64, slot: usize },
}

/// One card, or a whole group, slid sideways by a finger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    pub group: usize,
    /// the one card of an expanded group; None slides the whole group
    pub id: Option<u64>,
    /// px, negative to the left
    pub dx: f32,
}

/// What a list needs to know besides the notifications.
#[derive(Clone, Copy, Default)]
pub struct Opts<'a> {
    /// lock-screen groups shown expanded
    pub expanded: &'a [AppId],
    pub muted: &'a [AppId],
    /// apps whose Show Previews setting hides their titles and bodies in this list
    pub hidden: &'a [AppId],
    /// hide titles and bodies (a passcode is set and the bifrost is locked)
    pub private: bool,
}

/// One rendered notification.
pub struct Card {
    pub id: u64,
    pub app: AppId,
    icon: Image,
    title: Image,
    body: Image,
    time: Image,
    /// the header row's app name (uppercase, secondary)
    name: Image,
    /// the first attachment, at the right of the card; none while the text is private
    thumb: Option<Image>,
    /// drawn with the app's name and "Notification" instead of its text (Show Previews)
    pub private: bool,
    /// the category's action buttons, drawn in a row under the text (empty for none)
    actions: Vec<ActionButton>,
}

/// One action button under a card: its identifier (sent back as the action), its title and an optional icon.
struct ActionButton {
    id: String,
    label: Image,
    icon: Option<Image>,
    destructive: bool,
}

/// Button height and spacing, in points.
const ACTION_H_PT: f32 = 26.0;
const ACTION_ROW_PT: f32 = 36.0;
const ACTION_PAD_PT: f32 = 12.0;
const ACTION_ICON_PT: f32 = 16.0;
const ACTION_GAP_PT: f32 = 8.0;

/// The buttons' rectangles (x, y, w, h) laid out left to right from `x`, `h` tall, `gap` apart.
pub fn lay_out_actions(widths: &[f32], x: f32, y: f32, h: f32, gap: f32) -> Vec<[f32; 4]> {
    let mut left = x;
    widths
        .iter()
        .map(|&w| {
            let r = [left, y, w, h];
            left += w + gap;
            r
        })
        .collect()
}

/// The index of the button whose rectangle holds the point (px, py), if any.
pub fn action_index_at(rects: &[[f32; 4]], px: f32, py: f32) -> Option<usize> {
    rects.iter().position(|r| px >= r[0] && px < r[0] + r[2] && py >= r[1] && py < r[1] + r[3])
}

impl Card {
    /// Renders `n` for a card `CARD_W_PT` wide; `icon` is the app's icon at any size.
    /// `private` shows the app's name and "Notification" instead of the text.
    pub fn new(ui: &Ui, n: &Notification, icon: &Image, now: i64, private: bool) -> Self {
        let d = ui.pt(ICON_PT).round() as usize;
        let time = ui.text(&age_text(n.time, now), 13.0, Weight::Regular, TIME);
        let name = ui.text(&app_name(n.app).to_uppercase(), 13.0, Weight::Regular, TIME);
        let thumb = if private { None } else { n.image.as_ref().map(|i| i.resized(ui.pt(THUMB_PT).round() as usize, ui.pt(THUMB_PT).round() as usize)) };
        let beside = if thumb.is_some() { THUMB_PT + THUMB_GAP_PT } else { 0.0 };
        let text_w = ui.pt(CARD_W_PT - TEXT_X_PT - PAD_PT - beside);
        let title_w = text_w;
        let (title, body) = if private { (app_name(n.app).to_string(), "Notification".to_string()) } else { (n.title.clone(), n.body.clone()) };
        Card {
            id: n.id,
            app: n.app,
            icon: icon.resized(d, d),
            title: ui.text(&ui.fit(&title, 15.0, title_w), 15.0, Weight::Semibold, WHITE),
            body: ui.text(&ui.fit(&body, 15.0, text_w), 15.0, Weight::Regular, BODY),
            time,
            name,
            thumb,
            private,
            actions: Vec::new(),
        }
    }

    /// Gives the card its category's action buttons. `icon` looks up an icon by its name (None when it is not
    /// available, and the button is drawn without one).
    pub fn set_actions(&mut self, ui: &Ui, actions: &[NotifAction], icon: &dyn Fn(&str) -> Option<Image>) {
        let d = ui.pt(ACTION_ICON_PT).round() as usize;
        self.actions = actions
            .iter()
            .map(|a| ActionButton {
                id: a.id.clone(),
                label: ui.text(&a.title, 13.0, Weight::Semibold, WHITE),
                icon: if a.icon.is_empty() { None } else { icon(&a.icon).map(|i| i.resized(d, d)) },
                destructive: a.destructive,
            })
            .collect();
    }

    /// The card's height: the base card, plus the action row when it has buttons.
    pub fn height(&self, ui: &Ui) -> f32 {
        ui.pt(CARD_H_PT) + if self.actions.is_empty() { 0.0 } else { ui.pt(ACTION_ROW_PT) }
    }

    /// Each button's width in px: its title, its icon when it has one, and the padding either side.
    fn action_widths(&self, ui: &Ui) -> Vec<f32> {
        let pad = ui.pt(ACTION_PAD_PT);
        self.actions.iter().map(|a| a.label.w as f32 + a.icon.as_ref().map_or(0.0, |i| i.w as f32 + ui.pt(ACTION_GAP_PT) / 2.0) + 2.0 * pad).collect()
    }

    /// The button rectangles for a card with its top-left at (x, y), in the order of its actions.
    fn action_rects(&self, ui: &Ui, x: f32, y: f32) -> Vec<[f32; 4]> {
        let row_y = y + ui.pt(CARD_H_PT) + (ui.pt(ACTION_ROW_PT) - ui.pt(ACTION_H_PT)) / 2.0;
        lay_out_actions(&self.action_widths(ui), x + ui.pt(TEXT_X_PT), row_y, ui.pt(ACTION_H_PT), ui.pt(ACTION_GAP_PT))
    }

    /// The identifier of the action whose button holds (px, py), for a card with its top-left at (x, y).
    pub fn action_at(&self, ui: &Ui, x: f32, y: f32, px: f32, py: f32) -> Option<&str> {
        self.action_slot_at(ui, x, y, px, py).map(|i| self.actions[i].id.as_str())
    }

    /// The index in the category of the button under (px, py), for a card with its top-left at (x, y).
    pub fn action_slot_at(&self, ui: &Ui, x: f32, y: f32, px: f32, py: f32) -> Option<usize> {
        action_index_at(&self.action_rects(ui, x, y), px, py)
    }

    /// Draws the card with its top-left at (x, y), faded by `alpha` / 255.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, x: f32, y: f32, alpha: u32) {
        let (w, h) = (ui.pt(CARD_W_PT), ui.pt(CARD_H_PT));
        s.fill_rrect(x, y, w, self.height(ui), ui.pt(RADIUS_PT), scale(CARD, alpha));
        for (a, r) in self.actions.iter().zip(self.action_rects(ui, x, y)) {
            let fill = if a.destructive { SYSTEM_RED } else { SYSTEM_BLUE };
            s.fill_rrect(r[0], r[1], r[2], r[3], r[3] / 2.0, scale(fill, alpha));
            let mut lx = r[0] + ui.pt(ACTION_PAD_PT);
            if let Some(icon) = &a.icon {
                s.blit(icon, lx as i32, (r[1] + (r[3] - icon.h as f32) / 2.0).round() as i32, 255, alpha);
                lx += icon.w as f32 + ui.pt(ACTION_GAP_PT) / 2.0;
            }
            s.blit(&a.label, lx as i32, (r[1] + (r[3] - a.label.h as f32) / 2.0).round() as i32, 255, alpha);
        }
        let head_mid = y + ui.pt(20.0);
        s.blit(&self.icon, (x + ui.pt(PAD_PT)) as i32, (head_mid - self.icon.h as f32 / 2.0).round() as i32, 255, alpha);
        s.blit(&self.name, (x + ui.pt(PAD_PT + ICON_PT + 8.0)) as i32, (head_mid - self.name.h as f32 / 2.0).round() as i32, 255, alpha);
        let tx = (x + ui.pt(TEXT_X_PT)) as i32;
        let head_mid = y + ui.pt(20.0);
        let title_mid = y + ui.pt(43.0);
        let body_mid = y + ui.pt(62.0);
        s.blit(&self.title, tx, (title_mid - self.title.h as f32 / 2.0).round() as i32, 255, alpha);
        s.blit(&self.body, tx, (body_mid - self.body.h as f32 / 2.0).round() as i32, 255, alpha);
        s.blit(&self.time, (x + w - ui.pt(PAD_PT) - self.time.w as f32) as i32, (head_mid - self.time.h as f32 / 2.0).round() as i32, 255, alpha);
        if let Some(t) = &self.thumb {
            s.blit(t, (x + w - ui.pt(PAD_PT) - t.w as f32) as i32, (y + (h - t.h as f32) / 2.0).round() as i32, 255, alpha);
        }
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;

    #[test]
    fn buttons_sit_left_to_right_with_the_gap_between_them() {
        let r = lay_out_actions(&[60.0, 80.0], 10.0, 5.0, 26.0, 8.0);
        assert_eq!(r, vec![[10.0, 5.0, 60.0, 26.0], [78.0, 5.0, 80.0, 26.0]]);
        assert!(lay_out_actions(&[], 10.0, 5.0, 26.0, 8.0).is_empty());
    }

    #[test]
    fn a_tap_inside_a_button_names_it_and_a_tap_outside_names_none() {
        let r = lay_out_actions(&[60.0, 80.0], 10.0, 5.0, 26.0, 8.0);
        assert_eq!(action_index_at(&r, 12.0, 6.0), Some(0));
        assert_eq!(action_index_at(&r, 150.0, 30.0), Some(1));
        assert_eq!(action_index_at(&r, 75.0, 6.0), None, "the gap between buttons is not a button");
        assert_eq!(action_index_at(&r, 70.0, 31.0), None, "below the row");
        assert_eq!(action_index_at(&r, 10.0, 5.0), Some(0), "the top-left corner is inside");
        assert_eq!(action_index_at(&r, 70.0, 5.0), None, "the right edge is exclusive");
    }
}

fn app_name(app: AppId) -> &'static str {
    super::super::apps().iter().find(|a| a.id == app).map_or("", |a| a.name)
}

/// An app's group as drawn: its newest cards and how many more there are.
pub struct GroupView {
    pub app: AppId,
    /// the thread of the stack; empty for notifications without one
    pub thread: String,
    /// notifications in the group
    pub count: usize,
    /// drawn as an expanded group (always in Notification Center)
    pub expanded: bool,
    pub muted: bool,
    name: Image,
    clear: Image,
    mute: Image,
    less: Image,
    /// "5 notifications" under a stacked group
    caption: Option<Image>,
    cards: Vec<Card>,
    more: Option<Image>,
}

impl GroupView {
    /// How many cards are drawn (the rest are counted in a footer).
    pub fn cards_len(&self) -> usize {
        self.cards.len()
    }

    /// Whether the card shows the newest of several, collapsed: a tap expands it.
    pub fn stacked(&self) -> bool {
        !self.expanded && self.count > 1
    }
}

/// Whether an app's page lets its notifications into a list of `mode`: Lock Screen for the lock screen, Notification
/// Centre for the sheet and its desktop column.
pub fn listed_in(n: &AppNotif, mode: Mode) -> bool {
    match mode {
        Mode::Lock => n.lock_screen,
        Mode::Center => n.center,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// one card per app, the rest peeking out underneath; a group may expand
    Lock,
    /// every group expanded under its name
    Center,
}

pub struct ListView {
    pub mode: Mode,
    pub groups: Vec<GroupView>,
    clear_all: Image,
    /// Options, View, Clear: the labels under a swiped card
    reveal: [Image; 3],
}

type Rect = (f32, f32, f32, f32);

impl ListView {
    /// Renders `groups` (as `Store::groups` orders them); `icon` gives an app's icon.
    pub fn new(ui: &Ui, mode: Mode, groups: &[Group], icon: &dyn Fn(AppId) -> Image, now: i64, opts: Opts) -> Self {
        let groups = groups
            .iter()
            .map(|g| {
                let img = icon(g.app);
                let expanded = mode == Mode::Center || opts.expanded.contains(&g.app);
                let shown = if expanded { EXPANDED.min(g.items.len()) } else { 1 };
                let hidden = g.items.len() - shown;
                let muted = opts.muted.contains(&g.app);
                let title = if g.thread.is_empty() { app_name(g.app).to_string() } else { format!("{} \u{b7} {}", app_name(g.app), g.thread) };
                let name = if muted { format!("{title} (Muted)") } else { title };
                GroupView {
                    app: g.app,
                    thread: g.thread.clone(),
                    count: g.items.len(),
                    expanded,
                    muted,
                    name: ui.text(&name, 15.0, Weight::Semibold, WHITE),
                    clear: ui.text("Clear", 13.0, Weight::Semibold, WHITE),
                    mute: ui.text(if muted { "Unmute" } else { "Mute" }, 13.0, Weight::Semibold, WHITE),
                    less: ui.text("Show Less", 13.0, Weight::Semibold, WHITE),
                    caption: (!expanded && g.items.len() > 1).then(|| ui.text(&format!("{} notifications", g.items.len()), 13.0, Weight::Regular, TIME)),
                    cards: g.items[..shown].iter().map(|n| Card::new(ui, n, &img, now, opts.private || opts.hidden.contains(&g.app))).collect(),
                    more: (hidden > 0).then(|| ui.text(&format!("{hidden} more notification{}", if hidden == 1 { "" } else { "s" }), 13.0, Weight::Regular, TIME)),
                }
            })
            .collect();
        let label = |t: &str| ui.text(t, 15.0, Weight::Regular, WHITE);
        ListView { mode, groups, clear_all: ui.text("Clear All", 13.0, Weight::Semibold, WHITE), reveal: [label("Options"), label("View"), label("Clear")] }
    }

    /// Gives every card its category's action buttons. `actions` lists a notification's buttons by its store id.
    /// Cards that hide their text (private previews) get none.
    pub fn set_actions(&mut self, ui: &Ui, actions: &dyn Fn(u64) -> Vec<NotifAction>, icon: &dyn Fn(&str) -> Option<Image>) {
        for g in &mut self.groups {
            for c in g.cards.iter_mut().filter(|c| !c.private) {
                let a = actions(c.id);
                if !a.is_empty() {
                    c.set_actions(ui, &a, icon);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Left edge of the cards, centred on a `w` px wide screen.
    pub fn x(ui: &Ui, w: i32) -> f32 {
        ((w as f32 - ui.pt(CARD_W_PT)) / 2.0).round()
    }

    /// Height of group `i` in px.
    pub fn group_h(&self, ui: &Ui, i: usize) -> f32 {
        let g = &self.groups[i];
        if g.expanded {
            let cards: f32 = g.cards.iter().map(|c| c.height(ui)).sum();
            ui.pt(HEADER_PT) + cards + (g.cards.len() as f32 - 1.0).max(0.0) * ui.pt(GAP_PT) + if g.more.is_some() { ui.pt(MORE_PT) } else { 0.0 }
        } else {
            let top = g.cards.first().map_or(ui.pt(CARD_H_PT), |c| c.height(ui));
            top + (g.count - 1).min(MAX_PEEK) as f32 * ui.pt(PEEK_PT) + if g.caption.is_some() { ui.pt(CAPTION_PT) } else { 0.0 }
        }
    }

    /// Top of group `i` when the list starts at `top`.
    pub fn group_top(&self, ui: &Ui, top: f32, i: usize) -> f32 {
        (0..i).fold(top, |y, k| y + self.group_h(ui, k) + ui.pt(GROUP_GAP_PT))
    }

    /// Height of the groups.
    pub fn height(&self, ui: &Ui) -> f32 {
        match self.groups.len() {
            0 => 0.0,
            n => self.group_top(ui, 0.0, n - 1) + self.group_h(ui, n - 1),
        }
    }

    /// Height of the Clear All button and the gap above it (0 for an empty list).
    pub fn footer_h(&self, ui: &Ui) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            ui.pt(GROUP_GAP_PT + CLEAR_ALL_H_PT)
        }
    }

    /// Everything a scroll has to bring into view.
    pub fn scroll_height(&self, ui: &Ui) -> f32 {
        self.height(ui) + self.footer_h(ui)
    }

    /// Card `j` of group `i`: (x, y) when the list starts at `top`.
    fn card_pos(&self, ui: &Ui, w: i32, top: f32, i: usize, j: usize) -> (f32, f32) {
        let gy = self.group_top(ui, top, i);
        let above: f32 = self.groups[i].cards[..j].iter().map(|c| c.height(ui) + ui.pt(GAP_PT)).sum();
        let y = if self.groups[i].expanded { gy + ui.pt(HEADER_PT) + above } else { gy };
        (Self::x(ui, w), y)
    }

    /// Header button `slot` of group `i` (0 Clear, 1 Mute, 2 Show Less), right to left.
    fn button_rect(&self, ui: &Ui, w: i32, top: f32, i: usize, slot: usize) -> Rect {
        let gy = self.group_top(ui, top, i);
        let bh = ui.pt(BTN_H_PT);
        let right = Self::x(ui, w) + ui.pt(CARD_W_PT);
        let offset: f32 = BTN_W_PT[..slot].iter().map(|bw| ui.pt(bw + BTN_GAP_PT)).sum();
        let bw = ui.pt(BTN_W_PT[slot]);
        (right - offset - bw, gy + (ui.pt(HEADER_PT) - bh) / 2.0, bw, bh)
    }

    /// How many header buttons group `i` has: Clear and Mute, plus Show Less on the lock screen.
    fn buttons(&self, i: usize) -> usize {
        if !self.groups[i].expanded {
            0
        } else if self.mode == Mode::Lock {
            3
        } else {
            2
        }
    }

    /// How far a card rests when swiped open: room for the three buttons.
    pub fn reveal_w(ui: &Ui) -> f32 {
        ui.pt(3.0 * (REVEAL_BTN_W_PT + REVEAL_GAP_PT))
    }

    /// The buttons uncovered by a card at `dx` (negative), left to right; Clear
    /// takes whatever a full swipe pulls beyond the resting width.
    fn reveal_rects(ui: &Ui, left: f32, y: f32, dx: f32) -> [Rect; 3] {
        let (bw, gap, h) = (ui.pt(REVEAL_BTN_W_PT), ui.pt(REVEAL_GAP_PT), ui.pt(CARD_H_PT));
        let right = left + ui.pt(CARD_W_PT);
        let shown = (-dx).max(0.0);
        let k = (shown / Self::reveal_w(ui)).min(1.0);
        let (fw, fg) = (bw * k, gap * k);
        let extra = (shown - Self::reveal_w(ui)).max(0.0);
        let clear_left = right + dx + fg + 2.0 * (fw + fg);
        [
            (right + dx + fg, y, fw, h),
            (right + dx + fg + fw + fg, y, fw, h),
            (clear_left, y, fw + extra, h),
        ]
    }

    /// Which of a card's revealed buttons (0 Options, 1 View, 2 Clear) a touch
    /// lands on while the card rests swiped open.
    pub fn reveal_hit(&self, ui: &Ui, w: i32, top: f32, group: usize, id: Option<u64>, x: f32, y: f32) -> Option<usize> {
        let g = self.groups.get(group)?;
        let j = id.and_then(|id| g.cards.iter().position(|c| c.id == id)).unwrap_or(0);
        let (cx, cy) = self.card_pos(ui, w, top, group, j);
        Self::reveal_rects(ui, cx, cy, -Self::reveal_w(ui)).iter().position(|&(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
    }

    /// The Clear All button, right-aligned under the last group.
    pub fn clear_all_rect(&self, ui: &Ui, w: i32, top: f32) -> Rect {
        let (bw, bh) = (ui.pt(CLEAR_ALL_W_PT), ui.pt(CLEAR_ALL_H_PT));
        (Self::x(ui, w) + ui.pt(CARD_W_PT) - bw, top + self.height(ui) + ui.pt(GROUP_GAP_PT), bw, bh)
    }

    /// What lies under (x, y) when the list starts at `top` on a `w` px wide screen.
    pub fn hit(&self, ui: &Ui, w: i32, top: f32, x: f32, y: f32) -> Option<Hit> {
        let (cw, ch) = (ui.pt(CARD_W_PT), ui.pt(CARD_H_PT));
        let inside = |(rx, ry, rw, rh): Rect| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        if !self.is_empty() && self.mode == Mode::Center && inside(self.clear_all_rect(ui, w, top)) {
            return Some(Hit::ClearAll);
        }
        for (i, g) in self.groups.iter().enumerate() {
            for slot in 0..self.buttons(i) {
                if inside(self.button_rect(ui, w, top, i, slot)) {
                    return Some([Hit::Clear(i), Hit::Mute(i), Hit::Collapse(i)][slot]);
                }
            }
            for (j, c) in g.cards.iter().enumerate() {
                let (cx, cy) = self.card_pos(ui, w, top, i, j);
                if let Some(slot) = c.action_slot_at(ui, cx, cy, x, y) {
                    return Some(Hit::Action { group: i, id: c.id, slot });
                }
                let h = if g.expanded { c.height(ui) } else { self.group_h(ui, i) };
                if inside((cx, cy, cw, h)) {
                    return Some(Hit::Card { group: i, id: c.id, app: c.app });
                }
            }
        }
        None
    }

    /// Draws the list from `top`; `slide` moves a group or one card sideways
    /// (fading as it goes); everything is faded by `alpha` / 255.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, top: f32, slide: Option<Slide>, alpha: u32) {
        let (cw, ch) = (ui.pt(CARD_W_PT), ui.pt(CARD_H_PT));
        let rw = Self::reveal_w(ui);
        // a card rests swiped open at the reveal width and only fades on its way out
        let fade = |a: u32, dx: f32| (a as f32 * (1.0 - ((-dx - rw).max(0.0) / (cw - rw)).min(1.0))) as u32;
        for (i, g) in self.groups.iter().enumerate() {
            let mine = slide.filter(|sl| sl.group == i);
            let (dx, a) = match mine {
                Some(Slide { id: None, dx, .. }) => (dx, fade(alpha, dx)),
                _ => (0.0, alpha),
            };
            let gy = self.group_top(ui, top, i);
            let x = Self::x(ui, w) + dx;
            if let Some(Slide { id, dx: sdx, .. }) = mine.filter(|sl| sl.dx < 0.0) {
                let j = id.and_then(|id| g.cards.iter().position(|c| c.id == id)).unwrap_or(0);
                let (lx, cy) = self.card_pos(ui, w, top, i, j);
                for (k, (bx, by, bw, bh)) in Self::reveal_rects(ui, lx, cy, sdx).into_iter().enumerate() {
                    if bw < 1.0 {
                        continue;
                    }
                    s.fill_rrect(bx, by, bw, bh, ui.pt(RADIUS_PT), scale(REVEAL_FILL[k], alpha));
                    let l = &self.reveal[k];
                    if (l.w as f32) < bw {
                        s.blit(l, (bx + (bw - l.w as f32) / 2.0).round() as i32, (by + (bh - l.h as f32) / 2.0).round() as i32, 255, alpha);
                    }
                }
            }
            if g.expanded {
                let mid = gy + ui.pt(HEADER_PT) / 2.0;
                s.blit(&g.name, (x + ui.pt(PAD_PT)) as i32, (mid - g.name.h as f32 / 2.0).round() as i32, 255, a);
                for slot in 0..self.buttons(i) {
                    let (bx, by, bw, bh) = self.button_rect(ui, w, top, i, slot);
                    let label = [&g.clear, &g.mute, &g.less][slot];
                    s.fill_rrect(bx + dx, by, bw, bh, ui.pt(BTN_RADIUS_PT), scale(CARD, a));
                    s.blit(label, (bx + dx + (bw - label.w as f32) / 2.0).round() as i32, (by + (bh - label.h as f32) / 2.0).round() as i32, 255, a);
                }
                for (j, c) in g.cards.iter().enumerate() {
                    let (_, cy) = self.card_pos(ui, w, top, i, j);
                    match mine {
                        Some(Slide { id: Some(id), dx: cdx, .. }) if id == c.id => c.draw(s, ui, x + cdx, cy, fade(alpha, cdx)),
                        _ => c.draw(s, ui, x, cy, a),
                    }
                }
                if let Some(m) = &g.more {
                    let (_, last) = self.card_pos(ui, w, top, i, g.cards.len() - 1);
                    let my = last + g.cards.last().map_or(ch, |c| c.height(ui)) + ui.pt(MORE_PT) / 2.0;
                    s.blit(m, (x + ui.pt(PAD_PT)) as i32, (my - m.h as f32 / 2.0).round() as i32, 255, a);
                }
            } else {
                for k in (1..=(g.count - 1).min(MAX_PEEK)).rev() {
                    let inset = k as f32 * ui.pt(PEEK_INSET_PT);
                    let shade = 255 - k as u32 * 70;
                    s.fill_rrect(x + inset, gy + k as f32 * ui.pt(PEEK_PT), cw - 2.0 * inset, ch, ui.pt(RADIUS_PT), scale(CARD, a * shade / 255));
                }
                g.cards[0].draw(s, ui, x, gy, a);
                if let Some(c) = &g.caption {
                    let cy = gy + g.cards[0].height(ui) + (g.count - 1).min(MAX_PEEK) as f32 * ui.pt(PEEK_PT) + ui.pt(CAPTION_PT) / 2.0;
                    s.blit(c, (x + ui.pt(PAD_PT)) as i32, (cy - c.h as f32 / 2.0).round() as i32, 255, a);
                }
            }
        }
        // bifrost offers Clear All only in Notification Center, not on the lock screen
        if !self.is_empty() && self.mode == Mode::Center {
            let (bx, by, bw, bh) = self.clear_all_rect(ui, w, top);
            s.fill_rrect(bx, by, bw, bh, ui.pt(BTN_RADIUS_PT), scale(CARD, alpha));
            s.blit(&self.clear_all, (bx + (bw - self.clear_all.w as f32) / 2.0).round() as i32, (by + (bh - self.clear_all.h as f32) / 2.0).round() as i32, 255, alpha);
        }
    }
}

/// The bifrost red of a badge (premultiplied, opaque).
pub const BADGE_RED: u32 = pack(255, 59, 48, 255);
const BADGE_PT: f32 = 24.0;

/// A red badge reading `count` ("99+" past 99) for a home-screen icon.
pub fn badge_image(ui: &Ui, count: usize) -> Image {
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let label = ui.text(&text, 14.0, Weight::Semibold, WHITE);
    let d = ui.pt(BADGE_PT).round();
    let w = d.max(label.w as f32 - ui.pt(2.0) + ui.pt(12.0)).round();
    let mut img = Image::new(w as usize, d as usize);
    let mut s = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
    s.fill_rrect(0.0, 0.0, w, d, ui.pt(RADIUS_PT).min(d / 2.0), BADGE_RED);
    s.blit(&label, ((w - label.w as f32) / 2.0).round() as i32, ((d - label.h as f32) / 2.0).round() as i32, 255, 255);
    img
}

/// Where a badge sits on an icon at (x, y): centred 8 pt in from the
/// icon's top-right corner.
pub fn badge_pos(ui: &Ui, icon_x: i32, icon_y: i32, icon_w: i32, badge: &Image) -> (i32, i32) {
    let cx = icon_x as f32 + icon_w as f32 - ui.pt(8.0);
    let cy = icon_y as f32 + ui.pt(8.0);
    ((cx - badge.w as f32 / 2.0).round() as i32, (cy - badge.h as f32 / 2.0).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn groups() -> Vec<Group> {
        let n = |app, title: &str, id| Notification { image: None, app, title: title.into(), body: "body".into(), time: 0, id, thread: String::new() };
        vec![
            Group { app: AppId::Files, thread: String::new(), items: vec![n(AppId::Files, "Alarm", 9), n(AppId::Files, "Timer", 8), n(AppId::Files, "Older", 7), n(AppId::Files, "Oldest", 6), n(AppId::Files, "Ancient", 5)] },
            Group { app: AppId::Settings, thread: String::new(), items: vec![n(AppId::Settings, "Low Battery", 4)] },
        ]
    }

    fn icon(_: AppId) -> Image {
        Image { w: 100, h: 100, px: vec![WHITE; 10_000] }
    }

    const NONE: Opts<'static> = Opts { expanded: &[], muted: &[], hidden: &[], private: false };

    #[test]
    fn lock_list_stacks_one_card_per_app_with_peeks_and_a_count() {
        let ui = ui();
        let l = ListView::new(&ui, Mode::Lock, &groups(), &icon, 0, NONE);
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].cards.len(), 1, "only the newest card is rendered");
        assert_eq!(l.groups[0].count, 5);
        assert!(l.groups[0].stacked() && !l.groups[1].stacked());
        assert_eq!(l.group_h(&ui, 0), ui.pt(CARD_H_PT) + 2.0 * ui.pt(PEEK_PT) + ui.pt(CAPTION_PT), "two cards peek out, the count line follows");
        assert_eq!(l.group_h(&ui, 1), ui.pt(CARD_H_PT), "a single notification has no count line");
        assert_eq!(l.group_top(&ui, 100.0, 1), 100.0 + l.group_h(&ui, 0) + ui.pt(GROUP_GAP_PT));
        assert_eq!(l.height(&ui), l.group_top(&ui, 0.0, 1) + ui.pt(CARD_H_PT));
        assert_eq!(l.scroll_height(&ui), l.height(&ui) + ui.pt(GROUP_GAP_PT + CLEAR_ALL_H_PT));
        let x = ListView::x(&ui, 1200);
        assert_eq!(x, ((1200.0 - ui.pt(CARD_W_PT)) / 2.0).round());
        assert_eq!(l.hit(&ui, 1200, 100.0, x + 10.0, 110.0), Some(Hit::Card { group: 0, id: 9, app: AppId::Files }));
        let peek_y = 100.0 + ui.pt(CARD_H_PT) + 5.0;
        assert_eq!(l.hit(&ui, 1200, 100.0, x + 100.0, peek_y), Some(Hit::Card { group: 0, id: 9, app: AppId::Files }), "the peeking cards belong to the group");
        let caption_y = 100.0 + l.group_h(&ui, 0) - 3.0;
        assert_eq!(l.hit(&ui, 1200, 100.0, x + 100.0, caption_y), Some(Hit::Card { group: 0, id: 9, app: AppId::Files }), "so does the count line");
        assert_eq!(l.hit(&ui, 1200, 100.0, x + 10.0, l.group_top(&ui, 100.0, 1) + 1.0), Some(Hit::Card { group: 1, id: 4, app: AppId::Settings }));
        assert_eq!(l.hit(&ui, 1200, 100.0, x - 1.0, 110.0), None);
        assert_eq!(l.hit(&ui, 1200, 100.0, x + 10.0, 99.0), None);
    }

    #[test]
    fn an_expanded_lock_group_lists_its_cards_with_buttons_and_leaves_the_rest_stacked() {
        let ui = ui();
        let opts = Opts { expanded: &[AppId::Files], ..NONE };
        let l = ListView::new(&ui, Mode::Lock, &groups(), &icon, 0, opts);
        assert!(l.groups[0].expanded && !l.groups[1].expanded);
        assert_eq!(l.groups[0].cards.len(), EXPANDED);
        assert_eq!(l.buttons(0), 3, "Clear, Mute and Show Less");
        assert_eq!(l.group_h(&ui, 0), ui.pt(HEADER_PT) + 4.0 * ui.pt(CARD_H_PT) + 3.0 * ui.pt(GAP_PT) + ui.pt(MORE_PT));
        let (x, top) = (ListView::x(&ui, 1200), 300.0);
        let mid = top + ui.pt(HEADER_PT) / 2.0;
        let right = x + ui.pt(CARD_W_PT);
        assert_eq!(l.hit(&ui, 1200, top, right - 10.0, mid), Some(Hit::Clear(0)));
        assert_eq!(l.hit(&ui, 1200, top, right - ui.pt(BTN_W_PT[0] + BTN_GAP_PT) - 10.0, mid), Some(Hit::Mute(0)));
        assert_eq!(l.hit(&ui, 1200, top, right - ui.pt(BTN_W_PT[0] + BTN_W_PT[1] + 2.0 * BTN_GAP_PT) - 10.0, mid), Some(Hit::Collapse(0)));
        assert_eq!(l.hit(&ui, 1200, top, x + 10.0, mid), None, "the name is not a button");
        let third = top + ui.pt(HEADER_PT) + 2.0 * ui.pt(CARD_H_PT + GAP_PT) + 5.0;
        assert_eq!(l.hit(&ui, 1200, top, x + 10.0, third), Some(Hit::Card { group: 0, id: 7, app: AppId::Files }));
        let g1 = l.group_top(&ui, top, 1);
        assert_eq!(l.hit(&ui, 1200, top, right - 10.0, g1 + 5.0), Some(Hit::Card { group: 1, id: 4, app: AppId::Settings }), "a stacked group has no buttons");
    }

    #[test]
    fn center_list_expands_groups_with_headers_and_clear_buttons() {
        let ui = ui();
        let l = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, NONE);
        assert_eq!(l.groups[0].cards.len(), EXPANDED);
        assert!(l.groups[0].more.is_some(), "the fifth is counted, not drawn");
        assert!(l.groups[1].more.is_none());
        assert_eq!(l.buttons(0), 2, "no Show Less: the sheet is always expanded");
        let h0 = ui.pt(HEADER_PT) + 4.0 * ui.pt(CARD_H_PT) + 3.0 * ui.pt(GAP_PT) + ui.pt(MORE_PT);
        assert_eq!(l.group_h(&ui, 0), h0);
        assert_eq!(l.group_h(&ui, 1), ui.pt(HEADER_PT) + ui.pt(CARD_H_PT));
        let x = ListView::x(&ui, 1200);
        let top = 400.0;
        assert_eq!(l.hit(&ui, 1200, top, x + ui.pt(CARD_W_PT) - 10.0, top + ui.pt(HEADER_PT) / 2.0), Some(Hit::Clear(0)));
        assert_eq!(l.hit(&ui, 1200, top, x + 10.0, top + ui.pt(HEADER_PT) / 2.0), None, "the name is not a button");
        let second = top + ui.pt(HEADER_PT) + ui.pt(CARD_H_PT + GAP_PT) + 5.0;
        assert_eq!(l.hit(&ui, 1200, top, x + 10.0, second), Some(Hit::Card { group: 0, id: 8, app: AppId::Files }));
        let gap = top + ui.pt(HEADER_PT) + ui.pt(CARD_H_PT) + 2.0;
        assert_eq!(l.hit(&ui, 1200, top, x + 10.0, gap), None, "between cards");
        let g1 = l.group_top(&ui, top, 1);
        assert_eq!(l.hit(&ui, 1200, top, x + ui.pt(CARD_W_PT) - 10.0, g1 + 5.0), Some(Hit::Clear(1)));
        assert_eq!(l.hit(&ui, 1200, top, x + 10.0, g1 + ui.pt(HEADER_PT) + 5.0), Some(Hit::Card { group: 1, id: 4, app: AppId::Settings }));
    }

    #[test]
    fn clear_all_sits_under_the_last_group() {
        let ui = ui();
        let lock = ListView::new(&ui, Mode::Lock, &groups(), &icon, 0, NONE);
        let (bx, by, bw, bh) = lock.clear_all_rect(&ui, 1200, 200.0);
        assert_eq!(lock.hit(&ui, 1200, 200.0, bx + bw / 2.0, by + bh / 2.0), None, "no Clear All on the lock screen");
        let l = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, NONE);
        let (x, top) = (ListView::x(&ui, 1200), 200.0);
        let (bx, by, bw, bh) = l.clear_all_rect(&ui, 1200, top);
        assert_eq!(by, top + l.height(&ui) + ui.pt(GROUP_GAP_PT));
        assert_eq!(bx + bw, x + ui.pt(CARD_W_PT), "right-aligned with the cards");
        assert_eq!(l.hit(&ui, 1200, top, bx + bw / 2.0, by + bh / 2.0), Some(Hit::ClearAll));
        assert_eq!(l.hit(&ui, 1200, top, bx - 5.0, by + bh / 2.0), None);
        let empty = ListView::new(&ui, Mode::Lock, &[], &icon, 0, NONE);
        assert_eq!(empty.footer_h(&ui), 0.0);
        assert_eq!(empty.hit(&ui, 1200, top, bx + bw / 2.0, top + ui.pt(GROUP_GAP_PT) + 5.0), None, "nothing to clear");
    }

    #[test]
    fn a_card_with_actions_is_taller_and_its_buttons_route_as_hits() {
        let ui = ui();
        let mut l = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, NONE);
        let before = l.group_h(&ui, 1);
        let acts = |id: u64| if id == 4 { vec![NotifAction::button("archive", "Archive"), NotifAction::button("delete", "Delete")] } else { vec![] };
        l.set_actions(&ui, &acts, &|_| None);
        assert_eq!(l.group_h(&ui, 1), before + ui.pt(ACTION_ROW_PT), "the Low Battery card gains its action row");
        let (cx, cy) = l.card_pos(&ui, 1200, 0.0, 1, 0);
        let [bx, by, _, _] = l.groups[1].cards[0].action_rects(&ui, cx, cy)[0];
        assert_eq!(l.hit(&ui, 1200, 0.0, bx + 2.0, by + 2.0), Some(Hit::Action { group: 1, id: 4, slot: 0 }));
        assert_eq!(l.hit(&ui, 1200, 0.0, cx + 2.0, cy + 2.0), Some(Hit::Card { group: 1, id: 4, app: AppId::Settings }));
    }

    #[test]
    fn an_attachment_shows_as_a_thumbnail_unless_previews_are_hidden() {
        let ui = ui();
        let icon = Image::new(8, 8);
        let mut n = Notification::new(AppId::Files, "T", "B", 0);
        n.image = Some(Image::new(96, 96));
        assert!(Card::new(&ui, &n, &icon, 0, false).thumb.is_some());
        assert!(Card::new(&ui, &n, &icon, 0, true).thumb.is_none(), "private text hides the picture too");
        n.image = None;
        assert!(Card::new(&ui, &n, &icon, 0, false).thumb.is_none());
    }

    #[test]
    fn previews_hidden_per_app_hide_only_that_app_text() {
        let ui = ui();
        let icon = |_: AppId| Image::new(1, 1);
        let opts = Opts { hidden: &[AppId::Settings], ..NONE };
        let l = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, opts);
        assert!(l.groups.iter().all(|g| g.cards.iter().all(|c| c.private == (g.app == AppId::Settings))));
    }

    #[test]
    fn muted_groups_say_so_and_private_cards_hide_the_text() {
        let ui = ui();
        let plain = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, NONE);
        let opts = Opts { muted: &[AppId::Files], private: true, ..NONE };
        let l = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, opts);
        assert!(l.groups[0].muted && !l.groups[1].muted);
        assert_ne!(l.groups[0].mute.px, plain.groups[0].mute.px, "Mute reads Unmute");
        assert_ne!(l.groups[0].name.w, plain.groups[0].name.w, "the name carries (Muted)");
        assert_ne!(l.groups[0].cards[0].body.px, plain.groups[0].cards[0].body.px, "the body is hidden");
    }

    #[test]
    fn draws_both_modes_and_a_dragged_group_or_card_fades() {
        let ui = ui();
        let (w, h) = (1200usize, 1920usize);
        let mut buf = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut buf, y0: 0 };
        let l = ListView::new(&ui, Mode::Lock, &groups(), &icon, 0, NONE);
        l.draw(&mut s, &ui, 1200, 300.0, None, 255);
        let x = ListView::x(&ui, 1200) as usize;
        let inside = buf[(300 + 20) * w + x + 20];
        assert_ne!(inside, 0, "card painted");
        let mut buf2 = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut buf2, y0: 0 };
        l.draw(&mut s, &ui, 1200, 300.0, Some(Slide { group: 0, id: None, dx: -200.0 }), 255);
        let right = x + ui.pt(CARD_W_PT) as usize - 20;
        assert_ne!(buf[(300 + 20) * w + right], 0);
        assert_ne!(buf2[(300 + 20) * w + right], buf[(300 + 20) * w + right], "the dragged card moved left and uncovered a button");
        // an expanded group slides one card, not the header
        let opts = Opts { expanded: &[AppId::Files], ..NONE };
        let e = ListView::new(&ui, Mode::Lock, &groups(), &icon, 0, opts);
        let mut buf4 = vec![0u32; w * h];
        e.draw(&mut Surface { w, h, stride: w, px: &mut buf4, y0: 0 }, &ui, 1200, 300.0, Some(Slide { group: 0, id: Some(9), dx: -300.0 }), 255);
        let first = 300 + ui.pt(HEADER_PT) as usize + 20;
        let second = first + ui.pt(CARD_H_PT + GAP_PT) as usize;
        assert_ne!(buf4[first * w + right], buf4[second * w + right], "the dragged card left its place");
        assert_ne!(buf4[second * w + right], 0, "the next card stayed");
        let c = ListView::new(&ui, Mode::Center, &groups(), &icon, 0, NONE);
        let mut buf3 = vec![0u32; w * h];
        c.draw(&mut Surface { w, h, stride: w, px: &mut buf3, y0: 0 }, &ui, 1200, 300.0, None, 255);
        let (bx, by, bw, bh) = c.button_rect(&ui, 1200, 300.0, 0, 0);
        assert_ne!(buf3[(by + bh / 2.0) as usize * w + (bx + bw / 2.0) as usize], 0, "clear button painted");
        let (bx, by, bw, bh) = c.clear_all_rect(&ui, 1200, 300.0);
        assert_ne!(buf3[(by + bh / 2.0) as usize * w + (bx + bw / 2.0) as usize], 0, "clear-all button painted");
    }

    #[test]
    fn badges_grow_with_the_count_and_sit_on_the_corner() {
        let ui = ui();
        let one = badge_image(&ui, 1);
        let many = badge_image(&ui, 123);
        assert_eq!(one.h, ui.pt(BADGE_PT).round() as usize);
        assert_eq!(one.w, one.h, "a single digit is a circle");
        assert!(many.w > one.w, "99+ needs a wider badge");
        assert!(one.px.contains(&BADGE_RED));
        let (x, y) = badge_pos(&ui, 100, 200, 100, &one);
        assert_eq!(x + one.w as i32 / 2, (200.0 - ui.pt(8.0)).round() as i32);
        assert_eq!(y + one.h as i32 / 2, (200.0 + ui.pt(8.0)).round() as i32);
    }
}

//! Notification Center: a swipe down from the left two thirds of the status
//! bar (the right third is Control Center) drags the cover sheet over a
//! frosted copy of the current screen: the clock and date as on the lock
//! screen, a "Notification Center" title and every app's notifications
//! expanded with a Clear button. A tap on a card opens its app, a tap on
//! empty space or a swipe up puts the sheet away; a long list scrolls.

use super::super::control_center::Frost;
use super::super::services::widgets_timeline::{self, Listing, Row};
use super::super::gestures::{ease_out_quartic, progress, Drag, SPRING_MS};
use super::super::ui::{centred, Weight, WHITE};
use super::super::{apps, AppId, Effect, Event, Screen, Shell};
use bifrost_ui::material_request::ShellSurface;
use super::list::{Hit, ListView, Mode, Slide, CARD_W_PT};
use crate::deildu::scroll::Scroller;
use crate::gfx::{rgba, Image, Surface};

/// finger travel for a full reveal
const REVEAL_PT: f32 = 300.0;
/// a touch that moves less than this is a tap
const TAP_PT: f32 = 8.0;
/// blur radius on the quarter-size copy, in pt
const BLUR_PT: f32 = 8.0;
const DIM: u32 = 140;
/// the clock's centre line, as on the bifrost's cover sheet
const TIME_MID_PT: f32 = 128.0;
const DATE_MID_PT: f32 = 199.0;
const TITLE_TOP_PT: f32 = 238.0;
pub const LIST_TOP_PT: f32 = 275.0;
const EMPTY_MID_PT: f32 = 293.0;
const BOTTOM_PAD_PT: f32 = 40.0;

#[derive(Clone, Copy, Debug)]
enum State {
    Hidden,
    Dragging(Drag),
    Settling { from: f32, to: f32, start: f64 },
    Shown,
}

/// A finger on the shown sheet.
#[derive(Clone, Copy, Debug)]
struct Touch {
    hit: Option<Hit>,
    drag: Drag,
    x0: f32,
    y0: f32,
    x: f32,
    /// the card was resting swiped open when the finger landed
    base: f32,
    /// a card touch decided to be a sideways swipe
    sideways: bool,
}

pub struct Center {
    state: State,
    touch: Option<Touch>,
    /// the card resting swiped open: group, id, app
    revealed: Option<(usize, u64, AppId)>,
    backdrop: Option<Frost>,
    /// the expanded list, rebuilt when the store or the minute changes
    pub(super) list: Option<ListView>,
    pub(super) list_key: (u32, i64),
    scroll: Scroller,
    title: Option<Image>,
    empty: Option<Image>,
    /// the desktop column's control under the pointer at press
    desk_press: Option<DeskHit>,
    /// the desktop column shows the Edit Widgets gallery instead of its widgets and cards
    editing: bool,
    /// the gallery row (listing index) whose Options are open in the gallery's place
    configuring: Option<usize>,
}

impl Default for Center {
    fn default() -> Self {
        Center { state: State::Hidden, touch: None, revealed: None, backdrop: None, list: None, list_key: (u32::MAX, 0), scroll: Scroller::new(), title: None, empty: None, desk_press: None, editing: false, configuring: None }
    }
}

impl Center {
    pub fn is_hidden(&self) -> bool {
        matches!(self.state, State::Hidden)
    }

    pub fn is_shown(&self) -> bool {
        matches!(self.state, State::Shown)
    }

    pub fn animating(&self) -> bool {
        matches!(self.state, State::Settling { .. }) || self.scroll.animating()
    }

    /// 0 hidden .. 1 shown, past the ends under the rubber band.
    pub fn progress(&self, t: f64) -> f32 {
        match (self.state, self.touch) {
            (State::Hidden, _) => 0.0,
            (State::Dragging(d), _) | (State::Shown, Some(Touch { drag: d, .. })) => d.progress(),
            (State::Settling { from, to, start }, _) => from + (to - from) * ease_out_quartic(progress(start, t, SPRING_MS)),
            (State::Shown, None) => 1.0,
        }
    }

    /// Drops the sheet and its backdrop at once.
    pub fn hide(&mut self) {
        self.state = State::Hidden;
        self.touch = None;
        self.backdrop = None;
        self.scroll.jump_top();
    }

    fn settle(&mut self, from: f32, shown: bool, t: f64) {
        self.touch = None;
        self.state = State::Settling { from, to: if shown { 1.0 } else { 0.0 }, start: t };
    }

    pub fn describe(&self) -> &'static str {
        match self.state {
            State::Hidden => "hidden",
            State::Dragging(_) => "dragging",
            State::Settling { to, .. } if to >= 1.0 => "opening",
            State::Settling { .. } => "closing",
            State::Shown => "shown",
        }
    }

    /// Content offset of a scrolled list.
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.offset()
    }
}

impl Shell {
    /// True while the sheet covers the screen in any way.
    pub(crate) fn center_active(&self) -> bool {
        !self.notify.center.is_hidden()
    }

    /// Opens the same sheet used by a status-bar swipe, for AssistiveTouch.
    pub(crate) fn open_notification_center(&mut self, t: f64) {
        if self.desk_active() {
            return self.open_desk_notification_center(t);
        }
        if !self.screen_on || self.screen == Screen::Lock {
            return;
        }
        self.cc.hide();
        let sharp = self.snapshot_screen(t);
        let c = &mut self.notify.center;
        c.backdrop = Some(Frost::new(sharp, (self.ui.pt(BLUR_PT) / 4.0) as usize, DIM));
        c.scroll.jump_top();
        c.settle(0.0, true, t);
        self.center_prepare();
        self.dirty = true;
    }

    /// Whether a touch at (x, y) starts a reveal: the left two thirds of the
    /// status bar (Control Center owns the right third).
    fn center_reveal_zone(&self, x: f32, y: f32) -> bool {
        self.screen != Screen::Lock && y < self.layout.status_h as f32 && x <= self.w as f32 * 2.0 / 3.0
    }

    /// Where the list may scroll to: its height past the space under the title.
    pub(super) fn center_scroll_max(&self) -> f32 {
        let ui = &self.ui;
        let viewport = self.h as f32 - ui.pt(LIST_TOP_PT + BOTTOM_PAD_PT);
        self.notify.center.list.as_ref().map_or(0.0, |l| (l.scroll_height(ui) - viewport).max(0.0))
    }

    /// Handles a touch that belongs to the sheet; None leaves it to the screen.
    pub(super) fn center_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if self.desk_active() {
            return match self.notify.center.state {
                State::Hidden => None,
                State::Shown => Some(self.center_desk_event(ev, t)),
                _ => Some(vec![]),
            };
        }
        let range = self.ui.pt(REVEAL_PT);
        match (self.notify.center.state, ev) {
            (State::Hidden, Event::Down(x, y)) if self.center_reveal_zone(x, y) => {
                let sharp = self.snapshot_screen(t);
                let c = &mut self.notify.center;
                c.backdrop = Some(Frost::new(sharp, (self.ui.pt(BLUR_PT) / 4.0) as usize, DIM));
                c.state = State::Dragging(Drag::new(y, t, 0.0, range));
                c.scroll.jump_top();
                Some(vec![])
            }
            (State::Hidden, _) => None,
            (State::Dragging(mut d), Event::Move(_, y)) => {
                d.moved(y, t);
                self.notify.center.state = State::Dragging(d);
                Some(vec![])
            }
            (State::Dragging(mut d), Event::Up(_, y)) => {
                d.moved(y, t);
                self.notify.center.settle(d.progress(), d.release(t), t);
                Some(vec![])
            }
            (State::Shown, Event::Down(x, y)) => {
                let overflow = self.center_scroll_max() > 0.0;
                let c = &mut self.notify.center;
                let top = self.ui.pt(LIST_TOP_PT) - c.scroll.offset();
                let hit = c.list.as_ref().and_then(|l| l.hit(&self.ui, self.w, top, x, y));
                let mut base = 0.0;
                if let Some((g, id, _)) = c.revealed {
                    let slot = c.list.as_ref().and_then(|l| l.reveal_hit(&self.ui, self.w, top, g, Some(id), x, y));
                    if let Some(slot) = slot {
                        c.revealed = None;
                        c.touch = None;
                        self.center_reveal_action(slot, g, id, t);
                        return Some(vec![]);
                    }
                    if matches!(hit, Some(Hit::Card { id: hid, .. }) if hid == id) {
                        base = -ListView::reveal_w(&self.ui);
                    } else {
                        c.revealed = None;
                    }
                }
                c.touch = Some(Touch { hit, drag: Drag::new(y, t, 1.0, range), x0: x, y0: y, x, base, sideways: false });
                if overflow {
                    c.scroll.down(y, t);
                }
                Some(vec![])
            }
            (State::Shown, Event::Move(x, y)) => {
                let overflow = self.center_scroll_max() > 0.0;
                let slop = self.ui.pt(TAP_PT);
                let c = &mut self.notify.center;
                if let Some(tc) = &mut c.touch {
                    tc.x = x;
                    if !tc.sideways && matches!(tc.hit, Some(Hit::Card { .. })) && (x - tc.x0).abs() > slop && (x - tc.x0).abs() > (y - tc.y0).abs() {
                        tc.sideways = true;
                    }
                    if tc.sideways {
                        // the card follows the finger; the sheet and the list stay put
                    } else if overflow {
                        c.scroll.moved(y, t);
                        if c.scroll.dragging() {
                            tc.hit = None;
                        }
                    } else {
                        tc.drag.moved(y, t);
                    }
                }
                Some(vec![])
            }
            (State::Shown, Event::Up(_, y)) => Some(self.center_up(y, t)),
            _ => Some(vec![]),
        }
    }

    /// The action behind a revealed button: 0 Options (mute the app), 1 View, 2 Clear.
    fn center_reveal_action(&mut self, slot: usize, group: usize, id: u64, t: f64) {
        let app = self.notify.center.list.as_ref().and_then(|l| l.groups.get(group)).map(|g| g.app);
        let Some(app) = app else { return };
        match slot {
            0 => self.toggle_mute(app),
            1 => {
                self.notify.center.hide();
                self.open_from_notification(app, t);
            }
            _ => self.clear_notification(id),
        }
    }

    fn center_up(&mut self, y: f32, t: f64) -> Vec<Effect> {
        let overflow = self.center_scroll_max() > 0.0;
        let tap_px = self.ui.pt(TAP_PT);
        let (cw, rw) = (self.ui.pt(CARD_W_PT), ListView::reveal_w(&self.ui));
        let c = &mut self.notify.center;
        let Some(mut tc) = c.touch.take() else { return vec![] };
        if tc.sideways || (tc.base < 0.0 && (tc.x - tc.x0).abs() <= tap_px) {
            if let Some(Hit::Card { group, id, app }) = tc.hit.or_else(|| c.list.as_ref().and_then(|_| c.revealed.map(|(g, id, app)| Hit::Card { group: g, id, app }))) {
                let total = tc.base + tc.x - tc.x0;
                c.revealed = None;
                if !tc.sideways {
                    // a tap on the open card closes it
                } else if -total > cw * 0.4 {
                    self.clear_notification(id);
                } else if -total > rw * 0.5 {
                    self.notify.center.revealed = Some((group, id, app));
                } else if total > cw * 0.2 {
                    self.notify.center.hide();
                    self.open_from_notification(app, t);
                }
            }
            return vec![];
        }
        let tap = if overflow {
            c.scroll.up(y, t)
        } else {
            tc.drag.moved(y, t);
            tc.drag.travel().abs() < tap_px
        };
        if !tap {
            if !overflow {
                c.settle(tc.drag.progress(), tc.drag.release(t), t);
            }
            return vec![];
        }
        match tc.hit {
            Some(Hit::Card { app, .. }) => {
                c.hide();
                self.open_from_notification(app, t);
            }
            Some(Hit::Clear(i)) => {
                if let Some((app, thread)) = c.list.as_ref().and_then(|l| l.groups.get(i)).map(|g| (g.app, g.thread.clone())) {
                    self.clear_group(app, &thread);
                }
            }
            Some(Hit::Mute(i)) => {
                if let Some(app) = c.list.as_ref().and_then(|l| l.groups.get(i)).map(|g| g.app) {
                    self.toggle_mute(app);
                }
            }
            Some(Hit::ClearAll) => self.clear_all_notifications(),
            Some(Hit::Action { id, slot, .. }) => self.list_action(id, slot, t),
            Some(Hit::Collapse(_)) => {}
            None => c.settle(1.0, false, t),
        }
        vec![]
    }

    /// Finishes a settle animation and scrolling.
    pub(super) fn center_tick(&mut self, t: f64) {
        let c = &mut self.notify.center;
        if let State::Settling { to, start, .. } = c.state {
            if progress(start, t, SPRING_MS) >= 1.0 {
                if to >= 1.0 {
                    c.state = State::Shown;
                } else {
                    c.hide();
                }
            }
            self.dirty = true;
        }
        if c.scroll.tick(t) {
            self.dirty = true;
        }
    }

    /// Rebuilds the list when the store or the minute changed.
    pub(super) fn center_prepare(&mut self) {
        let key = (self.notify.gen, self.notify.now() / 60);
        if self.notify.center.list_key != key {
            let list = self.render_list(Mode::Center);
            let c = &mut self.notify.center;
            c.list = Some(list);
            c.list_key = key;
        }
        let max = self.center_scroll_max();
        let c = &mut self.notify.center;
        c.scroll.set_max(max);
        if c.title.is_none() {
            c.title = Some(self.ui.text("Notification Center", 22.0, Weight::Bold, WHITE));
            c.empty = Some(self.ui.text("No Older Notifications", 17.0, Weight::Regular, rgba(255, 255, 255, 230)));
        }
    }

    /// Draws the frosted screen and the sheet for its progress at `t`.
    pub(in crate::shell) fn draw_center(&mut self, s: &mut Surface, t: f64) {
        if self.desk_active() {
            return self.draw_desk_center(s, t);
        }
        self.center_prepare();
        let c = &self.notify.center;
        let p = c.progress(t);
        let Some(frost) = &c.backdrop else { return };
        let fade = (p.clamp(0.0, 1.0) * 255.0) as u32;
        if fade < 255 {
            s.copy(frost.sharp.view(), 0, 0);
            s.blit(&frost.frosted, 0, 0, 255, fade);
        } else {
            s.copy(frost.frosted.view(), 0, 0);
        }
        let ui = &self.ui;
        let alpha = ((p * 2.0).clamp(0.0, 1.0) * 255.0) as u32;
        self.draw_bar(s, alpha);
        let slide = (p.min(1.3) - 1.0) * self.h as f32 * 0.6;
        let cx = self.w / 2;
        if let Some(tm) = &self.lock_time {
            centred(s, tm, cx, (ui.pt(TIME_MID_PT) + slide - tm.h as f32 / 2.0).round() as i32);
        }
        if let Some(d) = &self.lock_date {
            centred(s, d, cx, (ui.pt(DATE_MID_PT) + slide - d.h as f32 / 2.0).round() as i32);
        }
        let x = ListView::x(ui, self.w);
        match &c.list {
            Some(list) if !list.is_empty() => {
                if let Some(title) = &c.title {
                    s.blit(title, x as i32, (ui.pt(TITLE_TOP_PT) + slide) as i32, 255, alpha);
                }
                let top = ui.pt(LIST_TOP_PT) + slide - c.scroll.offset();
                let clip = (ui.pt(LIST_TOP_PT) + slide - ui.pt(6.0)).max(0.0) as usize;
                let sl = match (c.touch, c.revealed) {
                    (Some(tc), _) if tc.sideways => match tc.hit {
                        Some(Hit::Card { group, id, .. }) => {
                            let d = tc.base + tc.x - tc.x0;
                            Some(Slide { group, id: Some(id), dx: if d < 0.0 { d } else { d * 0.35 } })
                        }
                        _ => None,
                    },
                    (_, Some((group, id, _))) => Some(Slide { group, id: Some(id), dx: -ListView::reveal_w(ui) }),
                    _ => None,
                };
                list.draw(&mut s.band(clip), ui, self.w, top, sl, alpha);
            }
            _ => {
                if let Some(e) = &c.empty {
                    s.blit(e, cx - e.w as i32 / 2, (ui.pt(EMPTY_MID_PT) + slide - e.h as f32 / 2.0).round() as i32, 255, alpha);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Golden Gate desktop Notification Center: a right-hand column of glass cards
// under the menu bar (27-Golden-Gate-Notification-Center.png, 2x). Measured in
// pt: column 342 wide, 18 from the right edge; cards about 74 high with a 17 pt
// radius, 10 apart; a "Notification Center" title row with a 20 pt clear disc.
// One card per app (its newest notification), a stacked edge when the app has
// more; the card's disc clears that app, the title's clears all, a click on a
// card opens its app, a click elsewhere closes the column. Widgets are left
// out: the shell has no widget data source on the PC yet.
//
// Widget area (services/widgets_timeline.rs): under the title, one row per widget entry or live activity on screen,
// 58 pt high, 10 apart, above the notification cards.

const NC_WIDGET_PT: f32 = 58.0;
/// a widget set to Large in the Edit Widgets gallery
const NC_WIDGET_LARGE_PT: f32 = 96.0;
/// the Edit Widgets capsule at the foot of the column, its height and the gap above it
const NC_EDIT_PT: f32 = 28.0;
const NC_EDIT_TOP_PT: f32 = 14.0;
/// the gallery's Done capsule, the add/remove disc and the Small/Large chips
const NC_DONE_PT: f32 = 64.0;
const NC_DISC_PT: f32 = 24.0;
const NC_CHIP_W_PT: f32 = 56.0;
const NC_CHIP_GAP_PT: f32 = 6.0;
const NC_W_PT: f32 = 342.0;
const NC_RIGHT_PT: f32 = 18.0;
const NC_TOP_GAP_PT: f32 = 6.0;
const NC_TITLE_PT: f32 = 32.0;
const NC_CARD_PT: f32 = 74.0;
const NC_CARD_R_PT: f32 = 17.0;
const NC_GAP_PT: f32 = 10.0;
const NC_STACK_PT: f32 = 6.0;
const NC_PAD_PT: f32 = 14.0;
const NC_CLOSE_PT: f32 = 20.0;
/// room kept free above the dock
const NC_BOTTOM_PT: f32 = 90.0;

#[derive(Clone, Copy, PartialEq)]
enum DeskHit {
    ClearAll,
    Card(AppId),
    /// the close control of a stack: its index in the column's groups
    Close(usize),
    /// the Edit Widgets capsule at the foot of the column
    EditWidgets,
    /// the gallery's Done
    Done,
    /// the gallery row's add or remove disc (index into the gallery listing)
    Toggle(usize),
    /// the gallery row's Small (false) or Large (true) chip
    Size(usize, bool),
    /// the gallery row's Options chip (listing index): the widget's options replace the gallery
    Options(usize),
    /// an option row (option index) and the side tapped: -1 left half, +1 right half
    Param(usize, i64),
    /// a widget row in the column (index into `center_widget_rects`)
    Widget(usize),
}

/// One gallery row: (index in the listing, row rect, add/remove disc, Small chip, Large chip).
type GalleryRow = (usize, R4, R4, R4, R4, Option<R4>);

type R4 = (f32, f32, f32, f32);

/// The capsule's top: `gap` under the column's content ending at `end`, but no lower than `floor` (above the dock).
fn edit_capsule_y(end: f32, gap: f32, floor: f32) -> f32 {
    (end + gap).min(floor)
}

fn within((x, y, w, h): R4, px: f32, py: f32) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

impl Shell {
    /// Title row, then (group index, card rect, close rect) for the cards that fit.
    fn center_desk_layout(&self) -> (R4, R4, Vec<(usize, R4, R4)>) {
        let ui = &self.ui;
        let w = ui.pt(NC_W_PT);
        let x = self.w as f32 - ui.pt(NC_RIGHT_PT) - w;
        let mut y = self.desktop_bar_height() as f32 + ui.pt(NC_TOP_GAP_PT);
        let title = (x, y, w, ui.pt(NC_TITLE_PT));
        let c = ui.pt(NC_CLOSE_PT);
        let clear = (x + w - c, y + (ui.pt(NC_TITLE_PT) - c) / 2.0, c, c);
        y += ui.pt(NC_TITLE_PT);
        if let Some((r, _)) = self.center_widget_rects().last() {
            y = r.1 + r.3 + ui.pt(NC_GAP_PT);
        }
        let mut cards = vec![];
        for (i, g) in self.visible_groups(Mode::Center).iter().enumerate() {
            let h = ui.pt(NC_CARD_PT);
            let stack = if g.items.len() > 1 { ui.pt(NC_STACK_PT) } else { 0.0 };
            if y + h + stack > self.h as f32 - ui.pt(NC_BOTTOM_PT) {
                break;
            }
            cards.push((i, (x, y, w, h), (x + w - ui.pt(NC_PAD_PT) - c + ui.pt(6.0), y + ui.pt(6.0), c, c)));
            y += h + stack + ui.pt(NC_GAP_PT);
        }
        (title, clear, cards)
    }

    /// The widget area under the title: one glass row per widget entry or live activity on screen
    /// (`services::widgets_timeline::rows`), in the order the shell has them, as many as fit above the dock.
    fn center_widget_rects(&self) -> Vec<(R4, Row)> {
        let ui = &self.ui;
        let w = ui.pt(NC_W_PT);
        let x = self.w as f32 - ui.pt(NC_RIGHT_PT) - w;
        let mut y = self.desktop_bar_height() as f32 + ui.pt(NC_TOP_GAP_PT) + ui.pt(NC_TITLE_PT);
        let mut out = vec![];
        for row in widgets_timeline::rows(self.notify.now()) {
            let h = ui.pt(if row.large { NC_WIDGET_LARGE_PT } else { NC_WIDGET_PT });
            if y + h > self.h as f32 - ui.pt(NC_BOTTOM_PT) {
                break;
            }
            out.push(((x, y, w, h), row));
            y += h + ui.pt(NC_GAP_PT);
        }
        out
    }

    /// The Edit Widgets capsule: the column's width, just under the last widget or notification (under the title when
    /// the column is empty), and never lower than the room kept free above the dock.
    fn center_edit_rect(&self) -> R4 {
        let ui = &self.ui;
        let w = ui.pt(NC_W_PT);
        let x = self.w as f32 - ui.pt(NC_RIGHT_PT) - w;
        let (title, _, cards) = self.center_desk_layout();
        let widgets = self.center_widget_rects();
        let below = |r: &R4| r.1 + r.3;
        let end = widgets.iter().map(|(r, _)| below(r)).chain(cards.iter().map(|(_, r, _)| below(r))).fold(below(&title), f32::max);
        let h = ui.pt(NC_EDIT_PT);
        let floor = self.h as f32 - ui.pt(NC_BOTTOM_PT) - h;
        (x, edit_capsule_y(end, ui.pt(NC_EDIT_TOP_PT), floor), w, h)
    }

    /// The gallery's title row (with Done at its trailing end), then one row per widget in the listing that fits.
    fn center_gallery_layout(&self) -> (R4, R4, Vec<GalleryRow>) {
        let ui = &self.ui;
        let w = ui.pt(NC_W_PT);
        let x = self.w as f32 - ui.pt(NC_RIGHT_PT) - w;
        let top = self.desktop_bar_height() as f32 + ui.pt(NC_TOP_GAP_PT);
        let title = (x, top, w, ui.pt(NC_TITLE_PT));
        let (dw, disc, chip, gap) = (ui.pt(NC_DONE_PT), ui.pt(NC_DISC_PT), ui.pt(NC_CHIP_W_PT), ui.pt(NC_CHIP_GAP_PT));
        let cy_title = top + (ui.pt(NC_TITLE_PT) - ui.pt(NC_DISC_PT)) / 2.0;
        let done = (x + w - dw, cy_title, dw, ui.pt(NC_DISC_PT));
        let pad = ui.pt(NC_PAD_PT);
        let mut y = top + ui.pt(NC_TITLE_PT);
        let mut rows = vec![];
        let list = widgets_timeline::gallery(self.notify.now());
        for i in 0..list.len() {
            let h = ui.pt(NC_WIDGET_PT);
            if y + h > self.h as f32 - ui.pt(NC_BOTTOM_PT) {
                break;
            }
            let cy = y + h / 2.0;
            let toggle = (x + w - pad - disc, cy - disc / 2.0, disc, disc);
            let large = (toggle.0 - gap - chip, cy - disc / 2.0, chip, disc);
            let small = (large.0 - gap - chip, cy - disc / 2.0, chip, disc);
            // a widget with options offers an Options chip to the left of Small
            let opts = list[i].configurable.then(|| (small.0 - gap - 2.0 * chip, cy - disc / 2.0, 2.0 * chip, disc));
            rows.push((i, (x, y, w, h), toggle, small, large, opts));
            y += h + ui.pt(NC_GAP_PT);
        }
        (title, done, rows)
    }

    /// The column's bounds in px, for the presenter's opaque regions.
    pub(crate) fn center_desk_rect(&self) -> Option<R4> {
        if self.notify.center.is_hidden() || !self.desk_active() {
            return None;
        }
        if self.notify.center.editing {
            let (t, _, rows) = self.center_gallery_layout();
            let bottom = if self.notify.center.configuring.is_some() {
                self.center_options_layout().last().map(|(_, r)| r.1 + r.3)
            } else {
                rows.last().map(|r| r.1 .1 + r.1 .3)
            }
            .unwrap_or(t.1 + t.3);
            return Some((t.0, t.1, t.2, bottom - t.1));
        }
        let (t, _, cards) = self.center_desk_layout();
        let bottom = cards
            .last()
            .map(|c| c.1 .1 + c.1 .3 + self.ui.pt(NC_STACK_PT))
            .or_else(|| self.center_widget_rects().last().map(|(r, _)| r.1 + r.3))
            .unwrap_or(t.1 + t.3);
        // the Edit Widgets capsule is part of the column too
        let edit = self.center_edit_rect();
        Some((t.0, t.1, t.2, bottom.max(edit.1 + edit.3) - t.1))
    }

    fn center_gallery_hit(&self, x: f32, y: f32) -> Option<DeskHit> {
        let (_, done, rows) = self.center_gallery_layout();
        if within(done, x, y) {
            return Some(DeskHit::Done);
        }
        for (i, _, toggle, small, large, opts) in rows {
            if opts.is_some_and(|o| within(o, x, y)) {
                return Some(DeskHit::Options(i));
            }
            if within(toggle, x, y) {
                return Some(DeskHit::Toggle(i));
            }
            if within(small, x, y) {
                return Some(DeskHit::Size(i, false));
            }
            if within(large, x, y) {
                return Some(DeskHit::Size(i, true));
            }
        }
        None
    }

    fn center_desk_hit(&self, x: f32, y: f32) -> Option<DeskHit> {
        if self.notify.center.editing {
            return if self.notify.center.configuring.is_some() { self.center_options_hit(x, y) } else { self.center_gallery_hit(x, y) };
        }
        if let Some(i) = self.center_widget_rects().iter().position(|(r, _)| within(*r, x, y)) {
            return Some(DeskHit::Widget(i));
        }
        if within(self.center_edit_rect(), x, y) {
            return Some(DeskHit::EditWidgets);
        }
        let (_, clear, cards) = self.center_desk_layout();
        if within(clear, x, y) {
            return Some(DeskHit::ClearAll);
        }
        let groups = self.visible_groups(Mode::Center);
        cards.iter().find(|c| within(c.1, x, y)).map(|&(i, _, close)| if within(close, x, y) { DeskHit::Close(i) } else { DeskHit::Card(groups[i].app) })
    }

    /// The option rows of the widget being configured: (option index, row rect), as many as fit above the dock.
    fn center_options_layout(&self) -> Vec<(usize, R4)> {
        let ui = &self.ui;
        let w = ui.pt(NC_W_PT);
        let x = self.w as f32 - ui.pt(NC_RIGHT_PT) - w;
        let mut y = self.desktop_bar_height() as f32 + ui.pt(NC_TOP_GAP_PT) + ui.pt(NC_TITLE_PT);
        let mut rows = vec![];
        for j in 0..self.configuring_count() {
            let h = ui.pt(NC_WIDGET_PT);
            if y + h > self.h as f32 - ui.pt(NC_BOTTOM_PT) {
                break;
            }
            rows.push((j, (x, y, w, h)));
            y += h + ui.pt(NC_GAP_PT);
        }
        rows
    }

    /// How many options the widget being configured has (0 when none is being configured).
    fn configuring_count(&self) -> usize {
        self.notify.center.configuring.and_then(|i| self.gallery_item(i)).and_then(|item| widgets_timeline::configuration(&item.app, &item.kind)).map_or(0, |c| c.params().len())
    }

    /// The app, kind and key of option `j` of the widget being configured.
    fn option_at(&self, j: usize) -> Option<(String, String, String)> {
        let item = self.gallery_item(self.notify.center.configuring?)?;
        let key = widgets_timeline::configuration(&item.app, &item.kind)?.params().get(j)?.key().to_string();
        Some((item.app, item.kind, key))
    }

    /// The hit in the options view: Done, or an option row with the side of the row that was pressed.
    fn center_options_hit(&self, x: f32, y: f32) -> Option<DeskHit> {
        let (_, done, _) = self.center_gallery_layout();
        if within(done, x, y) {
            return Some(DeskHit::Done);
        }
        for (j, r) in self.center_options_layout() {
            if within(r, x, y) {
                let dir = if x < r.0 + r.2 / 2.0 { -1 } else { 1 };
                return Some(DeskHit::Param(j, dir));
            }
        }
        None
    }

    /// Opens the app of a widget row. A widget with a link opens the app at it: the app gets `Open` with the link.
    fn open_widget(&mut self, i: usize, t: f64) {
        let Some((_, row)) = self.center_widget_rects().into_iter().nth(i) else { return };
        let Some((app, kind)) = row.widget else { return };
        let link = widgets_timeline::link(&app, &kind).map(|l| l.to_url());
        let Some(def) = apps::apps().iter().find(|a| a.icon == app.as_str()).map(|a| a.id) else { return };
        self.notify.center.hide();
        self.desk_open_or_start(def, t);
        if let (AppId::External(slot), Some(url)) = (def, link) {
            self.ext_send(slot as usize, &bifrost_proto::ToApp::Open { url });
        }
        self.dirty = true;
    }

    /// Opens the desktop column (the menu bar clock, as on Golden Gate).
    pub(crate) fn open_desk_notification_center(&mut self, t: f64) {
        if !self.screen_on || self.is_locked() {
            return;
        }
        self.cc.hide();
        // a live overlay over the desktop, like the menu-bar menus: no snapshot
        let c = &mut self.notify.center;
        c.backdrop = None;
        // it opens on the widgets, not on the gallery of an earlier visit
        c.editing = false;
        c.configuring = None;
        c.settle(0.0, true, t);
        self.dirty = true;
    }

    /// The gallery's listing entry at `i`, as the service has it now.
    fn gallery_item(&self, i: usize) -> Option<Listing> {
        widgets_timeline::gallery(self.notify.now()).into_iter().nth(i)
    }

    fn center_desk_event(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        match ev {
            Event::Down(x, y) => {
                let hit = self.center_desk_hit(x, y);
                if hit.is_none() && !self.center_desk_rect().is_some_and(|r| within(r, x, y)) {
                    self.notify.center.settle(1.0, false, t);
                }
                self.notify.center.desk_press = hit;
            }
            Event::Up(x, y) => {
                let hit = self.notify.center.desk_press.take();
                if hit.is_some() && hit == self.center_desk_hit(x, y) {
                    match hit.unwrap() {
                        DeskHit::ClearAll => self.clear_all_notifications(),
                        DeskHit::Close(i) => {
                            let stack = self.visible_groups(Mode::Center).get(i).map(|g| (g.app, g.thread.clone()));
                            if let Some((app, thread)) = stack {
                                self.clear_group(app, &thread);
                            }
                        }
                        DeskHit::Card(app) => {
                            self.notify.center.hide();
                            self.open_from_notification(app, t);
                        }
                        DeskHit::EditWidgets => self.notify.center.editing = true,
                        DeskHit::Done => {
                            if self.notify.center.configuring.is_some() {
                                self.notify.center.configuring = None;
                            } else {
                                self.notify.center.editing = false;
                            }
                        }
                        DeskHit::Options(i) => self.notify.center.configuring = Some(i),
                        DeskHit::Param(j, dir) => {
                            if let Some((app, kind, key)) = self.option_at(j) {
                                widgets_timeline::step_option(&app, &kind, &key, dir);
                            }
                        }
                        DeskHit::Widget(i) => self.open_widget(i, t),
                        DeskHit::Toggle(i) => {
                            if let Some(item) = self.gallery_item(i) {
                                widgets_timeline::place(&item.app, &item.kind, !item.placed);
                            }
                        }
                        DeskHit::Size(i, large) => {
                            if let Some(item) = self.gallery_item(i) {
                                widgets_timeline::resize(&item.app, &item.kind, large);
                            }
                        }
                    }
                    self.dirty = true;
                }
            }
            _ => {}
        }
        vec![]
    }

    fn draw_desk_center(&mut self, s: &mut Surface, t: f64) {
        use super::super::control_center::desk_glass;
        use super::super::ui::{FIELD_BG, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, BAR_TEXT, BAR_TEXT_SHADOW, BAR_TEXT_SHADOW_ALPHA};
        let c = &self.notify.center;
        let p = c.progress(t).clamp(0.0, 1.0);
        if let Some((x, y, w, h)) = self.center_desk_rect().filter(|_| bifrost_render::glass::gpu()) {
            // the column floats over the live desktop: clear only its own rect (and the slide) for the glass
            let pad = self.ui.pt(32.0);
            s.fill_rect((x - pad).max(0.0) as usize, (y - pad).max(0.0) as usize, (w + 2.0 * pad) as usize, (h + 2.0 * pad) as usize, 0);
        }
        if self.notify.center.editing {
            return self.draw_desk_gallery(s, p);
        }
        let alpha = (p * 255.0) as u32;
        let ui = &self.ui;
        let slide = (1.0 - p) * ui.pt(24.0);
        let (title, clear, cards) = self.center_desk_layout();
        let groups = self.visible_groups(Mode::Center);
        let now = self.notify.now();
        let (label, secondary) = (LABEL.now(), LABEL_2.now());
        let disc = |s: &mut Surface, (x, y, w, h): R4, fill: u32, ink: u32| {
            s.fill_rrect(x + slide, y, w, h, w / 2.0, crate::gfx::scale(fill, alpha));
            let k = w * 0.18;
            let (cx, cy) = (x + slide + w / 2.0, y + h / 2.0);
            for d in [-1.0f32, 1.0] {
                for i in 0..((2.0 * k) as i32).max(1) {
                    let o = i as f32 - k;
                    s.fill_rect((cx + o) as usize, (cy + o * d) as usize, ui.pt(1.5) as usize, ui.pt(1.5) as usize, crate::gfx::scale(ink, alpha));
                }
            }
        };
        // the title sits on the wallpaper like the menu bar's text: white with a soft shadow
        let name = if groups.is_empty() { "No Notifications" } else { "Notification Center" };
        let tm = ui.text(name, MAC_TITLE3_PT, Weight::Bold, WHITE);
        let ty = (title.1 + (title.3 - tm.h as f32) / 2.0) as i32;
        s.blit_colored(&tm, (title.0 + slide) as i32, ty + ui.pt(0.5).max(1.0) as i32, BAR_TEXT_SHADOW, BAR_TEXT_SHADOW_ALPHA * alpha / 255);
        s.blit_colored(&tm, (title.0 + slide) as i32, ty, BAR_TEXT, alpha);
        if !groups.is_empty() {
            disc(s, clear, FIELD_BG.now(), label);
        }
        let pad = ui.pt(NC_PAD_PT);
        for ((x, y, w, h), row) in self.center_widget_rects() {
            let x = x + slide;
            let r = ui.pt(NC_CARD_R_PT);
            desk_glass(s, ShellSurface::NotificationPanel, (x, y, w, h), r, alpha);
            let room = w - 2.0 * pad;
            let head = ui.text(&ui.fit(&row.title, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, Weight::Regular, secondary);
            let value = ui.text(&ui.fit(&row.text, MAC_HEADLINE_PT, room), MAC_HEADLINE_PT, Weight::Semibold, label);
            let ly = y + (h - (head.h + value.h) as f32) / 2.0;
            s.blit(&head, (x + pad) as i32, ly as i32, 255, alpha);
            s.blit(&value, (x + pad) as i32, (ly + head.h as f32) as i32, 255, alpha);
        }
        for (i, (x, y, w, h), close) in cards {
            let g = &groups[i];
            let x = x + slide;
            let r = ui.pt(NC_CARD_R_PT);
            if g.items.len() > 1 {
                let inset = ui.pt(10.0);
                desk_glass(s, ShellSurface::NotificationPanel, (x + inset, y + h - r, w - 2.0 * inset, r + ui.pt(NC_STACK_PT)), r, alpha);
            }
            desk_glass(s, ShellSurface::NotificationPanel, (x, y, w, h), r, alpha);
            let n = g.latest();
            let app = apps().iter().find(|a| a.id == g.app).map_or("", |a| a.name);
            let room = w - 2.0 * pad - ui.pt(NC_CLOSE_PT);
            let more = if g.items.len() > 1 { format!("{app} \u{b7} {} more", g.items.len() - 1) } else { app.to_string() };
            let head = ui.text(&ui.fit(&more, MAC_SUBHEAD_PT, room - ui.pt(60.0)), MAC_SUBHEAD_PT, Weight::Regular, secondary);
            let age = ui.text(&super::store::age_text(n.time, now), MAC_SUBHEAD_PT, Weight::Regular, secondary);
            let t1 = ui.text(&ui.fit(&n.title, MAC_HEADLINE_PT, room), MAC_HEADLINE_PT, Weight::Semibold, label);
            let t2 = ui.text(&ui.fit(&n.body, MAC_BODY_PT, w - 2.0 * pad), MAC_BODY_PT, Weight::Regular, label);
            let mut ly = y + (h - (head.h + t1.h + t2.h) as f32) / 2.0;
            s.blit(&head, (x + pad) as i32, ly as i32, 255, alpha);
            s.blit(&age, (x + w - pad - ui.pt(NC_CLOSE_PT) - ui.pt(6.0) - age.w as f32) as i32, ly as i32, 255, alpha);
            ly += head.h as f32;
            s.blit(&t1, (x + pad) as i32, ly as i32, 255, alpha);
            ly += t1.h as f32;
            s.blit(&t2, (x + pad) as i32, ly as i32, 255, alpha);
            disc(s, close, FIELD_BG.now(), label);
        }
        // Edit Widgets: a glass capsule with a link-coloured label, at the foot of the column above the dock
        let (ex, ey, ew, eh) = self.center_edit_rect();
        desk_glass(s, ShellSurface::NotificationPanel, (ex + slide, ey, ew, eh), eh / 2.0, alpha);
        let et = ui.text("Edit Widgets", MAC_SUBHEAD_PT, Weight::Semibold, LINK);
        s.blit(&et, (ex + slide + (ew - et.w as f32) / 2.0) as i32, (ey + (eh - et.h as f32) / 2.0) as i32, 255, alpha);
    }

    /// The Edit Widgets gallery in the column's place: the title with Done, then a row per widget with a disc that adds
    /// it to or removes it from the column, and Small and Large chips for its size.
    /// The options of the widget being configured: one glass row per option, its name on the left and its value in
    /// `LINK` on the right. A tap on the right half of a row steps the value up, a tap on the left half steps it down.
    fn draw_option_rows(&self, s: &mut Surface, slide: f32, alpha: u32) {
        use super::super::control_center::desk_glass;
        use super::super::ui::{LABEL, LINK, MAC_HEADLINE_PT, MAC_SUBHEAD_PT};
        use bifrost_ui::widget_config::Value;
        let ui = &self.ui;
        let label = LABEL.now();
        let r = ui.pt(NC_CARD_R_PT);
        let pad = ui.pt(NC_PAD_PT);
        let Some(i) = self.notify.center.configuring else { return };
        let Some(item) = self.gallery_item(i) else { return };
        let Some(c) = widgets_timeline::configuration(&item.app, &item.kind) else { return };
        for (j, (x, y, w, h)) in self.center_options_layout() {
            let Some(param) = c.params().get(j) else { continue };
            let key = param.key();
            let value = match c.value(key) {
                Some(Value::Choice(_)) => c.choice_label(key).unwrap_or_default().to_string(),
                Some(Value::Flag(on)) => {
                    if on {
                        "On".to_string()
                    } else {
                        "Off".to_string()
                    }
                }
                Some(Value::Number(n)) => n.to_string(),
                None => String::new(),
            };
            let x = x + slide;
            desk_glass(s, ShellSurface::Popover, (x, y, w, h), r, alpha);
            let vt = ui.text(&value, MAC_SUBHEAD_PT, Weight::Semibold, LINK);
            let name = ui.text(&ui.fit(key, MAC_HEADLINE_PT, w - 3.0 * pad - vt.w as f32), MAC_HEADLINE_PT, Weight::Semibold, label);
            s.blit(&name, (x + pad) as i32, (y + (h - name.h as f32) / 2.0) as i32, 255, alpha);
            s.blit(&vt, (x + w - pad - vt.w as f32) as i32, (y + (h - vt.h as f32) / 2.0) as i32, 255, alpha);
        }
    }

    fn draw_desk_gallery(&self, s: &mut Surface, p: f32) {
        use super::super::control_center::desk_glass;
        use super::super::ui::{FIELD_BG, LABEL, LABEL_2, LINK, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, BAR_TEXT, BAR_TEXT_SHADOW, BAR_TEXT_SHADOW_ALPHA};
        let alpha = (p * 255.0) as u32;
        let ui = &self.ui;
        let slide = (1.0 - p) * ui.pt(24.0);
        let (title, done, rows) = self.center_gallery_layout();
        let list = widgets_timeline::gallery(self.notify.now());
        let (label, secondary, field) = (LABEL.now(), LABEL_2.now(), FIELD_BG.now());
        let r = ui.pt(NC_CARD_R_PT);
        let pad = ui.pt(NC_PAD_PT);
        // the title sits on the wallpaper like the column's title: white with a soft shadow
        let heading = if self.notify.center.configuring.is_some() { "Options" } else { "Edit Widgets" };
        let tm = ui.text(heading, MAC_TITLE3_PT, Weight::Bold, WHITE);
        let ty = (title.1 + (title.3 - tm.h as f32) / 2.0) as i32;
        s.blit_colored(&tm, (title.0 + slide) as i32, ty + ui.pt(0.5).max(1.0) as i32, BAR_TEXT_SHADOW, BAR_TEXT_SHADOW_ALPHA * alpha / 255);
        s.blit_colored(&tm, (title.0 + slide) as i32, ty, BAR_TEXT, alpha);
        // Done: a glass capsule with a link-coloured label
        let (dx, dy, dw, dh) = (done.0 + slide, done.1, done.2, done.3);
        desk_glass(s, ShellSurface::Popover, (dx, dy, dw, dh), dh / 2.0, alpha);
        let dl = ui.text("Done", MAC_SUBHEAD_PT, Weight::Semibold, LINK);
        s.blit(&dl, (dx + (dw - dl.w as f32) / 2.0) as i32, (dy + (dh - dl.h as f32) / 2.0) as i32, 255, alpha);
        if self.notify.center.configuring.is_some() {
            return self.draw_option_rows(s, slide, alpha);
        }
        if list.is_empty() {
            let none = ui.text("No widgets yet", MAC_SUBHEAD_PT, Weight::Regular, secondary);
            s.blit(&none, (title.0 + slide) as i32, (title.1 + title.3 + ui.pt(NC_GAP_PT)) as i32, 255, alpha);
        }
        for (i, (x, y, w, h), toggle, small, large, opts) in rows {
            let item = &list[i];
            let x = x + slide;
            desk_glass(s, ShellSurface::Popover, (x, y, w, h), r, alpha);
            // the name, and the search view's description under it, as far as the room between the pad and the controls
            let options_room = if opts.is_some() { 2.0 * ui.pt(NC_CHIP_W_PT) + ui.pt(NC_CHIP_GAP_PT) } else { 0.0 };
            let room = w - 2.0 * pad - ui.pt(NC_DISC_PT) - 3.0 * ui.pt(NC_CHIP_GAP_PT) - 2.0 * ui.pt(NC_CHIP_W_PT) - options_room;
            let name = ui.text(&ui.fit(&item.title, MAC_HEADLINE_PT, room), MAC_HEADLINE_PT, Weight::Semibold, label);
            let desc = item.description.as_ref().map(|d| ui.text(&ui.fit(d, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, Weight::Regular, secondary));
            let block = name.h as f32 + desc.as_ref().map_or(0.0, |d| d.h as f32);
            let mut ly = y + (h - block) / 2.0;
            s.blit(&name, (x + pad) as i32, ly as i32, 255, alpha);
            ly += name.h as f32;
            if let Some(d) = &desc {
                s.blit(d, (x + pad) as i32, ly as i32, 255, alpha);
            }
            // the add (+, link colour) or remove (-, label colour) disc
            let (tx, ty2, tw, th) = (toggle.0 + slide, toggle.1, toggle.2, toggle.3);
            let ink = if item.placed { label } else { LINK };
            s.fill_rrect(tx, ty2, tw, th, tw / 2.0, crate::gfx::scale(field, alpha));
            let (cx, cy) = (tx + tw / 2.0, ty2 + th / 2.0);
            let (arm, bar) = (tw * 0.28, ui.pt(1.5));
            s.fill_rect((cx - arm) as usize, (cy - bar / 2.0) as usize, (2.0 * arm) as usize, bar as usize, crate::gfx::scale(ink, alpha));
            if !item.placed {
                s.fill_rect((cx - bar / 2.0) as usize, (cy - arm) as usize, bar as usize, (2.0 * arm) as usize, crate::gfx::scale(ink, alpha));
            }
            // the Options chip: a plate with a link-coloured label, when the widget has options
            if let Some((ox, oy, ow, oh)) = opts {
                let (ox, oy) = (ox + slide, oy);
                s.fill_rrect(ox, oy, ow, oh, oh / 2.0, crate::gfx::scale(field, alpha));
                let ot = ui.text("Options", MAC_SUBHEAD_PT, Weight::Semibold, LINK);
                s.blit(&ot, (ox + (ow - ot.w as f32) / 2.0) as i32, (oy + (oh - ot.h as f32) / 2.0) as i32, 255, alpha);
            }
            // the size chips: the chosen one has a plate and the label colour
            for (rect, text, selected) in [(small, "Small", !item.large), (large, "Large", item.large)] {
                let (cx0, cy0, cw, ch) = (rect.0 + slide, rect.1, rect.2, rect.3);
                if selected {
                    s.fill_rrect(cx0, cy0, cw, ch, ch / 2.0, crate::gfx::scale(field, alpha));
                }
                let (weight, col) = if selected { (Weight::Semibold, label) } else { (Weight::Regular, secondary) };
                let t = ui.text(text, MAC_SUBHEAD_PT, weight, col);
                s.blit(&t, (cx0 + (cw - t.w as f32) / 2.0) as i32, (cy0 + (ch - t.h as f32) / 2.0) as i32, 255, alpha);
            }
        }
    }
}

#[cfg(test)]
mod edit_capsule_tests {
    #[test]
    fn the_edit_widgets_capsule_follows_the_column_and_stays_above_the_dock() {
        assert_eq!(super::edit_capsule_y(200.0, 12.0, 900.0), 212.0, "just under the last row");
        assert_eq!(super::edit_capsule_y(895.0, 12.0, 900.0), 900.0, "a full column keeps it above the dock");
        assert_eq!(super::edit_capsule_y(0.0, 0.0, 0.0), 0.0);
    }
}

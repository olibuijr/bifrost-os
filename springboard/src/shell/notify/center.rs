//! Notification Center: a swipe down from the left two thirds of the status
//! bar (the right third is Control Center) drags the cover sheet over a
//! frosted copy of the current screen: the clock and date as on the lock
//! screen, a "Notification Center" title and every app's notifications
//! expanded with a Clear button. A tap on a card opens its app, a tap on
//! empty space or a swipe up puts the sheet away; a long list scrolls.

use super::super::control_center::Frost;
use super::super::services::widgets_timeline::{self, Row};
use super::super::gestures::{ease_out_quartic, progress, Drag, SPRING_MS};
use super::super::ui::{centred, Weight, WHITE};
use super::super::{apps, AppId, Effect, Event, Screen, Shell};
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
}

impl Default for Center {
    fn default() -> Self {
        Center { state: State::Hidden, touch: None, revealed: None, backdrop: None, list: None, list_key: (u32::MAX, 0), scroll: Scroller::new(), title: None, empty: None, desk_press: None }
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
                if let Some(app) = c.list.as_ref().and_then(|l| l.groups.get(i)).map(|g| g.app) {
                    self.clear_notifications(app);
                }
            }
            Some(Hit::Mute(i)) => {
                if let Some(app) = c.list.as_ref().and_then(|l| l.groups.get(i)).map(|g| g.app) {
                    self.toggle_mute(app);
                }
            }
            Some(Hit::ClearAll) => self.clear_all_notifications(),
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
    Close(AppId),
}

type R4 = (f32, f32, f32, f32);

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
        for (i, g) in self.notify.store().groups().iter().enumerate() {
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
            let h = ui.pt(NC_WIDGET_PT);
            if y + h > self.h as f32 - ui.pt(NC_BOTTOM_PT) {
                break;
            }
            out.push(((x, y, w, h), row));
            y += h + ui.pt(NC_GAP_PT);
        }
        out
    }

    /// The column's bounds in px, for the presenter's opaque regions.
    pub(crate) fn center_desk_rect(&self) -> Option<R4> {
        if self.notify.center.is_hidden() || !self.desk_active() {
            return None;
        }
        let (t, _, cards) = self.center_desk_layout();
        let bottom = cards
            .last()
            .map(|c| c.1 .1 + c.1 .3 + self.ui.pt(NC_STACK_PT))
            .or_else(|| self.center_widget_rects().last().map(|(r, _)| r.1 + r.3))
            .unwrap_or(t.1 + t.3);
        Some((t.0, t.1, t.2, bottom - t.1))
    }

    fn center_desk_hit(&self, x: f32, y: f32) -> Option<DeskHit> {
        let (_, clear, cards) = self.center_desk_layout();
        if within(clear, x, y) {
            return Some(DeskHit::ClearAll);
        }
        let groups = self.notify.store().groups();
        cards.iter().find(|c| within(c.1, x, y)).map(|&(i, _, close)| if within(close, x, y) { DeskHit::Close(groups[i].app) } else { DeskHit::Card(groups[i].app) })
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
        c.settle(0.0, true, t);
        self.dirty = true;
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
                        DeskHit::Close(app) => self.clear_notifications(app),
                        DeskHit::Card(app) => {
                            self.notify.center.hide();
                            self.open_from_notification(app, t);
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
        use super::super::ui::{FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, BAR_TEXT, BAR_TEXT_SHADOW, BAR_TEXT_SHADOW_ALPHA};
        let c = &self.notify.center;
        let p = c.progress(t).clamp(0.0, 1.0);
        if let Some((x, y, w, h)) = self.center_desk_rect().filter(|_| bifrost_render::glass::gpu()) {
            // the column floats over the live desktop: clear only its own rect (and the slide) for the glass
            let pad = self.ui.pt(32.0);
            s.fill_rect((x - pad).max(0.0) as usize, (y - pad).max(0.0) as usize, (w + 2.0 * pad) as usize, (h + 2.0 * pad) as usize, 0);
        }
        let c = &self.notify.center;
        let alpha = (p * 255.0) as u32;
        let ui = &self.ui;
        let slide = (1.0 - p) * ui.pt(24.0);
        let (title, clear, cards) = self.center_desk_layout();
        let groups = self.notify.store().groups();
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
            desk_glass(s, (x, y, w, h), r, alpha);
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
                desk_glass(s, (x + inset, y + h - r, w - 2.0 * inset, r + ui.pt(NC_STACK_PT)), r, alpha);
            }
            desk_glass(s, (x, y, w, h), r, alpha);
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
    }
}

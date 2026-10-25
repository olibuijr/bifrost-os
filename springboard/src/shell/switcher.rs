//! The app switcher and the go-home swipe. Swiping up from the home
//! indicator shrinks the app under the finger; a flick goes home, a pause
//! lands in the switcher, a short pull springs back. The switcher shows the
//! running apps as cards (their last frame) over a frosted home screen, single
//! horizontal row anchored to the right like the bifrost: tap resumes, a swipe
//! up throws a card away, a drag sideways scrolls to older apps.

use super::control_center::Frost;
use super::accessibility::anim_ms;
use super::gestures::{ease_out_quartic, progress, HomeGesture, HomeOutcome, Velocity, SPRING_MS};
use super::ui::{home_zone, Weight, WHITE};
use super::{Anim, AppId, Event, Screen, Shell, apps, IN_DOCK};
use crate::gfx::{over, rrect_coverage, scale, Image, Surface};
use crate::layout::Rect;

/// Cards are the screen at this scale.
const CARD_SCALE: f32 = 0.42;
const RIGHT_PT: f32 = 48.0;
const TOP_PT: f32 = 135.0;
const COL_GAP_PT: f32 = 40.0;
const ROW_GAP_PT: f32 = 71.0;
/// Label row (icon + name) centre above the card.
const LABEL_PT: f32 = 31.0;
const LABEL_ICON_PT: f32 = 26.0;
const ROWS: usize = 1;
const SLOP_PT: f32 = 8.0;
/// A card thrown up faster than this (px per ms) closes.
const THROW: f32 = 0.9;
/// The lift, as a fraction of the screen, that shrinks the app fully.
const SHRINK_RANGE: f32 = 0.45;
/// A go-home swipe shorter than this (pt) springs back.
const MIN_LIFT_PT: f32 = 40.0;
/// The quarter-size blur radius and brightness of the switcher backdrop.
const BLUR_PT: f32 = 8.0;
const DIM: u32 = 200;

static NEXT_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_gen() -> u64 {
    NEXT_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A running app's last frame at card size with its label.
pub struct Card {
    pub app: AppId,
    /// unique per captured frame: the GPU keeps one texture per generation
    pub gen: u64,
    pub image: Image,
    icon: Image,
    name: Image,
}

#[derive(Clone, Copy, Debug)]
enum State {
    Closed,
    /// cards settle in; card 0 flies from `from` (the gesture card) when set
    Entering { from: Option<Rect>, start: f64 },
    Open,
    Exiting { start: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Tap,
    Scroll,
    Lift,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    card: Option<usize>,
    x0: f32,
    y0: f32,
    x: f32,
    y: f32,
    scroll0: f32,
    vel: Velocity,
    mode: Mode,
    /// landed on the home indicator: releasing dismisses
    zone: bool,
}

impl Touch {
    fn lift(&self) -> f32 {
        (self.y0 - self.y).max(0.0)
    }
}

/// What a released touch asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Resume(usize),
    Close(usize),
    Dismiss,
}

pub struct Switcher {
    cards: Vec<Card>,
    state: State,
    /// px the columns are pushed right to reveal older apps
    scroll: f32,
    touch: Option<Touch>,
    frost: Option<Frost>,
    /// a thrown card: its image, slot and lift when released, and when
    closing: Option<(Card, Rect, f32, f64)>,
    /// bumped whenever `frost` is replaced (a texture key for the GPU path)
    frost_gen: u64,
    /// the status bar and dock drawn on their own for the GPU path, for `frost_gen`
    overlay: Option<(u64, Overlay)>,
}

/// The crisp parts drawn over the frosted backdrop, as images with their screen rects.
struct Overlay {
    bar: (Image, Rect),
    dock: (Image, Rect),
}

impl Default for Switcher {
    fn default() -> Self {
        Switcher { cards: Vec::new(), state: State::Closed, scroll: 0.0, touch: None, frost: None, closing: None, frost_gen: 0, overlay: None }
    }
}

impl Switcher {
    pub fn apps(&self) -> Vec<AppId> {
        self.cards.iter().map(|c| c.app).collect()
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, State::Open)
    }

    pub fn animating(&self) -> bool {
        matches!(self.state, State::Entering { .. } | State::Exiting { .. }) || self.closing.is_some()
    }

    /// Puts a card first, replacing the app's older one.
    fn store(&mut self, card: Card) {
        self.remove(card.app);
        self.cards.insert(0, card);
    }

    fn remove(&mut self, app: AppId) -> Option<Card> {
        let i = self.cards.iter().position(|c| c.app == app)?;
        Some(self.cards.remove(i))
    }

    fn enter(&mut self, frost: Frost, from: Option<Rect>, t: f64) {
        self.frost = Some(frost);
        self.frost_gen = next_gen();
        self.state = State::Entering { from, start: t };
        self.scroll = 0.0;
        self.touch = None;
    }

    /// Leaves without animating (screen off, resume).
    fn close(&mut self) {
        self.state = State::Closed;
        self.touch = None;
        self.closing = None;
    }

    pub fn describe(&self) -> String {
        let names: Vec<String> = self.cards.iter().map(|c| format!("{:?}", c.app).to_lowercase()).collect();
        names.join(",")
    }
}

/// Card size in px for a w x h screen.
fn card_size(w: i32, h: i32) -> (i32, i32) {
    ((w as f32 * CARD_SCALE).round() as i32, (h as f32 * CARD_SCALE).round() as i32)
}

impl Shell {
    /// Where card `i` (0 = most recent) sits at the current scroll.
    fn slot(&self, i: usize) -> Rect {
        let (cw, ch) = card_size(self.w, self.h);
        #[allow(clippy::modulo_one)] // ROWS is a layout constant (1 today); the formula stays general
        let (col, row) = ((i / ROWS) as f32, (i % ROWS) as f32);
        let right = self.w as f32 - self.ui.pt(RIGHT_PT);
        Rect {
            x: (right - (col + 1.0) * cw as f32 - col * self.ui.pt(COL_GAP_PT) + self.switcher.scroll).round() as i32,
            y: (self.ui.pt(TOP_PT).max((self.h - ch) as f32 / 2.0 - self.ui.pt(LABEL_PT) / 2.0) + row * (ch as f32 + self.ui.pt(ROW_GAP_PT))).round() as i32,
            w: cw,
            h: ch,
        }
    }

    /// How far the columns can scroll right before the oldest is in view.
    fn max_scroll(&self) -> f32 {
        let cols = self.switcher.cards.len().div_ceil(ROWS) as f32;
        let (cw, _) = card_size(self.w, self.h);
        let span = cols * cw as f32 + (cols - 1.0).max(0.0) * self.ui.pt(COL_GAP_PT);
        (span - (self.w as f32 - 2.0 * self.ui.pt(RIGHT_PT))).max(0.0)
    }

    fn card_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.switcher.cards.len()).find(|&i| self.slot(i).contains(x, y))
    }

    /// Corner radius for a zooming frame `w` px wide: the icon's at icon
    /// size, none at full screen.
    pub(super) fn zoom_radius(&self, w: f32) -> f32 {
        let (icon, full) = (self.layout.icon as f32, self.w as f32);
        (icon * 0.225 * (full - w) / (full - icon)).max(0.0)
    }

    fn app_icon(&self, app: AppId) -> &Image {
        let i = apps().iter().position(|a| a.id == app).unwrap_or(0);
        if i < IN_DOCK {
            &self.dock[i].icon
        } else {
            &self.grid[i - IN_DOCK].icon
        }
    }

    /// Draws the app's current frame at full size.
    pub(super) fn render_app_frame(&mut self, app: AppId) -> Image {
        self.end_multi();
        let mut img = Image::new(self.w as usize, self.h as usize);
        let t = self.last_input;
        self.draw_app(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, app, t);
        img
    }

    fn make_card(&self, app: AppId, frame: &Image) -> Card {
        let (cw, ch) = card_size(self.w, self.h);
        let icon = self.ui.pt(LABEL_ICON_PT).round() as usize;
        let name = apps().iter().find(|a| a.id == app).map_or("", |a| a.name);
        Card {
            app,
            gen: next_gen(),
            image: frame.resized(cw as usize, ch as usize),
            icon: self.app_icon(app).resized(icon, icon),
            name: self.ui.text(name, 17.0, Weight::Semibold, WHITE),
        }
    }

    /// Captures the app's frame as its switcher card and returns the card
    /// image; the app becomes the most recent.
    pub fn snapshot_app(&mut self, app: AppId) -> &Image {
        let frame = self.render_app_frame(app);
        self.store_card(app, &frame);
        &self.switcher.cards[0].image
    }

    /// Keeps a card for a running app; a dead Terminal loses its card.
    pub(super) fn store_card(&mut self, app: AppId, frame: &Image) {
        if app == AppId::Terminal && self.term.is_none() {
            self.switcher.remove(app);
        } else {
            let card = self.make_card(app, frame);
            self.switcher.store(card);
        }
    }

    /// Stops an app for good: the Terminal's shell dies, Settings forgets its page.
    pub(super) fn close_app(&mut self, app: AppId) {
        match app {
            AppId::Terminal => self.term.clear(),
            AppId::Settings => self.settings = None,
            AppId::Deildu => self.deildu = None,
            AppId::Files => self.files = None,
            AppId::External(n) => self.ext_close(n as usize),
            // an extra window closes through its app's table, the app keeps running (WindowClosed)
            AppId::ExternalWindow(n, id) => {
                if let Some(Some(h)) = self.ext.get_mut(n as usize) {
                    h.close_window_from_shell(u32::from(id));
                }
            }
            AppId::AppStore => self.store = None,
            AppId::Browser => {
                if let Some(mut b) = self.browser.take() {
                    b.flush();
                }
            }
            AppId::Camera => self.camera = None,
            AppId::Magnifier => self.magnifier = None,
            AppId::Photos => self.photos = None,
        }
        self.switcher.remove(app);
    }

    /// Leaves the switcher at once (screen off); the cards stay.
    pub(super) fn switcher_sleep(&mut self) {
        self.switcher.close();
        self.switcher.frost = None;
    }

    /// Lays the open switcher out again after the frame changed size: a
    /// fresh frosted home screen, cards in their resting slots.
    pub(super) fn switcher_resized(&mut self) {
        self.switcher.touch = None;
        self.switcher.closing = None;
        self.switcher.scroll = 0.0;
        if let State::Entering { .. } = self.switcher.state {
            self.switcher.state = State::Open;
        }
        if self.switcher.frost.is_some() {
            self.switcher.frost = Some(self.home_frost());
            self.switcher.frost_gen = next_gen();
        }
    }

    /// Hides the status bar and home indicator over an app (video playback).
    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        self.dirty = true;
    }

    fn home_frost(&mut self) -> Frost {
        let mut img = Image::new(self.w as usize, self.h as usize);
        self.draw_home(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 });
        Frost::new(img, (self.ui.pt(BLUR_PT) / 4.0) as usize, DIM)
    }

    pub(super) fn open_app_switcher(&mut self, t: f64) {
        if self.desk_active() {
            self.switcher_cycle(false);
            return;
        }
        if let Screen::App(app) = self.screen {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        self.anim = None;
        self.transition = None;
        self.home_gesture = None;
        self.cc.hide();
        self.notify_sleep();
        self.quicksearch.hide();
        let frost = self.home_frost();
        self.screen = Screen::Switcher;
        self.switcher.enter(frost, None, t);
        self.dirty = true;
    }

    /// The go-home swipe on an app or the home screen; true when the touch was its.
    pub(super) fn home_gesture_event(&mut self, ev: Event, t: f64) -> bool {
        if self.desk_active() {
            return false;
        }
        let zone = self.h as f32 - home_zone(&self.ui);
        match (ev, self.home_gesture) {
            (Event::Down(x, y), None) if y >= zone => {
                if let Screen::App(app) = self.screen {
                    self.transition = Some(self.render_app_frame(app));
                }
                self.home_gesture = Some(HomeGesture::new(x, y, t));
                true
            }
            (Event::Move(x, y), Some(mut g)) => {
                g.moved(x, y, t);
                self.home_gesture = Some(g);
                true
            }
            (Event::Up(x, y), Some(mut g)) => {
                g.moved(x, y, t);
                self.home_gesture = None;
                self.finish_home_gesture(g, t);
                true
            }
            _ => false,
        }
    }

    /// The system ended the home swipe in progress without a release (the recognizer was cancelled): the app springs
    /// back, nothing else happens. A touch that was never a swipe has no gesture to cancel.
    pub fn cancel_home_gesture(&mut self, t: f64) {
        if let Some(g) = self.home_gesture.take() {
            self.finish_home_gesture_as(g, HomeOutcome::Cancel, t);
        }
    }

    fn finish_home_gesture(&mut self, g: HomeGesture, t: f64) {
        let outcome = g.release(t, self.ui.pt(MIN_LIFT_PT));
        self.finish_home_gesture_as(g, outcome, t);
    }

    fn finish_home_gesture_as(&mut self, g: HomeGesture, outcome: HomeOutcome, t: f64) {
        let rect = self.gesture_rect(&g);
        match (self.screen, outcome) {
            (Screen::App(app), HomeOutcome::Home) => {
                if let Some(frame) = self.transition.take() {
                    self.store_card(app, &frame);
                    self.transition = Some(frame);
                }
                self.anim = Some((Anim::Close { from: rect, rect: self.icon_rect(app) }, t));
                self.screen = Screen::Home;
            }
            (Screen::App(app), HomeOutcome::Switcher) => {
                if let Some(frame) = self.transition.take() {
                    self.store_card(app, &frame);
                    self.transition = Some(frame);
                }
                let frost = self.home_frost();
                self.screen = Screen::Switcher;
                self.switcher.enter(frost, Some(rect), t);
            }
            (Screen::App(app), HomeOutcome::Cancel) => {
                if g.lift() > 0.0 {
                    self.anim = Some((Anim::Open { app, rect }, t));
                } else {
                    self.transition = None;
                }
            }
            (Screen::Home, HomeOutcome::Switcher) if !self.switcher.cards.is_empty() => {
                let frost = self.home_frost();
                self.screen = Screen::Switcher;
                self.switcher.enter(frost, None, t);
            }
            (Screen::Home, HomeOutcome::Home) => self.pager.home(t),
            _ => {}
        }
    }

    /// The shrinking app under the finger: smaller and higher with the lift,
    /// drifting sideways with it.
    pub(super) fn gesture_rect(&self, g: &HomeGesture) -> Rect {
        let p = g.progress(self.h as f32 * SHRINK_RANGE);
        let k = p * (2.0 - p);
        let scale = 1.0 - (1.0 - CARD_SCALE) * k;
        let (w, h) = (self.w as f32 * scale, self.h as f32 * scale);
        let cx = self.w as f32 / 2.0 + g.drift() * 0.6;
        let cy = self.h as f32 / 2.0 - g.lift() * 0.6;
        Rect { x: (cx - w / 2.0).round() as i32, y: (cy - h / 2.0).round() as i32, w: w.round() as i32, h: h.round() as i32 }
    }

    pub(super) fn draw_home_gesture(&mut self, s: &mut Surface) {
        let Some(g) = self.home_gesture else { return };
        let rect = self.gesture_rect(&g);
        if rect.w >= self.w {
            if let Screen::App(app) = self.screen {
                let t = self.last_input;
                self.draw_app(s, app, t);
            }
            return;
        }
        self.draw_home(s);
        let r = self.zoom_radius(rect.w as f32);
        if let Some(img) = &self.transition {
            draw_scaled(s, img, rect, r, 255);
        }
    }

    pub(super) fn switcher_touch(&mut self, ev: Event, t: f64) {
        let slop = self.ui.pt(SLOP_PT);
        match (ev, self.switcher.touch) {
            (Event::Down(x, y), _) if self.switcher.is_open() => {
                let zone = y >= self.h as f32 - home_zone(&self.ui);
                let card = if zone { None } else { self.card_at(x, y) };
                self.switcher.touch = Some(Touch { card, x0: x, y0: y, x, y, scroll0: self.switcher.scroll, vel: Velocity::new(y, t), mode: Mode::Tap, zone });
            }
            (Event::Move(x, y), Some(mut tc)) => {
                tc.x = x;
                tc.y = y;
                tc.vel.push(y, t);
                if tc.mode == Mode::Tap && !tc.zone {
                    if (x - tc.x0).abs() > slop {
                        tc.mode = Mode::Scroll;
                    } else if tc.card.is_some() && tc.y0 - y > slop {
                        tc.mode = Mode::Lift;
                    }
                }
                if tc.mode == Mode::Scroll {
                    self.switcher.scroll = (tc.scroll0 + x - tc.x0).clamp(0.0, self.max_scroll());
                }
                self.switcher.touch = Some(tc);
            }
            (Event::Up(x, y), Some(mut tc)) => {
                tc.x = x;
                tc.y = y;
                tc.vel.push(y, t);
                self.switcher.touch = None;
                let (_, ch) = card_size(self.w, self.h);
                let action = match (tc.mode, tc.card) {
                    (Mode::Tap, Some(i)) => Some(Action::Resume(i)),
                    (Mode::Tap, None) => Some(Action::Dismiss),
                    (Mode::Lift, Some(i)) if tc.vel.at(t) < -THROW || tc.lift() > ch as f32 * 0.4 => Some(Action::Close(i)),
                    _ => None,
                };
                if let Some(a) = action {
                    self.switcher_act(a, t, tc.lift());
                }
            }
            _ => {}
        }
    }

    fn switcher_act(&mut self, action: Action, t: f64, lift: f32) {
        match action {
            Action::Resume(i) => {
                let app = self.switcher.cards[i].app;
                let rect = self.slot(i);
                self.switcher.close();
                self.open(app, t);
                self.anim = Some((Anim::Open { app, rect }, t));
            }
            Action::Close(i) => {
                let slot = self.slot(i);
                let card = self.switcher.cards.remove(i);
                self.close_app(card.app);
                self.switcher.closing = Some((card, slot, lift, t));
                if self.switcher.cards.is_empty() {
                    self.switcher.state = State::Exiting { start: t };
                }
            }
            Action::Dismiss => self.switcher.state = State::Exiting { start: t },
        }
    }

    /// Advances the switcher's own animations.
    pub(super) fn switcher_tick(&mut self, t: f64) {
        match self.switcher.state {
            State::Entering { start, .. } if progress(start, t, anim_ms(SPRING_MS)) >= 1.0 => {
                self.switcher.state = State::Open;
                self.transition = None;
            }
            State::Exiting { start } if progress(start, t, anim_ms(SPRING_MS)) >= 1.0 => {
                self.switcher.close();
                self.switcher.frost = None;
                if self.screen == Screen::Switcher {
                    self.screen = Screen::Home;
                }
            }
            State::Closed if self.screen != Screen::Switcher && self.anim.is_none() => self.switcher.frost = None,
            _ => {}
        }
        if self.switcher.closing.as_ref().is_some_and(|c| progress(c.3, t, anim_ms(SPRING_MS)) >= 1.0) {
            self.switcher.closing = None;
        }
    }

    /// How far the cards have settled (0..1) and the gesture card's rect it flies from.
    fn switcher_phase(&self, t: f64) -> (f32, Option<Rect>) {
        match self.switcher.state {
            State::Entering { from, start } => (ease_out_quartic(progress(start, t, anim_ms(SPRING_MS))), from),
            State::Exiting { start } => (1.0 - ease_out_quartic(progress(start, t, anim_ms(SPRING_MS))), None),
            State::Open | State::Closed => (1.0, None),
        }
    }

    pub(super) fn draw_switcher(&mut self, s: &mut Surface, t: f64) {
        let Some(frost) = &self.switcher.frost else {
            self.draw_home(s);
            return;
        };
        let (p, from) = self.switcher_phase(t);
        let fade = (p * 255.0) as u32;
        if fade < 255 {
            s.copy(frost.sharp.view(), 0, 0);
            s.blit(&frost.frosted, 0, 0, 255, fade);
        } else {
            s.copy(frost.frosted.view(), 0, 0);
        }
        self.draw_bar(s, fade);
        self.draw_dock(s);
        let lifted = self.switcher.touch.filter(|tc| tc.mode == Mode::Lift).and_then(|tc| tc.card.map(|i| (i, tc.lift())));
        for i in 0..self.switcher.cards.len() {
            let mut rect = self.slot(i);
            let mut alpha = fade;
            let mut img = &self.switcher.cards[i].image;
            if i == 0 {
                if let Some(f) = from {
                    rect = lerp_rect(f, rect, p);
                    alpha = 255;
                    img = self.transition.as_ref().unwrap_or(img);
                }
            }
            if let Some((li, lift)) = lifted {
                if li == i {
                    rect.y -= lift.round() as i32;
                }
            }
            let card = &self.switcher.cards[i];
            self.draw_label(s, card, rect, alpha);
            draw_scaled(s, img, rect, self.zoom_radius(rect.w as f32), alpha);
        }
        if let Some((card, slot, lift, start)) = &self.switcher.closing {
            let k = ease_out_quartic(progress(*start, t, anim_ms(SPRING_MS)));
            let rect = Rect { y: slot.y - (lift + (slot.y as f32 + slot.h as f32 - lift) * k).round() as i32, ..*slot };
            let alpha = ((1.0 - k) * 255.0) as u32;
            self.draw_label(s, card, rect, alpha);
            draw_scaled(s, &card.image, rect, self.zoom_radius(rect.w as f32), alpha);
        }
    }

    /// The icon and name above a card.
    fn draw_label(&self, s: &mut Surface, card: &Card, rect: Rect, alpha: u32) {
        let mid = rect.y - self.ui.pt(LABEL_PT) as i32;
        let alpha = label_alpha(alpha, rect.w, card_size(self.w, self.h).0);
        if alpha == 0 {
            return;
        }
        s.blit(&card.icon, rect.x, mid - card.icon.h as i32 / 2, 255, alpha);
        s.blit(&card.name, rect.x + card.icon.w as i32 + self.ui.pt(10.0) as i32, mid - card.name.h as i32 / 2, 255, alpha);
    }

    /// The home screen's dock, crisp over the frosted backdrop.
    fn draw_dock(&self, s: &mut Surface) {
        let l = &self.layout;
        if let Some(bg) = &self.dock_bg {
            s.blit(bg, l.dock.x, l.dock.y, 255, 255);
        }
        for (tile, r) in self.dock.iter().zip(&l.dock_icons) {
            s.blit(&tile.icon, r.x, r.y, 255, 255);
        }
    }
}

/// The label's opacity: it fades out as the card grows to or shrinks from its resting size.
fn label_alpha(alpha: u32, card_w: i32, rest_w: i32) -> u32 {
    let k = card_w as f32 / rest_w as f32;
    (alpha as f32 * (1.0 - (k - 1.0).abs().min(1.0))) as u32
}

fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
    let l = |x: i32, y: i32| (x as f32 + (y as f32 - x as f32) * k).round() as i32;
    Rect { x: l(a.x, b.x), y: l(a.y, b.y), w: l(a.w, b.w), h: l(a.h, b.h) }
}

/// Draws `img` stretched into `r` with rounded corners of radius `radius`,
/// nearest-neighbour sampled so a full-screen frame costs a copy.
pub fn draw_scaled(s: &mut Surface, img: &Image, r: Rect, radius: f32, alpha: u32) {
    if r.w <= 0 || r.h <= 0 || img.w == 0 || img.h == 0 {
        return;
    }
    let (x0, x1) = (r.x.max(0), (r.x + r.w).min(s.w as i32));
    let (y0, y1) = (r.y.max(s.y0 as i32), (r.y + r.h).min(s.h as i32));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let cols: Vec<usize> = (x0..x1).map(|x| ((x - r.x) as usize * img.w / r.w as usize).min(img.w - 1)).collect();
    // Only the corner squares need coverage; everything between them is a
    // plain scaled copy. The quarter mask is cached while the radius holds.
    let rad = radius.min(r.w.min(r.h) as f32 / 2.0).max(0.0);
    let n = rad.ceil() as usize;
    let quarter = corner_mask(rad, n);
    let cover = |lx: i32, ly: i32| -> u32 {
        // lx, ly: pixel offset inside the rect; mirror into the top-left corner
        let mx = if lx >= r.w - n as i32 { r.w - 1 - lx } else { lx };
        let my = if ly >= r.h - n as i32 { r.h - 1 - ly } else { ly };
        if (mx as usize) < n && (my as usize) < n { quarter[my as usize * n + mx as usize] as u32 } else { 255 }
    };
    for y in y0..y1 {
        let sy = ((y - r.y) as usize * img.h / r.h as usize).min(img.h - 1);
        let row = &img.px[sy * img.w..][..img.w];
        let ly = y - r.y;
        let corner_row = n > 0 && (ly < n as i32 || ly >= r.h - n as i32);
        let drow = &mut s.px[(y as usize - s.y0) * s.stride..];
        for (i, x) in (x0..x1).enumerate() {
            let mut p = row[cols[i]];
            let lx = x - r.x;
            if corner_row && (lx < n as i32 || lx >= r.w - n as i32) {
                let c = cover(lx, ly);
                if c == 0 {
                    continue;
                }
                if c < 255 {
                    p = scale(p, c);
                }
            }
            if alpha != 255 {
                p = scale(p, alpha);
            }
            let d = &mut drow[x as usize];
            *d = if p >> 24 == 255 { p } else { over(*d, p) };
        }
    }
}

/// Coverage of one `n` x `n` rounded corner of radius `rad`, cached for the
/// last radius used (a zoom changes it every frame; a held card does not).
fn corner_mask(rad: f32, n: usize) -> std::rc::Rc<Vec<u8>> {
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(u32, std::rc::Rc<Vec<u8>>)>> = const { std::cell::RefCell::new(None) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((k, m)) = &*c {
            if *k == rad.to_bits() {
                return m.clone();
            }
        }
        let side = 2.0 * rad;
        let mut m = vec![0u8; n * n];
        for y in 0..n {
            for x in 0..n {
                m[y * n + x] = (rrect_coverage(x as f32 + 0.5, y as f32 + 0.5, side, side, rad) * 255.0) as u8;
            }
        }
        let m = std::rc::Rc::new(m);
        *c = Some((rad.to_bits(), m.clone()));
        m
    })
}

/// Identity of one image the GPU switcher keeps as a texture: uploaded once per generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexKey {
    FrostSharp(u64),
    FrostBlur(u64),
    Bar(u64),
    Dock(u64),
    Card(u64),
    Icon(u64),
    Name(u64),
    /// the gesture card flying into slot 0 (per frost generation)
    Gesture(u64),
    /// the app's frame zooming open over the switcher (per animation start, in f64 bits)
    Zoom(u64),
}

/// One textured quad: the whole `image` drawn into `rect` with rounded corners and opacity.
#[derive(Clone, Copy)]
pub struct Quad<'a> {
    pub key: TexKey,
    pub image: &'a Image,
    pub rect: Rect,
    pub radius: f32,
    pub alpha: f32,
}

/// One frame of the app switcher for the GPU compositor: the quads in draw order
/// (backdrop, status bar, dock, then each card's label and card, a thrown card, the zoom).
pub struct SwitcherView<'a> {
    pub layers: Vec<Quad<'a>>,
}

impl SwitcherView<'_> {
    /// The textures the frame needs, each once, in draw order.
    pub fn keys(&self) -> Vec<TexKey> {
        let mut out: Vec<TexKey> = Vec::with_capacity(self.layers.len());
        for q in &self.layers {
            if !out.contains(&q.key) {
                out.push(q.key);
            }
        }
        out
    }

    /// The image behind a texture key of this frame.
    pub fn image(&self, key: TexKey) -> Option<&Image> {
        self.layers.iter().find(|q| q.key == key).map(|q| q.image)
    }
}

impl Shell {
    /// Why the GPU switcher cannot draw right now (None when it can): the same overlays as
    /// the zoom, plus the performance overlay, all painted by the CPU over the frame.
    pub fn switcher_blocker(&self) -> Option<&'static str> {
        if self.screen != Screen::Switcher {
            return Some("not-switcher");
        }
        if self.perf.enabled {
            return Some("perf-overlay");
        }
        if self.home_gesture.is_some() {
            return Some("home-gesture");
        }
        if self.switcher.frost.is_none() {
            return Some("no-frost");
        }
        if !matches!(self.anim, None | Some((Anim::Open { .. }, _))) {
            return Some("anim");
        }
        self.overlay_blocker()
    }

    /// Renders the status bar and dock on their own once per backdrop (the GPU draws them
    /// over it); a no-op while the cached pair is current.
    pub fn switcher_prepare(&mut self) {
        let g = self.switcher.frost_gen;
        if self.switcher.frost.is_none() || matches!(&self.switcher.overlay, Some((og, _)) if *og == g) {
            return;
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let mut full = Image::new(w, h);
        self.draw_bar(&mut Surface { w, h, stride: w, px: &mut full.px, y0: 0 }, 255);
        let bar_r = Rect { x: 0, y: 0, w: self.w, h: self.layout.status_h.clamp(1, self.h) };
        let bar = crop(&full, bar_r);
        full.px.fill(0);
        self.draw_dock(&mut Surface { w, h, stride: w, px: &mut full.px, y0: 0 });
        let l = &self.layout;
        let (mut x0, mut y0, mut x1, mut y1) = (l.dock.x, l.dock.y, l.dock.x + l.dock.w, l.dock.y + l.dock.h);
        for r in &l.dock_icons {
            (x0, y0, x1, y1) = (x0.min(r.x), y0.min(r.y), x1.max(r.x + r.w), y1.max(r.y + r.h));
        }
        let dock_r = Rect { x: x0.max(0), y: y0.max(0), w: (x1.min(self.w) - x0.max(0)).max(1), h: (y1.min(self.h) - y0.max(0)).max(1) };
        let dock = crop(&full, dock_r);
        self.switcher.overlay = Some((g, Overlay { bar: (bar, bar_r), dock: (dock, dock_r) }));
    }

    /// The GPU drew the frame `switcher_view` described: the bookkeeping `draw` does after `draw_switcher`.
    pub fn switcher_drawn(&mut self) {
        self.dirty = false;
        self.drawn = super::Drawn::App;
    }

    /// The switcher as quads for the GPU (None: draw it on the CPU). Pure data, the same
    /// rects, radii and fades as `draw_switcher`. Call `switcher_prepare` first.
    pub fn switcher_view(&self, t: f64) -> Option<SwitcherView<'_>> {
        if self.switcher_blocker().is_some() {
            return None;
        }
        let frost = self.switcher.frost.as_ref()?;
        let g = self.switcher.frost_gen;
        let overlay = match &self.switcher.overlay {
            Some((og, o)) if *og == g => o,
            _ => return None,
        };
        let (p, from) = self.switcher_phase(t);
        let fade = (p * 255.0) as u32;
        let a = |v: u32| v as f32 / 255.0;
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        let mut layers = Vec::with_capacity(8 + 2 * self.switcher.cards.len());
        if fade < 255 {
            layers.push(Quad { key: TexKey::FrostSharp(g), image: &frost.sharp, rect: full, radius: 0.0, alpha: 1.0 });
            layers.push(Quad { key: TexKey::FrostBlur(g), image: &frost.frosted, rect: full, radius: 0.0, alpha: a(fade) });
        } else {
            layers.push(Quad { key: TexKey::FrostBlur(g), image: &frost.frosted, rect: full, radius: 0.0, alpha: 1.0 });
        }
        layers.push(Quad { key: TexKey::Bar(g), image: &overlay.bar.0, rect: overlay.bar.1, radius: 0.0, alpha: a(fade) });
        layers.push(Quad { key: TexKey::Dock(g), image: &overlay.dock.0, rect: overlay.dock.1, radius: 0.0, alpha: 1.0 });
        let lifted = self.switcher.touch.filter(|tc| tc.mode == Mode::Lift).and_then(|tc| tc.card.map(|i| (i, tc.lift())));
        for (i, card) in self.switcher.cards.iter().enumerate() {
            let mut rect = self.slot(i);
            let mut alpha = fade;
            let mut img = (TexKey::Card(card.gen), &card.image);
            if i == 0 {
                if let (Some(f), Some(tr)) = (from, self.transition.as_ref()) {
                    rect = lerp_rect(f, rect, p);
                    alpha = 255;
                    img = (TexKey::Gesture(g), tr);
                } else if let Some(f) = from {
                    rect = lerp_rect(f, rect, p);
                    alpha = 255;
                }
            }
            if let Some((li, lift)) = lifted {
                if li == i {
                    rect.y -= lift.round() as i32;
                }
            }
            self.card_quads(&mut layers, card, img, rect, alpha);
        }
        if let Some((card, slot, lift, start)) = &self.switcher.closing {
            let k = ease_out_quartic(progress(*start, t, anim_ms(SPRING_MS)));
            let rect = Rect { y: slot.y - (lift + (slot.y as f32 + slot.h as f32 - lift) * k).round() as i32, ..*slot };
            let alpha = ((1.0 - k) * 255.0) as u32;
            self.card_quads(&mut layers, card, (TexKey::Card(card.gen), &card.image), rect, alpha);
        }
        if let Some((Anim::Open { rect, .. }, start)) = self.anim {
            let ease = ease_out_quartic(self.progress(t));
            let r = self.zoom_rect(rect, full, ease);
            if Self::zoom_alpha(true, ease) != 255 {
                return None;
            }
            if let Some(tr) = self.transition.as_ref() {
                layers.push(Quad { key: TexKey::Zoom(start.to_bits()), image: tr, rect: r, radius: self.zoom_radius(r.w as f32), alpha: 1.0 });
            }
        }
        Some(SwitcherView { layers })
    }

    /// The label (icon, name) and the card itself for one slot, in `draw_switcher`'s order.
    fn card_quads<'a>(&self, out: &mut Vec<Quad<'a>>, card: &'a Card, img: (TexKey, &'a Image), rect: Rect, alpha: u32) {
        let label = label_alpha(alpha, rect.w, card_size(self.w, self.h).0);
        if label > 0 {
            let mid = rect.y - self.ui.pt(LABEL_PT) as i32;
            let ir = Rect { x: rect.x, y: mid - card.icon.h as i32 / 2, w: card.icon.w as i32, h: card.icon.h as i32 };
            let nr = Rect { x: rect.x + card.icon.w as i32 + self.ui.pt(10.0) as i32, y: mid - card.name.h as i32 / 2, w: card.name.w as i32, h: card.name.h as i32 };
            out.push(Quad { key: TexKey::Icon(card.gen), image: &card.icon, rect: ir, radius: 0.0, alpha: label as f32 / 255.0 });
            out.push(Quad { key: TexKey::Name(card.gen), image: &card.name, rect: nr, radius: 0.0, alpha: label as f32 / 255.0 });
        }
        out.push(Quad { key: img.0, image: img.1, rect, radius: self.zoom_radius(rect.w as f32), alpha: alpha as f32 / 255.0 });
    }
}

fn crop(img: &Image, r: Rect) -> Image {
    let (x, y, w, h) = (r.x as usize, r.y as usize, r.w as usize, r.h as usize);
    let mut out = Image::new(w, h);
    for row in 0..h {
        out.px[row * w..][..w].copy_from_slice(&img.px[(y + row) * img.w + x..][..w]);
    }
    out
}

/// The Cmd+Tab strip's entries: one per app, most recently used first. `front` is an app whose window is in front of
/// every desk window (a native client's active toplevel); `windows` are the desk's windows front first, then the apps
/// that show native toplevels. An extra window stands for its app, so an app with two windows in front is one entry
/// and the next one is the app used before it (E2E11 4: the second entry was the front app's other window, and Cmd+Tab
/// did nothing).
pub(super) fn app_switch_order(front: Option<AppId>, windows: impl IntoIterator<Item = AppId>) -> Vec<AppId> {
    let mut order: Vec<AppId> = Vec::new();
    for app in front.into_iter().chain(windows).map(AppId::parent) {
        if !order.contains(&app) {
            order.push(app);
        }
    }
    order
}

/// The installed app (`External(slot)`) a native client's toplevel belongs to, by its Wayland app id or package id.
#[cfg(feature = "desktop-wayland")]
fn native_window_app(installed: &[super::apphost::Installed], w: &crate::bare::wayland::ForeignWindow) -> Option<AppId> {
    if w.app_id.is_empty() {
        return None;
    }
    let slot = installed.iter().position(|a| a.manifest.wayland_app_id == w.app_id || a.manifest.id == w.app_id)?;
    u8::try_from(slot).ok().map(AppId::External)
}

/// The desktop's app switcher: Cmd+Tab walks the running apps most recently used first (Shift reverses), showing the
/// icon strip drawn by `draw_cmdtab`; releasing the modifier activates the selection with all of its windows
/// (`cmdtab_release`, `desk_open`). Cmd+` walks the windows of the front app. Esc cancels, the arrows move the selection.
impl Shell {
    /// Opens the strip on the previously used app, or moves the selection one step.
    pub(super) fn switcher_cycle(&mut self, reverse: bool) {
        // a running app with no window is still an app to switch to (E2E13 1): the active one leads, the others follow
        // the apps with windows; releasing on one reopens its window (`cmdtab_release`, `desk_open`, `ToApp::Reopen`)
        let windowless_front = self.desk_windowless_front();
        let windowless = self.ext_windowless_apps();
        if let Some(d) = &mut self.desk {
            if d.cmdtab.is_none() {
                #[cfg(feature = "desktop-wayland")]
                let order = {
                    let installed = super::apphost::installed();
                    let front = windowless_front.or_else(|| d.native_windows.iter().find(|w| w.active).and_then(|w| native_window_app(&installed, w)));
                    let native: Vec<AppId> = d.native_windows.iter().filter_map(|w| native_window_app(&installed, w)).collect();
                    app_switch_order(front, d.wins.iter().rev().map(|w| w.app).chain(native).chain(windowless))
                };
                #[cfg(not(feature = "desktop-wayland"))]
                let order = app_switch_order(windowless_front, d.wins.iter().rev().map(|w| w.app).chain(windowless));
                if order.len() >= 2 {
                    let sel = if reverse { order.len() - 1 } else { 1 };
                    d.cmdtab = Some((order, sel));
                }
            } else if let Some((order, sel)) = &mut d.cmdtab {
                let n = order.len();
                *sel = if reverse { (*sel + n - 1) % n } else { (*sel + 1) % n };
            }
        }
        self.dirty = true;
    }

    /// Whether the window in front is a native client's (a window the window server owns), so Cmd+` cycles by
    /// activating its app's other native windows (`switcher_same_app`) instead of the desk's windows.
    pub(super) fn foreign_cycle_owned(&self) -> bool {
        #[cfg(feature = "desktop-wayland")]
        { self.desk.as_ref().is_some_and(|d| d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a| a.windows.iter())).any(|w| w.active)) }
        #[cfg(not(feature = "desktop-wayland"))]
        { false }
    }

    /// Cmd+` (Shift: backwards) in front of a native client: its app's next native window; the compositor raises the
    /// one named.
    pub(super) fn switcher_same_app(&mut self, reverse: bool) -> Vec<super::Effect> {
        #[cfg(feature = "desktop-wayland")]
        {
            let Some(d) = self.desk.as_ref() else { return vec![] };
            let all = || d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a| a.windows.iter()));
            let Some(active) = all().find(|w| w.active) else { return vec![] };
            let same: Vec<_> = all().filter(|w| w.app_id == active.app_id).collect();
            if same.len() < 2 {
                return vec![];
            }
            let pos = same.iter().position(|w| w.active).unwrap_or(0);
            let next = if reverse { (pos + same.len() - 1) % same.len() } else { (pos + 1) % same.len() };
            self.dirty = true;
            return vec![super::Effect::ActivateForeign(same[next].id)];
        }
        #[cfg(not(feature = "desktop-wayland"))]
        {
            let _ = reverse;
            vec![]
        }
    }

    /// Hardware keys for the switcher; `None` when the key is not its. Call with key-down events and the
    /// modifier state; the release of the last modifier is handled by `cmdtab_release`.
    pub fn switcher_key(&mut self, code: u16, mods: super::overview::KeyMods) -> Option<Vec<super::Effect>> {
        use super::overview::{KEY_ESC, KEY_GRAVE, KEY_LEFT, KEY_RIGHT, KEY_TAB};
        if !self.desk_active() || self.is_locked() {
            return None;
        }
        let open = self.desk.as_ref().is_some_and(|d| d.cmdtab.is_some());
        match code {
            KEY_TAB if mods.cmd => {
                self.switcher_cycle(mods.shift);
                Some(vec![])
            }
            KEY_GRAVE if mods.cmd => Some(self.switcher_same_app(mods.shift)),
            KEY_ESC if open => {
                if let Some(d) = &mut self.desk {
                    d.cmdtab = None;
                }
                self.dirty = true;
                Some(vec![])
            }
            KEY_LEFT | KEY_RIGHT if open => {
                self.switcher_cycle(code == KEY_LEFT);
                Some(vec![])
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{settle, shell, swipe, unlock};
    use super::super::{AppId, Event, Screen, Shell};
    use super::*;
    use crate::gfx::pack;

    fn frame(sh: &mut Shell, t: f64) -> Vec<u32> {
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        buf
    }

    /// Lifts the finger from the home indicator to `y1` over `ms`, then
    /// rests `pause` ms before releasing.
    fn lift(sh: &mut Shell, y1: f32, ms: f64, pause: f64, t: f64) {
        sh.event(Event::Down(600.0, 1915.0), t);
        for i in 1..=10 {
            sh.event(Event::Move(600.0, 1915.0 + (y1 - 1915.0) * i as f32 / 10.0), t + ms * i as f64 / 10.0);
        }
        sh.event(Event::Up(600.0, y1), t + ms + pause);
    }

    /// E2E11 4: the strip lists apps, most recent first, each once; a native client's window in front leads it.
    #[test]
    fn the_app_switcher_lists_each_app_once_most_recent_first() {
        let (te, te2, files) = (AppId::External(3), AppId::ExternalWindow(3, 1), AppId::External(5));
        assert_eq!(app_switch_order(None, [te2, te, AppId::Settings, te]), vec![te, AppId::Settings], "an extra window stands for its app");
        assert_eq!(app_switch_order(Some(files), [te, AppId::Settings, files]), vec![files, te, AppId::Settings], "the active native window's app first");
        assert_eq!(app_switch_order(None, [te2, te]), vec![te], "one app: nothing to switch to");
    }

    fn open_app(sh: &mut Shell, app: AppId, t: f64) {
        sh.open(app, t);
        settle(sh, t);
        assert_eq!(sh.screen, Screen::App(app));
    }

    #[test]
    fn slots_form_one_row_from_the_right_like_the_bifrost() {
        let sh = shell();
        let (cw, ch) = card_size(1200, 1920);
        let s0 = sh.slot(0);
        assert_eq!(s0.x + s0.w, 1200 - sh.ui.pt(48.0).round() as i32);
        assert!(s0.y > sh.ui.pt(100.0).round() as i32 && s0.y + ch < 1920);
        let s1 = sh.slot(1);
        assert_eq!(s1.y, s0.y);
        assert!((s1.x + s1.w + sh.ui.pt(40.0).round() as i32 - s0.x).abs() <= 1, "next card to the left");
        assert!((sh.zoom_radius(1200.0)).abs() < 0.01);
        assert!((sh.zoom_radius(100.0) - 22.5).abs() < 0.01);
        assert!(sh.zoom_radius(cw as f32) > 10.0 && sh.zoom_radius(cw as f32) < 20.0);
    }

    #[test]
    fn going_home_keeps_a_card_and_snapshot_app_is_most_recent() {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Files, 1000.0);
        sh.go_home(2000.0);
        settle(&mut sh, 2000.0);
        assert_eq!(sh.switcher.apps(), vec![AppId::Files]);
        open_app(&mut sh, AppId::Settings, 3000.0);
        sh.go_home(4000.0);
        settle(&mut sh, 4000.0);
        assert_eq!(sh.switcher.apps(), vec![AppId::Settings, AppId::Files]);
        let card = sh.snapshot_app(AppId::Files);
        assert_eq!((card.w, card.h), (504, 806));
        let mid = card.px[card.w * card.h / 2 + card.w / 2];
        assert!(mid >> 24 == 255, "the card is opaque: {mid:08x}");
        assert_eq!(sh.switcher.apps(), vec![AppId::Files, AppId::Settings]);
        assert!(sh.describe().contains("cards=files,settings"));
    }

    #[test]
    fn app_follows_the_finger_and_a_short_pull_springs_back() {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Files, 1000.0);
        sh.event(Event::Down(600.0, 1915.0), 2000.0);
        sh.event(Event::Move(650.0, 1600.0), 2100.0);
        let g = sh.home_gesture.unwrap();
        let r = sh.gesture_rect(&g);
        assert!(r.w < 1200 && r.w > 348, "shrinking: {r:?}");
        assert!(r.x + r.w / 2 > 600, "drifts with the finger");
        assert!(r.y + r.h / 2 < 960, "rises with the finger");
        let buf = frame(&mut sh, 2100.0);
        assert_ne!(buf[10 * 1200 + 10], pack(0, 0, 0, 255), "home shows around the card");
        let app = sh.render_app_frame(AppId::Files);
        let app_px = app.px[(app.h / 4) * app.w + app.w / 4];
        assert_eq!(buf[(r.y + r.h / 4) as usize * 1200 + (r.x + r.w / 4) as usize], app_px, "the card is the app");
        sh.event(Event::Up(650.0, 1890.0), 2400.0);
        assert!(sh.home_gesture.is_none());
        assert!(matches!(sh.anim, Some((Anim::Open { app: AppId::Files, .. }, _))), "springs back");
        settle(&mut sh, 2400.0);
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        assert!(sh.switcher.apps().is_empty());
    }

    #[test]
    fn flick_goes_home_from_the_card_position() {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Files, 1000.0);
        lift(&mut sh, 1700.0, 60.0, 0.0, 2000.0);
        match sh.anim {
            Some((Anim::Close { from, rect }, _)) => {
                assert!(from.w < 1200 && from.w > 348);
                assert_eq!(rect, sh.icon_rect(AppId::Files));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(sh.screen, Screen::Home);
        frame(&mut sh, 2100.0);
        settle(&mut sh, 2100.0);
        assert_eq!(sh.switcher.apps(), vec![AppId::Files]);
        assert!(sh.transition.is_none(), "snapshot released after the zoom");
    }

    #[test]
    fn lift_and_pause_opens_the_switcher_and_tap_resumes() {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Settings, 500.0);
        sh.go_home(600.0);
        settle(&mut sh, 600.0);
        open_app(&mut sh, AppId::Files, 1000.0);
        lift(&mut sh, 1200.0, 300.0, 300.0, 2000.0);
        assert_eq!(sh.screen, Screen::Switcher);
        assert!(sh.needs_frame());
        frame(&mut sh, 2700.0);
        settle(&mut sh, 2700.0);
        assert!(sh.switcher.is_open());
        assert_eq!(sh.switcher.apps(), vec![AppId::Files, AppId::Settings]);
        assert!(sh.describe().contains("screen=switcher"));
        let s1 = sh.slot(1);
        let (x, y) = (s1.x as f32 + 20.0, s1.y as f32 + 20.0);
        sh.event(Event::Down(x, y), 5000.0);
        sh.event(Event::Up(x, y), 5030.0);
        assert!(matches!(sh.anim, Some((Anim::Open { app: AppId::Settings, rect }, _)) if rect == s1));
        frame(&mut sh, 5100.0);
        settle(&mut sh, 5100.0);
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        assert!(!sh.switcher.is_open());
    }

    #[test]
    fn throwing_a_card_closes_the_app_and_tapping_outside_goes_home() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.term_cmd = &["/bin/sh"];
        open_app(&mut sh, AppId::Terminal, 500.0);
        sh.go_home(600.0);
        settle(&mut sh, 600.0);
        open_app(&mut sh, AppId::Settings, 1000.0);
        lift(&mut sh, 1200.0, 300.0, 300.0, 2000.0);
        settle(&mut sh, 2700.0);
        assert_eq!(sh.switcher.apps(), vec![AppId::Settings, AppId::Terminal]);
        assert_eq!(sh.app_poll_fds().len(), 1);
        let s1 = sh.slot(1);
        let (x, y) = (s1.x as f32 + 30.0, s1.y as f32 + 300.0);
        swipe(&mut sh, x, y, y - 400.0, 5000.0);
        assert_eq!(sh.switcher.apps(), vec![AppId::Settings]);
        assert!(sh.term.is_none() && sh.app_poll_fds().is_empty(), "the shell is dropped");
        assert!(sh.switcher.animating(), "the card flies off");
        frame(&mut sh, 5200.0);
        settle(&mut sh, 5200.0);
        // a short lift lets go of the card
        let s0 = sh.slot(0);
        let (x, y) = (s0.x as f32 + 30.0, s0.y as f32 + 300.0);
        sh.event(Event::Down(x, y), 7000.0);
        sh.event(Event::Move(x, y - 40.0), 7200.0);
        sh.event(Event::Up(x, y - 40.0), 7500.0);
        assert_eq!(sh.switcher.apps(), vec![AppId::Settings]);
        assert!(sh.settings.is_some());
        sh.event(Event::Down(100.0, 1700.0), 8000.0);
        sh.event(Event::Up(100.0, 1700.0), 8030.0);
        assert!(sh.needs_frame());
        frame(&mut sh, 8100.0);
        settle(&mut sh, 8100.0);
        assert_eq!(sh.screen, Screen::Home);
        assert_eq!(sh.switcher.apps(), vec![AppId::Settings], "cards survive dismissal");
        // closing the last card leaves the switcher
        lift(&mut sh, 1200.0, 300.0, 300.0, 9000.0);
        settle(&mut sh, 9700.0);
        assert_eq!(sh.screen, Screen::Switcher, "the home screen opens the switcher too");
        let s0 = sh.slot(0);
        swipe(&mut sh, s0.x as f32 + 30.0, s0.y as f32 + 300.0, s0.y as f32 - 100.0, 10000.0);
        settle(&mut sh, 10200.0);
        assert_eq!(sh.screen, Screen::Home);
        assert!(sh.settings.is_none());
        lift(&mut sh, 1200.0, 300.0, 300.0, 11000.0);
        assert_eq!(sh.screen, Screen::Home, "nothing to switch to");
    }

    #[test]
    fn sideways_drag_scrolls_only_when_there_are_more_columns() {
        let mut sh = shell();
        unlock(&mut sh);
        for (i, app) in [AppId::Files, AppId::Settings].into_iter().enumerate() {
            open_app(&mut sh, app, 1000.0 * i as f64);
            sh.go_home(1000.0 * i as f64 + 500.0);
            settle(&mut sh, 1000.0 * i as f64 + 500.0);
        }
        lift(&mut sh, 1200.0, 300.0, 300.0, 5000.0);
        settle(&mut sh, 5700.0);
        let s0 = sh.slot(0);
        let (x, y) = (s0.x as f32 + 30.0, s0.y as f32 + 100.0);
        sh.event(Event::Down(x, y), 6000.0);
        sh.event(Event::Move(x + 200.0, y), 6050.0);
        assert_eq!(sh.switcher.scroll, sh.max_scroll(), "two 0.42 cards overflow a little, so the drag clamps to the end");
        sh.event(Event::Up(x + 200.0, y), 6100.0);
        assert!(sh.switcher.is_open(), "a scroll is not a tap");
        assert!(sh.max_scroll() > 0.0 && sh.max_scroll() < sh.ui.pt(40.0));
    }

    #[test]
    fn fullscreen_hides_the_status_bar() {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Terminal, 1000.0);
        let top = |buf: &[u32]| buf[..1200 * 40].to_vec();
        let with = top(&frame(&mut sh, 2000.0));
        sh.set_fullscreen(true);
        assert!(sh.needs_frame());
        let without = top(&frame(&mut sh, 2100.0));
        assert_ne!(with, without, "the bar is gone");
        sh.set_fullscreen(false);
        assert_eq!(top(&frame(&mut sh, 2200.0)), with);
    }

    fn open_switcher_shell() -> Shell {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Settings, 500.0);
        sh.go_home(600.0);
        settle(&mut sh, 600.0);
        open_app(&mut sh, AppId::Files, 1000.0);
        lift(&mut sh, 1200.0, 300.0, 300.0, 2000.0);
        frame(&mut sh, 2700.0);
        settle(&mut sh, 2700.0);
        sh
    }

    #[test]
    fn switcher_view_rects_match_the_cpu_slots() {
        let mut sh = open_switcher_shell();
        assert!(sh.switcher_view(5000.0).is_none(), "no overlay prepared yet");
        sh.switcher_prepare();
        let slots: Vec<Rect> = (0..sh.switcher.cards.len()).map(|i| sh.slot(i)).collect();
        let gens: Vec<u64> = sh.switcher.cards.iter().map(|c| c.gen).collect();
        let v = sh.switcher_view(5000.0).expect("plain open switcher is drawable");
        for (i, g) in gens.iter().enumerate() {
            let q = v.layers.iter().find(|q| q.key == TexKey::Card(*g)).expect("card quad");
            assert_eq!(q.rect, slots[i]);
            assert_eq!(q.alpha, 1.0);
            assert_eq!(q.radius, sh.zoom_radius(slots[i].w as f32));
            assert!(v.layers.iter().any(|q| q.key == TexKey::Icon(*g) && q.alpha == 1.0), "label drawn at rest");
        }
        assert!(matches!(v.layers[0].key, TexKey::FrostBlur(_)) && v.layers[0].alpha == 1.0, "backdrop first, no sharp layer at rest");
        assert!(!v.layers.iter().any(|q| matches!(q.key, TexKey::FrostSharp(_))));
        let keys = v.keys();
        assert_eq!(keys.len(), v.layers.len(), "every texture is distinct");
    }

    #[test]
    fn switcher_view_follows_scroll_and_a_lifted_card() {
        let mut sh = open_switcher_shell();
        sh.switcher_prepare();
        sh.switcher.scroll = 10.0;
        let s0 = sh.slot(0);
        let g0 = sh.switcher.cards[0].gen;
        let v = sh.switcher_view(5000.0).unwrap();
        assert_eq!(v.layers.iter().find(|q| q.key == TexKey::Card(g0)).unwrap().rect, s0);
        sh.event(Event::Down(s0.x as f32 + 30.0, s0.y as f32 + 300.0), 6000.0);
        sh.event(Event::Move(s0.x as f32 + 30.0, s0.y as f32 + 200.0), 6050.0);
        let v = sh.switcher_view(6060.0).unwrap();
        assert_eq!(v.layers.iter().find(|q| q.key == TexKey::Card(g0)).unwrap().rect.y, s0.y - 100);
    }

    #[test]
    fn switcher_view_fades_in_over_the_sharp_backdrop_and_is_off_for_overlays() {
        let mut sh = shell();
        unlock(&mut sh);
        open_app(&mut sh, AppId::Files, 1000.0);
        lift(&mut sh, 1200.0, 300.0, 300.0, 2000.0);
        sh.switcher_prepare();
        let v = sh.switcher_view(2605.0).expect("entering");
        assert!(matches!(v.layers[0].key, TexKey::FrostSharp(_)));
        assert!(matches!(v.layers[1].key, TexKey::FrostBlur(_)) && v.layers[1].alpha < 1.0);
        drop(v);
        sh.perf.enabled = true;
        assert_eq!(sh.switcher_blocker(), Some("perf-overlay"));
        assert!(sh.switcher_view(2605.0).is_none());
    }

    #[test]
    fn card_generations_change_with_each_capture_and_frost() {
        let mut sh = open_switcher_shell();
        let before = sh.switcher.cards[0].gen;
        let frost = sh.switcher.frost_gen;
        let frame = sh.render_app_frame(AppId::Files);
        sh.store_card(AppId::Files, &frame);
        assert_ne!(sh.switcher.cards[0].gen, before);
        sh.switcher_resized();
        assert_ne!(sh.switcher.frost_gen, frost);
    }

    #[test]
    fn label_fades_away_from_the_resting_size() {
        assert_eq!(label_alpha(255, 100, 100), 255);
        assert_eq!(label_alpha(255, 200, 100), 0);
        assert_eq!(label_alpha(200, 150, 100), 100);
    }

    #[test]
    fn scaled_draw_rounds_corners_and_fills_the_middle() {
        let img = Image { w: 4, h: 4, px: vec![pack(255, 255, 255, 255); 16] };
        let mut buf = vec![0u32; 40 * 40];
        fn surface(buf: &mut [u32]) -> Surface<'_> {
            Surface { w: 40, h: 40, stride: 40, px: buf, y0: 0 }
        }
        draw_scaled(&mut surface(&mut buf), &img, Rect { x: 5, y: 5, w: 30, h: 30 }, 8.0, 255);
        assert_eq!(buf[20 * 40 + 20], pack(255, 255, 255, 255));
        assert_eq!(buf[5 * 40 + 5], 0, "corner clipped");
        assert_eq!(buf[0], 0);
        draw_scaled(&mut surface(&mut buf), &img, Rect { x: -10, y: -10, w: 100, h: 100 }, 0.0, 128);
        assert_eq!(buf[0], pack(128, 128, 128, 128), "clipped, scaled by alpha");
        assert_eq!(buf[20 * 40 + 20], pack(255, 255, 255, 255));
    }
}

//! Split View and Slide Over: a second app beside the open one (50/50, 70/30
//! or 30/70, resized by dragging an 8pt divider) or floating over it in a
//! narrow panel (about 360pt) that is dragged left/right and swiped away.
//!
//! This file holds the pure state and geometry (pane rects, divider and panel
//! drags, the touch-to-pane transform) and the small pieces of chrome: the
//! three-dot button at the top of an app, its menu and the divider. The shell
//! lays each app out at its pane's size, draws it into an offscreen image and
//! blits that image into the pane.

use super::ui::{Ui, Weight, WHITE};
use super::{AppId, apps};
use crate::gfx::{pack, rgba, Surface};

pub const DIVIDER_PT: f32 = 8.0;
pub const SLIDE_PT: f32 = 360.0;
const SLIDE_MARGIN_PT: f32 = 8.0;
/// The divider's touch target, each side of its centre.
const GRAB_PT: f32 = 16.0;
const SNAPS: [f32; 3] = [0.3, 0.5, 0.7];
/// Dragging the divider past these shares closes the smaller pane.
const CLOSE_BELOW: f32 = 0.15;
/// A rightward panel swipe faster than this (px/s) dismisses it.
const FLICK_PX_S: f32 = 700.0;
/// Height of the panel's grab strip at its top.
const GRAB_STRIP_PT: f32 = 28.0;
/// How much of a stashed Slide Over panel stays visible at the screen edge.
const STASH_PEEK_PT: f32 = 14.0;

/// Apps that can be laid out at any width. The terminal and Files derive
/// their layout from the full screen and stay single-window.
pub fn splittable(app: AppId) -> bool {
    !matches!(app.parent(), AppId::Terminal | AppId::Files | AppId::External(_))
}

/// The snap share (of the width left by the divider) nearest `ratio`.
pub fn snap(ratio: f32) -> f32 {
    SNAPS.iter().copied().min_by(|a, b| (a - ratio).abs().total_cmp(&(b - ratio).abs())).unwrap_or(0.5)
}

/// Pane widths for a Split View: the first pane's width, the second's left
/// edge and its width; the divider fills the gap between them.
pub fn split_widths(w: i32, ratio: f32, divider: i32) -> (i32, i32, i32) {
    let first = ((w - divider) as f32 * ratio).round() as i32;
    let second_x = first + divider;
    (first, second_x, w - second_x)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// the first pane's share of the width, 0..1
    Split { ratio: f32 },
    /// the panel's left edge in px
    Slide { x: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    First,
    Second,
}

/// Where the two apps sit: (left edge, width) of each, and the divider's left
/// edge in a Split View. In Slide Over the first pane is the whole screen and
/// the second floats over it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panes {
    pub first: (i32, i32),
    pub second: (i32, i32),
    pub divider: Option<i32>,
}

/// What a touch did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    None,
    /// forward to this pane's app, at this pane-local x
    Pass(Pane, f32),
    /// the geometry changed: relayout and repaint
    Resized,
    CloseSecond,
    ClosePrimary,
}

#[derive(Clone, Copy, Debug)]
enum Grab {
    Divider,
    Panel { dx: f32, last: (f32, f64), vel: f32, start: f32 },
}

#[derive(Clone, Copy, Debug)]
pub struct Multi {
    pub second: AppId,
    pub mode: Mode,
    grab: Option<Grab>,
    capture: Option<Pane>,
}

impl Multi {
    pub fn split(second: AppId) -> Self {
        Multi { second, mode: Mode::Split { ratio: 0.5 }, grab: None, capture: None }
    }

    /// A Slide Over panel at the right edge.
    pub fn slide(second: AppId, ui: &Ui, w: i32) -> Self {
        let x = Self::right_x(ui, w);
        Multi { second, mode: Mode::Slide { x }, grab: None, capture: None }
    }

    fn panel_w(ui: &Ui, w: i32) -> f32 {
        ui.pt(SLIDE_PT).min(w as f32 * 0.9).round()
    }

    fn right_x(ui: &Ui, w: i32) -> f32 {
        w as f32 - Self::panel_w(ui, w) - ui.pt(SLIDE_MARGIN_PT)
    }

    fn left_x(ui: &Ui) -> f32 {
        ui.pt(SLIDE_MARGIN_PT)
    }

    /// Slide Over parked mostly off the left or right edge, a sliver left to pull it back.
    pub fn stashed(&self, ui: &Ui, w: i32) -> bool {
        match self.mode {
            Mode::Slide { x } => x >= w as f32 - ui.pt(STASH_PEEK_PT) - 1.0 || x <= ui.pt(STASH_PEEK_PT) - Self::panel_w(ui, w) + 1.0,
            _ => false,
        }
    }

    pub fn is_split(&self) -> bool {
        matches!(self.mode, Mode::Split { .. })
    }

    pub fn panes(&self, ui: &Ui, w: i32) -> Panes {
        match self.mode {
            Mode::Split { ratio } => {
                let div = ui.pt(DIVIDER_PT).round() as i32;
                let (first, sx, sw) = split_widths(w, ratio, div);
                Panes { first: (0, first), second: (sx, sw), divider: Some(first) }
            }
            Mode::Slide { x } => Panes { first: (0, w), second: (x.round() as i32, Self::panel_w(ui, w) as i32), divider: None },
        }
    }

    fn local(&self, p: &Panes, pane: Pane, x: f32) -> f32 {
        match pane {
            Pane::First => x,
            Pane::Second => x - p.second.0 as f32,
        }
    }

    pub fn down(&mut self, x: f32, y: f32, t: f64, ui: &Ui, w: i32) -> Out {
        let p = self.panes(ui, w);
        self.grab = None;
        self.capture = None;
        let pane = match (self.mode, p.divider) {
            (Mode::Split { .. }, Some(d)) => {
                let centre = d as f32 + (p.second.0 - d) as f32 / 2.0;
                if (x - centre).abs() <= ui.pt(GRAB_PT) {
                    self.grab = Some(Grab::Divider);
                    return Out::None;
                }
                if x < p.second.0 as f32 { Pane::First } else { Pane::Second }
            }
            _ => {
                let (px, pw) = (p.second.0 as f32, p.second.1 as f32);
                if x >= px && x < px + pw {
                    if y < ui.pt(GRAB_STRIP_PT) || self.stashed(ui, w) {
                        self.grab = Some(Grab::Panel { dx: x - px, last: (x, t), vel: 0.0, start: x });
                        return Out::None;
                    }
                    Pane::Second
                } else {
                    Pane::First
                }
            }
        };
        self.capture = Some(pane);
        Out::Pass(pane, self.local(&p, pane, x))
    }

    pub fn moved(&mut self, x: f32, t: f64, ui: &Ui, w: i32) -> Out {
        match (self.grab, self.mode) {
            (Some(Grab::Divider), Mode::Split { .. }) => {
                let div = ui.pt(DIVIDER_PT);
                let ratio = ((x - div / 2.0) / (w as f32 - div)).clamp(0.05, 0.95);
                self.mode = Mode::Split { ratio };
                Out::Resized
            }
            (Some(Grab::Panel { dx, last, vel, start }), Mode::Slide { .. }) => {
                let dt = (t - last.1) as f32;
                let vel = if dt > 0.0 { 0.5 * vel + 0.5 * (x - last.0) / dt } else { vel };
                let min = ui.pt(STASH_PEEK_PT) - Self::panel_w(ui, w);
                self.mode = Mode::Slide { x: (x - dx).clamp(min, w as f32) };
                self.grab = Some(Grab::Panel { dx, last: (x, t), vel, start });
                Out::Resized
            }
            _ => match self.capture {
                Some(pane) => Out::Pass(pane, self.local(&self.panes(ui, w), pane, x)),
                None => Out::None,
            },
        }
    }

    pub fn up(&mut self, x: f32, ui: &Ui, w: i32) -> Out {
        let grab = self.grab.take();
        let capture = self.capture.take();
        match (grab, self.mode) {
            (Some(Grab::Divider), Mode::Split { ratio }) => {
                if ratio < CLOSE_BELOW {
                    Out::ClosePrimary
                } else if ratio > 1.0 - CLOSE_BELOW {
                    Out::CloseSecond
                } else {
                    self.mode = Mode::Split { ratio: snap(ratio) };
                    Out::Resized
                }
            }
            (Some(Grab::Panel { vel, start, .. }), Mode::Slide { x: px }) => {
                let moved_little = (x - start).abs() < ui.pt(8.0);
                let pw = Self::panel_w(ui, w);
                let peek = ui.pt(STASH_PEEK_PT);
                let was_stashed = self.stashed(ui, w);
                if was_stashed && moved_little {
                    // a tap on the sliver brings the panel back
                    let x = if px < 0.0 { Self::left_x(ui) } else { Self::right_x(ui, w) };
                    self.mode = Mode::Slide { x };
                    Out::Resized
                } else if vel > FLICK_PX_S || px > w as f32 - pw * 0.6 {
                    self.mode = Mode::Slide { x: w as f32 - peek };
                    Out::Resized
                } else if vel < -FLICK_PX_S || px < -pw * 0.4 {
                    self.mode = Mode::Slide { x: peek - pw };
                    Out::Resized
                } else {
                    let x = if px + pw / 2.0 < w as f32 / 2.0 { Self::left_x(ui) } else { Self::right_x(ui, w) };
                    self.mode = Mode::Slide { x };
                    Out::Resized
                }
            }
            _ => match capture {
                Some(pane) => Out::Pass(pane, self.local(&self.panes(ui, w), pane, x)),
                None => Out::None,
            },
        }
    }

    /// Divider (Split View) or panel edge and grab bar (Slide Over), drawn
    /// over the composed panes.
    pub fn draw_chrome(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let p = self.panes(ui, w);
        match p.divider {
            Some(d) => {
                let gap = (p.second.0 - d) as f32;
                s.fill_rect(d as usize, 0, gap as usize, h as usize, pack(0, 0, 0, 255));
                let (gw, gh) = (ui.pt(3.0).max(2.0), ui.pt(44.0));
                s.fill_rrect(d as f32 + (gap - gw) / 2.0, (h as f32 - gh) / 2.0, gw, gh, gw / 2.0, rgba(200, 200, 205, 255));
            }
            None => {
                let (x, pw) = (p.second.0 as f32, p.second.1 as f32);
                s.stroke_rrect(x - 1.0, 0.0, pw + 2.0, h as f32, 0.0, 2.0, rgba(0, 0, 0, 255));
                let (gw, gh) = (ui.pt(36.0), ui.pt(5.0));
                s.fill_rrect(x + (pw - gw) / 2.0, ui.pt(8.0), gw, gh, gh / 2.0, rgba(200, 200, 205, 255));
            }
        }
    }
}

// ---- the three-dot button and its menu ---------------------------------------------

/// The three-dot button's touch rect at the top centre: (x, y, w, h).
pub fn dots_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    let (bw, bh) = (ui.pt(64.0), ui.pt(28.0));
    ((w as f32 - bw) / 2.0, 0.0, bw, bh)
}

pub fn hit_dots(ui: &Ui, w: i32, x: f32, y: f32) -> bool {
    let (rx, ry, rw, rh) = dots_rect(ui, w);
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

pub fn draw_dots(s: &mut Surface, ui: &Ui, w: i32) {
    let (rx, ry, rw, rh) = dots_rect(ui, w);
    let d = ui.pt(5.0).max(3.0);
    let gap = ui.pt(5.0);
    let x0 = rx + rw / 2.0 - (3.0 * d + 2.0 * gap) / 2.0;
    for i in 0..3 {
        s.fill_rrect(x0 + i as f32 * (d + gap), ry + rh / 2.0 - d / 2.0, d, d, d / 2.0, rgba(200, 200, 205, 200));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Split,
    Slide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Mode,
    Pick(Kind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOut {
    None,
    Close,
    FullScreen,
    Start(Kind, AppId),
}

/// The menu under the three dots: first Split View or Slide Over, then which
/// app goes in the second window.
pub struct Menu {
    stage: Stage,
    current: AppId,
    pressed: Option<usize>,
}

const ROW_PT: f32 = 44.0;
const MENU_W_PT: f32 = 220.0;
const MENU_BG: u32 = pack(51, 51, 51, 255);

impl Menu {
    pub fn new(current: AppId) -> Self {
        Menu { stage: Stage::Mode, current, pressed: None }
    }

    fn labels(&self) -> Vec<&'static str> {
        match self.stage {
            Stage::Mode => vec!["Full Screen", "Split View", "Slide Over"],
            Stage::Pick(_) => apps().iter().filter(|a| a.id != self.current && splittable(a.id)).map(|a| a.name).collect(),
        }
    }

    fn apps(&self) -> Vec<AppId> {
        apps().iter().filter(|a| a.id != self.current && splittable(a.id)).map(|a| a.id).collect()
    }

    fn rect(&self, ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let (mw, rh) = (ui.pt(MENU_W_PT), ui.pt(ROW_PT));
        ((w as f32 - mw) / 2.0, ui.pt(34.0), mw, rh * self.labels().len() as f32)
    }

    fn row_at(&self, ui: &Ui, w: i32, x: f32, y: f32) -> Option<usize> {
        let (mx, my, mw, mh) = self.rect(ui, w);
        if x < mx || x >= mx + mw || y < my || y >= my + mh {
            return None;
        }
        Some(((y - my) / ui.pt(ROW_PT)) as usize)
    }

    pub fn down(&mut self, ui: &Ui, w: i32, x: f32, y: f32) {
        self.pressed = self.row_at(ui, w, x, y);
    }

    pub fn up(&mut self, ui: &Ui, w: i32, x: f32, y: f32) -> MenuOut {
        let pressed = self.pressed.take();
        let Some(row) = self.row_at(ui, w, x, y) else { return MenuOut::Close };
        if pressed != Some(row) {
            return MenuOut::None;
        }
        match self.stage {
            Stage::Mode => {
                if row == 0 {
                    return MenuOut::FullScreen;
                }
                self.stage = Stage::Pick(if row == 1 { Kind::Split } else { Kind::Slide });
                MenuOut::None
            }
            Stage::Pick(kind) => self.apps().get(row).map_or(MenuOut::None, |&a| MenuOut::Start(kind, a)),
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32) {
        let (mx, my, mw, _) = self.rect(ui, w);
        let rh = ui.pt(ROW_PT);
        let labels = self.labels();
        s.fill_rrect(mx, my, mw, rh * labels.len() as f32, ui.pt(6.0).min(6.0), MENU_BG);
        for (i, label) in labels.iter().enumerate() {
            let y = my + rh * i as f32;
            if self.pressed == Some(i) {
                s.fill_rrect(mx, y, mw, rh, 0.0, pack(80, 80, 84, 255));
            }
            let img = ui.text(label, 16.0, Weight::Regular, WHITE);
            s.blit(&img, (mx + ui.pt(16.0)) as i32, (y + rh / 2.0 - img.h as f32 / 2.0) as i32, 255, 255);
        }
    }
}

// ---- the shell side ---------------------------------------------------------------

use super::{Effect, Event, Screen, Shell};

impl Shell {
    /// Lays `app` out for a `w` x `h` px window (Terminal and Files keep the screen).
    pub(super) fn resize_app(&mut self, app: AppId, w: i32, h: i32) {
        match app {
            AppId::Settings => {
                if let Some(a) = &mut self.settings {
                    a.set_size(&self.ui, w, h);
                }
            }
            AppId::AppStore => self.store.iter_mut().for_each(|a| a.set_size(&self.ui, w, h)),
            AppId::Deildu => self.deildu.iter_mut().for_each(|a| a.set_size(w, h)),
            AppId::Photos => self.photos.iter_mut().for_each(|a| a.set_size(w, h)),
            AppId::External(_) => {}
            AppId::ExternalWindow(slot, id) => self.ext_window_resize(slot as usize, u32::from(id), w, h),
            AppId::Magnifier => self.magnifier.iter_mut().for_each(|a| a.set_size(w, h)),
            // Terminal and Files keep the screen; the newer apps lay out from the full size
            _ => {}
        }
    }

    /// Gives each app the size of its pane.
    fn apply_panes(&mut self, primary: AppId, m: Multi) {
        let p = m.panes(&self.ui, self.w);
        let h = self.h;
        self.resize_app(primary, p.first.1.max(1), h);
        self.resize_app(m.second, p.second.1.max(1), h);
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }

    /// Back to one full-screen app; a no-op when none is split.
    pub(super) fn end_multi(&mut self) {
        let Some(m) = self.multi.take() else { return };
        self.multi_menu = None;
        // each app back at the size its own layout is made for: the frame, or the portrait panel
        // for the ones letterboxed in landscape
        let size = |sh: &Self, app| if super::rotate::landscape_native(app) { (sh.w, sh.h) } else { sh.portrait_size() };
        if let Screen::App(a) = self.screen {
            let (w, h) = size(self, a);
            self.resize_app(a, w, h);
        }
        let (w, h) = size(self, m.second);
        self.resize_app(m.second, w, h);
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }

    pub(super) fn split_tick(&mut self) {
        if let Some(m) = self.multi {
            if !matches!(self.screen, Screen::App(a) if a != m.second) {
                self.end_multi();
            }
        }
    }

    fn start_multi(&mut self, primary: AppId, kind: Kind, second: AppId, t: f64) {
        self.init_app(second, t);
        let m = match kind {
            Kind::Split => Multi::split(second),
            Kind::Slide => Multi::slide(second, &self.ui, self.w),
        };
        self.multi = Some(m);
        self.multi_menu = None;
        self.apply_panes(primary, m);
    }

    /// Touches for the three-dot menu, the split divider/panel and each pane.
    /// `None` leaves the touch to the single-app path.
    pub(super) fn split_touch(&mut self, app: AppId, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let w = self.w;
        if self.multi_menu.is_some() {
            let ui = &self.ui;
            let menu = self.multi_menu.as_mut()?;
            self.dirty = true;
            match ev {
                Event::Down(x, y) => menu.down(ui, w, x, y),
                Event::Up(x, y) => match menu.up(ui, w, x, y) {
                    MenuOut::Close => self.multi_menu = None,
                    MenuOut::FullScreen => {
                        self.multi_menu = None;
                        self.end_multi();
                    }
                    MenuOut::Start(kind, second) => self.start_multi(app, kind, second, t),
                    MenuOut::None => {}
                },
                _ => {}
            }
            return Some(vec![]);
        }
        let Some(mut m) = self.multi else {
            // the single-app path pays one match for the button
            return match ev {
                Event::Down(x, y) if !self.fullscreen && splittable(app) && !self.rot_landscape() && hit_dots(&self.ui, w, x, y) => {
                    self.multi_menu = Some(Menu::new(app));
                    self.dirty = true;
                    Some(vec![])
                }
                _ => None,
            };
        };
        let ui = &self.ui;
        let out = match ev {
            Event::Down(x, y) => {
                if y >= self.h as f32 - super::ui::home_zone(ui) {
                    // the go-home swipe: back to one window first
                    self.end_multi();
                    return None;
                }
                m.down(x, y, t, ui, w)
            }
            Event::Move(x, _) => m.moved(x, t, ui, w),
            Event::Up(x, _) => m.up(x, ui, w),
            _ => return None,
        };
        self.multi = Some(m);
        let at = |lx: f32| match ev {
            Event::Down(_, y) => Event::Down(lx, y),
            Event::Move(_, y) => Event::Move(lx, y),
            Event::Up(_, y) => Event::Up(lx, y),
            e => e,
        };
        Some(match out {
            Out::None => vec![],
            Out::Pass(pane, lx) => {
                let target = if pane == Pane::First { app } else { m.second };
                self.app_touch_single(target, at(lx), t)
            }
            Out::Resized => {
                self.apply_panes(app, m);
                vec![]
            }
            Out::CloseSecond => {
                self.end_multi();
                vec![]
            }
            Out::ClosePrimary => {
                self.end_multi();
                self.screen = Screen::App(m.second);
                vec![]
            }
        })
    }

    /// Both apps side by side (or the panel over the first), then the status bar.
    pub(super) fn draw_multi(&mut self, s: &mut Surface, app: AppId, t: f64) {
        let Some(m) = self.multi else { return };
        let p = m.panes(&self.ui, self.w);
        let h = self.h.max(1) as usize;
        s.fill_rect(0, s.y0, s.w, s.h - s.y0, pack(0, 0, 0, 255));
        for (pane_app, (x, pw)) in [(app, p.first), (m.second, p.second)] {
            let pw = pw.max(1) as usize;
            let mut buf = std::mem::take(&mut self.multi_buf);
            buf.resize(pw * h, 0);
            {
                let mut sub = Surface { w: pw, h, stride: pw, px: &mut buf, y0: 0 };
                self.draw_app_body(&mut sub, pane_app, t);
            }
            copy_pane(s, &buf, pw, h, x);
            self.multi_buf = buf;
        }
        if !self.fullscreen {
            self.draw_app_status(s);
            super::ui::home_indicator(s, &self.ui, self.w, self.h, 255);
        }
        m.draw_chrome(s, &self.ui, self.w, self.h);
    }

    /// The three dots (single window) and the open menu.
    pub(super) fn draw_split_ui(&mut self, s: &mut Surface, app: AppId) {
        if self.multi.is_none() && !self.fullscreen && splittable(app) && !self.rot_landscape() {
            draw_dots(s, &self.ui, self.w);
        }
        if let Some(menu) = &self.multi_menu {
            menu.draw(s, &self.ui, self.w);
        }
    }
}

/// Copies a `pw` x `h` pane image into `s` with its left edge at `x`, clipped.
fn copy_pane(s: &mut Surface, src: &[u32], pw: usize, h: usize, x: i32) {
    let (x0, x1) = (x.max(0), (x + pw as i32).min(s.w as i32));
    if x0 >= x1 {
        return;
    }
    let n = (x1 - x0) as usize;
    for y in s.y0..h.min(s.h) {
        let from = y * pw + (x0 - x) as usize;
        let to = (y - s.y0) * s.stride + x0 as usize;
        s.px[to..to + n].copy_from_slice(&src[from..from + n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    #[test]
    fn snap_picks_nearest_share() {
        assert_eq!(snap(0.22), 0.3);
        assert_eq!(snap(0.46), 0.5);
        assert_eq!(snap(0.9), 0.7);
    }

    #[test]
    fn split_widths_fill_the_screen() {
        let (a, sx, sw) = split_widths(1000, 0.5, 16);
        assert_eq!((a, sx), (492, 508));
        assert_eq!(sx + sw, 1000);
    }

    #[test]
    fn touches_land_in_their_pane_in_pane_local_x() {
        let ui = ui();
        let mut m = Multi::split(AppId::Photos);
        let p = m.panes(&ui, 1000);
        let x = p.second.0 as f32 + 50.0;
        assert_eq!(m.down(x, 400.0, 0.0, &ui, 1000), Out::Pass(Pane::Second, 50.0));
        assert_eq!(m.moved(x + 10.0, 0.1, &ui, 1000), Out::Pass(Pane::Second, 60.0));
        assert_eq!(m.up(x + 10.0, &ui, 1000), Out::Pass(Pane::Second, 60.0));
        assert_eq!(m.down(100.0, 400.0, 1.0, &ui, 1000), Out::Pass(Pane::First, 100.0));
    }

    #[test]
    fn divider_drag_snaps_and_closes() {
        let ui = ui();
        let mut m = Multi::split(AppId::Photos);
        let mid = m.panes(&ui, 1000).divider.unwrap() as f32 + 8.0;
        assert_eq!(m.down(mid, 300.0, 0.0, &ui, 1000), Out::None);
        assert_eq!(m.moved(690.0, 0.1, &ui, 1000), Out::Resized);
        assert_eq!(m.up(690.0, &ui, 1000), Out::Resized);
        assert_eq!(m.mode, Mode::Split { ratio: 0.7 });
        let d = m.panes(&ui, 1000).divider.unwrap() as f32 + 8.0;
        m.down(d, 300.0, 1.0, &ui, 1000);
        m.moved(990.0, 1.1, &ui, 1000);
        assert_eq!(m.up(990.0, &ui, 1000), Out::CloseSecond);
    }

    #[test]
    fn slide_over_drags_snaps_and_flicks_away() {
        let ui = ui();
        let w = 1600;
        let mut m = Multi::slide(AppId::Photos, &ui, w);
        let px = m.panes(&ui, w).second.0 as f32;
        // a tap below the grab strip goes to the panel, one left of it to the app under
        assert!(matches!(m.down(px + 20.0, 400.0, 0.0, &ui, w), Out::Pass(Pane::Second, _)));
        m.up(px + 20.0, &ui, w);
        assert!(matches!(m.down(100.0, 400.0, 0.0, &ui, w), Out::Pass(Pane::First, _)));
        m.up(100.0, &ui, w);
        // drag the grab strip to the left half: it settles at the left edge
        m.down(px + 20.0, 5.0, 1.0, &ui, w);
        m.moved(300.0, 3.0, &ui, w);
        assert_eq!(m.up(300.0, &ui, w), Out::Resized);
        assert_eq!(m.panes(&ui, w).second.0 as f32, ui.pt(SLIDE_MARGIN_PT).round());
        // a fast swipe right dismisses
        m.down(40.0, 5.0, 4.0, &ui, w);
        m.moved(400.0, 4.1, &ui, w);
        m.moved(900.0, 4.2, &ui, w);
        assert_eq!(m.up(900.0, &ui, w), Out::Resized);
        assert!(m.stashed(&ui, w));
    }

    #[test]
    fn menu_walks_mode_then_app() {
        let ui = ui();
        let mut menu = Menu::new(AppId::Settings);
        let (mx, my, _, _) = menu.rect(&ui, 1000);
        let (x, y) = (mx + 10.0, my + 10.0);
        menu.down(&ui, 1000, x, y);
        assert_eq!(menu.up(&ui, 1000, x, y), MenuOut::FullScreen);
        let y = y + ui.pt(ROW_PT);
        menu.down(&ui, 1000, x, y);
        assert_eq!(menu.up(&ui, 1000, x, y), MenuOut::None);
        let (x, y) = (mx + 10.0, my + 10.0);
        menu.down(&ui, 1000, x, y);
        let first = menu.apps()[0];
        assert_eq!(menu.up(&ui, 1000, x, y), MenuOut::Start(Kind::Split, first));
        assert_eq!(menu.up(&ui, 1000, 1.0, 900.0), MenuOut::Close);
    }

    #[test]
    fn terminal_and_files_stay_single() {
        assert!(!splittable(AppId::Terminal) && !splittable(AppId::Files) && splittable(AppId::Photos));
    }
}

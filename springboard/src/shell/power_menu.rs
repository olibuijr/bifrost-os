//! The power menu: holding the Power key for a second with the screen on
//! brings up the "slide to power off" sheet with Restart underneath,
//! and Settings › General offers the same
//! actions behind confirmation alerts.
//!
//! `PowerKey` is the hold detector the binary feeds with the key's
//! transitions; `Slide` is the slider's drag logic; `PowerMenu` is the
//! sheet itself. All of it is pure state, apart from drawing.

use super::ui::{centred, Ui, Weight, BLACK, CELL, TRACK, WHITE};
use super::Event;
use crate::gfx::{rgba, Image, Surface};

/// Holding Power this long (screen on) opens the menu.
pub const HOLD_MS: f64 = 1000.0;
/// How far the knob must travel (fraction of the track) to commit.
pub const COMMIT: f32 = 0.9;

/// What a power action asks the binary to do (see `Effect::Power`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    /// sys.powerctl=shutdown
    Off,
    /// sys.powerctl=reboot
    Restart,
}

/// Tells a long press of Power from a tap or a chord.
///
/// With the screen on, a Power press that would turn it off is held back
/// until the key is released, so holding it can open the menu instead;
/// with the screen off the press goes straight through to wake it.
#[derive(Debug, Default)]
pub struct PowerKey {
    down_at: Option<f64>,
    /// the press happened with the screen on and nothing has used it yet
    armed: bool,
    /// a Power event was swallowed and waits for the release
    held_back: bool,
}

impl PowerKey {
    /// The key went down; `screen_on` as of now.
    pub fn down(&mut self, t: f64, screen_on: bool) {
        self.down_at = Some(t);
        self.armed = screen_on;
        self.held_back = false;
    }

    /// Another key or a chord took this press: no menu, no held-back tap.
    pub fn taint(&mut self) {
        self.armed = false;
        self.held_back = false;
    }

    /// Passes a shell event the chord detector produced through: a Power
    /// event during an armed press is held back until release; a chord
    /// (screenshot) cancels the press.
    pub fn filter(&mut self, ev: Option<Event>) -> Option<Event> {
        match ev {
            Some(Event::Power) if self.down_at.is_some() && self.armed => {
                self.held_back = true;
                None
            }
            Some(Event::Power | Event::Screenshot) => {
                self.armed = false;
                ev
            }
            _ => ev,
        }
    }

    /// The key went up: the held-back tap, if the menu did not claim it.
    pub fn up(&mut self) -> Option<Event> {
        let tap = (self.held_back && self.armed).then_some(Event::Power);
        *self = PowerKey::default();
        tap
    }

    /// When the hold completes, if a press is waiting on it.
    pub fn deadline(&self) -> Option<f64> {
        self.down_at.filter(|_| self.armed).map(|t| t + HOLD_MS)
    }

    /// True once, when the hold is long enough: open the menu. The held
    /// tap is dropped so releasing the key does not also lock the screen.
    pub fn due(&mut self, t: f64) -> bool {
        if self.deadline().is_some_and(|d| t >= d) {
            self.armed = false;
            self.held_back = false;
            return true;
        }
        false
    }
}

/// The slider's drag state: `pos` 0..=1 along the track.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slide {
    pub pos: f32,
    /// where in the knob it was grabbed (px)
    grab: Option<f32>,
}

impl Slide {
    /// Knob left edge for `pos`, given the track's left edge, travel and padding.
    fn knob_x(&self, track_x: f32, pad: f32, travel: f32) -> f32 {
        track_x + pad + self.pos * travel
    }

    /// Grabs the knob if `x` is on it. `knob` is its width.
    pub fn down(&mut self, x: f32, track_x: f32, pad: f32, travel: f32, knob: f32) -> bool {
        let left = self.knob_x(track_x, pad, travel);
        if x < left || x >= left + knob {
            return false;
        }
        self.grab = Some(x - left);
        true
    }

    pub fn drag(&mut self, x: f32, track_x: f32, pad: f32, travel: f32) {
        if let Some(g) = self.grab {
            self.pos = if travel > 0.0 { ((x - g - track_x - pad) / travel).clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    pub fn grabbed(&self) -> bool {
        self.grab.is_some()
    }

    /// Lets go: true when the knob got far enough to commit, otherwise it
    /// returns to the start.
    pub fn release(&mut self) -> bool {
        let done = self.grab.is_some() && self.pos >= COMMIT;
        self.grab = None;
        if !done {
            self.pos = 0.0;
        }
        done
    }
}

/// What a touch did to the sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// nothing to redraw
    Idle,
    /// the sheet changed (knob moved, button pressed)
    Redraw,
    Pick(PowerAction),
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Button {
    Restart,
    Cancel,
}

type Rect = (f32, f32, f32, f32);

fn inside((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// Flat corners: never more than 6 px.
const RADIUS_PX: f32 = 5.0;
const TRACK_W_PT: f32 = 320.0;
const TRACK_H_PT: f32 = 56.0;
const PAD_PT: f32 = 4.0;
const BUTTON_H_PT: f32 = 50.0;
const GAP_PT: f32 = 12.0;

/// Where everything sits for a `w` x `h` screen.
struct Layout {
    track: Rect,
    pad: f32,
    knob: f32,
    travel: f32,
    restart: Rect,
    cancel: Rect,
}

fn layout(ui: &Ui, w: i32, h: i32) -> Layout {
    let tw = ui.pt(TRACK_W_PT).min(w as f32 - ui.pt(32.0));
    let th = ui.pt(TRACK_H_PT);
    let tx = ((w as f32 - tw) / 2.0).round();
    let ty = (h as f32 * 0.18).round();
    let (pad, bh, gap) = (ui.pt(PAD_PT), ui.pt(BUTTON_H_PT), ui.pt(GAP_PT));
    let knob = th - 2.0 * pad;
    let restart = (tx, ty + th + 3.0 * gap, tw, bh);
    let cancel = (tx, restart.1 + bh + 3.0 * gap, tw, bh);
    Layout { track: (tx, ty, tw, th), pad, knob, travel: tw - knob - 2.0 * pad, restart, cancel }
}

pub struct PowerMenu {
    slide: Slide,
    pressed: Option<Button>,
    caption: Image,
    chevron: Image,
    restart: Image,
    cancel: Image,
}

impl PowerMenu {
    pub fn new(ui: &Ui) -> Self {
        PowerMenu {
            slide: Slide::default(),
            pressed: None,
            caption: ui.text("slide to power off", 20.0, Weight::Regular, WHITE),
            chevron: ui.text("›", 34.0, Weight::Medium, BLACK),
            restart: ui.text("Restart", 17.0, Weight::Regular, WHITE),
            cancel: ui.text("Cancel", 17.0, Weight::Semibold, WHITE),
        }
    }

    fn button_at(l: &Layout, x: f32, y: f32) -> Option<Button> {
        [(Button::Restart, l.restart), (Button::Cancel, l.cancel)].into_iter().find(|(_, r)| inside(*r, x, y)).map(|(b, _)| b)
    }

    /// Feeds a touch (screen px); anything outside the controls is ignored,
    /// only Cancel (or the Power key) closes the sheet.
    pub fn event(&mut self, ui: &Ui, w: i32, h: i32, ev: Event) -> Outcome {
        let l = layout(ui, w, h);
        match ev {
            Event::Down(x, y) => {
                if inside(l.track, x, y) && self.slide.down(x, l.track.0, l.pad, l.travel, l.knob) {
                    return Outcome::Redraw;
                }
                self.slide = Slide::default();
                self.pressed = Self::button_at(&l, x, y);
                Outcome::Redraw
            }
            Event::Move(x, _) if self.slide.grabbed() => {
                self.slide.drag(x, l.track.0, l.pad, l.travel);
                Outcome::Redraw
            }
            Event::Up(x, y) => {
                if self.slide.grabbed() {
                    return if self.slide.release() { Outcome::Pick(PowerAction::Off) } else { Outcome::Redraw };
                }
                let pressed = self.pressed.take();
                match pressed.filter(|b| Self::button_at(&l, x, y) == Some(*b)) {
                    Some(Button::Restart) => Outcome::Pick(PowerAction::Restart),
                    Some(Button::Cancel) => Outcome::Dismiss,
                    None => Outcome::Redraw,
                }
            }
            _ => Outcome::Idle,
        }
    }

    /// Centres of the knob, Restart and Cancel, for tests.
    pub fn targets(&self, ui: &Ui, w: i32, h: i32) -> [(f32, f32); 3] {
        let l = layout(ui, w, h);
        let c = |(x, y, w, h): Rect| (x + w / 2.0, y + h / 2.0);
        let knob = (l.track.0 + l.pad + self.slide.pos * l.travel + l.knob / 2.0, l.track.1 + l.track.3 / 2.0);
        [knob, c(l.restart), c(l.cancel)]
    }

    /// X of the knob's centre when slid to `pos` (0..=1), for tests.
    pub fn knob_at(&self, ui: &Ui, w: i32, h: i32, pos: f32) -> f32 {
        let l = layout(ui, w, h);
        l.track.0 + l.pad + pos * l.travel + l.knob / 2.0
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 175));
        let l = layout(ui, w, h);
        let (tx, ty, tw, th) = l.track;
        s.fill_rrect(tx, ty, tw, th, RADIUS_PX, TRACK);
        // the caption fades as the knob covers it
        let fade = ((1.0 - self.slide.pos * 1.6).clamp(0.0, 1.0) * 255.0) as u32;
        let cx = tx + l.pad + l.knob + (tw - l.knob - l.pad * 2.0) / 2.0;
        s.blit(&self.caption, (cx - self.caption.w as f32 / 2.0) as i32, (ty + (th - self.caption.h as f32) / 2.0) as i32, 255, fade);
        let kx = tx + l.pad + self.slide.pos * l.travel;
        s.fill_rrect(kx, ty + l.pad, l.knob, l.knob, RADIUS_PX, WHITE);
        s.blit(&self.chevron, (kx + (l.knob - self.chevron.w as f32) / 2.0) as i32, (ty + (th - self.chevron.h as f32) / 2.0) as i32, 255, 255);
        for (which, rect, label) in [(Button::Restart, l.restart, &self.restart), (Button::Cancel, l.cancel, &self.cancel)] {
            let fill = if self.pressed == Some(which) { TRACK } else { CELL };
            s.fill_rrect(rect.0, rect.1, rect.2, rect.3, RADIUS_PX, fill);
            centred(s, label, (rect.0 + rect.2 / 2.0) as i32, (rect.1 + (rect.3 - label.h as f32) / 2.0) as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    #[test]
    fn a_hold_opens_the_menu_and_swallows_the_tap() {
        let mut k = PowerKey::default();
        k.down(0.0, true);
        assert_eq!(k.deadline(), Some(HOLD_MS));
        // the chord's 150 ms window closes: the tap is held back
        assert_eq!(k.filter(Some(Event::Power)), None);
        assert!(!k.due(999.0));
        assert!(k.due(1000.0));
        assert!(!k.due(1100.0), "once");
        assert_eq!(k.up(), None, "releasing after the menu opened locks nothing");
    }

    #[test]
    fn a_short_press_is_a_tap_on_release() {
        let mut k = PowerKey::default();
        k.down(0.0, true);
        assert_eq!(k.filter(Some(Event::Power)), None);
        assert_eq!(k.up(), Some(Event::Power));
        assert_eq!(k.deadline(), None);
        // a quick tap resolved by the chord on release
        k.down(2000.0, true);
        assert_eq!(k.filter(Some(Event::Power)), None);
        assert_eq!(k.up(), Some(Event::Power));
    }

    #[test]
    fn with_the_screen_off_the_press_goes_straight_through() {
        let mut k = PowerKey::default();
        k.down(0.0, false);
        assert_eq!(k.deadline(), None);
        assert_eq!(k.filter(Some(Event::Power)), Some(Event::Power));
        assert!(!k.due(5000.0), "the wake press never opens the menu");
        assert_eq!(k.up(), None);
    }

    #[test]
    fn a_chord_or_another_key_cancels_the_hold() {
        let mut k = PowerKey::default();
        k.down(0.0, true);
        assert_eq!(k.filter(Some(Event::Screenshot)), Some(Event::Screenshot));
        assert!(!k.due(2000.0));
        assert_eq!(k.up(), None);
        k.down(0.0, true);
        k.taint();
        assert_eq!(k.deadline(), None);
        assert_eq!(k.up(), None);
    }

    #[test]
    fn a_power_event_without_a_press_passes_through() {
        // the control socket's `key power` never goes through `down`
        let mut k = PowerKey::default();
        assert_eq!(k.filter(Some(Event::Power)), Some(Event::Power));
        assert_eq!(k.filter(None), None);
        assert_eq!(k.filter(Some(Event::VolumeUp)), Some(Event::VolumeUp));
    }

    #[test]
    fn the_slider_commits_only_near_the_end_and_springs_back() {
        let mut s = Slide::default();
        let (tx, pad, travel, knob) = (100.0, 4.0, 300.0, 48.0);
        assert!(!s.down(50.0, tx, pad, travel, knob), "off the knob");
        assert!(s.down(110.0, tx, pad, travel, knob));
        s.drag(110.0 + 150.0, tx, pad, travel);
        assert!((s.pos - 0.5).abs() < 1e-4);
        assert!(!s.release());
        assert_eq!(s.pos, 0.0);
        assert!(s.down(110.0, tx, pad, travel, knob));
        s.drag(110.0 + 290.0, tx, pad, travel);
        assert!(s.release());
        s.drag(5000.0, tx, pad, travel);
        assert!(!s.grabbed());
        let mut s = Slide::default();
        s.down(110.0, tx, pad, travel, knob);
        s.drag(9999.0, tx, pad, travel);
        assert_eq!(s.pos, 1.0, "clamped");
        s.drag(-9999.0, tx, pad, travel);
        assert_eq!(s.pos, 0.0);
    }

    fn tap(m: &mut PowerMenu, ui: &Ui, (x, y): (f32, f32)) -> Outcome {
        m.event(ui, 1200, 1920, Event::Down(x, y));
        m.event(ui, 1200, 1920, Event::Up(x, y))
    }

    #[test]
    fn buttons_pick_restart_and_cancel_dismisses() {
        let ui = ui();
        let mut m = PowerMenu::new(&ui);
        let [_, restart, cancel] = m.targets(&ui, 1200, 1920);
        assert_eq!(tap(&mut m, &ui, restart), Outcome::Pick(PowerAction::Restart));
        assert_eq!(tap(&mut m, &ui, cancel), Outcome::Dismiss);
        assert_eq!(tap(&mut m, &ui, (5.0, 5.0)), Outcome::Redraw, "outside does not close it");
        // pressing one button and releasing on another does nothing
        m.event(&ui, 1200, 1920, Event::Down(restart.0, restart.1));
        assert_eq!(m.event(&ui, 1200, 1920, Event::Up(cancel.0, cancel.1)), Outcome::Redraw);
    }

    #[test]
    fn sliding_all_the_way_powers_off_and_a_short_slide_does_not() {
        let ui = ui();
        let mut m = PowerMenu::new(&ui);
        let [knob, ..] = m.targets(&ui, 1200, 1920);
        m.event(&ui, 1200, 1920, Event::Down(knob.0, knob.1));
        m.event(&ui, 1200, 1920, Event::Move(m.knob_at(&ui, 1200, 1920, 0.4), knob.1));
        assert_eq!(m.event(&ui, 1200, 1920, Event::Up(0.0, knob.1)), Outcome::Redraw);
        let [knob, ..] = m.targets(&ui, 1200, 1920);
        m.event(&ui, 1200, 1920, Event::Down(knob.0, knob.1));
        m.event(&ui, 1200, 1920, Event::Move(m.knob_at(&ui, 1200, 1920, 1.0), knob.1));
        assert_eq!(m.event(&ui, 1200, 1920, Event::Up(0.0, knob.1)), Outcome::Pick(PowerAction::Off));
    }

    #[test]
    fn the_sheet_draws_without_panicking() {
        let ui = ui();
        let m = PowerMenu::new(&ui);
        let mut px = vec![0u32; 1200 * 1920];
        let mut s = Surface { w: 1200, h: 1920, stride: 1200, px: &mut px, y0: 0 };
        m.draw(&mut s, &ui, 1200, 1920);
    }
}

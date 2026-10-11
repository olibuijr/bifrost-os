//! The in-stream controls: the client core's `Ui` (sticks, buttons, menu, gestures) running at
//! the landscape frame's size and a touch density, and text entry through the shell's own
//! keyboard (`Ui::use_system_keyboard`). The shell turns the frame to the panel, so this is a
//! plain landscape app.

use super::render::Painter;
use crate::gfx::Surface;
use crate::shell::keyboard::{self, Keyboard, Out};
use crate::shell::ui::{home_zone, Ui};
use crate::shell::Event;
use wowstream_client::draw::DrawList;
use wowstream_client::overlay::Insets;
use wowstream_client::proto::{keys, Input};
use wowstream_client::ui::{Effect, Ui as Core};

/// Controls are drawn at this multiple of the shell's px per point (about 2 dp on a
/// touch screen: comfortable under thumbs, not huge).
const DENSITY: f32 = 1.3;

pub struct Controls {
    pub core: Core,
    w: i32,
    h: i32,
    dp: f32,
    insets: Insets,
    stream: (u16, u16),
    painter: Painter,
    /// The overlay last painted into the frame, to skip repainting identical controls.
    last_overlay: Option<([DrawList; 3], (usize, usize))>,
    keyboard: Keyboard,
    kb_touch: bool,
}

/// dp and the safe area: the bottom strip starts the go-home swipe.
fn metrics(ui: &Ui) -> (f32, Insets) {
    (ui.s * DENSITY, Insets { left: 0.0, top: 0.0, right: 0.0, bottom: home_zone(ui) })
}

/// One key tap for the game, with shift around it when needed.
fn tap(code: u16, shift: bool, fx: &mut Vec<Effect>) {
    if shift {
        fx.push(Effect::Send(Input::Key { code: keys::LEFTSHIFT, down: true }));
    }
    fx.push(Effect::Send(Input::Key { code, down: true }));
    fx.push(Effect::Send(Input::Key { code, down: false }));
    if shift {
        fx.push(Effect::Send(Input::Key { code: keys::LEFTSHIFT, down: false }));
    }
}

impl Controls {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Self {
        let (dp, insets) = metrics(ui);
        let mut core = Core::new(w as f32, h as f32, dp, insets);
        core.use_system_keyboard(true);
        Controls {
            core,
            w,
            h,
            dp,
            insets,
            stream: (1280, 720),
            painter: Painter::default(),
            last_overlay: None,
            keyboard: Keyboard::new_hidden(w as f32, ui.s),
            kb_touch: false,
        }
    }

    /// Density or safe area changed: rebuild the core's layout.
    pub fn sync(&mut self, ui: &Ui, fx: &mut Vec<Effect>) -> bool {
        let (dp, mut insets) = metrics(ui);
        let scaled = dp != self.dp;
        if scaled {
            self.keyboard.set_scale(ui.s);
        }
        // while the shell keyboard is up the picture must not hide under it
        if self.keyboard.shown() {
            insets.bottom += self.keyboard.h();
        }
        if !scaled && insets == self.insets {
            return false;
        }
        (self.dp, self.insets) = (dp, insets);
        self.core.resize(self.w as f32, self.h as f32, dp, insets, fx);
        self.core.set_stream(self.stream.0, self.stream.1);
        true
    }

    pub fn set_stream(&mut self, width: u16, height: u16) {
        self.stream = (width, height);
        self.core.set_stream(width, height);
    }

    /// Where the picture is shown, in pixels.
    pub fn view(&self) -> (f32, f32, f32, f32) {
        let v = self.core.letterbox.view;
        (v.x, v.y, v.w, v.h)
    }

    fn kb_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - self.keyboard.h()
    }

    pub fn keyboard_active(&self) -> bool {
        self.keyboard.shown() || self.keyboard.animating()
    }

    /// A touch on the surface (physical pixels).
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64, fx: &mut Vec<Effect>) {
        let now = t as u64;
        let kb_top = self.kb_top(ui);
        match ev {
            Event::Down(x, y) => {
                if self.keyboard.shown() && y >= kb_top {
                    self.kb_touch = true;
                    self.keyboard.down(x, y - kb_top, t);
                    return;
                }
                self.core.pointer_down(0, (x, y), now, fx);
                let before = fx.len();
                fx.retain(|e| !matches!(e, Effect::SystemKeyboard));
                if fx.len() != before {
                    let show = !self.keyboard.shown();
                    self.keyboard.set_shown(show, t);
                }
            }
            Event::Move(x, y) => {
                if self.kb_touch {
                    self.keyboard.moved(x, y - kb_top);
                } else {
                    self.core.pointers_moved(&[(0, (x, y))], fx);
                }
            }
            Event::Up(x, y) => {
                if std::mem::take(&mut self.kb_touch) {
                    if let Some(out) = self.keyboard.up(x, y - kb_top, t) {
                        self.key(out, fx);
                    }
                } else {
                    self.core.pointer_up(0, (x, y), fx);
                }
            }
            _ => {}
        }
    }

    /// A key from the shell's keyboard, as game keys.
    fn key(&mut self, out: Out, fx: &mut Vec<Effect>) {
        match out {
            Out::Text(s) => {
                for c in s.chars() {
                    if let Some((code, shift)) = keys::for_char(c) {
                        tap(code, shift, fx);
                    }
                }
            }
            Out::Backspace => tap(keys::BACKSPACE, false, fx),
            Out::Enter => tap(keys::ENTER, false, fx),
            Out::Named("Escape") => tap(keys::ESC, false, fx),
            Out::Hide => self.keyboard.set_shown(false, 0.0),
            Out::Named(_) => {}
        }
    }

    pub fn hardware_key(&mut self, code: u16, down: bool, ch: Option<char>, fx: &mut Vec<Effect>) {
        self.core.hardware_key(code, down, ch, fx);
    }

    pub fn tick(&mut self, t: f64, fx: &mut Vec<Effect>) -> bool {
        self.core.tick(t as u64, fx);
        let was = self.keyboard.animating();
        for out in self.keyboard.tick(t) {
            self.key(out, fx);
        }
        was || self.keyboard.animating()
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let ui = self.core.deadline(t as u64).map(|d| d as f64);
        let kb = self.keyboard.animating().then_some(t);
        match (ui, kb) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The controls' draw lists for this frame (cheap: no pixels).
    pub fn overlay(&mut self, hud: Option<&str>, banner: Option<&str>) -> [DrawList; 3] {
        let (mut base, mut top, mut dialog) = (DrawList::default(), DrawList::default(), DrawList::default());
        self.core.draw([&mut base, &mut top, &mut dialog], hud, banner, "");
        [base, top, dialog]
    }

    /// True when `lists` at this size were already painted last time and no keyboard animates,
    /// so the overlay pixels in the frame can stay as they are. Remembers `lists` otherwise.
    pub fn overlay_unchanged(&mut self, lists: &[DrawList; 3], size: (usize, usize)) -> bool {
        if !self.keyboard_active() && self.last_overlay.as_ref().is_some_and(|(l, sz)| l == lists && *sz == size) {
            return true;
        }
        self.last_overlay = Some((lists.clone(), size));
        false
    }

    /// Forget the painted overlay (the frame was drawn by someone else).
    pub fn overlay_invalidate(&mut self) {
        self.last_overlay = None;
    }

    pub fn paint(&mut self, s: &mut Surface, ui: &Ui, lists: &[DrawList; 3], t: f64) {
        for list in lists {
            self.painter.draw(s, ui, list);
        }
        if self.keyboard_active() {
            let top = self.kb_top(ui);
            self.keyboard.draw(s, ui, top, t);
            let below = (top + self.keyboard.h() + self.keyboard.offset(t)).max(0.0) as usize;
            s.fill_rect(0, below, s.w, s.h, keyboard::BG);
        }
    }
}

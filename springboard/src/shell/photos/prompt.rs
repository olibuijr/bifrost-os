//! A one-line text prompt over the app: a title, a text field, Cancel and Done, and the keyboard below. Photos uses
//! it wherever a name or a phrase is typed (album and folder names, title, caption, keywords, dates, search).
//! Typed keys come from the on-screen keyboard or, through `key`, from a hardware keyboard.

use super::{flat, BLUE, SHEET};
use crate::gfx::{rgba, Image, Surface};
use crate::layout::in_rect as within;
use crate::shell::keyboard::{self, Keyboard, Out};
use crate::shell::textfield::TextField;
use crate::shell::ui::{home_zone, Ui, Weight, SECONDARY, WHITE};
use crate::shell::Event;

const CARD_W_PT: f32 = 460.0;
const CARD_TOP_PT: f32 = 76.0;
const BAR_PT: f32 = 44.0;
const FIELD_H_PT: f32 = 40.0;
const PAD_PT: f32 = 14.0;
const FIELD_BG: u32 = crate::gfx::pack(28, 28, 30, 255);

/// What a touch or key did to the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptEvent {
    None,
    Cancel,
    Done(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Keyboard,
    Cancel,
    Done,
    Outside,
    Field,
}

pub struct Prompt {
    w: i32,
    h: i32,
    field: TextField,
    keyboard: Keyboard,
    pressed: Option<Press>,
    title: Image,
    cancel: Image,
    done: Image,
    hint: Option<Image>,
}

impl Prompt {
    /// Opens with the keyboard sliding in. `initial` is the text to edit, `hint` an optional line under the field.
    pub fn new(ui: &Ui, w: i32, h: i32, title: &str, placeholder: &str, initial: &str, hint: &str, t: f64) -> Self {
        let mut keyboard = Keyboard::new_hidden(w as f32, ui.s);
        keyboard.set_shown(true, t);
        let mut field = TextField::new(ui, placeholder, 17.0, false, 200);
        field.set_text(ui, initial);
        Prompt {
            w,
            h,
            field,
            keyboard,
            pressed: None,
            title: ui.text(title, 17.0, Weight::Semibold, WHITE),
            cancel: ui.text("Cancel", 17.0, Weight::Regular, BLUE),
            done: ui.text("Done", 17.0, Weight::Semibold, BLUE),
            hint: (!hint.is_empty()).then(|| ui.text(hint, 13.0, Weight::Regular, SECONDARY)),
        }
    }

    pub fn text(&self) -> &str {
        self.field.text()
    }

    fn keyboard_top(&self, ui: &Ui) -> f32 {
        let docked = if self.keyboard.shown() { self.keyboard.h() } else { 0.0 };
        self.h as f32 - home_zone(ui) - docked
    }

    /// The card (x, y, w, h).
    fn card(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let width = ui.pt(CARD_W_PT).min(self.w as f32 - ui.pt(32.0));
        let height = ui.pt(BAR_PT + FIELD_H_PT + 2.0 * PAD_PT) + if self.hint.is_some() { ui.pt(24.0) } else { 0.0 };
        ((self.w as f32 - width) / 2.0, ui.pt(CARD_TOP_PT), width, height)
    }

    fn field_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.card(ui);
        (x + ui.pt(PAD_PT), y + ui.pt(BAR_PT + 4.0), w - ui.pt(2.0 * PAD_PT), ui.pt(FIELD_H_PT))
    }

    fn field_frame(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.field_rect(ui);
        (x + ui.pt(10.0), y, w - ui.pt(20.0), h)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Press> {
        if self.keyboard.shown() && y >= self.keyboard_top(ui) {
            return Some(Press::Keyboard);
        }
        let (cx, cy, cw, ch) = self.card(ui);
        if !within(x, y, (cx, cy, cw, ch)) {
            return Some(Press::Outside);
        }
        if y < cy + ui.pt(BAR_PT) {
            return Some(if x < cx + cw / 2.0 { Press::Cancel } else { Press::Done });
        }
        within(x, y, self.field_rect(ui)).then_some(Press::Field)
    }

    /// Typed input from the on-screen or a hardware keyboard.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) -> PromptEvent {
        match out {
            Out::Hide => {
                self.keyboard.set_shown(false, t);
                PromptEvent::None
            }
            Out::Enter => PromptEvent::Done(self.field.text().trim().to_string()),
            Out::Named("Escape") => PromptEvent::Cancel,
            out => {
                self.field.focused = true;
                self.field.edit(ui, &out, t);
                PromptEvent::None
            }
        }
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> PromptEvent {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return PromptEvent::None,
        };
        let frame = self.field_frame(ui);
        if self.field.touch(ui, ev, frame, self.w as f32, t) {
            if self.field.focused {
                self.keyboard.set_shown(true, t);
            }
            return PromptEvent::None;
        }
        let top = self.keyboard_top(ui);
        let hit = self.hit(ui, x, y);
        match ev {
            Event::Down(..) => {
                self.pressed = hit;
                if hit == Some(Press::Keyboard) {
                    self.keyboard.down(x, y - top, t);
                }
            }
            Event::Move(..) => {
                if self.pressed == Some(Press::Keyboard) {
                    self.keyboard.moved(x, y - top);
                }
            }
            _ => match self.pressed.take() {
                Some(Press::Keyboard) => {
                    if let Some(out) = self.keyboard.up(x, y - top, t) {
                        return self.key(ui, out, t);
                    }
                }
                Some(p @ (Press::Cancel | Press::Outside)) if hit == Some(p) => return PromptEvent::Cancel,
                Some(Press::Done) if hit == Some(Press::Done) => return PromptEvent::Done(self.field.text().trim().to_string()),
                Some(Press::Field) => {
                    self.field.focused = true;
                    self.keyboard.set_shown(true, t);
                }
                _ => {}
            },
        }
        PromptEvent::None
    }

    /// Repeats held keys and blinks the caret; true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut dirty = self.keyboard.animating() | self.field.tick(ui, t);
        for out in self.keyboard.tick(t) {
            dirty = true;
            // a held key repeats text and Backspace; a result is reported by the touch path only
            let _ = self.key(ui, out, t);
        }
        dirty | self.field.next_wake(t).is_some_and(|due| due <= t + 1.0)
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.keyboard.animating() {
            return Some(t);
        }
        self.field.next_wake(t)
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 140));
        let (x, y, w, h) = self.card(ui);
        s.fill_rrect(x, y, w, h, flat(ui), SHEET);
        let mid = y + ui.pt(BAR_PT) / 2.0;
        s.blit(&self.title, (x + (w - self.title.w as f32) / 2.0) as i32, (mid - self.title.h as f32 / 2.0) as i32, 255, 255);
        s.blit(&self.cancel, (x + ui.pt(16.0)) as i32, (mid - self.cancel.h as f32 / 2.0) as i32, 255, 255);
        s.blit(&self.done, (x + w - ui.pt(16.0) - self.done.w as f32) as i32, (mid - self.done.h as f32 / 2.0) as i32, 255, 255);
        let f = self.field_rect(ui);
        s.fill_rrect(f.0, f.1, f.2, f.3, flat(ui), FIELD_BG);
        self.field.draw(s, ui, self.field_frame(ui), t);
        if let Some(hint) = &self.hint {
            s.blit(hint, (x + ui.pt(PAD_PT)) as i32, (f.1 + f.3 + ui.pt(12.0) - hint.h as f32 / 2.0) as i32, 255, 255);
        }
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = self.h as f32 - home_zone(ui) - self.keyboard.h();
            self.keyboard.draw(s, ui, top, t);
            let bottom = top + self.keyboard.h() + self.keyboard.offset(t);
            s.fill_rrect(0.0, bottom, self.w as f32, self.h as f32 - bottom, 0.0, keyboard::BG);
        }
    }
}

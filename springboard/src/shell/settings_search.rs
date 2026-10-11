//! The Settings search field: a capsule with a magnifier under the root
//! page's large title, its docked keyboard, and the title filter that
//! narrows the root rows like bifrost Settings.

use super::glyphs;
use super::keyboard::{self, Keyboard, Out};
use super::list::{Group, Spec};
use super::textfield::TextField;
use super::ui::{home_zone, Ui, FIELD_BG, MAC_BODY_PT, SECONDARY};
use super::Event;
use crate::gfx::Surface;

/// Height of the capsule.
pub const FIELD_PT: f32 = 36.0;
const GLASS_X_PT: f32 = 15.0;
const GLASS_R_PT: f32 = 5.0;
const TEXT_X_PT: f32 = 28.0;

pub struct Search {
    field: TextField,
    keyboard: Keyboard,
    /// the keyboard is up and keys go to the field
    pub active: bool,
    /// the finger went down on the capsule
    pressed_field: bool,
    /// the finger went down on the keyboard
    pressed_keys: bool,
}

impl Search {
    pub fn new(ui: &Ui, w: i32) -> Self {
        let mut field = TextField::new(ui, "Search", MAC_BODY_PT, false, 64);
        field.focused = false;
        let mut keyboard = Keyboard::new(w as f32, ui.s, false);
        keyboard.set_shown(false, 0.0);
        Search { field, keyboard, active: false, pressed_field: false, pressed_keys: false }
    }

    /// No keyboard up, rising or falling: the field and keyboard draw nothing that moves.
    pub fn idle(&self) -> bool {
        !self.active && !self.keyboard.shown() && !self.keyboard.animating()
    }

    pub fn query(&self) -> &str {
        self.field.text()
    }

    pub fn resize(&mut self, ui: &Ui, w: i32) {
        self.keyboard = Keyboard::new(w as f32, ui.s, false);
        self.keyboard.set_shown(self.active, 0.0);
    }

    fn show(&mut self, on: bool, t: f64) {
        self.active = on;
        self.field.focused = on;
        // a desktop has a physical keyboard: focusing the field never raises the on-screen one
        self.keyboard.set_shown(false, t);
    }

    /// Applies a key; true when the query changed.
    pub fn edit(&mut self, ui: &Ui, out: Out, t: f64) -> bool {
        match out {
            Out::Hide | Out::Enter => {
                self.show(false, t);
                false
            }
            out => {
                let before = self.field.text().to_string();
                self.field.edit(ui, &out, t);
                before != self.field.text()
            }
        }
    }

    fn keyboard_top(&self, ui: &Ui, h: i32) -> Option<f32> {
        self.keyboard.shown().then(|| h as f32 - home_zone(ui) - self.keyboard.h())
    }

    /// Routes a touch; `frame` is the capsule's rect on screen when the
    /// root page shows it. Returns (consumed, query changed).
    pub fn touch(&mut self, ui: &Ui, ev: Event, frame: Option<(f32, f32, f32, f32)>, h: i32, t: f64) -> (bool, bool) {
        let inside = |x: f32, y: f32| frame.is_some_and(|(fx, fy, fw, fh)| x >= fx && x < fx + fw && y >= fy && y < fy + fh);
        match ev {
            Event::Down(x, y) => {
                if let Some(top) = self.keyboard_top(ui, h).filter(|&top| y >= top) {
                    self.keyboard.down(x, y - top, t);
                    self.pressed_keys = true;
                    return (true, false);
                }
                self.pressed_field = inside(x, y);
                (self.pressed_field, false)
            }
            Event::Move(x, y) => {
                if self.pressed_keys {
                    let top = self.keyboard_top(ui, h).unwrap_or(0.0);
                    self.keyboard.moved(x, y - top);
                }
                (self.pressed_keys || self.pressed_field, false)
            }
            Event::Up(x, y) => {
                if std::mem::take(&mut self.pressed_keys) {
                    let top = self.keyboard_top(ui, h).unwrap_or(0.0);
                    let changed = self.keyboard.up(x, y - top, t).is_some_and(|o| self.edit(ui, o, t));
                    return (true, changed);
                }
                if std::mem::take(&mut self.pressed_field) {
                    if inside(x, y) {
                        let (fx, ..) = frame.expect("frame");
                        self.show(true, t);
                        self.field.tap(ui, x - fx - ui.pt(TEXT_X_PT), t);
                    }
                    return (true, false);
                }
                // a tap elsewhere puts the keyboard away
                if self.active {
                    self.show(false, t);
                }
                (false, false)
            }
            _ => (false, false),
        }
    }

    /// Repeats held keys and the keyboard slide; (query changed, frame needed).
    pub fn tick(&mut self, ui: &Ui, t: f64) -> (bool, bool) {
        let outs = self.keyboard.tick(t);
        let mut changed = false;
        for out in outs.iter().cloned() {
            changed |= self.edit(ui, out, t);
        }
        let blinking = self.field.next_blink(t).is_some_and(|due| due <= t + 1.0);
        (changed, !outs.is_empty() || blinking || self.keyboard.animating())
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.keyboard.animating() {
            return Some(t);
        }
        self.field.next_wake(t)
    }

    /// The capsule, drawn in `frame`.
    pub fn draw_field(&self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), t: f64) {
        // a capsule: the radius is half the height
        s.fill_rrect(x, y, w, h, h / 2.0, FIELD_BG.now());
        let mid = y + h / 2.0;
        let (gx, gr, stroke) = (x + ui.pt(GLASS_X_PT), ui.pt(GLASS_R_PT), ui.pt(1.8));
        let glass = glyphs::either(glyphs::ring((gx, mid - ui.pt(1.0)), gr, stroke), glyphs::segment((gx + gr * 0.72, mid - ui.pt(1.0) + gr * 0.72), (gx + gr * 1.5, mid - ui.pt(1.0) + gr * 1.5), stroke * 1.2));
        glyphs::paint(s, (gx - gr - stroke, mid - gr - stroke - ui.pt(2.0), gr * 2.6 + stroke, gr * 2.6 + stroke), SECONDARY, glass);
        self.field.draw(s, ui, (x + ui.pt(TEXT_X_PT), y, w - ui.pt(TEXT_X_PT + 14.0), h), t);
    }

    /// The docked keyboard over the bottom of the page.
    pub fn draw_keyboard(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = h as f32 - home_zone(ui) - self.keyboard.h();
            self.keyboard.draw(s, ui, top, t);
            let below = top + self.keyboard.h() + self.keyboard.offset(t);
            s.fill_rrect(0.0, below, w as f32, h as f32 - below, 0.0, keyboard::BG);
        }
    }
}

/// Keeps the rows whose title contains `query` (case-insensitive) and
/// drops the groups left empty; an empty query keeps everything.
pub fn filter(spec: &mut Spec, query: &str) {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return;
    }
    for g in &mut spec.groups {
        g.items.retain(|i| i.label.to_lowercase().contains(&q));
    }
    spec.groups.retain(|g: &Group| !g.items.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::list::Item;
    use crate::shell::pages::PageId;

    #[test]
    fn filter_keeps_matching_rows_and_drops_empty_groups() {
        let mut spec = Spec { id: PageId::Root, title: "Settings".into(), large: true, groups: vec![Group::new(vec![Item::new("Wi-Fi"), Item::new("Bluetooth")]), Group::new(vec![Item::new("Battery")])] };
        filter(&mut spec, " BLUE ");
        assert_eq!(spec.groups.len(), 1);
        assert_eq!(spec.groups[0].items[0].label, "Bluetooth");
    }
}

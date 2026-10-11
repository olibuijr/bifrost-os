//! The sign-in sheet: username and password fields over the docked keyboard.

use super::{within, Target, BLUE, FIELD, RED};
use crate::gfx::{rgba, Image, Surface};
use crate::shell::keyboard::Out;
use crate::shell::ui::{centred, Ui, Weight, CELL, SECONDARY, SEPARATOR, WHITE};

const CARD_W_PT: f32 = 540.0;
const CARD_H_PT: f32 = 236.0;
const BAR_PT: f32 = 76.0;
const BUTTON_W_PT: f32 = 120.0;
const FIELD_PT: f32 = 44.0;
const BULLET_PT: f32 = 24.0;
const MAX_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    User,
    Pass,
}

/// What the keyboard did to the sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    Changed,
    Submit,
    Hide,
}

pub struct Login {
    pub user: String,
    pub pass: String,
    pub focus: Field,
    /// keyboard shown
    pub typing: bool,
    pub busy: bool,
    pub error: Option<String>,
    title: Image,
    prompt: Image,
    cancel: Image,
    sign_in: Image,
    sign_in_off: Image,
    user_label: Image,
    pass_label: Image,
    user_img: Option<Image>,
    bullets: Option<Image>,
    error_img: Option<Image>,
    busy_img: Image,
}

impl Login {
    pub fn new(ui: &Ui) -> Self {
        Login {
            user: String::new(),
            pass: String::new(),
            focus: Field::User,
            typing: true,
            busy: false,
            error: None,
            title: ui.text("Sign In", 17.0, Weight::Semibold, WHITE),
            prompt: ui.text("Sign in to Deildu to browse the library", 13.0, Weight::Semibold, WHITE),
            cancel: ui.text("Cancel", 17.0, Weight::Regular, BLUE),
            sign_in: ui.text("Sign In", 17.0, Weight::Semibold, BLUE),
            sign_in_off: ui.text("Sign In", 17.0, Weight::Semibold, rgba(235, 235, 245, 77)),
            user_label: ui.text("Username", 17.0, Weight::Regular, WHITE),
            pass_label: ui.text("Password", 17.0, Weight::Regular, WHITE),
            user_img: None,
            bullets: None,
            error_img: None,
            busy_img: ui.text("Signing in…", 13.0, Weight::Regular, SECONDARY),
        }
    }

    pub fn can_submit(&self) -> bool {
        !self.user.is_empty() && !self.pass.is_empty() && !self.busy
    }

    pub fn set_error(&mut self, ui: &Ui, e: Option<&str>) {
        self.error = e.map(str::to_string);
        self.error_img = e.map(|e| ui.text(&ui.fit(e, 13.0, ui.pt(CARD_W_PT - 40.0)), 13.0, Weight::Regular, RED));
        self.busy = false;
    }

    /// Applies a key to the focused field.
    pub fn edit(&mut self, ui: &Ui, out: Out) -> Edit {
        if self.busy {
            return Edit::Changed;
        }
        let field = match self.focus {
            Field::User => &mut self.user,
            Field::Pass => &mut self.pass,
        };
        let r = match out {
            Out::Text(t) => {
                let room = MAX_LEN.saturating_sub(field.chars().count());
                field.extend(t.chars().filter(|c| !c.is_control()).take(room));
                Edit::Changed
            }
            Out::Backspace => {
                field.pop();
                Edit::Changed
            }
            Out::Enter => {
                if self.focus == Field::User {
                    self.focus = Field::Pass;
                    Edit::Changed
                } else {
                    Edit::Submit
                }
            }
            Out::Hide => {
                self.typing = false;
                Edit::Hide
            }
            Out::Named(_) => Edit::Changed,
        };
        self.render(ui);
        r
    }

    fn render(&mut self, ui: &Ui) {
        let fw = self.field_rect(ui, 0, 0, Field::User).2 - ui.pt(150.0);
        self.user_img = (!self.user.is_empty()).then(|| ui.text(&ui.fit(&self.user, 17.0, fw), 17.0, Weight::Regular, WHITE));
        let fit = (fw / ui.font.measure("•", ui.pt(BULLET_PT))) as usize;
        let n = self.pass.chars().count().min(fit);
        self.bullets = (n > 0).then(|| ui.text(&"•".repeat(n), BULLET_PT, Weight::Semibold, WHITE));
    }

    pub fn keyboard_top(&self, ui: &Ui, h: i32, kb_h: f32) -> Option<f32> {
        self.typing.then(|| h as f32 - crate::shell::ui::home_zone(ui) - kb_h)
    }

    fn card(&self, ui: &Ui, w: i32, h: i32, kb_h: f32) -> (f32, f32, f32, f32) {
        let (cw, ch) = (ui.pt(CARD_W_PT), ui.pt(CARD_H_PT));
        let (top, bottom) = (ui.pt(24.0), self.keyboard_top(ui, h, kb_h).unwrap_or(h as f32));
        ((w as f32 - cw) / 2.0, (top + (bottom - top - ch) / 2.0).round(), cw, ch)
    }

    fn cancel_rect(&self, ui: &Ui, w: i32, h: i32, kb_h: f32) -> (f32, f32, f32, f32) {
        let (x, y, _, _) = self.card(ui, w, h, kb_h);
        (x, y, ui.pt(BUTTON_W_PT), ui.pt(BAR_PT))
    }

    fn submit_rect(&self, ui: &Ui, w: i32, h: i32, kb_h: f32) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = self.card(ui, w, h, kb_h);
        (x + cw - ui.pt(BUTTON_W_PT), y, ui.pt(BUTTON_W_PT), ui.pt(BAR_PT))
    }

    fn field_rect(&self, ui: &Ui, w: i32, h: i32, f: Field) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = self.card(ui, w, h, 0.0);
        let row = if f == Field::User { 0.0 } else { 1.0 };
        (x + ui.pt(16.0), y + ui.pt(BAR_PT + 16.0) + row * ui.pt(FIELD_PT + 10.0), cw - ui.pt(32.0), ui.pt(FIELD_PT))
    }

    /// Centre of a field, for tests and taps.
    pub fn field_centre(&self, ui: &Ui, w: i32, h: i32, kb_h: f32, f: Field) -> (f32, f32) {
        let (x, y, fw, fh) = self.field_rect_kb(ui, w, h, kb_h, f);
        (x + fw / 2.0, y + fh / 2.0)
    }

    fn field_rect_kb(&self, ui: &Ui, w: i32, h: i32, kb_h: f32, f: Field) -> (f32, f32, f32, f32) {
        let (cx, cy, _, _) = self.card(ui, w, h, kb_h);
        let (x, y, fw, fh) = self.field_rect(ui, w, h, f);
        let (x0, y0, _, _) = self.card(ui, w, h, 0.0);
        (x - x0 + cx, y - y0 + cy, fw, fh)
    }

    pub fn button_centre(&self, ui: &Ui, w: i32, h: i32, kb_h: f32, submit: bool) -> (f32, f32) {
        let (x, y, bw, bh) = if submit { self.submit_rect(ui, w, h, kb_h) } else { self.cancel_rect(ui, w, h, kb_h) };
        (x + bw / 2.0, y + bh / 2.0)
    }

    pub fn hit(&self, ui: &Ui, w: i32, h: i32, kb_h: f32, x: f32, y: f32) -> Option<Target> {
        if self.keyboard_top(ui, h, kb_h).is_some_and(|top| y >= top) {
            Some(Target::Keyboard)
        } else if within(x, y, self.cancel_rect(ui, w, h, kb_h)) {
            Some(Target::Cancel)
        } else if within(x, y, self.submit_rect(ui, w, h, kb_h)) {
            Some(Target::Submit)
        } else if within(x, y, self.field_rect_kb(ui, w, h, kb_h, Field::User)) {
            Some(Target::Field(Field::User))
        } else if within(x, y, self.field_rect_kb(ui, w, h, kb_h, Field::Pass)) {
            Some(Target::Field(Field::Pass))
        } else {
            None
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, kb_h: f32) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 160));
        let (x, y, cw, ch) = self.card(ui, w, h, kb_h);
        s.fill_rrect(x, y, cw, ch, ui.pt(13.0), CELL);
        let cx = (x + cw / 2.0) as i32;
        centred(s, &self.prompt, cx, (y + ui.pt(20.0) - self.prompt.h as f32 / 2.0) as i32);
        let bar = y + ui.pt(52.0);
        let mid = |img: &Image| (bar - img.h as f32 / 2.0) as i32;
        centred(s, &self.title, cx, mid(&self.title));
        s.blit(&self.cancel, (x + ui.pt(16.0)) as i32, mid(&self.cancel), 255, 255);
        let submit = if self.can_submit() { &self.sign_in } else { &self.sign_in_off };
        s.blit(submit, (x + cw - ui.pt(16.0) - submit.w as f32) as i32, mid(submit), 255, 255);
        s.fill_rrect(x, y + ui.pt(BAR_PT) - 1.0, cw, 1.0, 0.0, SEPARATOR);
        for f in [Field::User, Field::Pass] {
            let (fx, fy, fw, fh) = self.field_rect_kb(ui, w, h, kb_h, f);
            s.fill_rrect(fx, fy, fw, fh, ui.pt(10.0), FIELD);
            let fmid = fy + fh / 2.0;
            let label = if f == Field::User { &self.user_label } else { &self.pass_label };
            s.blit(label, (fx + ui.pt(16.0)) as i32, (fmid - label.h as f32 / 2.0) as i32, 255, 255);
            let mut tx = fx + ui.pt(126.0);
            let value = if f == Field::User { &self.user_img } else { &self.bullets };
            if let Some(v) = value {
                s.blit(v, tx as i32, (fmid - v.h as f32 / 2.0) as i32, 255, 255);
                tx += v.w as f32 - ui.pt(1.0);
            }
            if self.typing && self.focus == f {
                let c = ui.pt(22.0);
                s.fill_rrect(tx, fmid - c / 2.0, ui.pt(2.0), c, ui.pt(1.0), BLUE);
            }
        }
        let (_, fy, _, fh) = self.field_rect_kb(ui, w, h, kb_h, Field::Pass);
        let note = if self.busy { Some(&self.busy_img) } else { self.error_img.as_ref() };
        if let Some(n) = note {
            centred(s, n, cx, (fy + fh + ui.pt(12.0)) as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    #[test]
    fn typing_moves_between_fields_and_submits() {
        let ui = ui();
        let mut l = Login::new(&ui);
        assert!(!l.can_submit());
        assert_eq!(l.edit(&ui, Out::Text("me".into())), Edit::Changed);
        assert_eq!(l.edit(&ui, Out::Enter), Edit::Changed, "return on the username moves on");
        assert_eq!(l.focus, Field::Pass);
        l.edit(&ui, Out::Text("pw\u{7}".into()));
        assert_eq!(l.pass, "pw", "control characters dropped");
        l.edit(&ui, Out::Backspace);
        assert_eq!(l.pass, "p");
        assert!(l.can_submit());
        assert_eq!(l.edit(&ui, Out::Enter), Edit::Submit);
        assert_eq!(l.edit(&ui, Out::Hide), Edit::Hide);
        assert!(!l.typing);
        l.edit(&ui, Out::Text("x".repeat(100)));
        assert_eq!(l.pass.chars().count(), MAX_LEN);
        l.busy = true;
        assert!(!l.can_submit());
        l.edit(&ui, Out::Backspace);
        assert_eq!(l.pass.chars().count(), MAX_LEN, "no edits while busy");
        l.set_error(&ui, Some("nope"));
        assert!(!l.busy && l.error.is_some());
    }

    #[test]
    fn hits_follow_the_keyboard() {
        let ui = ui();
        let mut l = Login::new(&ui);
        let kb = 500.0;
        let top = l.keyboard_top(&ui, 1920, kb).unwrap();
        assert_eq!(l.hit(&ui, 1200, 1920, kb, 600.0, top + 5.0), Some(Target::Keyboard));
        let (ux, uy) = l.field_centre(&ui, 1200, 1920, kb, Field::User);
        assert_eq!(l.hit(&ui, 1200, 1920, kb, ux, uy), Some(Target::Field(Field::User)));
        let (px, py) = l.field_centre(&ui, 1200, 1920, kb, Field::Pass);
        assert_eq!(l.hit(&ui, 1200, 1920, kb, px, py), Some(Target::Field(Field::Pass)));
        assert!(py > uy && py < top);
        let (cx, cy) = l.button_centre(&ui, 1200, 1920, kb, false);
        assert_eq!(l.hit(&ui, 1200, 1920, kb, cx, cy), Some(Target::Cancel));
        let (sx, sy) = l.button_centre(&ui, 1200, 1920, kb, true);
        assert_eq!(l.hit(&ui, 1200, 1920, kb, sx, sy), Some(Target::Submit));
        assert_eq!(l.hit(&ui, 1200, 1920, kb, 5.0, 5.0), None);
        l.typing = false;
        let (_, uy2) = l.field_centre(&ui, 1200, 1920, kb, Field::User);
        assert!(uy2 > uy, "the card recentres without the keyboard");
        assert_eq!(l.hit(&ui, 1200, 1920, kb, 600.0, top + 5.0), None);
        let mut buf = vec![0u32; 1200 * 1920];
        l.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, kb);
    }
}

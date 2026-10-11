//! The Accept / Decline prompt for an incoming nearby file.

use crate::gfx::Surface;
use crate::net::share::Offer;
use crate::shell::ui::{Ui, Weight};

const W_PT: f32 = 300.0;
const H_PT: f32 = 150.0;
const BTN_PT: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Accept,
    Decline,
}

pub struct Prompt {
    pub offer: Offer,
}

impl Prompt {
    pub fn new(offer: Offer) -> Self {
        Prompt { offer }
    }

    fn rect(ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (pw, ph) = (ui.pt(W_PT), ui.pt(H_PT));
        ((w as f32 - pw) / 2.0, (h as f32 - ph) / 2.0, pw, ph)
    }

    /// The two buttons, Decline left and Accept right.
    fn buttons(ui: &Ui, w: i32, h: i32) -> [(Choice, (f32, f32, f32, f32)); 2] {
        let (x, y, pw, ph) = Self::rect(ui, w, h);
        let (bh, half) = (ui.pt(BTN_PT), pw / 2.0);
        [(Choice::Decline, (x, y + ph - bh, half, bh)), (Choice::Accept, (x + half, y + ph - bh, half, bh))]
    }

    pub fn hit(ui: &Ui, w: i32, h: i32, px: f32, py: f32) -> Option<Choice> {
        Self::buttons(ui, w, h).into_iter().find(|(_, (x, y, bw, bh))| px >= *x && px < x + bw && py >= *y && py < y + bh).map(|(c, _)| c)
    }

    pub fn draw(&self, s: &mut Surface<'_>, ui: &Ui, w: i32, h: i32) {
        let (x, y, pw, ph) = Self::rect(ui, w, h);
        s.fill_rrect(x, y, pw, ph, ui.pt(6.0), 0xFF2C2C2E);
        let (title, body) = self.offer.banner();
        let pad = ui.pt(16.0);
        let t = ui.text(&ui.fit(&title, 17.0, pw - pad * 2.0), 17.0, Weight::Semibold, 0xFFFFFFFF);
        s.blit(&t, (x + pad) as i32, (y + pad) as i32, 255, 255);
        let b = ui.text(&ui.fit(&body, 15.0, pw - pad * 2.0), 15.0, Weight::Regular, 0xFF98989D);
        s.blit(&b, (x + pad) as i32, (y + pad + ui.pt(30.0)) as i32, 255, 255);
        for (c, (bx, by, bw, bh)) in Self::buttons(ui, w, h) {
            s.fill_rect(bx as usize, by as usize, bw as usize, 1, 0xFF48484A);
            let (label, color) = if c == Choice::Accept { ("Accept", 0xFF0A84FF) } else { ("Decline", 0xFFFF453A) };
            let l = ui.text(label, 17.0, Weight::Semibold, color);
            s.blit(&l, (bx + (bw - l.w as f32) / 2.0) as i32, (by + (bh - l.h as f32) / 2.0) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_hit_left_and_right() {
        let ui = crate::shell::tests::ui();
        let (x, y, pw, ph) = Prompt::rect(&ui, 1200, 1920);
        let by = y + ph - 5.0;
        assert_eq!(Prompt::hit(&ui, 1200, 1920, x + 5.0, by), Some(Choice::Decline));
        assert_eq!(Prompt::hit(&ui, 1200, 1920, x + pw - 5.0, by), Some(Choice::Accept));
        assert_eq!(Prompt::hit(&ui, 1200, 1920, x + 5.0, y + 5.0), None);
    }
}

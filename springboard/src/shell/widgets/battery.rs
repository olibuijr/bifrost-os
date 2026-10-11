//! The Batteries widget: a ring that fills with the charge level, the
//! percentage inside it and a bolt while charging.

use super::{put, surface, tile, Kind, PAD_PT};
use crate::gfx::{pack, rgba, Image};
use crate::shell::glyphs;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};
use crate::status;

pub const GREEN: u32 = pack(48, 209, 88, 255);
pub const RED: u32 = pack(255, 69, 58, 255);
const RING_R_PT: f32 = 42.0;
const RING_T_PT: f32 = 9.0;
const RING_CY_PT: f32 = 68.0;

pub fn key(level: u8, charging: bool) -> String {
    format!("battery:{level}:{charging}")
}

pub fn render(ui: &Ui, level: u8, charging: bool) -> Image {
    let s = ui.s;
    let (w, h) = Kind::Battery.size(s);
    let mut img = tile(s, w, h);
    let c = (w as f32 / 2.0, ui.pt(RING_CY_PT));
    let (r, t) = (ui.pt(RING_R_PT), ui.pt(RING_T_PT));
    let bbox = (c.0 - r - t, c.1 - r - t, 2.0 * (r + t), 2.0 * (r + t));
    let color = if charging || level > 20 { GREEN } else { RED };
    {
        let mut sf = surface(&mut img);
        glyphs::paint(&mut sf, bbox, rgba(255, 255, 255, 46), glyphs::ring(c, r, t));
        let sweep = level.min(100) as f32 / 100.0 * std::f32::consts::TAU;
        if level >= 100 {
            glyphs::paint(&mut sf, bbox, color, glyphs::ring(c, r, t));
        } else if level > 0 {
            glyphs::paint(&mut sf, bbox, color, glyphs::arc(c, r, t, 0.0, sweep));
        }
        if charging {
            let (bw, bh) = (ui.pt(9.0), ui.pt(16.0));
            status::draw_bolt(&mut sf, c.0 - bw / 2.0, c.1 - r * 0.62 - bh / 2.0, bw, bh, WHITE);
        }
    }
    let pct = ui.text(&format!("{level}%"), 20.0, Weight::Semibold, WHITE);
    put(&mut img, &pct, c.0 - pct.w as f32 / 2.0, c.1 - pct.h as f32 / 2.0 + if charging { ui.pt(8.0) } else { 0.0 });
    let label = ui.text("bifrost", 12.0, Weight::Medium, SECONDARY);
    put(&mut img, &label, c.0 - label.w as f32 / 2.0, h as f32 - ui.pt(PAD_PT) - label.h as f32);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_fills_clockwise_and_turns_red_when_low() {
        let ui = crate::shell::tests::ui();
        let img = render(&ui, 50, false);
        let (w, _) = Kind::Battery.size(ui.s);
        let c = (w as f32 / 2.0, ui.pt(RING_CY_PT));
        let at = |dx: f32, dy: f32| img.px[(c.1 + dy) as usize * w + (c.0 + dx) as usize];
        let r = ui.pt(RING_R_PT);
        assert_eq!(at(r, 0.0), GREEN, "3 o'clock is inside the first half");
        assert_ne!(at(-r, 0.0), GREEN, "9 o'clock is only the track");
        // 10% sweeps to 36 degrees; probe the ring at 18 degrees past 12
        let early = |img: &Image| img.px[(c.1 - r * 0.951) as usize * w + (c.0 + r * 0.309) as usize];
        assert_eq!(early(&render(&ui, 10, false)), RED);
        assert_eq!(early(&render(&ui, 10, true)), GREEN, "charging is never red");
        assert_ne!(key(10, true), key(10, false));
    }
}

//! Glyph tiles for the rows of Settings detail pages: a coloured rounded square with a small white drawing, built from
//! our own shapes (lines, arcs, discs, polygons). They are separate from `icons::Icon`, which is the sidebar's set with
//! bitmap artwork. A tile can be drawn at any side length; every coordinate below is a fraction of the side.
use super::icons::{arc, disc, line, polygon};
use super::ui::Ui;
use crate::gfx::{pack, Surface};

const WHITE: u32 = pack(255, 255, 255, 255);
const TILE_GRAY: u32 = pack(142, 142, 147, 255);
const TILE_BLUE: u32 = pack(10, 132, 255, 255);
const TILE_GREEN: u32 = pack(52, 199, 89, 255);
const TILE_ORANGE: u32 = pack(255, 159, 10, 255);
const TILE_PURPLE: u32 = pack(175, 82, 222, 255);
const TILE_RED: u32 = pack(255, 69, 58, 255);
const TILE_TEAL: u32 = pack(50, 173, 230, 255);
const TILE_INDIGO: u32 = pack(94, 92, 230, 255);

/// Corner radius of a tile as a fraction of its side.
const CORNER: f32 = 0.22;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    About,
    Update,
    Storage,
    Refresh,
    Nearby,
    DateTime,
    Keyboard,
    Fonts,
    Language,
    Dictionary,
    DefaultApps,
    Backup,
    Reset,
    Info,
    /// Focus modes: Work, Personal, Sleep and any mode of the person's own (Do Not Disturb uses the Focus tile)
    ModeWork,
    ModePersonal,
    ModeSleep,
    ModeOther,
    /// the Energy pane of a computer without a battery: a lightning bolt
    Energy,
    /// an accent-colour swatch: the index into `Accent::ALL`
    Swatch(u8),
}

impl Glyph {
    pub const ALL: [Glyph; 19] = [
        Glyph::About,
        Glyph::Update,
        Glyph::Storage,
        Glyph::Refresh,
        Glyph::Nearby,
        Glyph::DateTime,
        Glyph::Keyboard,
        Glyph::Fonts,
        Glyph::Language,
        Glyph::Dictionary,
        Glyph::DefaultApps,
        Glyph::Backup,
        Glyph::Reset,
        Glyph::Info,
        Glyph::ModeWork,
        Glyph::ModePersonal,
        Glyph::ModeSleep,
        Glyph::ModeOther,
        Glyph::Energy,
    ];

    /// The tile's background colour.
    pub fn color(self) -> u32 {
        match self {
            Glyph::About | Glyph::Storage | Glyph::Keyboard | Glyph::Info => TILE_GRAY,
            Glyph::Update | Glyph::Reset => TILE_GRAY,
            Glyph::Refresh | Glyph::Language => TILE_BLUE,
            Glyph::Nearby => TILE_TEAL,
            Glyph::DateTime => TILE_BLUE,
            Glyph::Fonts => TILE_INDIGO,
            Glyph::Dictionary => TILE_ORANGE,
            Glyph::ModeWork => TILE_BLUE,
            Glyph::ModePersonal => TILE_PURPLE,
            Glyph::ModeSleep => TILE_TEAL,
            Glyph::ModeOther => TILE_INDIGO,
            Glyph::Energy => TILE_GREEN,
            Glyph::DefaultApps => TILE_PURPLE,
            Glyph::Backup => TILE_GREEN,
            Glyph::Swatch(i) => bifrost_render::appearance::Accent::ALL[(i as usize).min(bifrost_render::appearance::Accent::ALL.len() - 1)].colours().0,
        }
    }
}

/// `arc` over a sweep that may run past pi: the shared `arc` takes angles in -pi..pi, so a long sweep is two pieces.
fn sweep(s: &mut Surface, cx: f32, cy: f32, r: f32, t: f32, a0: f32, a1: f32, color: u32) {
    use std::f32::consts::{PI, TAU};
    debug_assert!((-PI..=PI).contains(&a0) && a1 <= a0 + TAU + 0.001);
    if a1 <= PI {
        arc(s, cx, cy, r, t, a0, a1, color);
    } else {
        arc(s, cx, cy, r, t, a0, PI, color);
        arc(s, cx, cy, r, t, -PI, a1 - TAU, color);
    }
}

/// Draws `glyph` with its top-left at (x, y) px, `side_pt` points wide.
pub fn tile(s: &mut Surface, ui: &Ui, glyph: Glyph, x: f32, y: f32, side_pt: f32) {
    let side = ui.pt(side_pt);
    let bg = glyph.color();
    if let Glyph::Swatch(_) = glyph {
        disc(s, x + side / 2.0, y + side / 2.0, side * 0.36, bg);
        return;
    }
    s.fill_rrect(x, y, side, side, side * CORNER, bg);
    let p = |fx: f32, fy: f32| (x + side * fx, y + side * fy);
    let t = (side * 0.075).max(1.0);
    let r = |f: f32| side * f;
    match glyph {
        Glyph::About => {
            // a computer: a screen on a stand
            let (sx, sy) = p(0.2, 0.28);
            s.fill_rrect(sx, sy, r(0.6), r(0.4), r(0.05), WHITE);
            let (ix, iy) = p(0.25, 0.33);
            s.fill_rrect(ix, iy, r(0.5), r(0.3), r(0.025), bg);
            line(s, p(0.5, 0.68), p(0.5, 0.75), t, WHITE);
            line(s, p(0.36, 0.76), p(0.64, 0.76), t, WHITE);
        }
        Glyph::Info => {
            let (cx, cy) = p(0.5, 0.5);
            sweep(s, cx, cy, r(0.27), t, 0.0, std::f32::consts::TAU, WHITE);
            let (dx, dy) = p(0.5, 0.36);
            disc(s, dx, dy, t * 0.7, WHITE);
            line(s, p(0.5, 0.47), p(0.5, 0.65), t, WHITE);
        }
        Glyph::Update | Glyph::Refresh | Glyph::Reset => {
            let (cx, cy) = p(0.5, 0.5);
            sweep(s, cx, cy, r(0.26), t, 0.6, 5.5, WHITE);
            let (tx, ty) = (cx + r(0.26) * 0.6f32.cos(), cy + r(0.26) * 0.6f32.sin());
            polygon(s, &[(tx - r(0.1), ty - r(0.02)), (tx + r(0.08), ty - r(0.1)), (tx + r(0.06), ty + r(0.1))], WHITE);
        }
        Glyph::Storage => {
            let (bx, by) = p(0.2, 0.42);
            s.fill_rrect(bx, by, r(0.6), r(0.24), r(0.05), WHITE);
            let (lx, ly) = p(0.7, 0.54);
            disc(s, lx, ly, r(0.026), bg);
            line(s, p(0.28, 0.54), p(0.5, 0.54), t * 0.7, bg);
        }
        Glyph::Nearby => {
            let (cx, cy) = p(0.5, 0.58);
            for k in 1..=3 {
                arc(s, cx, cy, r(0.1 * k as f32), t * 0.8, -2.4, -0.74, WHITE);
            }
            disc(s, cx, cy, r(0.03), WHITE);
        }
        Glyph::DateTime => {
            let (bx, by) = p(0.22, 0.26);
            s.fill_rrect(bx, by, r(0.56), r(0.5), r(0.06), WHITE);
            let (hx, hy) = p(0.22, 0.26);
            s.fill_rrect(hx, hy, r(0.56), r(0.12), r(0.05), TILE_RED);
            for row in 0..2 {
                for col in 0..3 {
                    let (dx, dy) = p(0.31 + 0.15 * col as f32, 0.5 + 0.14 * row as f32);
                    s.fill_rrect(dx, dy, r(0.07), r(0.07), r(0.015), bg);
                }
            }
        }
        Glyph::Keyboard => {
            let (bx, by) = p(0.17, 0.33);
            s.fill_rrect(bx, by, r(0.66), r(0.34), r(0.06), WHITE);
            for row in 0..2 {
                for col in 0..4 {
                    let (kx, ky) = p(0.23 + 0.14 * col as f32, 0.39 + 0.1 * row as f32);
                    s.fill_rrect(kx, ky, r(0.09), r(0.06), r(0.012), bg);
                }
            }
            let (sx, sy) = p(0.3, 0.57);
            s.fill_rrect(sx, sy, r(0.4), r(0.05), r(0.012), bg);
        }
        Glyph::Fonts => {
            line(s, p(0.5, 0.24), p(0.3, 0.74), t * 1.1, WHITE);
            line(s, p(0.5, 0.24), p(0.7, 0.74), t * 1.1, WHITE);
            line(s, p(0.38, 0.55), p(0.62, 0.55), t, WHITE);
        }
        Glyph::Language => {
            let (cx, cy) = p(0.5, 0.5);
            sweep(s, cx, cy, r(0.27), t * 0.9, 0.0, std::f32::consts::TAU, WHITE);
            line(s, p(0.23, 0.5), p(0.77, 0.5), t * 0.8, WHITE);
            line(s, p(0.5, 0.23), p(0.5, 0.77), t * 0.8, WHITE);
            arc(s, cx, cy, r(0.27), t * 0.8, -0.9, 0.9, WHITE);
        }
        Glyph::Dictionary => {
            let (lx, ly) = p(0.2, 0.28);
            s.fill_rrect(lx, ly, r(0.28), r(0.44), r(0.04), WHITE);
            let (rx, ry) = p(0.52, 0.28);
            s.fill_rrect(rx, ry, r(0.28), r(0.44), r(0.04), WHITE);
            line(s, p(0.26, 0.4), p(0.42, 0.4), t * 0.6, bg);
            line(s, p(0.58, 0.4), p(0.74, 0.4), t * 0.6, bg);
        }
        Glyph::DefaultApps => {
            for row in 0..2 {
                for col in 0..2 {
                    let (gx, gy) = p(0.25 + 0.27 * col as f32, 0.25 + 0.27 * row as f32);
                    s.fill_rrect(gx, gy, r(0.22), r(0.22), r(0.05), WHITE);
                }
            }
        }
        Glyph::Backup => {
            let (cx, cy) = p(0.5, 0.5);
            sweep(s, cx, cy, r(0.26), t, 0.9, 5.6, WHITE);
            line(s, p(0.5, 0.5), p(0.5, 0.34), t * 0.9, WHITE);
            line(s, p(0.5, 0.5), p(0.62, 0.56), t * 0.9, WHITE);
        }
        Glyph::ModeWork => {
            // a briefcase: a handle over a body with a clasp band
            let (bx, by) = p(0.22, 0.38);
            s.fill_rrect(bx, by, r(0.56), r(0.36), r(0.05), WHITE);
            line(s, p(0.4, 0.36), p(0.4, 0.3), t, WHITE);
            line(s, p(0.4, 0.3), p(0.6, 0.3), t, WHITE);
            line(s, p(0.6, 0.3), p(0.6, 0.36), t, WHITE);
            line(s, p(0.22, 0.54), p(0.78, 0.54), t * 0.6, bg);
        }
        Glyph::ModePersonal => {
            let (hx, hy) = p(0.5, 0.37);
            disc(s, hx, hy, r(0.1), WHITE);
            let (bx, by) = p(0.29, 0.53);
            s.fill_rrect(bx, by, r(0.42), r(0.22), r(0.11), WHITE);
        }
        Glyph::ModeSleep => {
            // a bed: a headboard, a mattress and a pillow
            line(s, p(0.22, 0.33), p(0.22, 0.68), t, WHITE);
            let (mx, my) = p(0.22, 0.5);
            s.fill_rrect(mx, my, r(0.56), r(0.13), r(0.03), WHITE);
            let (px, py) = p(0.28, 0.41);
            s.fill_rrect(px, py, r(0.14), r(0.07), r(0.03), WHITE);
            line(s, p(0.78, 0.68), p(0.78, 0.6), t, WHITE);
        }
        Glyph::ModeOther => {
            let (cx, cy) = p(0.5, 0.5);
            sweep(s, cx, cy, r(0.22), t, 0.0, std::f32::consts::TAU, WHITE);
            disc(s, cx, cy, r(0.07), WHITE);
        }
        Glyph::Energy => {
            polygon(s, &[p(0.56, 0.2), p(0.3, 0.55), p(0.47, 0.55), p(0.43, 0.8), p(0.7, 0.44), p(0.52, 0.44)], WHITE);
        }
        Glyph::Swatch(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Image;

    #[test]
    fn every_glyph_draws_inside_its_tile_and_leaves_the_rest_alone() {
        let ui = crate::shell::tests::ui();
        for g in Glyph::ALL {
            let mut img = Image::new(80, 80);
            {
                let mut s = Surface { w: 80, h: 80, stride: 80, px: &mut img.px, y0: 0 };
                tile(&mut s, &ui, g, 20.0, 20.0, 23.0);
            }
            let side = ui.pt(23.0).ceil() as usize;
            let painted = img.px.iter().filter(|p| **p >> 24 != 0).count();
            assert!(painted > side * side / 3, "{g:?} paints its tile");
            // nothing outside the tile's box (plus one px of anti-aliasing)
            for y in 0..80usize {
                for x in 0..80usize {
                    let outside = x + 1 < 20 || y + 1 < 20 || x > 20 + side + 1 || y > 20 + side + 1;
                    assert!(!outside || img.px[y * 80 + x] >> 24 == 0, "{g:?} stays inside at {x},{y}");
                }
            }
        }
    }
}

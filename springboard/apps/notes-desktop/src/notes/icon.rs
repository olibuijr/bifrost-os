//! The bifrost Notes app icon, drawn procedurally (the bifrost's icon bundle was
//! not pulled): an off-white page with a yellow band across the top, a row
//! of perforation holes under it and three ruled lines.

use springboard::gfx::{pack, Image, Surface};
use springboard::shell::glyphs::{disc, paint, segment};

// compliance-allow: COL-01 app icon artwork (the bifrost Notes palette), not UI chrome
const PAPER: u32 = pack(246, 245, 240, 255);
// compliance-allow: COL-01 app icon artwork (the bifrost Notes palette), not UI chrome
const BAND: u32 = pack(255, 212, 48, 255);
// compliance-allow: COL-01 app icon artwork (the bifrost Notes palette), not UI chrome
const HOLE: u32 = pack(205, 205, 200, 255);
// compliance-allow: COL-01 app icon artwork (the bifrost Notes palette), not UI chrome
const RULE: u32 = pack(190, 190, 186, 255);

/// The icon at `size` px, square and unmasked (the shell applies the
/// squircle like it does to packed icons).
pub fn icon(size: usize) -> Image {
    let mut img = Image { w: size, h: size, px: vec![PAPER; size * size] };
    let f = size as f32;
    let mut s = Surface { w: size, h: size, stride: size, px: &mut img.px, y0: 0 };
    s.fill_rect(0, 0, size, (f * 0.235).round() as usize, BAND);
    let holes = 7;
    let hole_y = f * 0.235 + f * 0.045;
    for i in 0..holes {
        let cx = f * (0.5 + (i as f32 - (holes - 1) as f32 / 2.0) * 0.113);
        paint(&mut s, (cx - f * 0.03, hole_y - f * 0.03, f * 0.06, f * 0.06), HOLE, disc((cx, hole_y), f * 0.018));
    }
    let t = f * 0.045;
    for (i, len) in [0.68, 0.68, 0.44].iter().enumerate() {
        let y = f * (0.47 + 0.155 * i as f32);
        let (x0, x1) = (f * 0.16, f * (0.16 + len));
        paint(&mut s, (x0 - t, y - t, x1 - x0 + 2.0 * t, 2.0 * t), RULE, segment((x0, y), (x1, y), t));
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_holes_and_rules_land_where_expected() {
        let n = 100;
        let i = icon(n);
        let at = |x: usize, y: usize| i.px[y * n + x];
        assert_eq!(at(50, 5), BAND, "yellow band on top");
        assert_eq!(at(50, 95), PAPER, "paper below");
        assert_eq!(at(50, 47), RULE, "first rule across the middle");
        assert_eq!(at(70, 78), PAPER, "the short third rule stops early");
        assert_eq!(at(20, 78), RULE);
        assert!(at(50, 28) != PAPER && at(50, 28) != BAND, "a perforation hole under the band");
        assert!(i.px.iter().all(|&p| p >> 24 == 255), "opaque");
    }
}

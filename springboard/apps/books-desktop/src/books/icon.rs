//! The Books app icon, drawn procedurally: an open book in white on an
//! orange tile, in the spirit of the bifrost icon.

use springboard::gfx::{Image, Surface};
use springboard::shell::glyphs::{paint, segment};
use bifrost_app::kit::ui::{BLACK, SYSTEM_ORANGE, WHITE};
use super::fade;

const TILE: u32 = SYSTEM_ORANGE;
const PAGE: u32 = WHITE;
const SPINE: u32 = fade(BLACK, 46);
const RULE: u32 = fade(SYSTEM_ORANGE, 150);

/// The icon at `size` px, square and unmasked (the shell applies the squircle).
pub fn icon(size: usize) -> Image {
    let mut img = Image { w: size, h: size, px: vec![TILE; size * size] };
    let f = size as f32;
    let mut s = Surface { w: size, h: size, stride: size, px: &mut img.px, y0: 0 };
    let (top, bottom) = (f * 0.27, f * 0.74);
    let (left, mid, right) = (f * 0.16, f * 0.5, f * 0.84);
    // two pages, each slightly dipping toward the spine
    s.fill_rrect(left, top, mid - left - f * 0.012, bottom - top, f * 0.02, PAGE);
    s.fill_rrect(mid + f * 0.012, top, right - mid - f * 0.012, bottom - top, f * 0.02, PAGE);
    let t = f * 0.012;
    paint(&mut s, (mid - t * 2.0, top, t * 4.0, bottom - top), SPINE, segment((mid, top), (mid, bottom), t));
    for i in 0..4 {
        let y = top + f * (0.08 + 0.095 * i as f32);
        for (x0, x1) in [(left + f * 0.05, mid - f * 0.06), (mid + f * 0.06, right - f * 0.05)] {
            let x1 = if i == 3 { x0 + (x1 - x0) * 0.6 } else { x1 };
            paint(&mut s, (x0 - t, y - t, x1 - x0 + 2.0 * t, 2.0 * t), RULE, segment((x0, y), (x1, y), t));
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_spine_and_tile() {
        let n = 100;
        let i = icon(n);
        let at = |x: usize, y: usize| i.px[y * n + x];
        assert_eq!(at(3, 3), TILE, "tile corner");
        assert_eq!(at(30, 31), PAGE, "left page above the rules");
        assert_eq!(at(70, 31), PAGE, "right page");
        assert_ne!(at(50, 50), PAGE, "the spine splits the pages");
        assert!(i.px.iter().all(|&p| p >> 24 == 255), "opaque");
    }
}

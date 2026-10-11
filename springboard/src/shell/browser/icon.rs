//! The Browser app icon, drawn procedurally: a blue gradient square with a
//! white compass ring and a red and white needle, unmasked (the shell
//! applies the squircle).

use crate::gfx::{pack, Image, Surface};
use crate::shell::glyphs::{disc, either, paint, ring, segment};

const TOP: (u32, u32, u32) = (64, 170, 255);
const BOTTOM: (u32, u32, u32) = (0, 98, 214);
const WHITE: u32 = pack(255, 255, 255, 255);
const RED: u32 = pack(255, 59, 48, 255); // WebBrowser compass needle (matches bifrost-ref icon)

pub fn icon(size: usize) -> Image {
    let mut img = Image { w: size, h: size, px: vec![0; size * size] };
    for y in 0..size {
        let k = y as f32 / size.max(1) as f32;
        let mix = |a: u32, b: u32| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
        let row = pack(mix(TOP.0, BOTTOM.0), mix(TOP.1, BOTTOM.1), mix(TOP.2, BOTTOM.2), 255);
        img.px[y * size..(y + 1) * size].fill(row);
    }
    let f = size as f32;
    let c = (f / 2.0, f / 2.0);
    let r = f * 0.36;
    let mut s = Surface { w: size, h: size, stride: size, px: &mut img.px, y0: 0 };
    let t = f * 0.035;
    paint(&mut s, (c.0 - r - t, c.1 - r - t, 2.0 * (r + t), 2.0 * (r + t)), WHITE, ring(c, r, t));
    // tick marks at the four points
    for (dx, dy) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
        let (a, b) = ((c.0 + dx * r * 0.82, c.1 + dy * r * 0.82), (c.0 + dx * r * 0.96, c.1 + dy * r * 0.96));
        paint(&mut s, (a.0.min(b.0) - t, a.1.min(b.1) - t, (a.0 - b.0).abs() + 2.0 * t, (a.1 - b.1).abs() + 2.0 * t), WHITE, segment(a, b, t));
    }
    // the needle runs south-west to north-east; the north-east half is red
    let n = r * 0.6;
    let (ne, sw) = ((c.0 + n, c.1 - n), (c.0 - n, c.1 + n));
    let w = f * 0.09;
    let bbox = (c.0 - n - w, c.1 - n - w, 2.0 * (n + w), 2.0 * (n + w));
    paint(&mut s, bbox, RED, segment(c, ne, w));
    paint(&mut s, bbox, WHITE, either(segment(c, sw, w), disc(c, w * 0.55)));
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_ring_and_needle_land_where_expected() {
        let n = 100;
        let i = icon(n);
        let at = |x: usize, y: usize| i.px[y * n + x];
        let (top, bottom) = (at(2, 2), at(2, 97));
        assert!((top & 0xff) < (bottom & 0xff) + 100 && top != bottom, "vertical gradient");
        assert_eq!(at(50, 14) & 0xff, 255, "ring top is white");
        assert_eq!(at(60, 40), RED, "needle's north-east half is red");
        assert_eq!(at(42, 58), WHITE, "south-west half is white");
        assert!(i.px.iter().all(|&p| p >> 24 == 255), "opaque");
    }
}

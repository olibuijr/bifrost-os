//! The vector icons of the Browser's toolbar and tab bar, built from the
//! shell's signed-distance shapes (the SF symbol font has no glyph for
//! these). Every function draws centred on `(cx, cy)`; `t` is the stroke
//! width in px.

use crate::gfx::Surface;
use crate::shell::glyphs::{arc, paint, segment};

fn line(s: &mut Surface, a: (f32, f32), b: (f32, f32), t: f32, color: u32) {
    let bbox = (a.0.min(b.0) - t, a.1.min(b.1) - t, (a.0 - b.0).abs() + 2.0 * t, (a.1 - b.1).abs() + 2.0 * t);
    paint(s, bbox, color, segment(a, b, t));
}

fn polyline(s: &mut Surface, pts: &[(f32, f32)], closed: bool, t: f32, color: u32) {
    for w in pts.windows(2) {
        line(s, w[0], w[1], t, color);
    }
    if closed && pts.len() > 2 {
        line(s, pts[pts.len() - 1], pts[0], t, color);
    }
}

/// A back (`left`) or forward chevron, `r` tall from the centre.
pub fn chevron(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32, left: bool) {
    let d = if left { -1.0 } else { 1.0 };
    polyline(s, &[(cx - d * r * 0.5, cy - r), (cx + d * r * 0.5, cy), (cx - d * r * 0.5, cy + r)], false, t, color);
}

/// Reload: an almost closed circle with an arrowhead on its end.
pub fn reload(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    let (a0, a1) = (0.9f32, 5.9f32);
    let bbox = (cx - r - t, cy - r - t, 2.0 * (r + t), 2.0 * (r + t));
    paint(s, bbox, color, arc((cx, cy), r, t, a0, a1));
    let tip = (cx + r * a1.sin(), cy - r * a1.cos());
    // travel direction at the tip is the tangent (cos a, sin a); the wings lean back from it
    let dir = (a1.cos(), a1.sin());
    for ang in [2.6f32, -2.6] {
        let (c, sn) = (ang.cos(), ang.sin());
        let wing = (dir.0 * c - dir.1 * sn, dir.0 * sn + dir.1 * c);
        line(s, tip, (tip.0 + wing.0 * r * 0.7, tip.1 + wing.1 * r * 0.7), t, color);
    }
}

/// An X: stop loading, close a tab.
pub fn cross(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    line(s, (cx - r, cy - r), (cx + r, cy + r), t, color);
    line(s, (cx - r, cy + r), (cx + r, cy - r), t, color);
}

pub fn plus(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    line(s, (cx - r, cy), (cx + r, cy), t, color);
    line(s, (cx, cy - r), (cx, cy + r), t, color);
}

/// The ten vertices of a five-pointed star, outer then inner, starting at the top.
pub fn star_points((cx, cy): (f32, f32), r: f32) -> [(f32, f32); 10] {
    std::array::from_fn(|i| {
        let rad = if i % 2 == 0 { r } else { r * 0.42 };
        let a = std::f32::consts::PI * i as f32 / 5.0;
        (cx + rad * a.sin(), cy - rad * a.cos())
    })
}

/// A star outline; `filled` also fills it in.
pub fn star(s: &mut Surface, c: (f32, f32), r: f32, t: f32, color: u32, filled: bool) {
    let pts = star_points(c, r);
    if filled {
        for i in (0..10).step_by(2) {
            line(s, c, pts[i], r * 0.5, color);
        }
        for i in (1..10).step_by(2) {
            line(s, c, pts[i], r * 0.7, color);
        }
    }
    polyline(s, &pts, true, t, color);
}

/// An open book: the library of bookmarks and history.
pub fn book(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    let (w, h) = (r * 1.2, r * 0.8);
    polyline(s, &[(cx - w, cy - h), (cx, cy - h * 0.6), (cx + w, cy - h), (cx + w, cy + h), (cx, cy + h * 1.2), (cx - w, cy + h)], true, t, color);
    line(s, (cx, cy - h * 0.6), (cx, cy + h * 1.2), t, color);
}

/// The tab overview button: two overlapping rounded squares; `n` is unused
/// by the outline but kept so a count badge can follow.
pub fn squares(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32, _n: usize) {
    let q = r * 0.8;
    polyline(s, &[(cx - r, cy - r + q * 0.4), (cx - r, cy + r), (cx + q * 0.4, cy + r), (cx + q * 0.4, cy - r + q * 0.4)], true, t, color);
    polyline(s, &[(cx - q * 0.4, cy - r), (cx + r, cy - r), (cx + r, cy + q * 0.4 + 0.0)], false, t, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: u32 = 0xffff_ffff;

    fn inked(draw: impl Fn(&mut Surface)) -> (usize, Vec<u32>) {
        let mut px = vec![0u32; 60 * 60];
        let mut s = Surface { w: 60, h: 60, stride: 60, px: &mut px, y0: 0 };
        draw(&mut s);
        (px.iter().filter(|&&p| p != 0).count(), px)
    }

    #[test]
    fn every_icon_draws_something_inside_its_box() {
        let c = (30.0, 30.0);
        for (name, n) in [
            ("back", inked(|s| chevron(s, c, 10.0, 3.0, INK, true)).0),
            ("forward", inked(|s| chevron(s, c, 10.0, 3.0, INK, false)).0),
            ("reload", inked(|s| reload(s, c, 10.0, 3.0, INK)).0),
            ("cross", inked(|s| cross(s, c, 8.0, 3.0, INK)).0),
            ("plus", inked(|s| plus(s, c, 8.0, 3.0, INK)).0),
            ("star", inked(|s| star(s, c, 12.0, 2.0, INK, false)).0),
            ("book", inked(|s| book(s, c, 10.0, 2.0, INK)).0),
        ] {
            assert!(n > 20, "{name} inked {n}");
        }
    }

    #[test]
    fn chevrons_point_opposite_ways() {
        let (_, left) = inked(|s| chevron(s, (30.0, 30.0), 10.0, 3.0, INK, true));
        let (_, right) = inked(|s| chevron(s, (30.0, 30.0), 10.0, 3.0, INK, false));
        let col = |px: &Vec<u32>, x: usize| (0..60).filter(|&y| px[y * 60 + x] != 0).count();
        assert!(col(&left, 25) > 0 && col(&right, 25) > 0);
        let tip = |px: &Vec<u32>| (0..60).find(|&x| px[30 * 60 + x] != 0).unwrap();
        assert!(tip(&left) < tip(&right), "the back chevron's tip is on the left");
    }

    #[test]
    fn a_filled_star_covers_more_than_its_outline() {
        let (outline, _) = inked(|s| star(s, (30.0, 30.0), 14.0, 2.0, INK, false));
        let (filled, px) = inked(|s| star(s, (30.0, 30.0), 14.0, 2.0, INK, true));
        assert!(filled > outline * 2);
        assert_ne!(px[30 * 60 + 30], 0, "the middle is solid");
    }

    #[test]
    fn star_points_alternate_outer_and_inner_radii() {
        let p = star_points((0.0, 0.0), 10.0);
        assert!((p[0].1 + 10.0).abs() < 1e-3 && p[0].0.abs() < 1e-3, "starts at the top");
        let r = |q: (f32, f32)| q.0.hypot(q.1);
        assert!((r(p[2]) - 10.0).abs() < 1e-3 && (r(p[1]) - 4.2).abs() < 1e-3);
    }
}

//! Procedural transport glyphs (play, pause, skip, shuffle, repeat), the
//! placeholder artwork note and the Music app icon, drawn from distance
//! functions and polygons so no symbol font is needed.

use super::super::glyphs::{arc, disc, either, paint, segment};
use crate::gfx::{over, pack, rgba, scale, squircle_mask, Image, Surface};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Music red.
pub const MUSIC_RED: u32 = pack(250, 36, 60, 255);

/// Fills a convex or concave polygon with the even-odd rule, antialiased by `bifrost_ui::raster`.
pub fn fill_poly(s: &mut Surface, pts: &[(f32, f32)], color: u32) {
    if pts.len() < 3 || pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return;
    }
    let (x0, x1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let (ox, oy) = (x0.floor().max(0.0) as usize, y0.floor().max(0.0) as usize);
    let (ex, ey) = ((x1.ceil().max(0.0) as usize).min(s.w), (y1.ceil().max(0.0) as usize).min(s.h));
    if ex <= ox || ey <= oy {
        return;
    }
    let local: Vec<(f32, f32)> = pts.iter().map(|p| (p.0 - ox as f32, p.1 - oy as f32)).collect();
    let cov = bifrost_ui::raster::fill_polygons(&[local], bifrost_ui::raster::FillRule::EvenOdd, ex - ox, ey - oy);
    for py in oy.max(s.y0)..ey {
        for px in ox..ex {
            let c = cov.at(px - ox, py - oy);
            if c != 0 {
                let d = s.at(px, py);
                *d = over(*d, scale(color, c as u32));
            }
        }
    }
}

/// A heart `size` px wide: two lobes over a point.
pub fn heart(s: &mut Surface, (cx, cy): (f32, f32), size: f32, color: u32) {
    let (r, dx) = (size * 0.27, size * 0.23);
    let (lobe_y, edge) = (cy - size * 0.14, size * 0.485);
    paint(s, (cx - size / 2.0, cy - size / 2.0, size, size), color, either(disc((cx - dx, lobe_y), r), disc((cx + dx, lobe_y), r)));
    fill_poly(s, &[(cx - edge, lobe_y + r * 0.35), (cx + edge, lobe_y + r * 0.35), (cx, cy + size * 0.46)], color);
}

/// A right-pointing triangle `size` px tall centred on (cx, cy).
pub fn play(s: &mut Surface, (cx, cy): (f32, f32), size: f32, color: u32) {
    let (h, w) = (size / 2.0, size * 0.46);
    fill_poly(s, &[(cx - w, cy - h), (cx + w, cy), (cx - w, cy + h)], color);
}

/// Two bars `size` px tall.
pub fn pause(s: &mut Surface, (cx, cy): (f32, f32), size: f32, color: u32) {
    let (bw, gap) = (size * 0.3, size * 0.14);
    s.fill_rrect(cx - gap - bw, cy - size / 2.0, bw, size, size * 0.06, color);
    s.fill_rrect(cx + gap, cy - size / 2.0, bw, size, size * 0.06, color);
}

/// Skip forward (two triangles) or back (`forward` false), `size` px tall.
pub fn skip(s: &mut Surface, (cx, cy): (f32, f32), size: f32, forward: bool, color: u32) {
    let (h, w) = (size / 2.0, size * 0.52);
    let dir = if forward { 1.0 } else { -1.0 };
    let tri = |x0: f32| [(cx + dir * x0, cy - h), (cx + dir * (x0 + w), cy), (cx + dir * x0, cy + h)];
    fill_poly(s, &tri(-w), color);
    fill_poly(s, &tri(0.0), color);
}

/// The transport glyph for a state: play or pause.
pub fn play_pause(s: &mut Surface, c: (f32, f32), size: f32, playing: bool, color: u32) {
    if playing {
        pause(s, c, size, color);
    } else {
        play(s, c, size, color);
    }
}

/// A speaker `size` px tall with `waves` sound arcs (0..=2) to its right.
pub fn speaker(s: &mut Surface, (cx, cy): (f32, f32), size: f32, waves: u8, color: u32) {
    let (h, bw) = (size / 2.0, size * 0.28);
    let x = cx - size * 0.5;
    fill_poly(s, &[(x, cy - h * 0.45), (x + bw, cy - h * 0.45), (x + bw * 2.2, cy - h), (x + bw * 2.2, cy + h), (x + bw, cy + h * 0.45), (x, cy + h * 0.45)], color);
    let t = (size * 0.1).max(1.0);
    let ax = x + bw * 2.2;
    for i in 0..waves {
        let r = size * (0.4 + 0.3 * i as f32);
        paint(s, (ax, cy - r - t, r + 2.0 * t, 2.0 * r + 2.0 * t), color, arc((ax, cy), r, t, 0.25 * std::f32::consts::PI, 0.75 * std::f32::consts::PI));
    }
}

/// An arrowhead pointing along `dir` (radians, 0 = right) at `tip`.
fn arrowhead(s: &mut Surface, tip: (f32, f32), dir: f32, size: f32, t: f32, color: u32) {
    let back = |a: f32| (tip.0 - size * (dir + a).cos(), tip.1 - size * (dir + a).sin());
    let shape = either(segment(tip, back(0.6), t), segment(tip, back(-0.6), t));
    paint(s, (tip.0 - size - t, tip.1 - size - t, 2.0 * (size + t), 2.0 * (size + t)), color, shape);
}

/// Two crossing lines with arrowheads at the right, `size` px wide.
pub fn shuffle(s: &mut Surface, (cx, cy): (f32, f32), size: f32, t: f32, color: u32) {
    let (hw, hh) = (size / 2.0, size * 0.28);
    let a = segment((cx - hw, cy - hh), (cx + hw * 0.6, cy + hh), t);
    let b = segment((cx - hw, cy + hh), (cx + hw * 0.6, cy - hh), t);
    paint(s, (cx - hw - t, cy - hh - t, size + 2.0 * t, 2.0 * hh + 2.0 * t), color, either(a, b));
    arrowhead(s, (cx + hw, cy + hh), 0.45, size * 0.22, t, color);
    arrowhead(s, (cx + hw, cy - hh), -0.45, size * 0.22, t, color);
}

/// A rounded loop with an arrowhead; `one` adds the "1" inside.
pub fn repeat(s: &mut Surface, c: (f32, f32), size: f32, t: f32, one: bool, color: u32) {
    use std::f32::consts::PI;
    let r = size / 2.0 * 0.78;
    paint(s, (c.0 - r - t, c.1 - r - t, 2.0 * r + 2.0 * t, 2.0 * r + 2.0 * t), color, arc(c, r, t, 0.35 * PI, 1.9 * PI));
    let tip = (c.0 + r * (0.35 * PI).sin(), c.1 - r * (0.35 * PI).cos());
    arrowhead(s, (tip.0 + t * 0.3, tip.1 + t * 0.2), 0.95 * PI, size * 0.2, t, color);
    if one {
        let (x, h) = (c.0, r * 0.5);
        paint(s, (x - t - r * 0.3, c.1 - h - t, 2.0 * t + r * 0.3, 2.0 * h + 2.0 * t), color, either(segment((x, c.1 - h), (x, c.1 + h), t), segment((x - r * 0.25, c.1 - h * 0.5), (x, c.1 - h), t)));
    }
}

/// A beamed pair of eighth notes, `size` px tall, centred.
pub fn note(s: &mut Surface, (cx, cy): (f32, f32), size: f32, color: u32) {
    let (t, r) = (size * 0.07, size * 0.13);
    let (x0, x1) = (cx - size * 0.22, cx + size * 0.26);
    let (top, bottom) = (cy - size * 0.42, cy + size * 0.3);
    let stems = either(segment((x0, top + size * 0.06), (x0, bottom), t), segment((x1, top), (x1, bottom - size * 0.06), t));
    let beam = |px: f32, py: f32| {
        let k = (px - x0) / (x1 - x0);
        let y = top + size * 0.06 - k * size * 0.06;
        let inside = px >= x0 - t / 2.0 && px <= x1 + t / 2.0;
        let d = (py - y - t * 1.1).abs() - t * 1.1;
        if inside {
            d
        } else {
            d.max((px - x0).abs().min((px - x1).abs()) - t / 2.0)
        }
    };
    paint(s, (cx - size / 2.0, cy - size / 2.0, size, size), color, either(either(stems, beam), either(disc((x0 - r * 0.8, bottom), r), disc((x1 - r * 0.8, bottom - size * 0.06), r))));
}

/// Placeholders drawn so far, by (size, radius): a grid of covers asks for the same one every frame.
static PLACEHOLDERS: Mutex<Option<HashMap<(usize, u32), Arc<Image>>>> = Mutex::new(None);
/// More sizes than any screen needs; past it the cache starts over.
const MAX_PLACEHOLDERS: usize = 16;

/// The grey placeholder shown where artwork is missing: a note on a dark tile.
/// Drawn once per size and radius and shared after that.
pub fn placeholder(size: usize, radius: f32) -> Arc<Image> {
    let mut cache = PLACEHOLDERS.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if map.len() >= MAX_PLACEHOLDERS && !map.contains_key(&(size, radius.to_bits())) {
        map.clear();
    }
    map.entry((size, radius.to_bits())).or_insert_with(|| Arc::new(draw_placeholder(size, radius))).clone()
}

fn draw_placeholder(size: usize, radius: f32) -> Image {
    let mut img = Image::new(size, size);
    let mut s = Surface { w: size, h: size, stride: size, px: &mut img.px, y0: 0 };
    s.fill_rrect(0.0, 0.0, size as f32, size as f32, radius, pack(44, 44, 46, 255));
    note(&mut s, (size as f32 / 2.0, size as f32 / 2.0), size as f32 * 0.5, pack(142, 142, 147, 255));
    img
}

/// The bifrost Music app icon: a white squircle with the red beamed notes.
pub fn app_icon(size: usize) -> Image {
    let mut img = Image::new(size, size);
    {
        let mut s = Surface { w: size, h: size, stride: size, px: &mut img.px, y0: 0 };
        s.fill_rect(0, 0, size, size, pack(255, 255, 255, 255));
        let n = size as f32;
        // a soft red-to-pink wash behind the note, as on the real icon
        for y in 0..size {
            let k = y as f32 / n;
            let c = rgba(252, (60.0 + 10.0 * k) as u8, (70.0 + 50.0 * k) as u8, (30.0 * (1.0 - k)) as u8);
            s.fill_rrect(0.0, y as f32, n, 1.0, 0.0, c);
        }
        note(&mut s, (n / 2.0, n / 2.0), n * 0.62, MUSIC_RED);
    }
    img.apply_mask(&squircle_mask(size));
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(f: impl FnOnce(&mut Surface)) -> (usize, Vec<u32>) {
        let mut buf = vec![0u32; 80 * 80];
        f(&mut Surface { w: 80, h: 80, stride: 80, px: &mut buf, y0: 0 });
        (buf.iter().filter(|&&p| p >> 24 > 0).count(), buf)
    }

    #[test]
    fn glyphs_draw_inside_their_boxes() {
        let white = pack(255, 255, 255, 255);
        let c = (40.0, 40.0);
        for i in 0..8 {
            let (n, buf) = ink(|s| match i {
                0 => play(s, c, 30.0, white),
                1 => pause(s, c, 30.0, white),
                2 => skip(s, c, 24.0, true, white),
                3 => skip(s, c, 24.0, false, white),
                4 => shuffle(s, c, 30.0, 3.0, white),
                5 => repeat(s, c, 30.0, 3.0, true, white),
                6 => speaker(s, c, 24.0, 2, white),
                _ => note(s, c, 40.0, white),
            });
            assert!(n > 30, "glyph {i} drew {n} px");
            assert!(buf[..80].iter().all(|&p| p == 0) && buf.iter().step_by(80).all(|&p| p == 0), "glyph {i} stays inside");
        }
        let (fwd, _) = ink(|s| skip(s, c, 24.0, true, white));
        let (_, back) = ink(|s| skip(s, c, 24.0, false, white));
        assert!(back[40 * 80 + 31] >> 24 > 0 && back[40 * 80 + 56] >> 24 == 0 && fwd > 0, "backward points left");
    }

    #[test]
    fn icon_and_placeholder_have_the_right_look() {
        let icon = app_icon(64);
        assert_eq!((icon.w, icon.h), (64, 64));
        assert_eq!(icon.px[0], 0, "squircle corner is clear");
        let edge = icon.px[32 * 64 + 2];
        assert!(edge >> 24 == 255 && edge & 0xff > 240, "white body");
        assert!(icon.px.iter().any(|&p| p & 0xff > 200 && (p >> 8) & 0xff < 100), "red note");
        let ph = placeholder(40, 6.0);
        assert_eq!(ph.px[0], 0);
        assert_eq!(ph.px[20 * 40 + 2], pack(44, 44, 46, 255));
        assert!(ph.px.iter().any(|&p| p & 0xff > 100), "grey note");
    }

    #[test]
    fn placeholders_are_drawn_once_per_size_and_radius() {
        let a = placeholder(48, 4.0);
        assert!(Arc::ptr_eq(&a, &placeholder(48, 4.0)));
        assert!(!Arc::ptr_eq(&a, &placeholder(48, 6.0)));
        assert!(!Arc::ptr_eq(&a, &placeholder(49, 4.0)));
        assert_eq!((a.w, a.h), (48, 48));
    }
}

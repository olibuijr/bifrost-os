//! Anti-aliased vector shapes from signed distance functions: the few symbols the SF font has no glyph for (globe,
//! dismiss-keyboard, the Clock tab icons) and the hands and rings of clock faces. The shell
//! (`springboard::shell::glyphs`) and the SDK (`bifrost_app::glyph_draw`) both re-export this module; render owns the
//! code because both crates depend on it.

use crate::gfx::{over, scale, Surface};

/// Paints every pixel inside the box `(x, y, w, h)` whose signed distance
/// `dist(px, py)` (negative inside) is within half a pixel of the shape.
pub fn paint(s: &mut Surface, (x, y, w, h): (f32, f32, f32, f32), color: u32, dist: impl Fn(f32, f32) -> f32) {
    let x0 = x.floor().max(0.0) as usize;
    let y0 = (y.floor().max(0.0) as usize).max(s.y0);
    let x1 = ((x + w).ceil().max(0.0) as usize).min(s.w);
    let y1 = ((y + h).ceil().max(0.0) as usize).min(s.h);
    for py in y0..y1 {
        for px in x0..x1 {
            let c = (0.5 - dist(px as f32 + 0.5, py as f32 + 0.5)).clamp(0.0, 1.0);
            if c > 0.0 {
                let d = s.at(px, py);
                *d = over(*d, scale(color, (c * 255.0) as u32));
            }
        }
    }
}

/// Distance to the line from `a` to `b`, stroked `t` wide with round caps.
pub fn segment((ax, ay): (f32, f32), (bx, by): (f32, f32), t: f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| {
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let k = if len2 > 0.0 { ((x - ax) * dx + (y - ay) * dy) / len2 } else { 0.0 }.clamp(0.0, 1.0);
        let (px, py) = (x - ax - dx * k, y - ay - dy * k);
        (px * px + py * py).sqrt() - t / 2.0
    }
}

/// Distance to a filled disc.
pub fn disc((cx, cy): (f32, f32), r: f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
}

/// Distance to a circle outline of radius `r`, stroked `t` wide.
pub fn ring((cx, cy): (f32, f32), r: f32, t: f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| (((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r).abs() - t / 2.0
}

/// Distance to the arc of a circle from angle `a0` to `a1` (radians,
/// clockwise from 12 o'clock), stroked `t` wide with round caps.
pub fn arc((cx, cy): (f32, f32), r: f32, t: f32, a0: f32, a1: f32) -> impl Fn(f32, f32) -> f32 {
    let end = |a: f32| (cx + r * a.sin(), cy - r * a.cos());
    let (e0, e1) = (end(a0), end(a1));
    move |x, y| {
        let a = (x - cx).atan2(-(y - cy)).rem_euclid(std::f32::consts::TAU);
        let inside = if a1 >= a0 { a >= a0 && a <= a1 } else { a >= a0 || a <= a1 };
        if inside {
            return (((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r).abs() - t / 2.0;
        }
        let d = |(ex, ey): (f32, f32)| ((x - ex).powi(2) + (y - ey).powi(2)).sqrt();
        d(e0).min(d(e1)) - t / 2.0
    }
}

/// Approximate distance to an ellipse outline with radii `rx`, `ry`.
pub fn ellipse((cx, cy): (f32, f32), rx: f32, ry: f32, t: f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| {
        let (u, v) = ((x - cx) / rx, (y - cy) / ry);
        ((u * u + v * v).sqrt() - 1.0).abs() * rx.min(ry) - t / 2.0
    }
}

/// Union of two shapes.
pub fn either(a: impl Fn(f32, f32) -> f32, b: impl Fn(f32, f32) -> f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| a(x, y).min(b(x, y))
}

fn bbox((cx, cy): (f32, f32), r: f32) -> (f32, f32, f32, f32) {
    (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0)
}

/// The language globe: a circle with an equator, a meridian and the two
/// curved meridians; `r` is its radius.
pub fn globe(s: &mut Surface, c: (f32, f32), r: f32, t: f32, color: u32) {
    let (cx, cy) = c;
    let shape = either(either(ring(c, r, t), segment((cx - r, cy), (cx + r, cy), t)), either(segment((cx, cy - r), (cx, cy + r), t), ellipse(c, r * 0.45, r, t)));
    paint(s, bbox(c, r), color, shape);
}

/// The dictation microphone (the system symbol set mic): a capsule, a cradle arc, a
/// stem and a foot; `h` is the glyph height.
pub fn mic(s: &mut Surface, (cx, cy): (f32, f32), h: f32, t: f32, color: u32) {
    let (cw, ch) = (h * 0.36, h * 0.6);
    let top = cy - h / 2.0;
    let (cap_r, cap_c) = (cw / 2.0, top + ch / 2.0);
    let capsule = move |x: f32, y: f32| {
        let (qx, qy) = ((x - cx).abs() - cap_r + t / 2.0, (y - cap_c).abs() - (ch / 2.0 - cap_r));
        let o = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0);
        (o - cap_r).abs() - t / 2.0
    };
    let cradle_r = cw * 0.9;
    let cradle_c = (cx, cap_c + ch * 0.12);
    let cradle = move |x: f32, y: f32| {
        let d = (((x - cradle_c.0).powi(2) + (y - cradle_c.1).powi(2)).sqrt() - cradle_r).abs() - t / 2.0;
        if y < cradle_c.1 { 1e3 } else { d }
    };
    let stem_top = cradle_c.1 + cradle_r;
    let foot = top + h;
    let rest = either(segment((cx, stem_top), (cx, foot), t), segment((cx - cw * 0.5, foot), (cx + cw * 0.5, foot), t));
    paint(s, (cx - h, top - 2.0, 2.0 * h, h + 4.0), color, either(either(capsule, cradle), rest));
}

/// The dismiss-keyboard key: a keyboard outline with a chevron under it.
/// `w` is the keyboard width; the glyph is about 0.9 w tall.
pub fn dismiss_keyboard(s: &mut Surface, (cx, cy): (f32, f32), w: f32, t: f32, color: u32) {
    let (kw, kh) = (w, w * 0.52);
    let (x, y) = (cx - kw / 2.0, cy - kh * 0.9);
    let outline = move |px: f32, py: f32| {
        let (qx, qy) = ((px - cx).abs() - kw / 2.0 + t, (py - y - kh / 2.0).abs() - kh / 2.0 + t);
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0);
        outside.abs() - t / 2.0
    };
    paint(s, (x - 1.0, y - 1.0, kw + 2.0, kh + 2.0), color, outline);
    // three rows of key dots
    let dot = t * 0.8;
    for row in 0..3 {
        let n = if row == 2 { 1 } else { 5 };
        let py = y + kh * (0.27 + 0.23 * row as f32);
        for i in 0..n {
            let px = if n == 1 { cx } else { x + kw * (0.2 + 0.15 * i as f32) };
            let dw = if n == 1 { kw * 0.5 } else { dot };
            paint(s, (px - dw / 2.0 - 1.0, py - dot - 1.0, dw + 2.0, 2.0 * dot + 2.0), color, segment((px - dw / 2.0 + dot / 2.0, py), (px + dw / 2.0 - dot / 2.0, py), dot));
        }
    }
    let (vy, vh, vw) = (y + kh + t * 1.6, w * 0.16, w * 0.4);
    let chevron = either(segment((cx - vw / 2.0, vy), (cx, vy + vh), t), segment((cx, vy + vh), (cx + vw / 2.0, vy), t));
    paint(s, (cx - vw / 2.0 - t, vy - t, vw + 2.0 * t, vh + 2.0 * t), color, chevron);
}

/// Tab-bar alarm clock: a ring on two short legs with bells and hands.
pub fn alarm_clock(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    let c = (cx, cy + r * 0.1);
    let hands = either(segment(c, (cx, cy - r * 0.45), t), segment(c, (cx + r * 0.4, c.1), t));
    paint(s, bbox(c, r * 1.3), color, either(either(ring(c, r * 0.85, t * 1.1), hands), either(segment((cx - r * 0.95, cy - r * 0.75), (cx - r * 0.55, cy - r * 1.0), t * 1.4), segment((cx + r * 0.95, cy - r * 0.75), (cx + r * 0.55, cy - r * 1.0), t * 1.4))));
    let legs = either(segment((cx - r * 0.5, cy + r * 0.85), (cx - r * 0.7, cy + r * 1.05), t), segment((cx + r * 0.5, cy + r * 0.85), (cx + r * 0.7, cy + r * 1.05), t));
    paint(s, bbox(c, r * 1.3), color, legs);
}

/// Tab-bar stopwatch: a filled disc with a crown, a tick hand and a
/// background-coloured face mark.
pub fn stopwatch(s: &mut Surface, (cx, cy): (f32, f32), r: f32, color: u32, bg: u32) {
    let c = (cx, cy + r * 0.15);
    paint(s, bbox(c, r * 1.3), color, either(disc(c, r * 0.85), segment((cx, cy - r * 0.85), (cx, cy - r * 1.1), r * 0.3)));
    paint(s, bbox(c, r), bg, either(segment(c, (cx, c.1 - r * 0.55), r * 0.14), disc(c, r * 0.1)));
}

/// Tab-bar timer: a circle with a short counter-clockwise arrow tail and a
/// hand pointing to 12.
pub fn timer(s: &mut Surface, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    use std::f32::consts::PI;
    let c = (cx, cy);
    let outline = arc(c, r, t, 0.0, 1.75 * PI);
    let (hx, hy) = (cx + r * 0.0, cy - r);
    let head = either(segment((hx - r * 0.3, hy - r * 0.28), (hx, hy), t), segment((hx - r * 0.3, hy + r * 0.28), (hx, hy), t));
    paint(s, bbox(c, r * 1.2), color, either(either(outline, head), segment(c, (cx + r * 0.0, cy - r * 0.55), t)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn canvas() -> (Vec<u32>, usize) {
        (vec![0u32; 64 * 64], 64)
    }

    fn alpha(buf: &[u32], w: usize, x: usize, y: usize) -> u32 {
        buf[y * w + x] >> 24
    }

    #[test]
    fn disc_and_ring_cover_the_expected_pixels() {
        let (mut buf, w) = canvas();
        let white = pack(255, 255, 255, 255);
        {
            let mut s = Surface { w, h: 64, stride: w, px: &mut buf, y0: 0 };
            paint(&mut s, (0.0, 0.0, 64.0, 64.0), white, disc((32.0, 32.0), 10.0));
        }
        assert_eq!(alpha(&buf, w, 32, 32), 255);
        assert_eq!(alpha(&buf, w, 32, 10), 0);
        let (mut buf, w) = canvas();
        {
            let mut s = Surface { w, h: 64, stride: w, px: &mut buf, y0: 0 };
            paint(&mut s, (0.0, 0.0, 64.0, 64.0), white, ring((32.0, 32.0), 20.0, 2.0));
        }
        assert_eq!(alpha(&buf, w, 32, 32), 0, "ring is hollow");
        assert!(alpha(&buf, w, 52, 32) > 250, "on the ring");
    }

    #[test]
    fn segment_has_round_caps_and_arc_stops_at_its_angles() {
        let white = pack(255, 255, 255, 255);
        let (mut buf, w) = canvas();
        {
            let mut s = Surface { w, h: 64, stride: w, px: &mut buf, y0: 0 };
            paint(&mut s, (0.0, 0.0, 64.0, 64.0), white, segment((10.0, 32.0), (50.0, 32.0), 4.0));
        }
        assert_eq!(alpha(&buf, w, 30, 32), 255);
        assert_eq!(alpha(&buf, w, 30, 40), 0);
        assert!(alpha(&buf, w, 8, 32) > 0, "cap extends past the end point");
        let (mut buf, w) = canvas();
        {
            let mut s = Surface { w, h: 64, stride: w, px: &mut buf, y0: 0 };
            paint(&mut s, (0.0, 0.0, 64.0, 64.0), white, arc((32.0, 32.0), 20.0, 3.0, 0.0, std::f32::consts::FRAC_PI_2));
        }
        assert_eq!(alpha(&buf, w, 46, 18), 255, "quadrant between 12 and 3 o'clock");
        assert_eq!(alpha(&buf, w, 32, 52), 0, "6 o'clock untouched");
    }

    #[test]
    fn icons_draw_inside_their_boxes() {
        let white = pack(255, 255, 255, 255);
        for icon in 0..5 {
            let (mut buf, w) = canvas();
            {
                let mut s = Surface { w, h: 64, stride: w, px: &mut buf, y0: 0 };
                match icon {
                    0 => globe(&mut s, (32.0, 32.0), 14.0, 2.0, white),
                    1 => dismiss_keyboard(&mut s, (32.0, 32.0), 30.0, 2.0, white),
                    2 => alarm_clock(&mut s, (32.0, 32.0), 12.0, 2.0, white),
                    3 => stopwatch(&mut s, (32.0, 32.0), 12.0, white, 0),
                    _ => timer(&mut s, (32.0, 32.0), 12.0, 2.0, white),
                }
            }
            let ink = buf.iter().filter(|&&p| p >> 24 > 0).count();
            assert!(ink > 40, "icon {icon} drew something");
            assert!(buf[..64].iter().all(|&p| p == 0) && buf[63 * 64..].iter().all(|&p| p == 0), "icon {icon} stays inside");
        }
    }
}

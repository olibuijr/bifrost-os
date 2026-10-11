//! Condition glyphs drawn from distance functions (no image assets), and
//! the app icon.

use super::data::Sky;
use crate::gfx::{pack, Image, Surface};
use crate::shell::glyphs::{disc, paint, segment};

const SUN: u32 = pack(255, 204, 0, 255);
const MOON: u32 = pack(235, 235, 245, 255);
const CLOUD: u32 = pack(246, 247, 250, 255);
const STORM: u32 = pack(150, 158, 172, 255);
const DROP: u32 = pack(94, 190, 255, 255);

type Dist = Box<dyn Fn(f32, f32) -> f32>;

fn union(shapes: Vec<Dist>) -> impl Fn(f32, f32) -> f32 {
    move |x, y| shapes.iter().map(|d| d(x, y)).fold(f32::MAX, f32::min)
}

/// A cloud whose body spans about 1.4 r wide, centred on `c`.
fn cloud((cx, cy): (f32, f32), r: f32) -> impl Fn(f32, f32) -> f32 {
    union(vec![
        Box::new(disc((cx - 0.30 * r, cy + 0.05 * r), 0.27 * r)),
        Box::new(disc((cx + 0.02 * r, cy - 0.14 * r), 0.38 * r)),
        Box::new(disc((cx + 0.36 * r, cy + 0.07 * r), 0.25 * r)),
        Box::new(segment((cx - 0.32 * r, cy + 0.2 * r), (cx + 0.38 * r, cy + 0.2 * r), 0.36 * r)),
    ])
}

fn sun(s: &mut Surface, (cx, cy): (f32, f32), r: f32) {
    let b = (cx - r, cy - r, 2.0 * r, 2.0 * r);
    paint(s, b, SUN, disc((cx, cy), 0.34 * r));
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (sn, cs) = a.sin_cos();
        paint(s, b, SUN, segment((cx + 0.55 * r * sn, cy - 0.55 * r * cs), (cx + 0.86 * r * sn, cy - 0.86 * r * cs), 0.1 * r));
    }
}

fn moon(s: &mut Surface, (cx, cy): (f32, f32), r: f32) {
    let (body, bite) = (disc((cx, cy), 0.5 * r), disc((cx + 0.26 * r, cy - 0.16 * r), 0.42 * r));
    paint(s, (cx - r, cy - r, 2.0 * r, 2.0 * r), MOON, move |x, y| body(x, y).max(-bite(x, y)));
}

/// Draws the glyph for `sky` centred on `c`, fitting a box of 2 r.
pub fn draw(s: &mut Surface, sky: Sky, day: bool, c: (f32, f32), r: f32) {
    let (cx, cy) = c;
    let b = (cx - r, cy - r, 2.0 * r, 2.0 * r);
    let body = |s: &mut Surface, at: (f32, f32), size: f32, color: u32| paint(s, b, color, cloud(at, size));
    match sky {
        Sky::Clear if day => sun(s, c, r),
        Sky::Clear => moon(s, c, r),
        Sky::PartlyCloudy => {
            if day {
                sun(s, (cx - 0.22 * r, cy - 0.22 * r), 0.7 * r);
            } else {
                moon(s, (cx - 0.22 * r, cy - 0.22 * r), 0.7 * r);
            }
            body(s, (cx + 0.1 * r, cy + 0.2 * r), 0.95 * r, CLOUD);
        }
        Sky::Cloudy => body(s, (cx, cy + 0.05 * r), r, CLOUD),
        Sky::Fog => {
            body(s, (cx, cy - 0.2 * r), 0.95 * r, STORM);
            for i in 0..3 {
                let y = cy + (0.42 + 0.2 * i as f32) * r;
                let half = (0.55 - 0.1 * (i % 2) as f32) * r;
                paint(s, b, STORM, segment((cx - half, y), (cx + half, y), 0.1 * r));
            }
        }
        Sky::Drizzle | Sky::Rain => {
            body(s, (cx, cy - 0.2 * r), 0.95 * r, STORM);
            let (len, w) = if sky == Sky::Rain { (0.3, 0.1) } else { (0.16, 0.07) };
            for i in 0..3 {
                let x = cx + (-0.4 + 0.4 * i as f32) * r;
                let y = cy + (0.5 + 0.08 * (i % 2) as f32) * r;
                paint(s, b, DROP, segment((x + 0.08 * r, y), (x - 0.04 * r, y + len * r), w * r));
            }
        }
        Sky::Snow => {
            body(s, (cx, cy - 0.2 * r), 0.95 * r, CLOUD);
            for i in 0..3 {
                let x = cx + (-0.38 + 0.38 * i as f32) * r;
                paint(s, b, CLOUD, disc((x, cy + (0.58 + 0.12 * (i % 2) as f32) * r), 0.07 * r));
            }
        }
        Sky::Thunder => {
            body(s, (cx, cy - 0.2 * r), 0.95 * r, STORM);
            let p = |x: f32, y: f32| (cx + x * r, cy + y * r);
            for (a, z) in [((0.1, 0.25), (-0.1, 0.62)), ((-0.1, 0.62), (0.12, 0.62)), ((0.12, 0.62), (-0.08, 0.98))] {
                paint(s, b, SUN, segment(p(a.0, a.1), p(z.0, z.1), 0.11 * r));
            }
        }
    }
}

/// Procedural app icon: a blue sky with a sun peeking from behind a cloud,
/// square and unmasked.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![0; size * size] };
    let (top, bottom) = ((62, 150, 235), (118, 196, 250));
    for y in 0..size {
        let k = y as f32 / size.max(1) as f32;
        let mix = |a: i32, b: i32| (a as f32 + (b - a) as f32 * k) as u8;
        image.px[y * size..(y + 1) * size].fill(pack(mix(top.0, bottom.0), mix(top.1, bottom.1), mix(top.2, bottom.2), 255));
    }
    let mut s = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    let f = size as f32;
    draw(&mut s, Sky::PartlyCloudy, true, (f * 0.5, f * 0.5), f * 0.4);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(sky: Sky, day: bool) -> Vec<u32> {
        let mut px = vec![0u32; 64 * 64];
        draw(&mut Surface { w: 64, h: 64, stride: 64, px: &mut px, y0: 0 }, sky, day, (32.0, 32.0), 28.0);
        px
    }

    #[test]
    fn every_sky_paints_something_and_they_differ() {
        let all = [Sky::Clear, Sky::PartlyCloudy, Sky::Cloudy, Sky::Fog, Sky::Drizzle, Sky::Rain, Sky::Snow, Sky::Thunder];
        let imgs: Vec<_> = all.iter().map(|&s| render(s, true)).collect();
        for (i, img) in imgs.iter().enumerate() {
            assert!(img.iter().any(|&p| p >> 24 > 0), "{:?} paints", all[i]);
            for other in &imgs[i + 1..] {
                assert_ne!(img, other);
            }
        }
        assert_ne!(render(Sky::Clear, true), render(Sky::Clear, false), "a night sky shows the moon");
    }

    #[test]
    fn the_icon_is_opaque_and_square() {
        let i = icon(80);
        assert_eq!((i.w, i.h), (80, 80));
        assert!(i.px.iter().all(|p| p >> 24 == 255));
        assert_ne!(i.px[2 * 80 + 2], i.px[77 * 80 + 2], "a gradient");
    }
}

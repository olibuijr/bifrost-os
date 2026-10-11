//! The app icon, drawn in code: a gold ring and a "W" on a dark blue ground.

use crate::gfx::{pack, Image};

/// The "W" as a polyline in unit coordinates.
const W: [(f32, f32); 5] = [(0.22, 0.31), (0.36, 0.69), (0.50, 0.43), (0.64, 0.69), (0.78, 0.31)];
const TOP: [f32; 3] = [28.0, 34.0, 56.0];
const BOTTOM: [f32; 3] = [8.0, 10.0, 20.0];
const GOLD: [f32; 3] = [240.0, 190.0, 80.0];

fn segment_distance((px, py): (f32, f32), (ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - ax - t * dx).hypot(py - ay - t * dy)
}

/// Anti-aliased coverage of a stroke `half` wide (unit coordinates) at distance `d`.
fn cover(d: f32, half: f32, size: f32) -> f32 {
    ((half - d) * size + 0.5).clamp(0.0, 1.0)
}

pub fn icon(size: usize) -> Image {
    let n = size as f32;
    let mut img = Image::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) / n, (y as f32 + 0.5) / n);
            let ring = cover(((p.0 - 0.5).hypot(p.1 - 0.5) - 0.40).abs(), 0.025, n);
            let w = W.windows(2).map(|s| segment_distance(p, s[0], s[1])).fold(f32::MAX, f32::min);
            let a = ring.max(cover(w, 0.045, n));
            let mut c = [0u8; 3];
            for i in 0..3 {
                let ground = TOP[i] + (BOTTOM[i] - TOP[i]) * p.1;
                c[i] = (ground + (GOLD[i] - ground) * a).round() as u8;
            }
            img.px[y * size + x] = pack(c[0], c[1], c[2], 255);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_an_opaque_square_with_gold_in_it() {
        let img = icon(64);
        assert_eq!((img.w, img.h), (64, 64));
        assert!(img.px.iter().all(|p| p >> 24 == 255));
        assert!(img.px.iter().any(|&p| p & 0xff > 200), "gold pixels");
    }
}

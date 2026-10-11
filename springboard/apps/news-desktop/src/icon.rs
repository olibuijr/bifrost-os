use bifrost_app::kit::{pack, Image, Surface};
use springboard::shell::glyphs::{paint, segment};

// The icon is artwork, not UI chrome: its red tile and newspaper inks stay literal.
// compliance-allow: COL-01 app icon artwork
const PAPER: u32 = pack(250, 250, 252, 255);
// compliance-allow: COL-01 app icon artwork
const INK: u32 = pack(60, 62, 72, 255);
// compliance-allow: COL-01 app icon artwork
const RULE: u32 = pack(150, 152, 162, 255);

/// Procedural app icon: a red tile with a white newspaper page.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![0; size * size] };
    let (top, bottom) = ((255, 82, 72), (204, 36, 52));
    for y in 0..size {
        let k = y as f32 / size.max(1) as f32;
        let mix = |a: i32, b: i32| (a as f32 + (b - a) as f32 * k) as u8;
        image.px[y * size..(y + 1) * size].fill(pack(mix(top.0, bottom.0), mix(top.1, bottom.1), mix(top.2, bottom.2), 255));
    }
    let mut s = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    let f = size as f32;
    let b = (0.0, 0.0, f, f);
    let (cx, cy, hw, hh) = (f * 0.5, f * 0.5, f * 0.28, f * 0.3);
    paint(&mut s, b, PAPER, move |x, y| ((x - cx).abs() - hw).max((y - cy).abs() - hh));
    paint(&mut s, b, INK, segment((cx - hw * 0.8, cy - hh * 0.62), (cx + hw * 0.8, cy - hh * 0.62), f * 0.05));
    for i in 0..4 {
        let y = cy - hh * 0.12 + i as f32 * hh * 0.3;
        paint(&mut s, b, RULE, segment((cx - hw * 0.8, y), (cx + hw * (if i == 3 { 0.2 } else { 0.8 }), y), f * 0.028));
    }
    image
}

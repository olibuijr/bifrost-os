//! `cargo run --example keyboard_icon -- system/keyboard/icon.png`: renders the Keyboard system
//! package's 180 px icon in the bifrost Settings glyph style: a grey gradient tile with a white
//! keyboard (three rows of keys and a space bar). The shell masks it to a squircle itself.
use springboard::gfx::{pack, Surface};

fn main() {
    let size = 180usize;
    let mut px = vec![0u32; size * size];
    let (top, bottom) = ((165u8, 165u8, 170u8), (110u8, 110u8, 116u8));
    for y in 0..size {
        let k = y as f32 / size as f32;
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
        px[y * size..(y + 1) * size].fill(pack(mix(top.0, bottom.0), mix(top.1, bottom.1), mix(top.2, bottom.2), 255));
    }
    {
        let mut s = Surface { w: size, h: size, stride: size, px: &mut px, y0: 0 };
        let f = size as f32;
        let white = pack(255, 255, 255, 255);
        let key = pack(128, 128, 133, 255);
        let (bx, by, bw, bh) = (f * 0.14, f * 0.30, f * 0.72, f * 0.40);
        s.fill_rrect(bx, by, bw, bh, f * 0.06, white);
        // three rows of keys, then the space bar
        let (pad, gap, kh) = (f * 0.045, f * 0.02, f * 0.06);
        let kw = (bw - 2.0 * pad - 6.0 * gap) / 7.0;
        for row in 0..2 {
            for i in 0..7 {
                s.fill_rrect(bx + pad + i as f32 * (kw + gap), by + pad + row as f32 * (kh + gap), kw, kh, f * 0.012, key);
            }
        }
        let y3 = by + pad + 2.0 * (kh + gap);
        s.fill_rrect(bx + pad + (kw + gap) * 1.5, y3, bw - 2.0 * pad - (kw + gap) * 3.0, kh, f * 0.012, key);
    }
    let png = springboard::pngenc::encode_rgb(size, size, &px);
    std::fs::write(std::env::args().nth(1).unwrap_or_else(|| "system/keyboard/icon.png".into()), png).unwrap();
}

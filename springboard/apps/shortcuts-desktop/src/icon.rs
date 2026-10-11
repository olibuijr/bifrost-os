use bifrost_app::kit::ui::WHITE;
use bifrost_app::kit::{pack, Image, Surface};

/// App icon: a pink-to-orange gradient with two overlapping rounded squares.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![0; size * size] };
    for y in 0..size {
        let f = y as u32 * 255 / size.max(1) as u32;
        let c = pack((255 - f / 6) as u8, (55 + f / 2) as u8, (95 - f / 4) as u8, 255);
        image.px[y * size..(y + 1) * size].fill(c);
    }
    let f = size as f32;
    let mut s = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    let t = f * 0.045;
    s.stroke_rrect(f * 0.22, f * 0.22, f * 0.38, f * 0.38, f * 0.1, t, WHITE);
    s.stroke_rrect(f * 0.40, f * 0.40, f * 0.38, f * 0.38, f * 0.1, t, WHITE);
    image
}

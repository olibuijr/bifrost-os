use bifrost_app::kit::ui::{SYSTEM_PURPLE, WHITE};
use bifrost_app::kit::{Image, Surface};

/// `c` with its alpha replaced: a named colour made translucent.
fn with_alpha(c: u32, a: u8) -> u32 {
    (c & 0x00FF_FFFF) | (a as u32) << 24
}

/// The Podcasts icon, built with software shapes (no asset download).
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![SYSTEM_PURPLE; size * size] };
    let f = size as f32;
    let mut s = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    // the glyph is white on the accent tile: the marks are foreground, not a background fill
    let glyph = WHITE;
    // the broadcast mark: two rings (largest first) around a dot and a body
    for (r, al) in [(0.34, 160u8), (0.24, 255)] {
        let (x, y, d) = (f * (0.5 - r), f * (0.40 - r), f * 2.0 * r);
        let ring = with_alpha(glyph, al);
        s.fill_rrect(x, y, d, d, d / 2.0, ring);
        let inner = d - f * 0.07;
        s.fill_rrect(x + f * 0.035, y + f * 0.035, inner, inner, inner / 2.0, SYSTEM_PURPLE);
    }
    s.fill_rrect(f * 0.44, f * 0.34, f * 0.12, f * 0.12, f * 0.06, glyph);
    s.fill_rrect(f * 0.46, f * 0.48, f * 0.08, f * 0.26, f * 0.04, glyph);
    image
}

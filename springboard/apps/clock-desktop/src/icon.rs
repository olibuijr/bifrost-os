//! The Clock icon (unmasked; the shell applies its squircle mask).
use bifrost_app::kit::ui::SYSTEM_ORANGE as ORANGE;
use bifrost_app::kit::{Image, Surface};
use springboard::shell::glyphs::{paint, ring, segment};
use bifrost_app::kit::ui::{BLACK, WHITE};

/// Unmasked icon; shell applies its standard squircle mask.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![BLACK; size * size] };
    let f = size as f32;
    let c = (f / 2.0, f / 2.0);
    let mut surface = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    let r = f * 0.40;
    paint(&mut surface, (c.0 - r - 1.0, c.1 - r - 1.0, r * 2.0 + 2.0, r * 2.0 + 2.0), WHITE, ring(c, r, f * 0.025));
    for i in 0..12 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        let (dx, dy) = (a.cos(), a.sin());
        let (p, q) = ((c.0 + dx * r * 0.82, c.1 + dy * r * 0.82), (c.0 + dx * r * 0.92, c.1 + dy * r * 0.92));
        paint(&mut surface, (p.0.min(q.0) - 3.0, p.1.min(q.1) - 3.0, (p.0 - q.0).abs() + 6.0, (p.1 - q.1).abs() + 6.0), WHITE, segment(p, q, f * 0.008));
    }
    let hand = |s: &mut Surface, to: (f32, f32), w: f32, color: u32| {
        paint(s, (c.0.min(to.0) - 4.0, c.1.min(to.1) - 4.0, (c.0 - to.0).abs() + 8.0, (c.1 - to.1).abs() + 8.0), color, segment(c, to, w));
    };
    hand(&mut surface, (c.0, c.1 - r * 0.50), f * 0.014, WHITE);
    hand(&mut surface, (c.0 + r * 0.45, c.1 + r * 0.25), f * 0.014, WHITE);
    hand(&mut surface, (c.0 - r * 0.15, c.1 + r * 0.70), f * 0.007, ORANGE);
    image
}

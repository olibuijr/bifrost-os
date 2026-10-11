//! The Reminders icon (unmasked; the shell applies its squircle mask).
use bifrost_app::kit::ui::{GRAY_DIM, SYSTEM_BLUE as BLUE, SYSTEM_ORANGE as ORANGE, SYSTEM_RED as RED, WHITE};
use bifrost_app::kit::{Image, Surface};
use springboard::shell::glyphs::{paint, ring, segment};

/// Unmasked icon; shell applies its standard squircle mask.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![WHITE; size * size] };
    let f = size as f32;
    let mut surface = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    for (i, color) in [BLUE, ORANGE, RED].iter().enumerate() {
        let y = f * (0.27 + i as f32 * 0.23);
        let r = f * 0.06;
        paint(&mut surface, (f * 0.18 - r - 1.0, y - r - 1.0, r * 2.0 + 2.0, r * 2.0 + 2.0), *color, ring((f * 0.18, y), r, f * 0.018));
        for offset in [-0.028, 0.028] {
            let sy = y + f * offset;
            paint(&mut surface, (f * 0.32, sy - f * 0.015, f * 0.51, f * 0.03), GRAY_DIM, segment((f * 0.33, sy), (f * 0.83, sy), f * 0.012));
        }
    }
    image
}

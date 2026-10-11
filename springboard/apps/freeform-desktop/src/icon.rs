use bifrost_app::kit::colors::{SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_RED};
use bifrost_app::kit::ui::{TINT, WHITE};
use bifrost_app::kit::{Image, Surface};

/// App icon: a white page with a blue pen stroke and a red dot. The icon is a fixed
/// picture, so it takes the Light value of each system colour, not the live Dark one.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![WHITE; size * size] };
    let f = size as f32;
    let mut s = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    s.fill_rrect(f * 0.14, f * 0.58, f * 0.50, f * 0.07, f * 0.035, TINT);
    s.fill_rrect(f * 0.30, f * 0.30, f * 0.56, f * 0.07, f * 0.035, SYSTEM_ORANGE.light);
    s.fill_rrect(f * 0.22, f * 0.74, f * 0.30, f * 0.07, f * 0.035, SYSTEM_GREEN.light);
    s.fill_rrect(f * 0.62, f * 0.62, f * 0.18, f * 0.18, f * 0.09, SYSTEM_RED.light);
    image
}

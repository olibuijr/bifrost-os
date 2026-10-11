//! Procedural address-book fallback, square and unmasked.
use bifrost_app::kit::gfx::{Image, Surface};
use bifrost_app::kit::ui::{GRAY_DIM, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE, WHITE};
use springboard::shell::glyphs::{disc, paint};

pub fn icon(size: usize) -> Image {
    let paper = WHITE;
    let ink = GRAY_DIM;
    let mut image = Image {
        w: size,
        h: size,
        px: vec![paper; size * size],
    };
    let f = size as f32;
    let mut s = Surface {
        w: size,
        h: size,
        stride: size,
        px: &mut image.px,
        y0: 0,
    };
    s.fill_rect(0, 0, (f * 0.1) as usize, size, GRAY_DIM);
    for (i, color) in [SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE].into_iter().enumerate() {
        s.fill_rrect(
            f * 0.92,
            f * (0.15 + 0.23 * i as f32),
            f * 0.12,
            f * 0.2,
            f * 0.03,
            color,
        );
    }
    paint(
        &mut s,
        (f * 0.32, f * 0.23, f * 0.36, f * 0.36),
        ink,
        disc((f * 0.5, f * 0.4), f * 0.145),
    );
    s.fill_rrect(f * 0.24, f * 0.58, f * 0.52, f * 0.21, f * 0.105, ink);
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_has_a_person_and_colored_tabs() {
        let image = icon(100);
        assert_ne!(image.px[40 * 100 + 50], image.px[10 * 100 + 50]);
        assert_ne!(image.px[20 * 100 + 97], image.px[45 * 100 + 97]);
        assert!(image.px.iter().all(|p| p >> 24 == 255));
    }
}

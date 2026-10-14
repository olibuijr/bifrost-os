#![cfg(test)]

use bifrost_app::kit::{pack, Image, Surface};
use bifrost_app::kit::ui::{CARD_BG, LABEL_2, SYSTEM_ORANGE as ORANGE};

/// The classic Calculator icon, built with software shapes so it shares
/// the shell's existing icon mask and needs no asset download.
pub fn icon(size: usize) -> Image {
    let mut image = Image {
        w: size,
        h: size,
        px: vec![CARD_BG.dark; size * size],
    };
    let f = size as f32;
    let mut surface = Surface {
        w: size,
        h: size,
        stride: size,
        px: &mut image.px,
        y0: 0,
    };
    surface.fill_rrect(
        f * 0.18,
        f * 0.13,
        f * 0.64,
        f * 0.22,
        f * 0.03,
        LABEL_2.dark,
    );
    for row in 0..3 {
        for col in 0..3 {
            let x = f * (0.18 + col as f32 * 0.23);
            let y = f * (0.43 + row as f32 * 0.16);
            surface.fill_rrect(
                x,
                y,
                f * 0.18,
                f * 0.12,
                f * 0.028,
                if col == 2 {
                    ORANGE
                } else {
                    LABEL_2.light
                },
            );
        }
    }
    image
}


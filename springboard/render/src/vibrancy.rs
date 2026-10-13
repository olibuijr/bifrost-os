//! Vibrant drawing over a material: plus-lighter (Dark) and plus-darker (Light) for text, symbols and fills.
//! The level model lives in `bifrost_ui::vibrancy` (same tables as `ui::VIBRANT_LABEL` / `VIBRANT_FILL`); this
//! module is the pixel math, shared by the CPU path (`Surface`) and the glass path (`glass::Kind` regions draw
//! their tint, then content through these).
//! Pixels are premultiplied, red in the low byte, alpha on top; alpha is never changed, so a translucent
//! material composited later gives backdrop + amount (plus-lighter) or backdrop - amount (plus-darker).
use crate::gfx::{Image, Surface};

/// Adds `amount` (0..1) of light to one premultiplied pixel, alpha kept. Premultiplied RGB is capped at alpha
/// so the pixel stays valid.
pub fn plus_lighter(d: u32, amount: f32) -> u32 {
    let a = (amount.clamp(0.0, 1.0) * 255.0).round() as i32;
    let cap = (d >> 24) as i32;
    chan(d, |c| (c + a).min(cap.max(c)))
}

/// Removes `amount` (0..1) of light from one premultiplied pixel, alpha kept.
pub fn plus_darker(d: u32, amount: f32) -> u32 {
    let a = (amount.clamp(0.0, 1.0) * 255.0).round() as i32;
    chan(d, |c| (c - a).max(0))
}

fn chan(d: u32, f: impl Fn(i32) -> i32) -> u32 {
    let ch = |sh: u32| (f(((d >> sh) & 255) as i32).clamp(0, 255) as u32) << sh;
    ch(0) | ch(8) | ch(16) | (d & 0xff00_0000)
}

/// One pixel under the scheme's blend: plus-darker in Light, plus-lighter in Dark.
pub fn apply(d: u32, amount: f32, light: bool) -> u32 {
    if light { plus_darker(d, amount) } else { plus_lighter(d, amount) }
}

/// Strength of a label (`fill` false) or fill level (0 primary .. 3 quaternary) at the current scheme position.
pub fn strength(level: usize, fill: bool) -> f32 {
    use crate::ui::{VIBRANT_FILL, VIBRANT_LABEL};
    let t = if fill { VIBRANT_FILL } else { VIBRANT_LABEL };
    let i = level.min(3);
    t[0][i] + (t[1][i] - t[0][i]) * crate::appearance::scheme().clamp(0.0, 1.0)
}

/// Draws a white-rendered image (text, template symbol) vibrantly at `amount` x coverage. With Reduce Transparency
/// on there is no backdrop to blend into, so it draws in the opaque `fallback` colour at `amount` as opacity.
pub fn blit_image(s: &mut Surface, img: &Image, x: i32, y: i32, amount: f32, fallback: u32) {
    if crate::appearance::reduce_transparency() {
        s.blit_colored(img, x, y, fallback, (amount.clamp(0.0, 1.0) * 255.0).round() as u32);
        return;
    }
    let light = crate::appearance::scheme() >= 0.5;
    for row in 0..img.h as i32 {
        let py = y + row;
        if py < s.y0 as i32 || py >= s.h as i32 {
            continue;
        }
        let base = (py as usize - s.y0) * s.stride;
        for col in 0..img.w as i32 {
            let px = x + col;
            if px < 0 || px >= s.w as i32 {
                continue;
            }
            let cov = (img.px[row as usize * img.w + col as usize] >> 24) as f32 / 255.0;
            if cov > 0.0 {
                let d = &mut s.px[base + px as usize];
                *d = apply(*d, cov * amount, light);
            }
        }
    }
}

/// Fills a rectangle vibrantly (selection plates, field wells and separators on a material). With Reduce
/// Transparency on it blends the opaque `fallback` colour over the pixels at `amount` opacity instead.
pub fn fill_rect(s: &mut Surface, x: usize, y: usize, w: usize, h: usize, amount: f32, fallback: u32) {
    let reduce = crate::appearance::reduce_transparency();
    let light = crate::appearance::scheme() >= 0.5;
    let src = if reduce {
        let a = (amount.clamp(0.0, 1.0) * 255.0).round() as u32;
        let ch = |sh: u32| ((((fallback >> sh) & 255) * a + 127) / 255) << sh;
        ch(0) | ch(8) | ch(16) | (a << 24)
    } else {
        0
    };
    for row in y.max(s.y0)..(y + h).min(s.h) {
        let base = (row - s.y0) * s.stride;
        for col in x..(x + w).min(s.w) {
            let d = &mut s.px[base + col];
            *d = if reduce { crate::gfx::over(*d, src) } else { apply(*d, amount, light) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    #[test]
    fn plus_lighter_golden_values() {
        assert_eq!(plus_lighter(pack(100, 50, 0, 255), 0.5), pack(228, 178, 128, 255));
        assert_eq!(plus_lighter(pack(200, 200, 200, 255), 0.85), pack(255, 255, 255, 255));
        // premultiplied translucent pixel: channels never exceed alpha
        assert_eq!(plus_lighter(pack(10, 10, 10, 40), 1.0), pack(40, 40, 40, 40));
    }

    #[test]
    fn plus_darker_golden_values() {
        assert_eq!(plus_darker(pack(100, 50, 0, 255), 0.5), pack(0, 0, 0, 255));
        assert_eq!(plus_darker(pack(240, 200, 128, 255), 0.25), pack(176, 136, 64, 255));
        assert_eq!(plus_darker(pack(240, 200, 128, 255), 0.0), pack(240, 200, 128, 255));
    }

    #[test]
    fn alpha_is_untouched_and_apply_picks_the_blend() {
        let d = pack(80, 80, 80, 200);
        assert_eq!(apply(d, 0.1, false) >> 24, 200);
        assert_eq!(apply(d, 0.1, true) >> 24, 200);
        assert!(apply(d, 0.1, false) & 255 > 80);
        assert!(apply(d, 0.1, true) & 255 < 80);
    }

    #[test]
    fn level_tables_fall_with_the_level() {
        use crate::ui::{VIBRANT_FILL, VIBRANT_LABEL};
        for t in [VIBRANT_LABEL, VIBRANT_FILL] {
            for scheme in t {
                assert!(scheme.windows(2).all(|w| w[0] > w[1]));
            }
        }
        assert_eq!(VIBRANT_LABEL[0][1], 0.55);
        assert_eq!(VIBRANT_LABEL[1][1], 0.50);
    }
}

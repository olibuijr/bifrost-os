//! Blend modes and source-over compositing for premultiplied RGBA pixels, packed the way the
//! renderer packs them (red in the low byte, alpha in the top byte). `blend_channel` holds the
//! separable blend formulas and `composite` puts one pixel over another under a chosen mode.
//! Everything is pure std code, so it runs without a GPU.

/// A separable blend mode. `Normal` is plain source-over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

/// Blends one unpremultiplied channel. `b` is the backdrop value and `s` the source value, both in
/// 0..=1.
pub fn blend_channel(mode: Mode, b: f32, s: f32) -> f32 {
    match mode {
        Mode::Normal => s,
        Mode::Multiply => b * s,
        Mode::Screen => b + s - b * s,
        Mode::Overlay => hard_light(s, b),
        Mode::Darken => b.min(s),
        Mode::Lighten => b.max(s),
        Mode::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        Mode::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        Mode::HardLight => hard_light(b, s),
        Mode::SoftLight => soft_light(b, s),
        Mode::Difference => (b - s).abs(),
        Mode::Exclusion => b + s - 2.0 * b * s,
    }
}

/// Multiplies when the source is dark and screens when it is light, with the source as the switch.
fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        let t = 2.0 * s - 1.0;
        b + t - b * t
    }
}

/// The W3C soft-light formula: a gentle darken or lighten that follows the source.
fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

fn unpack(px: u32) -> [f32; 4] {
    let c = |shift: u32| ((px >> shift) & 0xff) as f32 / 255.0;
    [c(0), c(8), c(16), c(24)]
}

fn pack(v: [f32; 4]) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(v[0]) | (q(v[1]) << 8) | (q(v[2]) << 16) | (q(v[3]) << 24)
}

/// Puts the premultiplied pixel `src` over `dst` under `mode`. With `Mode::Normal` this is ordinary
/// source-over. A transparent source returns `dst` unchanged.
pub fn composite(src: u32, dst: u32, mode: Mode) -> u32 {
    let s = unpack(src);
    let d = unpack(dst);
    if s[3] <= 0.0 {
        return dst;
    }
    let (a_s, a_b) = (s[3], d[3]);
    let mut out = [0.0f32; 4];
    for c in 0..3 {
        let cs = s[c] / a_s;
        let cb = if a_b > 0.0 { d[c] / a_b } else { cs };
        let mixed = blend_channel(mode, cb, cs);
        out[c] = (1.0 - a_b) * s[c] + (1.0 - a_s) * d[c] + a_s * a_b * mixed;
    }
    out[3] = a_s + a_b * (1.0 - a_s);
    pack(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn multiply_and_screen_of_two_halves() {
        assert!(close(blend_channel(Mode::Multiply, 0.5, 0.5), 0.25));
        assert!(close(blend_channel(Mode::Screen, 0.5, 0.5), 0.75));
        assert!(close(blend_channel(Mode::Difference, 0.2, 0.7), 0.5));
        assert!(close(blend_channel(Mode::Exclusion, 0.5, 0.5), 0.5));
    }

    #[test]
    fn overlay_is_hard_light_with_the_roles_swapped() {
        // Backdrop 0.25 under a source of 0.5 multiplies to 0.25.
        assert!(close(blend_channel(Mode::Overlay, 0.25, 0.5), 0.25));
        // Backdrop 0.75 under a source of 0.25 screens 0.25 with 0.5: 0.25 + 0.5 - 0.125.
        assert!(close(blend_channel(Mode::Overlay, 0.75, 0.25), 0.625));
    }

    #[test]
    fn dodge_and_burn_clip_at_the_ends() {
        assert!(close(blend_channel(Mode::ColorDodge, 0.5, 0.5), 1.0));
        assert!(close(blend_channel(Mode::ColorBurn, 0.5, 0.5), 0.0));
        assert!(close(blend_channel(Mode::ColorDodge, 0.0, 0.9), 0.0));
        assert!(close(blend_channel(Mode::ColorBurn, 0.9, 0.0), 0.0));
    }

    #[test]
    fn soft_light_darkens_below_half_grey() {
        // 0.5 - (1 - 0.5) * 0.5 * 0.5 = 0.375
        assert!(close(blend_channel(Mode::SoftLight, 0.5, 0.25), 0.375));
        // Source above half grey: D(0.25) = ((4 - 12) * 0.25 + 4) * 0.25 = 0.5,
        // so 0.25 + (2 * 0.75 - 1) * (0.5 - 0.25) = 0.25 + 0.5 * 0.25 = 0.375.
        assert!(close(blend_channel(Mode::SoftLight, 0.25, 0.75), 0.375));
    }

    #[test]
    fn normal_source_over_of_half_black_on_white_is_grey() {
        // Source (0, 0, 0) at alpha 128 over opaque white leaves 127 / 255 of the white.
        let out = composite(pack([0.0, 0.0, 0.0, 128.0 / 255.0]), 0xffff_ffff, Mode::Normal);
        assert_eq!(out, 0xff7f_7f7f);
    }

    #[test]
    fn opaque_multiply_darkens_white_to_the_source_grey() {
        let grey = 0xff80_8080; // 128 in each colour channel, opaque
        assert_eq!(composite(grey, 0xffff_ffff, Mode::Multiply), grey);
    }

    #[test]
    fn transparent_source_leaves_the_destination() {
        let dst = 0xff10_2030;
        assert_eq!(composite(0, dst, Mode::Screen), dst);
    }
}

//! The composite filters that `composite_filters` does not cover: the Porter-Duff operators
//! (`CISourceInCompositing`, `CISourceOutCompositing`, `CISourceAtopCompositing`) and the channel
//! product (`CIMultiplyCompositing`), the seven newer separable blend modes (`CILinearBurnBlendMode`,
//! `CILinearDodgeBlendMode`, `CILinearLightBlendMode`, `CIPinLightBlendMode`, `CIVividLightBlendMode`,
//! `CIDivideBlendMode`, `CISubtractBlendMode`), and the masked blends (`CIBlendWithMask` reads the
//! mask's green channel, `CIBlendWithRedMask`, `CIBlendWithBlueMask`, `CIBlendWithAlphaMask`).
//!
//! Pixels are premultiplied RGBA packed with red in the low byte, as `gfx::Image` stores them. The
//! separable modes use the general compositing formula of the W3C compositing specification on
//! premultiplied values: `co = (1 - ab) * cs + (1 - as) * cb + as * ab * B(Cb, Cs)` and
//! `ao = as + ab - as * ab`, with `B` evaluated on unpremultiplied colours, the same shape as
//! `blend::composite`. Every name here is the reference filter's name in lower camel case.
//!
//! Differences from the reference: the blend formulas are the published ones (W3C and the usual
//! image-editor definitions); the reference evaluates in its working colour space (linear by default)
//! while this model works on the stored sRGB codes, as the rest of the CPU filters do.

/// A two-input Porter-Duff operator or channel product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    /// `CISourceInCompositing`: the source where the background is opaque, `S * ab`.
    SourceIn,
    /// `CISourceOutCompositing`: the source where the background is transparent, `S * (1 - ab)`.
    SourceOut,
    /// `CISourceAtopCompositing`: the source over the background, kept to the background's shape.
    SourceAtop,
    /// `CIMultiplyCompositing`: every premultiplied channel, alpha included, multiplied.
    Multiply,
}

/// A separable blend mode that `composite_filters::Blend` does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraBlend {
    /// `cb + cs - 1`, floored at 0.
    LinearBurn,
    /// `cb + cs`, capped at 1.
    LinearDodge,
    /// `cb + 2 cs - 1`, clamped: linear burn below half, linear dodge above.
    LinearLight,
    /// Darken with `2 cs` below half, lighten with `2 cs - 1` above.
    PinLight,
    /// Colour burn with `2 cs` below half, colour dodge with `2 cs - 1` above.
    VividLight,
    /// The background divided by the source, capped at 1.
    Divide,
    /// The source taken from the background, floored at 0.
    Subtract,
}

/// The channel of the mask that a masked blend reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskChannel {
    /// `CIBlendWithRedMask`.
    Red,
    /// `CIBlendWithMask`, whose grey mask is read through its green channel.
    Green,
    /// `CIBlendWithBlueMask`.
    Blue,
    /// `CIBlendWithAlphaMask`.
    Alpha,
}

const OPERATORS: [(&str, Operator); 4] = [
    ("sourceIn", Operator::SourceIn),
    ("sourceOut", Operator::SourceOut),
    ("sourceAtop", Operator::SourceAtop),
    ("multiplyCompositing", Operator::Multiply),
];

const BLENDS: [(&str, ExtraBlend); 7] = [
    ("linearBurn", ExtraBlend::LinearBurn),
    ("linearDodge", ExtraBlend::LinearDodge),
    ("linearLight", ExtraBlend::LinearLight),
    ("pinLight", ExtraBlend::PinLight),
    ("vividLight", ExtraBlend::VividLight),
    ("divide", ExtraBlend::Divide),
    ("subtract", ExtraBlend::Subtract),
];

impl Operator {
    /// The operator called `name`, or `None`.
    pub fn named(name: &str) -> Option<Operator> {
        OPERATORS.iter().find(|e| e.0 == name).map(|e| e.1)
    }

    /// The name this operator is called by.
    pub fn name(self) -> &'static str {
        OPERATORS.iter().find(|e| e.1 == self).map(|e| e.0).unwrap_or("sourceIn")
    }
}

impl ExtraBlend {
    /// The blend mode called `name`, or `None`.
    pub fn named(name: &str) -> Option<ExtraBlend> {
        BLENDS.iter().find(|e| e.0 == name).map(|e| e.1)
    }

    /// The name this blend mode is called by.
    pub fn name(self) -> &'static str {
        BLENDS.iter().find(|e| e.1 == self).map(|e| e.0).unwrap_or("linearBurn")
    }

    /// `B(cb, cs)` on unpremultiplied channels in 0..=1.
    pub fn channel(self, cb: f32, cs: f32) -> f32 {
        let v = match self {
            ExtraBlend::LinearBurn => cb + cs - 1.0,
            ExtraBlend::LinearDodge => cb + cs,
            ExtraBlend::LinearLight => cb + 2.0 * cs - 1.0,
            ExtraBlend::PinLight => {
                if cs < 0.5 {
                    cb.min(2.0 * cs)
                } else {
                    cb.max(2.0 * cs - 1.0)
                }
            }
            ExtraBlend::VividLight => {
                if cs < 0.5 {
                    burn(cb, 2.0 * cs)
                } else {
                    dodge(cb, 2.0 * cs - 1.0)
                }
            }
            ExtraBlend::Divide => {
                if cs <= 0.0 {
                    if cb > 0.0 { 1.0 } else { 0.0 }
                } else {
                    cb / cs
                }
            }
            ExtraBlend::Subtract => cb - cs,
        };
        v.clamp(0.0, 1.0)
    }
}

/// Colour burn of `cb` by `cs`.
fn burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

/// Colour dodge of `cb` by `cs`.
fn dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

/// The names of the operators, in catalogue order.
pub fn operator_names() -> Vec<&'static str> {
    OPERATORS.iter().map(|e| e.0).collect()
}

/// The names of the extra blend modes, in catalogue order.
pub fn blend_names() -> Vec<&'static str> {
    BLENDS.iter().map(|e| e.0).collect()
}

fn unpack(px: u32) -> [f32; 4] {
    let c = |shift: u32| ((px >> shift) & 0xff) as f32 / 255.0;
    [c(0), c(8), c(16), c(24)]
}

fn pack(v: [f32; 4]) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    // Premultiplied colour never exceeds alpha; rounding could push it one code over.
    let a = q(v[3]);
    q(v[0]).min(a) | (q(v[1]).min(a) << 8) | (q(v[2]).min(a) << 16) | (a << 24)
}

/// One pixel of `op` with source `src` and background `dst`, both premultiplied.
pub fn operator_pixel(src: u32, dst: u32, op: Operator) -> u32 {
    let (s, d) = (unpack(src), unpack(dst));
    let (a_s, a_b) = (s[3], d[3]);
    let out = match op {
        Operator::SourceIn => [0, 1, 2, 3].map(|c| s[c] * a_b),
        Operator::SourceOut => [0, 1, 2, 3].map(|c| s[c] * (1.0 - a_b)),
        Operator::SourceAtop => {
            let c = [0, 1, 2].map(|c| s[c] * a_b + d[c] * (1.0 - a_s));
            [c[0], c[1], c[2], a_b]
        }
        Operator::Multiply => [0, 1, 2, 3].map(|c| s[c] * d[c]),
    };
    pack(out)
}

/// One pixel of the blend mode `mode` with source `src` over background `dst`, both premultiplied.
/// A transparent source leaves the background unchanged.
pub fn blend_pixel(src: u32, dst: u32, mode: ExtraBlend) -> u32 {
    let (s, d) = (unpack(src), unpack(dst));
    let (a_s, a_b) = (s[3], d[3]);
    if a_s <= 0.0 {
        return dst;
    }
    let mut out = [0.0f32; 4];
    for c in 0..3 {
        let cs = s[c] / a_s;
        let cb = if a_b > 0.0 { d[c] / a_b } else { cs };
        out[c] = (1.0 - a_b) * s[c] + (1.0 - a_s) * d[c] + a_s * a_b * mode.channel(cb, cs);
    }
    out[3] = a_s + a_b - a_s * a_b;
    pack(out)
}

/// One pixel of a masked blend: the background where the mask channel is 0, the image where it is
/// 1, and a straight mix of the two premultiplied pixels in between.
pub fn mask_pixel(image: u32, background: u32, mask: u32, channel: MaskChannel) -> u32 {
    let shift = match channel {
        MaskChannel::Red => 0,
        MaskChannel::Green => 8,
        MaskChannel::Blue => 16,
        MaskChannel::Alpha => 24,
    };
    let m = ((mask >> shift) & 0xff) as f32 / 255.0;
    let (i, b) = (unpack(image), unpack(background));
    pack([0, 1, 2, 3].map(|c| b[c] + (i[c] - b[c]) * m))
}

fn check(len: &[usize], w: usize, h: usize) -> Option<()> {
    let n = w.checked_mul(h)?;
    len.iter().all(|&l| l == n).then_some(())
}

/// `op` over two `w` by `h` buffers. `None` when a length is not `w * h`.
pub fn operate(src: &[u32], dst: &[u32], w: usize, h: usize, op: Operator) -> Option<Vec<u32>> {
    check(&[src.len(), dst.len()], w, h)?;
    Some(src.iter().zip(dst).map(|(&s, &d)| operator_pixel(s, d, op)).collect())
}

/// `mode` over two `w` by `h` buffers. `None` when a length is not `w * h`.
pub fn blend(src: &[u32], dst: &[u32], w: usize, h: usize, mode: ExtraBlend) -> Option<Vec<u32>> {
    check(&[src.len(), dst.len()], w, h)?;
    Some(src.iter().zip(dst).map(|(&s, &d)| blend_pixel(s, d, mode)).collect())
}

/// A masked blend over three `w` by `h` buffers. `None` when a length is not `w * h`.
pub fn blend_with_mask(image: &[u32], background: &[u32], mask: &[u32], w: usize, h: usize, channel: MaskChannel) -> Option<Vec<u32>> {
    check(&[image.len(), background.len(), mask.len()], w, h)?;
    Some((0..image.len()).map(|i| mask_pixel(image[i], background[i], mask[i], channel)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0xff00_00ff;
    const BLUE: u32 = 0xffff_0000;
    const HALF_GREY: u32 = 0xff80_8080;
    const WHITE: u32 = 0xffff_ffff;

    fn red(px: u32) -> i32 {
        (px & 0xff) as i32
    }

    #[test]
    fn names_round_trip() {
        for n in operator_names() {
            assert_eq!(Operator::named(n).unwrap().name(), n);
        }
        for n in blend_names() {
            assert_eq!(ExtraBlend::named(n).unwrap().name(), n);
        }
        assert_eq!(Operator::named("nope"), None);
        assert_eq!(ExtraBlend::named("nope"), None);
    }

    #[test]
    fn source_in_and_out_split_the_source_by_the_background_alpha() {
        // Over an opaque background "in" keeps the source and "out" removes it; over a
        // transparent one it is the other way round.
        assert_eq!(operator_pixel(RED, BLUE, Operator::SourceIn), RED);
        assert_eq!(operator_pixel(RED, BLUE, Operator::SourceOut), 0);
        assert_eq!(operator_pixel(RED, 0, Operator::SourceIn), 0);
        assert_eq!(operator_pixel(RED, 0, Operator::SourceOut), RED);
    }

    #[test]
    fn source_atop_keeps_the_background_shape() {
        assert_eq!(operator_pixel(RED, BLUE, Operator::SourceAtop), RED);
        assert_eq!(operator_pixel(RED, 0, Operator::SourceAtop), 0);
        assert_eq!(operator_pixel(0, BLUE, Operator::SourceAtop), BLUE);
    }

    #[test]
    fn multiply_compositing_multiplies_alpha_too() {
        let half = 0x8080_8080; // premultiplied half-transparent grey
        let out = operator_pixel(half, WHITE, Operator::Multiply);
        assert_eq!(out, half);
        assert_eq!(operator_pixel(half, 0, Operator::Multiply), 0);
    }

    #[test]
    fn blend_formulas_match_the_published_values() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(ExtraBlend::LinearBurn.channel(0.5, 0.25), 0.0));
        assert!(close(ExtraBlend::LinearBurn.channel(0.75, 0.5), 0.25));
        assert!(close(ExtraBlend::LinearDodge.channel(0.75, 0.5), 1.0));
        assert!(close(ExtraBlend::LinearLight.channel(0.5, 0.5), 0.5));
        assert!(close(ExtraBlend::PinLight.channel(0.8, 0.2), 0.4));
        assert!(close(ExtraBlend::PinLight.channel(0.2, 0.8), 0.6));
        assert!(close(ExtraBlend::VividLight.channel(0.5, 0.5), 0.5));
        assert!(close(ExtraBlend::Divide.channel(0.25, 0.5), 0.5));
        assert!(close(ExtraBlend::Divide.channel(0.25, 0.0), 1.0));
        assert!(close(ExtraBlend::Subtract.channel(0.75, 0.25), 0.5));
        assert!(close(ExtraBlend::Subtract.channel(0.25, 0.75), 0.0));
    }

    #[test]
    fn opaque_blends_use_the_formula_and_a_clear_source_keeps_the_background() {
        // Subtract half grey from white: about 127.
        let out = blend_pixel(HALF_GREY, WHITE, ExtraBlend::Subtract);
        assert!((red(out) - 127).abs() <= 1, "red {}", red(out));
        assert_eq!(out >> 24, 0xff);
        for n in blend_names() {
            let m = ExtraBlend::named(n).unwrap();
            assert_eq!(blend_pixel(0, BLUE, m), BLUE, "{n}");
        }
    }

    #[test]
    fn a_transparent_background_shows_the_source() {
        for n in blend_names() {
            let m = ExtraBlend::named(n).unwrap();
            assert_eq!(blend_pixel(RED, 0, m), RED, "{n}");
        }
    }

    #[test]
    fn masked_blend_picks_by_the_chosen_channel() {
        let green_mask = 0xff00_ff00;
        assert_eq!(mask_pixel(RED, BLUE, green_mask, MaskChannel::Green), RED);
        assert_eq!(mask_pixel(RED, BLUE, green_mask, MaskChannel::Red), BLUE);
        assert_eq!(mask_pixel(RED, BLUE, green_mask, MaskChannel::Alpha), RED);
        let mid = mask_pixel(WHITE, 0xff00_0000, 0xff80_8080, MaskChannel::Blue);
        assert!((red(mid) - 128).abs() <= 1);
    }

    #[test]
    fn buffers_must_match_the_size() {
        assert!(operate(&[0; 3], &[0; 4], 2, 2, Operator::SourceIn).is_none());
        assert!(blend(&[0; 4], &[0; 4], 2, 2, ExtraBlend::Divide).is_some());
        assert!(blend_with_mask(&[0; 4], &[0; 4], &[0; 2], 2, 2, MaskChannel::Green).is_none());
    }

    #[test]
    fn packed_colour_never_exceeds_alpha() {
        for n in blend_names() {
            let m = ExtraBlend::named(n).unwrap();
            let out = blend_pixel(0x80ff_ffff & 0x8080_8080, 0x4040_4040, m);
            let a = out >> 24;
            assert!((out & 0xff) <= a && (out >> 8 & 0xff) <= a && (out >> 16 & 0xff) <= a, "{n}");
        }
    }
}

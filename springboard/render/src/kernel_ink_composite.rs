//! Six-channel ink composite. A stroke layer stores, per pixel, three premultiplied colour channels and
//! three independent coverage channels (`[r, g, b, a_r, a_g, a_b]`), one coverage per colour channel.
//! With equal coverages it is ordinary premultiplied source-over; with unequal coverages a stroke can
//! cover the channels by different amounts, which is what sub-pixel-positioned strokes and pigment inks
//! need (a pigment absorbs some wavelengths more than others). Two operators are provided: `over`
//! (per-channel source-over) and `pigment` (a subtractive, multiplicative ink that tints what lies
//! under it, the model for marker and watercolour washes). `flatten` resolves a layer onto paper. The
//! single-coverage compositing of finished layers is `blend`; stroke geometry is `ink` and
//! `stroke_coverage`. This is BifrOSt's own definition of the six-channel model, not a reproduction of
//! any other implementation.

/// One pixel: premultiplied colour then per-channel coverage.
pub type Px6 = [f32; 6];

/// A transparent pixel.
pub const CLEAR: Px6 = [0.0; 6];

/// A six-channel pixel from straight (not premultiplied) colour and one coverage for all channels.
pub fn from_straight(rgb: [f32; 3], alpha: f32) -> Px6 {
    let a = alpha.clamp(0.0, 1.0);
    [rgb[0] * a, rgb[1] * a, rgb[2] * a, a, a, a]
}

/// A six-channel pixel from straight colour with a separate coverage per channel.
pub fn from_channels(rgb: [f32; 3], coverage: [f32; 3]) -> Px6 {
    let a = [coverage[0].clamp(0.0, 1.0), coverage[1].clamp(0.0, 1.0), coverage[2].clamp(0.0, 1.0)];
    [rgb[0] * a[0], rgb[1] * a[1], rgb[2] * a[2], a[0], a[1], a[2]]
}

/// Per-channel source-over: `out = src + dst * (1 - a_src)` for colour, and
/// `a_out = a_src + a_dst * (1 - a_src)` for coverage, channel by channel.
pub fn over(src: Px6, dst: Px6) -> Px6 {
    let mut out = [0.0f32; 6];
    for c in 0..3 {
        let keep = 1.0 - src[3 + c];
        out[c] = src[c] + dst[c] * keep;
        out[3 + c] = src[3 + c] + dst[3 + c] * keep;
    }
    out
}

/// Pigment (subtractive) ink. The stroke's straight colour `tint` is what the ink lets through: under a
/// pixel of coverage `a`, each destination channel is scaled by `1 - a * (1 - tint)`, so a full-coverage
/// red-brown tint passes red and absorbs the rest, and overlapping washes darken. The stroke's own
/// coverage accumulates like `over`'s, and its colour is added so the layer still flattens correctly onto
/// paper.
pub fn pigment(tint: [f32; 3], coverage: [f32; 3], dst: Px6) -> Px6 {
    let mut out = [0.0f32; 6];
    for c in 0..3 {
        let a = coverage[c].clamp(0.0, 1.0);
        let pass = 1.0 - a * (1.0 - tint[c].clamp(0.0, 1.0));
        out[c] = dst[c] * pass;
        out[3 + c] = a + dst[3 + c] * (1.0 - a);
    }
    out
}

/// Resolves a layer onto opaque paper: `colour + paper * (1 - coverage)` per channel.
pub fn flatten(px: Px6, paper: [f32; 3]) -> [f32; 3] {
    [px[0] + paper[0] * (1.0 - px[3]), px[1] + paper[1] * (1.0 - px[4]), px[2] + paper[2] * (1.0 - px[5])]
}

/// Composites a stroke layer over a destination layer in place with [`over`]. `false` (and no change)
/// when the buffers differ in length.
pub fn over_buffer(src: &[Px6], dst: &mut [Px6]) -> bool {
    if src.len() != dst.len() {
        return false;
    }
    for (s, d) in src.iter().zip(dst.iter_mut()) {
        *d = over(*s, *d);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn equal_coverage_is_ordinary_premultiplied_source_over() {
        let src = from_straight([1.0, 0.0, 0.0], 0.5);
        let dst = from_straight([0.0, 0.0, 1.0], 1.0);
        let out = over(src, dst);
        // Premultiplied over: red 0.5, blue 0.5 on an opaque base.
        assert!(close(out[0], 0.5) && close(out[1], 0.0) && close(out[2], 0.5));
        assert!(close(out[3], 1.0) && close(out[4], 1.0) && close(out[5], 1.0));
    }

    #[test]
    fn unequal_coverage_covers_channels_separately() {
        let paper = [1.0f32; 3];
        let white = from_straight(paper, 1.0);
        // A black stroke that covers green and blue completely but not red leaves a red result.
        let stroke = from_channels([0.0, 0.0, 0.0], [0.0, 1.0, 1.0]);
        let out = flatten(over(stroke, white), paper);
        assert!(close(out[0], 1.0) && close(out[1], 0.0) && close(out[2], 0.0));
    }

    #[test]
    fn clear_over_anything_changes_nothing() {
        let dst = from_channels([0.2, 0.4, 0.6], [0.3, 0.6, 0.9]);
        assert_eq!(over(CLEAR, dst), dst);
        assert_eq!(over(dst, CLEAR), dst);
    }

    #[test]
    fn pigment_tints_what_is_under_it_and_washes_darken() {
        let white = from_straight([1.0; 3], 1.0);
        let tint = [0.2, 0.4, 1.0];
        let once = pigment(tint, [1.0; 3], white);
        assert!(close(once[0], 0.2) && close(once[1], 0.4) && close(once[2], 1.0));
        let twice = pigment(tint, [1.0; 3], once);
        assert!(close(twice[0], 0.04) && twice[1] < once[1]);
        // Half coverage is halfway between untouched and fully tinted.
        let half = pigment(tint, [0.5; 3], white);
        assert!(close(half[0], 0.6));
        // The destination coverage is preserved on an opaque base.
        assert!(close(once[3], 1.0));
    }

    #[test]
    fn flatten_puts_a_layer_onto_paper() {
        let faint = from_straight([0.0, 0.0, 0.0], 0.25);
        let out = flatten(faint, [1.0, 0.8, 0.6]);
        assert!(close(out[0], 0.75) && close(out[1], 0.6) && close(out[2], 0.45));
        assert_eq!(flatten(CLEAR, [0.3, 0.3, 0.3]), [0.3, 0.3, 0.3]);
    }

    #[test]
    fn buffer_composite_checks_lengths() {
        let src = [from_straight([1.0, 1.0, 1.0], 1.0), CLEAR];
        let mut dst = [CLEAR, from_straight([0.0, 1.0, 0.0], 1.0)];
        assert!(over_buffer(&src, &mut dst));
        assert_eq!(dst[0], from_straight([1.0, 1.0, 1.0], 1.0));
        assert_eq!(dst[1], from_straight([0.0, 1.0, 0.0], 1.0));
        assert!(!over_buffer(&src[..1], &mut dst));
    }
}

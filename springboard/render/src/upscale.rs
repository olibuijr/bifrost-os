//! Spatial upscaler for CPU rasters: a separable bicubic filter with a colour processing mode.
//!
//! The filter is Catmull-Rom (cubic convolution, a = -0.5). An output pixel centre maps onto the input grid as
//! `src = (dst + 0.5) * in / out - 0.5`. Taps outside the input clamp to the nearest edge sample. The four weights
//! sum to one, so a flat area stays flat. The horizontal pass runs first, then the vertical pass.
//!
//! The colour mode states how the samples are encoded, and so how the filter treats them. Only the filter is
//! modelled here: there is no GPU pass, no edge-directed analysis and no temporal input.

use crate::colorspace::Space;

/// How the samples of the input and output rasters are encoded, and so how the filter treats them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorProcessing {
    /// Linear light in 0..=1. Filtered as given, and the result is clamped to 0..=1.
    Linear,
    /// sRGB-encoded in 0..=1. Decoded to linear light, filtered, clamped and encoded again.
    Perceptual,
    /// Linear light where values above 1 are kept. Filtered as given and not clamped.
    Hdr,
}

/// Why an upscale request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscaleError {
    /// A source or output dimension is zero.
    ZeroSize,
    /// The sample count does not match the source dimensions.
    LengthMismatch,
    /// Values above 1 cannot be stored in 8 bits, so HDR needs `upscale_f32`.
    HdrNeedsFloat,
}

impl core::fmt::Display for UpscaleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            UpscaleError::ZeroSize => write!(f, "upscale: a dimension is zero"),
            UpscaleError::LengthMismatch => write!(f, "upscale: sample count does not match the source size"),
            UpscaleError::HdrNeedsFloat => write!(f, "upscale: HDR samples need the f32 path"),
        }
    }
}

/// Upscales (or downscales) an RGB raster of `[f32; 3]` samples.
pub fn upscale_f32(
    src: &[[f32; 3]],
    w: usize,
    h: usize,
    out_w: usize,
    out_h: usize,
    mode: ColorProcessing,
) -> Result<Vec<[f32; 3]>, UpscaleError> {
    check(src.len(), w, h, out_w, out_h)?;
    Ok(filter(src, w, h, out_w, out_h, mode))
}

/// Upscales an 8-bit sRGB or linear RGB raster (three bytes per pixel). `ColorProcessing::Hdr` is refused.
pub fn upscale_rgb8(
    src: &[u8],
    w: usize,
    h: usize,
    out_w: usize,
    out_h: usize,
    mode: ColorProcessing,
) -> Result<Vec<u8>, UpscaleError> {
    if mode == ColorProcessing::Hdr {
        return Err(UpscaleError::HdrNeedsFloat);
    }
    check(src.len() / 3, w, h, out_w, out_h)?;
    if !src.len().is_multiple_of(3) {
        return Err(UpscaleError::LengthMismatch);
    }
    let samples: Vec<[f32; 3]> = src
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
        .collect();
    let out = filter(&samples, w, h, out_w, out_h, mode);
    Ok(out.iter().flat_map(|p| p.map(|v| (v * 255.0).round() as u8)).collect())
}

/// Checks the dimensions and the sample count. `len` is the number of pixels in `src`.
fn check(len: usize, w: usize, h: usize, out_w: usize, out_h: usize) -> Result<(), UpscaleError> {
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return Err(UpscaleError::ZeroSize);
    }
    match w.checked_mul(h) {
        Some(n) if n == len => Ok(()),
        _ => Err(UpscaleError::LengthMismatch),
    }
}

/// Catmull-Rom weights for the four taps around a position with fraction `t` in 0..1.
fn weights(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

/// The base tap index and the fraction for output index `dst` on an axis of `out` samples over `src_len` samples.
fn locate(dst: usize, out: usize, src_len: usize) -> (isize, f32) {
    let s = (dst as f32 + 0.5) * src_len as f32 / out as f32 - 0.5;
    let i = s.floor();
    (i as isize, s - i)
}

/// Clamps a tap index to the valid range `0..n`. `n` is never zero here.
fn clamp_index(i: isize, n: usize) -> usize {
    i.clamp(0, n as isize - 1) as usize
}

fn filter(src: &[[f32; 3]], w: usize, h: usize, out_w: usize, out_h: usize, mode: ColorProcessing) -> Vec<[f32; 3]> {
    // Work in linear light for the perceptual mode; the other modes filter the samples as given.
    let lin: Vec<[f32; 3]> = if mode == ColorProcessing::Perceptual {
        src.iter().map(|p| p.map(|v| Space::Srgb.to_linear(v as f64) as f32)).collect()
    } else {
        src.to_vec()
    };

    // Horizontal pass: h rows of w samples become h rows of out_w samples.
    let mut tmp = vec![[0.0f32; 3]; out_w * h];
    for y in 0..h {
        for x in 0..out_w {
            let (i, t) = locate(x, out_w, w);
            let wt = weights(t);
            let mut acc = [0.0f32; 3];
            for (k, wk) in wt.iter().enumerate() {
                let p = lin[y * w + clamp_index(i + k as isize - 1, w)];
                for c in 0..3 {
                    acc[c] += wk * p[c];
                }
            }
            tmp[y * out_w + x] = acc;
        }
    }

    // Vertical pass: h rows of out_w samples become out_h rows.
    let mut out = vec![[0.0f32; 3]; out_w * out_h];
    for y in 0..out_h {
        let (j, t) = locate(y, out_h, h);
        let wt = weights(t);
        for x in 0..out_w {
            let mut acc = [0.0f32; 3];
            for (k, wk) in wt.iter().enumerate() {
                let p = tmp[clamp_index(j + k as isize - 1, h) * out_w + x];
                for c in 0..3 {
                    acc[c] += wk * p[c];
                }
            }
            out[y * out_w + x] = match mode {
                ColorProcessing::Linear => acc.map(|v| v.clamp(0.0, 1.0)),
                ColorProcessing::Perceptual => {
                    acc.map(|v| Space::Srgb.from_linear(v.clamp(0.0, 1.0) as f64) as f32)
                }
                ColorProcessing::Hdr => acc,
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn catmull_rom_weights_match_the_hand_worked_values() {
        // At t = 0 the sample itself has weight one; at t = 0.5 the two middle taps are 9/16 each.
        assert_eq!(weights(0.0), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(weights(0.5), [-0.0625, 0.5625, 0.5625, -0.0625]);
        let t = 0.3;
        let sum: f32 = weights(t).iter().sum();
        assert!(near(sum, 1.0));
    }

    #[test]
    fn same_size_returns_the_input_unchanged() {
        let src: Vec<u8> = (0..3 * 2 * 2).map(|i| (i * 7) as u8).collect();
        let out = upscale_rgb8(&src, 2, 2, 2, 2, ColorProcessing::Linear).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn flat_raster_stays_flat_at_any_output_size() {
        let src = vec![100u8; 3 * 3 * 3];
        let out = upscale_rgb8(&src, 3, 3, 5, 7, ColorProcessing::Linear).unwrap();
        assert_eq!(out.len(), 3 * 5 * 7);
        assert!(out.iter().all(|&v| v == 100));
    }

    #[test]
    fn hdr_keeps_the_overshoot_of_a_two_pixel_edge() {
        // Row [0, 1] upscaled to 4 samples. The tap positions are -0.25, 0.25, 0.75 and 1.25 in the source.
        // The first output sample is w3(0.75) = -0.0703125, the last is 1 - w0(0.25) = 1.0703125.
        let src = [[0.0f32; 3], [1.0; 3]];
        let out = upscale_f32(&src, 2, 1, 4, 1, ColorProcessing::Hdr).unwrap();
        let want = [-0.0703125f32, 0.203125, 0.796875, 1.0703125];
        for (got, w) in out.iter().zip(want) {
            assert!(near(got[0], w), "got {} want {}", got[0], w);
        }
    }

    #[test]
    fn linear_mode_clamps_the_same_edge_to_the_unit_range() {
        let src = [[0.0f32; 3], [1.0; 3]];
        let out = upscale_f32(&src, 2, 1, 4, 1, ColorProcessing::Linear).unwrap();
        let want = [0.0f32, 0.203125, 0.796875, 1.0];
        for (got, w) in out.iter().zip(want) {
            assert!(near(got[0], w), "got {} want {}", got[0], w);
        }
    }

    #[test]
    fn midpoint_of_black_and_white_depends_on_the_colour_mode() {
        // Two bytes 0 and 255 scaled to three samples. The middle sample lies halfway between them.
        // Linear mode averages the bytes: 0.5 * 255 = 127.5, which rounds to 128.
        // Perceptual mode averages in linear light: sRGB encoding of 0.5 is 0.7355, times 255 is 187.5, rounds to 188.
        let src = [0u8, 0, 0, 255, 255, 255];
        let lin = upscale_rgb8(&src, 2, 1, 3, 1, ColorProcessing::Linear).unwrap();
        assert_eq!(lin[3], 128);
        let perc = upscale_rgb8(&src, 2, 1, 3, 1, ColorProcessing::Perceptual).unwrap();
        assert_eq!(perc[3], 188);
    }

    #[test]
    fn refuses_zero_sizes_bad_lengths_and_hdr_bytes() {
        let src = [0u8; 12];
        assert_eq!(upscale_rgb8(&src, 0, 2, 2, 2, ColorProcessing::Linear), Err(UpscaleError::ZeroSize));
        assert_eq!(upscale_rgb8(&src, 2, 2, 0, 2, ColorProcessing::Linear), Err(UpscaleError::ZeroSize));
        assert_eq!(upscale_rgb8(&src, 3, 2, 2, 2, ColorProcessing::Linear), Err(UpscaleError::LengthMismatch));
        assert_eq!(upscale_rgb8(&src, 2, 2, 2, 2, ColorProcessing::Hdr), Err(UpscaleError::HdrNeedsFloat));
        assert_eq!(upscale_f32(&[[0.0; 3]; 3], 2, 2, 2, 2, ColorProcessing::Linear), Err(UpscaleError::LengthMismatch));
    }
}

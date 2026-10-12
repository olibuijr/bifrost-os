//! Bayer colour-filter-array demosaic with white balance, as a pure model (std only).
//!
//! A camera sensor records one colour per photosite, in a 2x2 tile that repeats over
//! the frame. `demosaic` turns such a mosaic into interleaved RGB: a photosite keeps
//! its own sample, and each missing colour is the mean of that colour's samples in the
//! 3x3 neighbourhood (cut at the frame edge). Per-channel white balance gains are then
//! applied and the result is rounded and clamped to 16 bits. It reads no sensor, parses
//! no RAW container and applies no colour matrix or tone curve.

/// The order of the four colours in the 2x2 tile. Names give the colour of the top-left,
/// top-right, bottom-left and bottom-right photosite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfaPattern {
    Rggb,
    Bggr,
    Grbg,
    Gbrg,
}

impl CfaPattern {
    /// Channel of the photosite at column `x` and row `y`: 0 red, 1 green, 2 blue.
    pub fn channel_at(self, x: usize, y: usize) -> usize {
        let tile: [usize; 4] = match self {
            CfaPattern::Rggb => [0, 1, 1, 2],
            CfaPattern::Bggr => [2, 1, 1, 0],
            CfaPattern::Grbg => [1, 0, 2, 1],
            CfaPattern::Gbrg => [1, 2, 0, 1],
        };
        tile[(y & 1) * 2 + (x & 1)]
    }
}

/// Why a frame was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum BayerError {
    /// The width or the height is zero.
    EmptyFrame,
    /// The sample count is not `width * height`.
    SizeMismatch { expected: usize, actual: usize },
    /// A white balance gain is negative or not finite. The field is the channel index.
    InvalidGain(usize),
}

/// Demosaics a mosaic of `width * height` 16-bit samples to `width * height * 3` RGB
/// samples, row by row, with `gains` for red, green and blue applied last.
pub fn demosaic(
    mosaic: &[u16],
    width: usize,
    height: usize,
    pattern: CfaPattern,
    gains: [f64; 3],
) -> Result<Vec<u16>, BayerError> {
    if width == 0 || height == 0 {
        return Err(BayerError::EmptyFrame);
    }
    let expected = width * height;
    if mosaic.len() != expected {
        return Err(BayerError::SizeMismatch { expected, actual: mosaic.len() });
    }
    for (channel, gain) in gains.iter().enumerate() {
        if !(gain.is_finite() && *gain >= 0.0) {
            return Err(BayerError::InvalidGain(channel));
        }
    }

    let mut out = vec![0u16; expected * 3];
    for y in 0..height {
        let y0 = y.saturating_sub(1);
        let y1 = (y + 1).min(height - 1);
        for x in 0..width {
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(width - 1);
            let own = pattern.channel_at(x, y);

            let mut sum = [0.0_f64; 3];
            let mut count = [0u32; 3];
            for ny in y0..=y1 {
                for nx in x0..=x1 {
                    let channel = pattern.channel_at(nx, ny);
                    sum[channel] += f64::from(mosaic[ny * width + nx]);
                    count[channel] += 1;
                }
            }

            let base = (y * width + x) * 3;
            for channel in 0..3 {
                let value = if channel == own {
                    f64::from(mosaic[y * width + x])
                } else if count[channel] == 0 {
                    0.0
                } else {
                    sum[channel] / f64::from(count[channel])
                };
                let gained = (value * gains[channel]).round().min(65535.0);
                out[base + channel] = gained as u16;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rggb_tile_places_red_top_left() {
        let p = CfaPattern::Rggb;
        assert_eq!(p.channel_at(0, 0), 0);
        assert_eq!(p.channel_at(1, 0), 1);
        assert_eq!(p.channel_at(0, 1), 1);
        assert_eq!(p.channel_at(1, 1), 2);
        assert_eq!(p.channel_at(2, 2), 0, "the tile repeats every two photosites");
    }

    #[test]
    fn other_patterns_shift_the_tile() {
        assert_eq!(CfaPattern::Bggr.channel_at(0, 0), 2);
        assert_eq!(CfaPattern::Grbg.channel_at(0, 0), 1);
        assert_eq!(CfaPattern::Grbg.channel_at(1, 0), 0);
        assert_eq!(CfaPattern::Gbrg.channel_at(1, 0), 2);
    }

    #[test]
    fn two_by_two_rggb_interpolates_the_missing_colours() {
        // R=100 at (0,0), G=40 at (1,0), G=60 at (0,1), B=20 at (1,1).
        let mosaic = [100u16, 40, 60, 20];
        let rgb = demosaic(&mosaic, 2, 2, CfaPattern::Rggb, [1.0, 1.0, 1.0]).unwrap();
        assert_eq!(&rgb[0..3], &[100, 50, 20], "red site: green is the mean of 40 and 60");
        assert_eq!(&rgb[3..6], &[100, 40, 20], "green site on the top row");
        assert_eq!(&rgb[6..9], &[100, 60, 20], "green site on the left column");
        assert_eq!(&rgb[9..12], &[100, 50, 20], "blue site: green is the mean of 40 and 60");
    }

    #[test]
    fn white_balance_gains_apply_after_interpolation() {
        let mosaic = [100u16, 40, 60, 20];
        let rgb = demosaic(&mosaic, 2, 2, CfaPattern::Rggb, [2.0, 1.0, 0.5]).unwrap();
        assert_eq!(&rgb[0..3], &[200, 50, 10]);
    }

    #[test]
    fn gained_values_clamp_to_sixteen_bits() {
        let rgb = demosaic(&[60000u16], 1, 1, CfaPattern::Rggb, [2.0, 1.0, 1.0]).unwrap();
        assert_eq!(rgb, vec![65535, 0, 0], "a 1x1 frame has no neighbours for green or blue");
    }

    #[test]
    fn a_flat_mosaic_stays_flat() {
        let mosaic = [1000u16; 9];
        let rgb = demosaic(&mosaic, 3, 3, CfaPattern::Gbrg, [1.0, 1.0, 1.0]).unwrap();
        assert_eq!(rgb.len(), 27);
        assert!(rgb.iter().all(|&v| v == 1000));
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(
            demosaic(&[], 0, 4, CfaPattern::Rggb, [1.0; 3]).unwrap_err(),
            BayerError::EmptyFrame
        );
        assert_eq!(
            demosaic(&[1, 2, 3], 2, 2, CfaPattern::Rggb, [1.0; 3]).unwrap_err(),
            BayerError::SizeMismatch { expected: 4, actual: 3 }
        );
        assert_eq!(
            demosaic(&[1, 2, 3, 4], 2, 2, CfaPattern::Rggb, [-1.0, 1.0, 1.0]).unwrap_err(),
            BayerError::InvalidGain(0)
        );
        assert_eq!(
            demosaic(&[1, 2, 3, 4], 2, 2, CfaPattern::Rggb, [1.0, 1.0, f64::NAN]).unwrap_err(),
            BayerError::InvalidGain(2)
        );
    }
}

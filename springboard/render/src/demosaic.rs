//! Bayer demosaicing for RAW sensor data. A sensor records one colour per photosite in a repeating
//! 2x2 colour filter array (CFA); `demosaic_bilinear` rebuilds full RGB at every pixel by averaging
//! the neighbours of each missing channel. Pure: no file or device access, so the RAW pipeline can
//! run it on any mosaic it has decoded.

/// The 2x2 colour filter layout of a sensor, named by its top-left pair of photosites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cfa {
    Rggb,
    Bggr,
    Grbg,
    Gbrg,
}

impl Cfa {
    /// The colour channel (0 red, 1 green, 2 blue) recorded at photosite `(x, y)`.
    pub fn channel(self, x: usize, y: usize) -> usize {
        let (px, py) = (x & 1, y & 1);
        let pattern = match self {
            Cfa::Rggb => [[0, 1], [1, 2]],
            Cfa::Bggr => [[2, 1], [1, 0]],
            Cfa::Grbg => [[1, 0], [2, 1]],
            Cfa::Gbrg => [[1, 2], [0, 1]],
        };
        pattern[py][px]
    }
}

/// Bilinear demosaic of a row-major `w` by `h` mosaic. Each recorded sample is kept as it is; each
/// missing channel is the mean of the same-channel samples in the 3x3 window around the pixel,
/// clipped at the image edge. `None` when either size is zero or `raw` does not hold `w * h` samples.
pub fn demosaic_bilinear(raw: &[u16], w: usize, h: usize, cfa: Cfa) -> Option<Vec<[u16; 3]>> {
    if w == 0 || h == 0 || raw.len() != w.checked_mul(h)? {
        return None;
    }
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let own = cfa.channel(x, y);
            let mut sum = [0u32; 3];
            let mut count = [0u32; 3];
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let c = cfa.channel(nx, ny);
                    sum[c] += u32::from(raw[ny * w + nx]);
                    count[c] += 1;
                }
            }
            let mut px = [0u16; 3];
            for c in 0..3 {
                px[c] = if c == own {
                    raw[y * w + x]
                } else if count[c] == 0 {
                    0
                } else {
                    ((sum[c] + count[c] / 2) / count[c]) as u16
                };
            }
            out.push(px);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Cfa; 4] = [Cfa::Rggb, Cfa::Bggr, Cfa::Grbg, Cfa::Gbrg];

    /// Samples a constant RGB scene through the CFA, so the mosaic holds only one colour per site.
    fn mosaic(rgb: [u16; 3], w: usize, h: usize, cfa: Cfa) -> Vec<u16> {
        let mut raw = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                raw.push(rgb[cfa.channel(x, y)]);
            }
        }
        raw
    }

    #[test]
    fn channel_follows_the_bayer_pattern() {
        assert_eq!(Cfa::Rggb.channel(0, 0), 0);
        assert_eq!(Cfa::Rggb.channel(1, 0), 1);
        assert_eq!(Cfa::Rggb.channel(0, 1), 1);
        assert_eq!(Cfa::Rggb.channel(1, 1), 2);
        assert_eq!(Cfa::Rggb.channel(2, 2), 0);
        assert_eq!(Cfa::Bggr.channel(0, 0), 2);
        assert_eq!(Cfa::Grbg.channel(0, 0), 1);
        assert_eq!(Cfa::Grbg.channel(1, 0), 0);
        assert_eq!(Cfa::Gbrg.channel(0, 0), 1);
        assert_eq!(Cfa::Gbrg.channel(0, 1), 0);
    }

    #[test]
    fn rejects_empty_and_mismatched_input() {
        assert!(demosaic_bilinear(&[], 0, 0, Cfa::Rggb).is_none());
        assert!(demosaic_bilinear(&[1, 2, 3], 2, 2, Cfa::Rggb).is_none());
        assert!(demosaic_bilinear(&[1, 2], 0, 2, Cfa::Rggb).is_none());
        assert!(demosaic_bilinear(&[7], 1, 1, Cfa::Rggb).is_some());
    }

    #[test]
    fn recorded_samples_are_kept_unchanged() {
        let raw = [10u16, 20, 30, 40];
        let rgb = demosaic_bilinear(&raw, 2, 2, Cfa::Rggb).unwrap();
        assert_eq!(rgb[0][0], 10);
        assert_eq!(rgb[1][1], 20);
        assert_eq!(rgb[2][1], 30);
        assert_eq!(rgb[3][2], 40);
    }

    #[test]
    fn a_constant_scene_is_recovered_at_every_pixel_for_every_layout() {
        let rgb = [200u16, 100, 50];
        for cfa in ALL {
            let (w, h) = (5, 4);
            let out = demosaic_bilinear(&mosaic(rgb, w, h, cfa), w, h, cfa).unwrap();
            assert_eq!(out.len(), w * h);
            for (i, px) in out.iter().enumerate() {
                assert_eq!(*px, rgb, "{cfa:?} pixel {i}");
            }
        }
    }

    #[test]
    fn a_missing_green_is_the_mean_of_its_four_neighbours() {
        // RGGB 3x3: the centre is a blue site, so its green is the mean of the four orthogonal greens.
        let raw = [0u16, 10, 0, 20, 0, 40, 0, 30, 0];
        let out = demosaic_bilinear(&raw, 3, 3, Cfa::Rggb).unwrap();
        assert_eq!(out[4][1], 25);
    }

    #[test]
    fn edges_average_only_the_neighbours_that_exist() {
        // Top-left red site in a 2x1 image: the blue channel has no sample in its window.
        let raw = [100u16, 50];
        let out = demosaic_bilinear(&raw, 2, 1, Cfa::Rggb).unwrap();
        assert_eq!(out[0][2], 0);
        assert_eq!(out[0][1], 50);
        assert_eq!(out[1][0], 100);
    }
}

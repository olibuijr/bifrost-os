//! Photo analysis on the device: the quality measures a photo library needs to rank, filter and de-duplicate pictures
//! without sending them anywhere. A `PhotoReport` carries the mean brightness, the share of crushed shadows and blown
//! highlights, the contrast, a focus measure (variance of the Laplacian) and the colourfulness (Hasler and Susstrunk),
//! with a plain verdict for exposure and focus. `difference_hash` is a 64-bit perceptual hash of the picture's
//! structure, so two pictures of the same scene have hashes a few bits apart whatever their size or brightness.
//!
//! These are the classical measures, written from their published definitions; no trained model is involved, and the
//! verdict thresholds are fixed rules of thumb, not learned. Learned scores (aesthetics, subject recognition) are not
//! covered here. Pixels are packed `u32` values, red in the low byte, as in `gfx::Image`; the channels are read as
//! straight (unpremultiplied) colour and the alpha channel is ignored.

use crate::gfx::Image;

/// A focus measure (variance of the 4-neighbour Laplacian on 0..=255 luma) below this is a blurred picture.
pub const BLURRY_BELOW: f32 = 100.0;
/// Below this (and at or above `BLURRY_BELOW`) the picture is soft; at or above it, sharp.
pub const SOFT_BELOW: f32 = 400.0;

/// How well exposed the picture is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    Under,
    Good,
    Over,
}

/// How sharp the picture is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Blurry,
    Soft,
    Sharp,
}

/// The measures of one picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoReport {
    /// Mean luma, 0 (black) to 1 (white).
    pub mean_luma: f32,
    /// Standard deviation of the luma, 0..=0.5 (0.5 is half black, half white).
    pub contrast: f32,
    /// Share of pixels with luma of 5 or less (of 255): crushed shadows.
    pub shadow_clip: f32,
    /// Share of pixels with luma of 250 or more: blown highlights.
    pub highlight_clip: f32,
    /// Variance of the Laplacian of the luma (0..=255 scale); higher is sharper.
    pub sharpness: f32,
    /// Hasler and Susstrunk colourfulness on the 0..=255 channel scale: 0 for grey, about 100 and more for vivid.
    pub colourfulness: f32,
    pub exposure: Exposure,
    pub focus: Focus,
}

/// Rec. 601 luma of a packed pixel on 0..=255.
fn luma(p: u32) -> f32 {
    let (r, g, b) = channels(p);
    0.299 * r + 0.587 * g + 0.114 * b
}

fn channels(p: u32) -> (f32, f32, f32) {
    ((p & 0xff) as f32, ((p >> 8) & 0xff) as f32, ((p >> 16) & 0xff) as f32)
}

/// The exposure verdict from the mean luma and the clipped shares.
pub fn classify_exposure(mean_luma: f32, shadow_clip: f32, highlight_clip: f32) -> Exposure {
    if mean_luma < 0.2 || shadow_clip > 0.5 {
        Exposure::Under
    } else if mean_luma > 0.8 || highlight_clip > 0.5 {
        Exposure::Over
    } else {
        Exposure::Good
    }
}

/// The focus verdict from a sharpness measure.
pub fn classify_focus(sharpness: f32) -> Focus {
    if sharpness < BLURRY_BELOW {
        Focus::Blurry
    } else if sharpness < SOFT_BELOW {
        Focus::Soft
    } else {
        Focus::Sharp
    }
}

/// Measures a picture. `None` when the pixel count does not match the size or either side is under 3 pixels (the
/// Laplacian needs a neighbourhood).
pub fn analyse(img: &Image) -> Option<PhotoReport> {
    let (w, h) = (img.w, img.h);
    if w < 3 || h < 3 || img.px.len() != w * h {
        return None;
    }
    let count = (w * h) as f64;
    let lumas: Vec<f32> = img.px.iter().map(|&p| luma(p)).collect();

    let mean = lumas.iter().map(|&l| l as f64).sum::<f64>() / count;
    let var = lumas.iter().map(|&l| (l as f64 - mean).powi(2)).sum::<f64>() / count;
    let shadow = lumas.iter().filter(|&&l| l <= 5.0).count() as f64 / count;
    let highlight = lumas.iter().filter(|&&l| l >= 250.0).count() as f64 / count;

    // Variance of the 4-neighbour Laplacian over the interior (the border has no full neighbourhood).
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut n = 0.0f64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let c = lumas[y * w + x];
            let lap = (lumas[y * w + x - 1] + lumas[y * w + x + 1] + lumas[(y - 1) * w + x] + lumas[(y + 1) * w + x]
                - 4.0 * c) as f64;
            sum += lap;
            sum_sq += lap * lap;
            n += 1.0;
        }
    }
    let sharpness = (sum_sq / n - (sum / n).powi(2)).max(0.0);

    // Colourfulness from the opponent channels rg = R - G and yb = (R + G) / 2 - B.
    let (mut rg_sum, mut rg_sq, mut yb_sum, mut yb_sq) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &p in &img.px {
        let (r, g, b) = channels(p);
        let rg = (r - g) as f64;
        let yb = (0.5 * (r + g) - b) as f64;
        rg_sum += rg;
        rg_sq += rg * rg;
        yb_sum += yb;
        yb_sq += yb * yb;
    }
    let (rg_mean, yb_mean) = (rg_sum / count, yb_sum / count);
    let rg_std = (rg_sq / count - rg_mean * rg_mean).max(0.0).sqrt();
    let yb_std = (yb_sq / count - yb_mean * yb_mean).max(0.0).sqrt();
    let colourfulness = (rg_std * rg_std + yb_std * yb_std).sqrt() + 0.3 * (rg_mean * rg_mean + yb_mean * yb_mean).sqrt();

    let mean_luma = (mean / 255.0) as f32;
    let (shadow_clip, highlight_clip) = (shadow as f32, highlight as f32);
    let sharpness = sharpness as f32;
    Some(PhotoReport {
        mean_luma,
        contrast: (var.sqrt() / 255.0) as f32,
        shadow_clip,
        highlight_clip,
        sharpness,
        colourfulness: colourfulness as f32,
        exposure: classify_exposure(mean_luma, shadow_clip, highlight_clip),
        focus: classify_focus(sharpness),
    })
}

/// A 64-bit difference hash: the picture is averaged down to a 9 by 8 grid of luma and each bit says whether a cell is
/// darker than its right-hand neighbour. It follows the picture's structure, so scaling, a brightness or contrast
/// change and mild compression leave it (almost) unchanged. `None` for an empty picture or a pixel count that does
/// not match the size.
pub fn difference_hash(img: &Image) -> Option<u64> {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() != w * h {
        return None;
    }
    let mut grid = [[0.0f32; 9]; 8];
    for (gy, row) in grid.iter_mut().enumerate() {
        let y0 = gy * h / 8;
        let y1 = ((gy + 1) * h / 8).max(y0 + 1).min(h);
        for (gx, cell) in row.iter_mut().enumerate() {
            let x0 = gx * w / 9;
            let x1 = ((gx + 1) * w / 9).max(x0 + 1).min(w);
            let mut sum = 0.0f32;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += luma(img.px[y * w + x]);
                }
            }
            *cell = sum / ((y1 - y0) * (x1 - x0)) as f32;
        }
    }
    let mut hash = 0u64;
    for (gy, row) in grid.iter().enumerate() {
        for gx in 0..8 {
            if row[gx] < row[gx + 1] {
                hash |= 1 << (gy * 8 + gx);
            }
        }
    }
    Some(hash)
}

/// The number of differing bits between two hashes, 0 (same structure) to 64.
pub fn hash_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Whether two hashes are close enough to be the same scene. A distance up to 10 of 64 bits is the usual cut.
pub fn is_near_duplicate(a: u64, b: u64) -> bool {
    hash_distance(a, b) <= 10
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> u32 {
        0xff00_0000 | (b as u32) << 16 | (g as u32) << 8 | r as u32
    }

    fn grey(v: u8) -> u32 {
        rgb(v, v, v)
    }

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> u32) -> Image {
        Image { w, h, px: (0..w * h).map(|i| f(i % w, i / w)).collect() }
    }

    #[test]
    fn a_flat_grey_picture_has_no_detail_contrast_or_colour() {
        let r = analyse(&image(16, 16, |_, _| grey(128))).unwrap();
        assert!((r.mean_luma - 128.0 / 255.0).abs() < 1e-3);
        assert_eq!(r.contrast, 0.0);
        assert_eq!(r.sharpness, 0.0);
        assert_eq!(r.colourfulness, 0.0);
        assert_eq!(r.focus, Focus::Blurry);
        assert_eq!(r.exposure, Exposure::Good);
    }

    #[test]
    fn a_checkerboard_is_sharp_and_a_smooth_gradient_is_blurry() {
        let checker = analyse(&image(16, 16, |x, y| if (x + y) % 2 == 0 { grey(255) } else { grey(0) })).unwrap();
        assert!(checker.sharpness > SOFT_BELOW);
        assert_eq!(checker.focus, Focus::Sharp);
        let ramp = analyse(&image(32, 8, |x, _| grey((x * 8) as u8))).unwrap();
        assert!(ramp.sharpness < 1.0, "a linear ramp has a zero Laplacian, got {}", ramp.sharpness);
        assert_eq!(ramp.focus, Focus::Blurry);
    }

    #[test]
    fn softening_a_sharp_picture_lowers_its_focus_measure() {
        let sharp = image(24, 24, |x, _| if (x / 3) % 2 == 0 { grey(230) } else { grey(25) });
        // The same stripes with the edges blurred by a 3-pixel horizontal box average.
        let soft = image(24, 24, |x, _| {
            let at = |i: isize| (sharp.px[(i.clamp(0, 23)) as usize] & 0xff) as u32;
            let v = (at(x as isize - 1) + at(x as isize) + at(x as isize + 1)) / 3;
            grey(v as u8)
        });
        let (a, b) = (analyse(&sharp).unwrap(), analyse(&soft).unwrap());
        assert!(b.sharpness < a.sharpness, "{} should be below {}", b.sharpness, a.sharpness);
    }

    #[test]
    fn exposure_is_judged_from_brightness_and_clipping() {
        let dark = analyse(&image(8, 8, |_, _| grey(0))).unwrap();
        assert_eq!((dark.exposure, dark.shadow_clip), (Exposure::Under, 1.0));
        let bright = analyse(&image(8, 8, |_, _| grey(255))).unwrap();
        assert_eq!((bright.exposure, bright.highlight_clip), (Exposure::Over, 1.0));
        let mid = analyse(&image(8, 8, |x, _| if x < 4 { grey(60) } else { grey(190) })).unwrap();
        assert_eq!(mid.exposure, Exposure::Good);
        assert_eq!((mid.shadow_clip, mid.highlight_clip), (0.0, 0.0));
        // Mostly crushed shadows with a bright patch: the mean is low enough, and the clip share is high.
        assert_eq!(classify_exposure(0.4, 0.6, 0.0), Exposure::Under);
        assert_eq!(classify_exposure(0.6, 0.0, 0.7), Exposure::Over);
    }

    #[test]
    fn contrast_is_the_luma_spread() {
        let split = analyse(&image(8, 8, |x, _| if x < 4 { grey(0) } else { grey(255) })).unwrap();
        assert!((split.contrast - 0.5).abs() < 1e-3, "half black, half white is 0.5, got {}", split.contrast);
    }

    #[test]
    fn colourful_pictures_score_above_grey_ones() {
        let vivid = analyse(&image(8, 8, |x, _| if x < 4 { rgb(255, 0, 0) } else { rgb(0, 0, 255) })).unwrap();
        let grey_img = analyse(&image(8, 8, |x, _| if x < 4 { grey(40) } else { grey(200) })).unwrap();
        assert_eq!(grey_img.colourfulness, 0.0);
        assert!(vivid.colourfulness > 100.0, "got {}", vivid.colourfulness);
    }

    #[test]
    fn unusable_pictures_give_no_report() {
        assert!(analyse(&Image { w: 0, h: 0, px: vec![] }).is_none());
        assert!(analyse(&image(2, 8, |_, _| grey(1))).is_none(), "narrower than the Laplacian neighbourhood");
        assert!(analyse(&Image { w: 4, h: 4, px: vec![0; 5] }).is_none(), "pixel count does not match");
    }

    #[test]
    fn verdict_thresholds_split_at_their_constants() {
        assert_eq!(classify_focus(BLURRY_BELOW - 0.1), Focus::Blurry);
        assert_eq!(classify_focus(BLURRY_BELOW), Focus::Soft);
        assert_eq!(classify_focus(SOFT_BELOW - 0.1), Focus::Soft);
        assert_eq!(classify_focus(SOFT_BELOW), Focus::Sharp);
    }

    fn scene(w: usize, h: usize, bias: i32) -> Image {
        // A bright disc on a darker ramp; `bias` shifts the whole picture's brightness.
        image(w, h, |x, y| {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let inside = (fx - 0.6).powi(2) + (fy - 0.4).powi(2) < 0.05;
            let base = if inside { 200.0 } else { 40.0 + 80.0 * fx };
            grey((base as i32 + bias).clamp(0, 255) as u8)
        })
    }

    #[test]
    fn the_same_scene_hashes_alike_at_another_size_and_brightness() {
        let a = difference_hash(&scene(90, 80, 0)).unwrap();
        let b = difference_hash(&scene(180, 160, 0)).unwrap();
        let c = difference_hash(&scene(90, 80, 15)).unwrap();
        assert!(is_near_duplicate(a, b), "a rescaled copy is {} bits off", hash_distance(a, b));
        assert!(is_near_duplicate(a, c), "a brighter copy is {} bits off", hash_distance(a, c));
    }

    #[test]
    fn different_structure_hashes_far_apart() {
        let rising = difference_hash(&image(90, 80, |x, _| grey((x * 2) as u8))).unwrap();
        let falling = difference_hash(&image(90, 80, |x, _| grey(255 - (x * 2) as u8))).unwrap();
        assert_eq!(rising, u64::MAX, "every cell is darker than the cell to its right");
        assert_eq!(falling, 0);
        assert_eq!(hash_distance(rising, falling), 64);
        assert!(!is_near_duplicate(rising, falling));
        assert_eq!(hash_distance(rising, rising), 0);
    }

    #[test]
    fn a_tiny_picture_still_hashes_and_a_bad_one_does_not() {
        assert!(difference_hash(&image(1, 1, |_, _| grey(9))).is_some());
        assert!(difference_hash(&Image { w: 0, h: 3, px: vec![] }).is_none());
        assert!(difference_hash(&Image { w: 3, h: 3, px: vec![0; 4] }).is_none());
    }
}

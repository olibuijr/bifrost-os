//! Backdrops: the person stays, the room behind them is swapped for a colour gradient. There is no person-segmentation
//! model on the system, so this works the way the first Photo Booth backdrops did: the app takes one picture of the
//! empty scene (the person steps out of the frame), and afterwards every pixel that differs from that picture is the
//! foreground. The comparison runs on a small copy of the picture and the resulting mask is smoothed and scaled back
//! up when the picture is composed. A moved camera, a changed light or a person wearing the colour of the wall breaks
//! the mask; taking the empty scene again repairs the first two.

use bifrost_app::kit::{pack, Image};

/// The comparison picture is at most this wide, in pixels.
pub const ANALYSIS_W: usize = 160;
/// A pixel is foreground when its channels differ from the empty scene by more than this in total (of 765).
pub const DIFF_THRESHOLD: u32 = 90;
/// How long the person has to step out of the frame before the empty scene is taken, in ms.
pub const SETTLE_MS: f64 = 3000.0;
/// The mask value below which a pixel is background and above which it is foreground; between the two it is blended.
const EDGE_LO: f32 = 0.3;
const EDGE_HI: f32 = 0.7;

/// What replaces the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    Sunset,
    Ocean,
    Forest,
    Night,
}

impl Backdrop {
    pub const ALL: [Backdrop; 5] = [Backdrop::None, Backdrop::Sunset, Backdrop::Ocean, Backdrop::Forest, Backdrop::Night];

    pub fn title(self) -> &'static str {
        match self {
            Backdrop::None => "None",
            Backdrop::Sunset => "Sunset",
            Backdrop::Ocean => "Ocean",
            Backdrop::Forest => "Forest",
            Backdrop::Night => "Night Sky",
        }
    }

    /// The top and bottom colour of the gradient; black for no backdrop.
    fn stops(self) -> ([u8; 3], [u8; 3]) {
        match self {
            Backdrop::None => ([0, 0, 0], [0, 0, 0]),
            Backdrop::Sunset => ([250, 170, 80], [150, 40, 90]),
            Backdrop::Ocean => ([120, 200, 235], [10, 60, 130]),
            Backdrop::Forest => ([170, 210, 120], [20, 80, 50]),
            Backdrop::Night => ([20, 30, 90], [5, 5, 25]),
        }
    }

    /// The backdrop's colour at `t` from the top (0) to the bottom (1) of the picture.
    pub fn colour_at(self, t: f32) -> [u8; 3] {
        let (top, bottom) = self.stops();
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8;
        [mix(top[0], bottom[0]), mix(top[1], bottom[1]), mix(top[2], bottom[2])]
    }
}

/// The size the comparison picture takes for a camera frame of `w` by `h`: at most `ANALYSIS_W` wide, same aspect.
pub fn analysis_size(w: usize, h: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (1, 1);
    }
    let aw = w.min(ANALYSIS_W);
    (aw, (h * aw / w).max(1))
}

/// `img` shrunk to `w` by `h` by averaging the source pixels that fall in each output pixel. Unlike a point-sampled
/// resize it gives the same small picture for the same scene whether it starts from the full camera frame or from a
/// smaller preview, which is what the comparison with the empty scene needs.
pub fn shrink(img: &Image, w: usize, h: usize) -> Image {
    if w == 0 || h == 0 || img.w == 0 || img.h == 0 {
        return Image { w, h, px: vec![0; w * h] };
    }
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let y0 = y * img.h / h;
        let y1 = ((y + 1) * img.h / h).clamp(y0 + 1, img.h);
        for x in 0..w {
            let x0 = x * img.w / w;
            let x1 = ((x + 1) * img.w / w).clamp(x0 + 1, img.w);
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for p in &img.px[sy * img.w + x0..sy * img.w + x1] {
                    let c = channels(*p);
                    sum[0] += c[0];
                    sum[1] += c[1];
                    sum[2] += c[2];
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            px.push(pack((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255));
        }
    }
    Image { w, h, px }
}

/// The picture of the empty scene, at analysis size.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    small: Image,
}

impl Reference {
    /// Keeps `small` (a picture of the empty scene at `analysis_size`) as the reference.
    pub fn new(small: Image) -> Reference {
        Reference { small }
    }

    /// The reference for a camera picture of the empty scene, at any size.
    pub fn from_picture(img: &Image) -> Reference {
        let (w, h) = analysis_size(img.w, img.h);
        Reference::new(shrink(img, w, h))
    }

    /// True when the reference was taken from a camera picture with the same shape as `w` by `h`.
    pub fn fits(&self, w: usize, h: usize) -> bool {
        (self.small.w, self.small.h) == analysis_size(w, h)
    }
}

fn channels(p: u32) -> [u32; 3] {
    [p & 255, p >> 8 & 255, p >> 16 & 255]
}

/// One 3 by 3 box blur with the edge pixels repeated.
fn blur(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; src.len()];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                    let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                    sum += src[sy * w + sx];
                }
            }
            out[y * w + x] = sum / 9.0;
        }
    }
    out
}

fn smoothstep(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The foreground of `frame` against `reference` (both the same size) as one value per pixel, 0 for the background up
/// to 255 for the person. The raw difference is blurred twice so that sensor noise drops out and holes close, then the
/// result is steepened around one half.
pub fn foreground_mask(frame: &Image, reference: &Image) -> Vec<u8> {
    let (w, h) = (frame.w, frame.h);
    if w == 0 || h == 0 || (reference.w, reference.h) != (w, h) {
        return vec![0; w * h];
    }
    let raw: Vec<f32> = frame
        .px
        .iter()
        .zip(&reference.px)
        .map(|(a, b)| {
            let (a, b) = (channels(*a), channels(*b));
            let diff: u32 = (0..3).map(|i| a[i].abs_diff(b[i])).sum();
            if diff > DIFF_THRESHOLD {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let soft = blur(&blur(&raw, w, h), w, h);
    soft.iter().map(|v| (smoothstep(EDGE_LO, EDGE_HI, *v) * 255.0 + 0.5) as u8).collect()
}

/// The mask value at the point (`fx`, `fy`) in mask pixels, interpolated between the four nearest values.
fn sample(mask: &[u8], w: usize, h: usize, fx: f32, fy: f32) -> f32 {
    let fx = fx.clamp(0.0, (w - 1) as f32);
    let fy = fy.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: usize, y: usize| mask[y * w + x] as f32;
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

/// `img` with everything but the foreground replaced by `backdrop`. `Backdrop::None` returns the picture unchanged.
pub fn replace(img: &Image, reference: &Reference, backdrop: Backdrop) -> Image {
    if backdrop == Backdrop::None || img.w == 0 || img.h == 0 {
        return img.clone();
    }
    let (mw, mh) = (reference.small.w, reference.small.h);
    let mask = foreground_mask(&shrink(img, mw, mh), &reference.small);
    let cols: Vec<f32> = (0..img.w).map(|x| (x as f32 + 0.5) * mw as f32 / img.w as f32 - 0.5).collect();
    let mut out = Vec::with_capacity(img.px.len());
    for y in 0..img.h {
        let fy = (y as f32 + 0.5) * mh as f32 / img.h as f32 - 0.5;
        let bg = backdrop.colour_at((y as f32 + 0.5) / img.h as f32);
        for (x, fx) in cols.iter().enumerate() {
            let a = (sample(&mask, mw, mh, *fx, fy) + 0.5) as u32;
            let fg = channels(img.px[y * img.w + x]);
            let mix = |i: usize| ((bg[i] as u32 * (255 - a) + fg[i] * a + 127) / 255) as u8;
            out.push(pack(mix(0), mix(1), mix(2), 255));
        }
    }
    Image { w: img.w, h: img.h, px: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL: u32 = pack(100, 100, 100, 255);
    const PERSON: u32 = pack(220, 160, 140, 255);

    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    /// A picture of the wall with a block of `PERSON` of `bw` by `bh` centred in it.
    fn scene(w: usize, h: usize, bw: usize, bh: usize) -> Image {
        let mut img = solid(w, h, WALL);
        for y in (h - bh) / 2..(h + bh) / 2 {
            for x in (w - bw) / 2..(w + bw) / 2 {
                img.px[y * w + x] = PERSON;
            }
        }
        img
    }

    #[test]
    fn the_mask_marks_the_person_and_not_the_wall() {
        let mask = foreground_mask(&scene(24, 24, 10, 10), &solid(24, 24, WALL));
        assert_eq!(mask[12 * 24 + 12], 255);
        assert_eq!(mask[0], 0);
        assert_eq!(mask[23 * 24 + 23], 0);
    }

    #[test]
    fn an_unchanged_scene_has_no_foreground() {
        let wall = solid(16, 12, WALL);
        assert!(foreground_mask(&wall, &wall).iter().all(|a| *a == 0));
    }

    #[test]
    fn a_picture_that_is_all_different_is_all_foreground() {
        let mask = foreground_mask(&solid(10, 10, PERSON), &solid(10, 10, WALL));
        assert!(mask.iter().all(|a| *a == 255));
    }

    #[test]
    fn a_single_noisy_pixel_is_dropped() {
        let mut frame = solid(24, 24, WALL);
        frame.px[12 * 24 + 12] = PERSON;
        assert!(foreground_mask(&frame, &solid(24, 24, WALL)).iter().all(|a| *a == 0));
    }

    #[test]
    fn small_brightness_drift_stays_background() {
        let drifted = solid(16, 16, pack(108, 104, 110, 255));
        assert!(foreground_mask(&drifted, &solid(16, 16, WALL)).iter().all(|a| *a == 0));
    }

    #[test]
    fn a_reference_of_another_size_gives_an_empty_mask() {
        assert_eq!(foreground_mask(&solid(4, 4, PERSON), &solid(5, 5, WALL)), vec![0; 16]);
        assert!(foreground_mask(&Image { w: 0, h: 0, px: Vec::new() }, &solid(0, 0, WALL)).is_empty());
    }

    #[test]
    fn replace_keeps_the_person_and_swaps_the_wall() {
        let reference = Reference::new(solid(32, 18, WALL));
        let frame = scene(32, 18, 12, 8);
        let out = replace(&frame, &reference, Backdrop::Sunset);
        assert_eq!((out.w, out.h), (32, 18));
        assert_eq!(out.px[9 * 32 + 16], PERSON);
        let top = Backdrop::Sunset.colour_at(0.5 / 18.0);
        assert_eq!(out.px[0], pack(top[0], top[1], top[2], 255));
        let bottom = Backdrop::Sunset.colour_at(17.5 / 18.0);
        assert_eq!(out.px[17 * 32 + 31], pack(bottom[0], bottom[1], bottom[2], 255));
    }

    #[test]
    fn replace_scales_the_mask_up_to_a_larger_picture() {
        let (w, h) = (320, 180);
        assert_eq!(analysis_size(w, h), (160, 90));
        let reference = Reference::new(solid(160, 90, WALL));
        let out = replace(&scene(w, h, 120, 80), &reference, Backdrop::Ocean);
        assert_eq!(out.px[90 * w + 160], PERSON);
        let bg = Backdrop::Ocean.colour_at(5.5 / 180.0);
        assert_eq!(out.px[5 * w + 5], pack(bg[0], bg[1], bg[2], 255));
    }

    #[test]
    fn no_backdrop_leaves_the_picture_alone() {
        let frame = scene(16, 16, 6, 6);
        let reference = Reference::new(solid(16, 16, WALL));
        assert_eq!(replace(&frame, &reference, Backdrop::None), frame);
    }

    #[test]
    fn shrink_averages_each_block_of_source_pixels() {
        let mut img = solid(4, 2, pack(0, 0, 0, 255));
        img.px[0] = pack(100, 0, 0, 255);
        img.px[1] = pack(200, 40, 0, 255);
        let small = shrink(&img, 2, 1);
        assert_eq!((small.w, small.h), (2, 1));
        // the left block holds (100,0,0), (200,40,0), (0,0,0), (0,0,0)
        assert_eq!(small.px[0], pack(75, 10, 0, 255));
        assert_eq!(small.px[1], pack(0, 0, 0, 255));
    }

    #[test]
    fn shrink_to_the_same_size_is_the_identity_and_a_flat_picture_stays_flat() {
        let img = scene(16, 9, 6, 4);
        assert_eq!(shrink(&img, 16, 9), img);
        let flat = shrink(&solid(30, 17, WALL), 7, 5);
        assert!(flat.px.iter().all(|p| *p == WALL));
    }

    #[test]
    fn the_same_scene_shrinks_to_the_same_reference_from_two_sizes() {
        let big = scene(320, 180, 120, 80);
        let half = scene(160, 90, 60, 40);
        assert_eq!(Reference::from_picture(&big), Reference::from_picture(&half));
    }

    #[test]
    fn the_analysis_size_keeps_the_aspect_and_never_exceeds_the_limit() {
        assert_eq!(analysis_size(1280, 720), (160, 90));
        assert_eq!(analysis_size(100, 50), (100, 50));
        assert_eq!(analysis_size(0, 0), (1, 1));
    }

    #[test]
    fn a_reference_fits_only_a_camera_of_the_same_shape() {
        let reference = Reference::new(solid(160, 90, WALL));
        assert!(reference.fits(1280, 720));
        assert!(reference.fits(640, 360));
        assert!(!reference.fits(640, 480));
    }

    #[test]
    fn every_backdrop_runs_from_its_top_colour_to_its_bottom_colour() {
        for backdrop in Backdrop::ALL.iter().skip(1) {
            assert_ne!(backdrop.colour_at(0.0), backdrop.colour_at(1.0), "{backdrop:?}");
            assert_eq!(backdrop.colour_at(-1.0), backdrop.colour_at(0.0));
            assert_eq!(backdrop.colour_at(2.0), backdrop.colour_at(1.0));
        }
    }
}

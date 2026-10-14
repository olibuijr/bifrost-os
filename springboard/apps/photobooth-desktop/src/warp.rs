//! Photo Booth's picture warps: the distortion effects (bulge, twirl, mirror) and the horizontal flip behind Auto Flip.
//! All of them take and return an image of the same size. The bulge and the twirl run the renderer's distortion
//! filters over a disc centred in the picture, small enough that every sample stays inside the picture.

use bifrost_app::kit::Image;
use bifrost_render::distortion_filters as distortion;

/// Bulge strength: 0 is no change, 1 the strongest.
const BULGE_SCALE: f32 = 0.85;
/// Twirl angle at the centre, in radians.
const TWIRL_ANGLE: f32 = 2.4;

/// The disc the warps act on: the picture's centre and a radius that never leaves the picture.
fn disc(img: &Image) -> ((f32, f32), f32) {
    let radius = (img.w.min(img.h) as f32 / 2.0 - 1.0).max(0.0);
    ((img.w as f32 / 2.0, img.h as f32 / 2.0), radius)
}

/// The picture turned left to right, like a mirror.
pub fn flip_horizontal(img: &Image) -> Image {
    let mut px = Vec::with_capacity(img.px.len());
    for row in img.px.chunks(img.w.max(1)) {
        px.extend(row.iter().rev());
    }
    Image { w: img.w, h: img.h, px }
}

/// The left half reflected onto the right half, so the picture is symmetric about its vertical centre line.
pub fn mirror_left(img: &Image) -> Image {
    let mut out = img.clone();
    let w = img.w;
    for y in 0..img.h {
        for x in 0..w / 2 {
            out.px[y * w + (w - 1 - x)] = img.px[y * w + x];
        }
    }
    out
}

/// The centre of the picture swollen outwards, as in a fish-eye lens.
pub fn bulge(img: &Image) -> Image {
    let (centre, radius) = disc(img);
    distortion::bump(img, centre, radius, BULGE_SCALE)
}

/// The picture rotated around its centre, most at the middle and not at all at the edge of the disc.
pub fn twirl(img: &Image) -> Image {
    let (centre, radius) = disc(img);
    distortion::twirl(img, centre, radius, TWIRL_ANGLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::pack;

    /// A picture whose red channel is the column and whose green channel is the row.
    fn ramp(w: usize, h: usize) -> Image {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.push(pack(x as u8, y as u8, 0, 255));
            }
        }
        Image { w, h, px }
    }

    fn red(p: u32) -> u32 {
        p & 255
    }

    #[test]
    fn flip_turns_each_row_around_and_twice_is_the_identity() {
        let img = ramp(5, 3);
        let flipped = flip_horizontal(&img);
        assert_eq!(flipped.px[0], img.px[4]);
        assert_eq!(flipped.px[4], img.px[0]);
        assert_eq!(flipped.px[5], img.px[9]);
        assert_eq!(flip_horizontal(&flipped), img);
    }

    #[test]
    fn mirror_copies_the_left_half_onto_the_right() {
        let img = ramp(6, 2);
        let out = mirror_left(&img);
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(out.px[y * 6 + x], img.px[y * 6 + x]);
                assert_eq!(out.px[y * 6 + (5 - x)], img.px[y * 6 + x]);
            }
        }
    }

    #[test]
    fn mirror_leaves_the_middle_column_of_an_odd_width_alone() {
        let img = ramp(5, 1);
        let out = mirror_left(&img);
        assert_eq!(out.px[2], img.px[2]);
        assert_eq!(out.px[3], img.px[1]);
        assert_eq!(out.px[4], img.px[0]);
    }

    #[test]
    fn the_warps_keep_the_size_of_the_picture() {
        let img = ramp(64, 40);
        for out in [bulge(&img), twirl(&img), mirror_left(&img), flip_horizontal(&img)] {
            assert_eq!((out.w, out.h, out.px.len()), (64, 40, 64 * 40));
        }
    }

    #[test]
    fn bulge_pulls_the_middle_outwards_and_keeps_the_centre() {
        let img = ramp(100, 100);
        let out = bulge(&img);
        // the centre barely moves
        let centre = 50 * 100 + 50;
        assert!(red(out.px[centre]).abs_diff(red(img.px[centre])) <= 1);
        // a pixel halfway out reads from nearer the centre, so its column number is smaller than its own
        let probe = 50 * 100 + 75;
        assert!(red(out.px[probe]) < red(img.px[probe]), "{} {}", red(out.px[probe]), red(img.px[probe]));
    }

    #[test]
    fn bulge_leaves_the_corners_untouched() {
        let img = ramp(100, 60);
        let out = bulge(&img);
        for corner in [0, 99, 59 * 100, 59 * 100 + 99] {
            assert_eq!(out.px[corner], img.px[corner]);
        }
    }

    #[test]
    fn twirl_moves_pixels_inside_the_disc_and_keeps_the_corners() {
        let img = ramp(100, 100);
        let out = twirl(&img);
        assert_eq!(out.px[0], img.px[0]);
        assert_eq!(out.px[99 * 100 + 99], img.px[99 * 100 + 99]);
        let probe = 50 * 100 + 70;
        assert_ne!(out.px[probe], img.px[probe]);
        // the picture stays opaque: no sample fell outside it
        assert!(out.px.iter().all(|p| p >> 24 == 255));
    }

    #[test]
    fn an_empty_picture_survives_every_warp() {
        let img = Image { w: 0, h: 0, px: Vec::new() };
        for out in [bulge(&img), twirl(&img), mirror_left(&img), flip_horizontal(&img)] {
            assert!(out.px.is_empty());
        }
    }
}

//! Edits that change a picture's pixels: mirroring, cropping, clearing a colour region to transparent (Mask) and
//! removing the background. Everything here works on plain pixels, so the tests run without a window. Pixels are
//! premultiplied RGBA packed with red in the low byte.

use crate::model::Norm;
use bifrost_app::kit::Image;

/// `img` mirrored left to right and/or top to bottom.
pub fn flip(img: &Image, horizontal: bool, vertical: bool) -> Image {
    if (!horizontal && !vertical) || img.w == 0 || img.h == 0 {
        return img.clone();
    }
    let (w, h) = (img.w, img.h);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let sy = if vertical { h - 1 - y } else { y };
        for x in 0..w {
            let sx = if horizontal { w - 1 - x } else { x };
            px.push(img.px[sy * w + sx]);
        }
    }
    Image { w, h, px }
}

/// The part of `img` inside `rect` (page coordinates), at least one pixel each way. None when the rectangle is empty
/// or lies outside the picture.
pub fn crop(img: &Image, rect: Norm) -> Option<Image> {
    if img.w == 0 || img.h == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let x0 = ((rect.x * img.w as f32).round().max(0.0) as usize).min(img.w - 1);
    let y0 = ((rect.y * img.h as f32).round().max(0.0) as usize).min(img.h - 1);
    let x1 = (((rect.x + rect.w) * img.w as f32).round().max(0.0) as usize).clamp(x0 + 1, img.w);
    let y1 = (((rect.y + rect.h) * img.h as f32).round().max(0.0) as usize).clamp(y0 + 1, img.h);
    let (w, h) = (x1 - x0, y1 - y0);
    let mut px = Vec::with_capacity(w * h);
    for y in y0..y1 {
        px.extend_from_slice(&img.px[y * img.w + x0..y * img.w + x1]);
    }
    Some(Image { w, h, px })
}

/// The channel distance between two pixels: the largest difference of any channel.
fn distance(a: u32, b: u32) -> i32 {
    (0..4).map(|i| (((a >> (8 * i)) & 0xff) as i32 - ((b >> (8 * i)) & 0xff) as i32).abs()).max().unwrap_or(0)
}

/// Clears to transparent the pixels connected to `seed` whose colour is within `tolerance` of the seed's colour.
/// Returns how many pixels were cleared; a seed outside the picture or already transparent clears none.
pub fn clear_region(img: &mut Image, seed: (usize, usize), tolerance: i32) -> usize {
    if seed.0 >= img.w || seed.1 >= img.h {
        return 0;
    }
    let reference = img.px[seed.1 * img.w + seed.0];
    if reference >> 24 == 0 {
        return 0;
    }
    let mut cleared = 0;
    let mut stack = vec![seed];
    while let Some((x, y)) = stack.pop() {
        let i = y * img.w + x;
        let p = img.px[i];
        if p >> 24 == 0 || distance(p, reference) > tolerance {
            continue;
        }
        img.px[i] = 0;
        cleared += 1;
        if x > 0 {
            stack.push((x - 1, y));
        }
        if x + 1 < img.w {
            stack.push((x + 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if y + 1 < img.h {
            stack.push((x, y + 1));
        }
    }
    cleared
}

/// Removes the background: every region touching a corner that matches that corner's colour within `tolerance` is
/// cleared. Returns how many pixels were cleared.
pub fn remove_background(img: &mut Image, tolerance: i32) -> usize {
    if img.w == 0 || img.h == 0 {
        return 0;
    }
    let (w, h) = (img.w - 1, img.h - 1);
    [(0, 0), (w, 0), (0, h), (w, h)].into_iter().map(|seed| clear_region(img, seed, tolerance)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::pack;

    const WHITE: u32 = pack(255, 255, 255, 255);
    const RED: u32 = pack(255, 0, 0, 255);

    fn grid(w: usize, h: usize, f: impl Fn(usize, usize) -> u32) -> Image {
        Image { w, h, px: (0..w * h).map(|i| f(i % w, i / w)).collect() }
    }

    #[test]
    fn flipping_mirrors_each_axis() {
        let img = grid(3, 2, |x, y| pack((y * 3 + x) as u8, 0, 0, 255));
        let h = flip(&img, true, false);
        assert_eq!(h.px[0], img.px[2]);
        let v = flip(&img, false, true);
        assert_eq!(v.px[0], img.px[3]);
        let both = flip(&img, true, true);
        assert_eq!(both.px[0], img.px[5]);
        assert_eq!(flip(&both, true, true), img);
        assert_eq!(flip(&img, false, false), img);
    }

    #[test]
    fn crop_takes_the_pixels_inside_the_rectangle() {
        let img = grid(4, 4, |x, y| pack((y * 4 + x) as u8, 0, 0, 255));
        let c = crop(&img, Norm { x: 0.25, y: 0.5, w: 0.5, h: 0.25 }).expect("a crop");
        assert_eq!((c.w, c.h), (2, 1));
        assert_eq!(c.px, vec![img.px[9], img.px[10]]);
        assert!(crop(&img, Norm { x: 0.0, y: 0.0, w: 0.0, h: 0.5 }).is_none());
        // a sliver still gives one pixel
        let s = crop(&img, Norm { x: 0.99, y: 0.99, w: 0.001, h: 0.001 }).expect("a crop");
        assert_eq!((s.w, s.h), (1, 1));
    }

    #[test]
    fn a_region_is_cleared_only_where_it_is_connected() {
        // white left and right of a red column: seeding the left clears only the left white
        let mut img = grid(5, 1, |x, _| if x == 2 { RED } else { WHITE });
        assert_eq!(clear_region(&mut img, (0, 0), 10), 2);
        assert_eq!(img.px, vec![0, 0, RED, WHITE, WHITE]);
        // a transparent seed does nothing
        assert_eq!(clear_region(&mut img, (0, 0), 10), 0);
        assert_eq!(clear_region(&mut img, (9, 0), 10), 0);
    }

    #[test]
    fn tolerance_widens_what_counts_as_the_same_colour() {
        let near = pack(250, 250, 250, 255);
        let mut img = grid(2, 1, |x, _| if x == 0 { WHITE } else { near });
        assert_eq!(clear_region(&mut img.clone(), (0, 0), 2), 1);
        assert_eq!(clear_region(&mut img, (0, 0), 10), 2);
    }

    #[test]
    fn removing_the_background_keeps_the_subject() {
        let mut img = grid(5, 5, |x, y| if (1..4).contains(&x) && (1..4).contains(&y) { RED } else { WHITE });
        let cleared = remove_background(&mut img, 8);
        assert_eq!(cleared, 16);
        assert_eq!(img.px[0], 0);
        assert_eq!(img.px[2 * 5 + 2], RED);
        assert_eq!(remove_background(&mut Image { w: 0, h: 0, px: vec![] }, 8), 0);
    }
}

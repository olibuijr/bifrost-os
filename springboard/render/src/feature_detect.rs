//! Region detection on images: finds bright connected regions and their bounding boxes, and keeps
//! the ones that fill their box, which are the axis-aligned rectangles. This covers the rectangle
//! case of the reference feature detector in a simple form. It does not detect perspective
//! (skewed) quadrilaterals, faces, text or barcodes.
//!
//! Pixels are packed `u32` values, red in the low byte, as in `gfx::Image`. Luma is the Rec. 601
//! weighting of the straight (unpremultiplied) channels read from the packed value.
use crate::gfx::Image;

/// An axis-aligned box in pixels: the top-left corner and the size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    /// Width times height.
    pub fn area(&self) -> usize {
        self.w * self.h
    }
}

/// A detected region: its bounding box and the number of bright pixels inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feature {
    pub bounds: Rect,
    pub pixels: usize,
}

impl Feature {
    /// The share of the bounding box that is bright, from 0 to 1. A filled rectangle gives 1.
    pub fn fill(&self) -> f32 {
        self.pixels as f32 / self.bounds.area() as f32
    }
}

/// Rec. 601 luma of a packed pixel, 0..=255, in integer arithmetic.
fn luma(p: u32) -> u32 {
    let r = p & 0xff;
    let g = (p >> 8) & 0xff;
    let b = (p >> 16) & 0xff;
    (r * 299 + g * 587 + b * 114) / 1000
}

/// Connected regions (4-connected) of pixels whose luma is at least `threshold`. Each region with at
/// least `min_pixels` pixels is returned with its bounding box. The result is sorted by pixel count,
/// largest first, then by position (top, then left), so the order is deterministic.
/// An image whose buffer does not match its size yields no regions.
pub fn bright_regions(img: &Image, threshold: u8, min_pixels: usize) -> Vec<Feature> {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() != w * h {
        return Vec::new();
    }
    let thr = threshold as u32;
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..w * h {
        if seen[start] || luma(img.px[start]) < thr {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        let mut count = 0usize;
        while let Some(i) = stack.pop() {
            count += 1;
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let mut next: [Option<usize>; 4] = [None; 4];
            if x > 0 {
                next[0] = Some(i - 1);
            }
            if x + 1 < w {
                next[1] = Some(i + 1);
            }
            if y > 0 {
                next[2] = Some(i - w);
            }
            if y + 1 < h {
                next[3] = Some(i + w);
            }
            for j in next.into_iter().flatten() {
                if !seen[j] && luma(img.px[j]) >= thr {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if count >= min_pixels {
            let bounds = Rect { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
            out.push(Feature { bounds, pixels: count });
        }
    }
    out.sort_by(|a, b| {
        b.pixels
            .cmp(&a.pixels)
            .then(a.bounds.y.cmp(&b.bounds.y))
            .then(a.bounds.x.cmp(&b.bounds.x))
    });
    out
}

/// The bright regions that fill at least `min_fill` (0..=1) of their bounding box. A value of 0.95
/// keeps solid rectangles and drops L shapes, rings and scattered pixels.
pub fn filled_rectangles(img: &Image, threshold: u8, min_pixels: usize, min_fill: f32) -> Vec<Feature> {
    bright_regions(img, threshold, min_pixels).into_iter().filter(|f| f.fill() >= min_fill).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: u32 = 0xffff_ffff;

    fn canvas(w: usize, h: usize) -> Image {
        Image::new(w, h)
    }

    fn fill_rect(img: &mut Image, x: usize, y: usize, w: usize, h: usize) {
        for yy in y..y + h {
            for xx in x..x + w {
                img.px[yy * img.w + xx] = WHITE;
            }
        }
    }

    #[test]
    fn finds_one_block_with_its_bounding_box() {
        let mut img = canvas(10, 10);
        fill_rect(&mut img, 2, 1, 3, 4);
        let found = bright_regions(&img, 128, 1);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bounds, Rect { x: 2, y: 1, w: 3, h: 4 });
        assert_eq!(found[0].pixels, 12);
        assert!((found[0].fill() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn min_pixels_drops_small_specks() {
        let mut img = canvas(10, 10);
        fill_rect(&mut img, 2, 1, 3, 4);
        img.px[9 * 10 + 9] = WHITE;
        assert_eq!(bright_regions(&img, 128, 1).len(), 2);
        let big = bright_regions(&img, 128, 2);
        assert_eq!(big.len(), 1);
        assert_eq!(big[0].bounds.w, 3);
    }

    #[test]
    fn diagonal_pixels_are_separate_regions() {
        let mut img = canvas(4, 4);
        img.px[0] = WHITE;
        img.px[5] = WHITE;
        assert_eq!(bright_regions(&img, 128, 1).len(), 2);
    }

    #[test]
    fn regions_are_sorted_largest_first() {
        let mut img = canvas(12, 6);
        fill_rect(&mut img, 0, 0, 2, 2);
        fill_rect(&mut img, 6, 1, 4, 3);
        let found = bright_regions(&img, 128, 1);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].pixels, 12);
        assert_eq!(found[1].pixels, 4);
    }

    #[test]
    fn dark_pixels_never_form_regions() {
        let img = canvas(5, 5);
        assert!(bright_regions(&img, 1, 1).is_empty());
        assert!(bright_regions(&img, 0, 1).len() == 1, "threshold 0 takes every pixel");
    }

    #[test]
    fn filled_rectangles_drop_l_shapes() {
        let mut img = canvas(8, 8);
        fill_rect(&mut img, 0, 0, 3, 1);
        fill_rect(&mut img, 0, 1, 1, 2);
        assert_eq!(bright_regions(&img, 128, 1).len(), 1);
        assert!(filled_rectangles(&img, 128, 1, 0.95).is_empty());

        let mut solid = canvas(8, 8);
        fill_rect(&mut solid, 1, 1, 4, 2);
        let rects = filled_rectangles(&solid, 128, 1, 0.95);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].bounds.area(), 8);
    }

    #[test]
    fn mismatched_buffer_yields_nothing() {
        let img = Image { w: 4, h: 4, px: vec![WHITE; 3] };
        assert!(bright_regions(&img, 0, 1).is_empty());
    }
}

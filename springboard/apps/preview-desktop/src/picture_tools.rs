//! Picture tools for Preview: flip, crop to a selection, instant alpha, remove background, redact, the magnifier lens
//! window, zoom to a selection, the contact sheet grid and slideshow stepping. Pure functions over `Image`
//! (`0xAARRGGBB` pixels) and normalised rectangles; the Tools and View menus call them.
#![allow(dead_code)] // the Tools menu items and the selection tool call these next

use crate::model::Norm;
use bifrost_app::kit::Image;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flip {
    pub horizontal: bool,
    pub vertical: bool,
}

impl Flip {
    pub fn toggled_horizontal(self) -> Flip {
        Flip { horizontal: !self.horizontal, ..self }
    }

    pub fn toggled_vertical(self) -> Flip {
        Flip { vertical: !self.vertical, ..self }
    }
}

/// The image mirrored as `flip` says.
pub fn flipped(img: &Image, flip: Flip) -> Image {
    let (w, h) = (img.w, img.h);
    let mut px = vec![0u32; w * h];
    for y in 0..h {
        let sy = if flip.vertical { h - 1 - y } else { y };
        for x in 0..w {
            let sx = if flip.horizontal { w - 1 - x } else { x };
            px[y * w + x] = img.px[sy * w + sx];
        }
    }
    Image { w, h, px }
}

/// A selection as pixel bounds `(x, y, w, h)` on a `w` by `h` image, clamped to it; None when it holds no pixel.
pub fn selection_px(sel: Norm, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let x0 = (sel.x.clamp(0.0, 1.0) * w as f32).round() as usize;
    let y0 = (sel.y.clamp(0.0, 1.0) * h as f32).round() as usize;
    let x1 = ((sel.x + sel.w).clamp(0.0, 1.0) * w as f32).round() as usize;
    let y1 = ((sel.y + sel.h).clamp(0.0, 1.0) * h as f32).round() as usize;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// The part of the image inside the selection (Tools > Crop).
pub fn crop(img: &Image, sel: Norm) -> Option<Image> {
    let (x, y, w, h) = selection_px(sel, img.w, img.h)?;
    let mut px = Vec::with_capacity(w * h);
    for row in y..y + h {
        px.extend_from_slice(&img.px[row * img.w + x..row * img.w + x + w]);
    }
    Some(Image { w, h, px })
}

/// Paints the selection solid black and opaque, removing what was under it (Tools > Redact on a picture).
pub fn redact(img: &mut Image, sel: Norm) -> bool {
    let Some((x, y, w, h)) = selection_px(sel, img.w, img.h) else { return false };
    for row in y..y + h {
        for p in &mut img.px[row * img.w + x..row * img.w + x + w] {
            *p = 0xff00_0000;
        }
    }
    true
}

fn channels(p: u32) -> [i32; 3] {
    [(p >> 16 & 0xff) as i32, (p >> 8 & 0xff) as i32, (p & 0xff) as i32]
}

fn close(a: u32, b: u32, tolerance: i32) -> bool {
    let (a, b) = (channels(a), channels(b));
    (0..3).all(|i| (a[i] - b[i]).abs() <= tolerance)
}

/// Instant Alpha: makes transparent the connected region around `(x, y)` whose colours are within `tolerance`
/// (0-255 per channel) of the pixel clicked. Returns the number of pixels cleared.
pub fn instant_alpha(img: &mut Image, x: usize, y: usize, tolerance: u8) -> usize {
    if x >= img.w || y >= img.h {
        return 0;
    }
    let seed = img.px[y * img.w + x];
    let region = flood(img, &[(x, y)], seed, i32::from(tolerance));
    clear(img, &region)
}

/// Remove Background: clears every region connected to the border whose colour is near the colour of a border
/// pixel (the most common border colour), within `tolerance`. Returns the number of pixels cleared.
pub fn remove_background(img: &mut Image, tolerance: u8) -> usize {
    if img.w == 0 || img.h == 0 {
        return 0;
    }
    let mut border: Vec<(usize, usize)> = vec![];
    for x in 0..img.w {
        border.push((x, 0));
        border.push((x, img.h - 1));
    }
    for y in 1..img.h.saturating_sub(1) {
        border.push((0, y));
        border.push((img.w - 1, y));
    }
    // the dominant border colour (by coarse bucket) is the background
    let mut counts: Vec<(u32, usize)> = vec![];
    for &(x, y) in &border {
        let p = img.px[y * img.w + x] & 0x00ff_ffff;
        match counts.iter_mut().find(|(c, _)| close(*c, p, i32::from(tolerance))) {
            Some((_, n)) => *n += 1,
            None => counts.push((p, 1)),
        }
    }
    let Some(&(bg, _)) = counts.iter().max_by_key(|(_, n)| *n) else { return 0 };
    let seeds: Vec<(usize, usize)> =
        border.into_iter().filter(|&(x, y)| close(img.px[y * img.w + x], bg, i32::from(tolerance))).collect();
    let region = flood(img, &seeds, bg, i32::from(tolerance));
    clear(img, &region)
}

fn flood(img: &Image, seeds: &[(usize, usize)], colour: u32, tolerance: i32) -> Vec<bool> {
    let mut seen = vec![false; img.w * img.h];
    let mut stack: Vec<(usize, usize)> = vec![];
    for &(x, y) in seeds {
        if !seen[y * img.w + x] && close(img.px[y * img.w + x], colour, tolerance) {
            seen[y * img.w + x] = true;
            stack.push((x, y));
        }
    }
    while let Some((x, y)) = stack.pop() {
        let mut visit = |nx: usize, ny: usize| {
            let i = ny * img.w + nx;
            if !seen[i] && close(img.px[i], colour, tolerance) {
                seen[i] = true;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            visit(x - 1, y);
        }
        if y > 0 {
            visit(x, y - 1);
        }
        if x + 1 < img.w {
            visit(x + 1, y);
        }
        if y + 1 < img.h {
            visit(x, y + 1);
        }
    }
    seen
}

fn clear(img: &mut Image, region: &[bool]) -> usize {
    let mut n = 0;
    for (p, hit) in img.px.iter_mut().zip(region) {
        if *hit {
            *p &= 0x00ff_ffff;
            n += 1;
        }
    }
    n
}

/// The source rectangle (pixels) the magnifier lens shows around the pointer: a `lens` pixel square centred on
/// `(px, py)` shrunk by `power` (the lens shows lens/power source pixels), kept inside the image.
pub fn magnifier_source(px: f32, py: f32, lens: f32, power: f32, w: usize, h: usize) -> (f32, f32, f32, f32) {
    let side = (lens / power.max(1.0)).min(w as f32).min(h as f32);
    let x = (px - side / 2.0).clamp(0.0, (w as f32 - side).max(0.0));
    let y = (py - side / 2.0).clamp(0.0, (h as f32 - side).max(0.0));
    (x, y, side, side)
}

/// Zoom To Selection: the zoom factor that makes the selection fill a `view_w` by `view_h` canvas, and the centre
/// of the selection in normalised page coordinates (to pan to). The factor is clamped to the viewer's limits.
pub fn zoom_to_selection(sel: Norm, page_w: f32, page_h: f32, view_w: f32, view_h: f32) -> Option<(f32, (f32, f32))> {
    let (sw, sh) = (sel.w * page_w, sel.h * page_h);
    if sw <= 0.0 || sh <= 0.0 || view_w <= 0.0 || view_h <= 0.0 {
        return None;
    }
    let factor = (view_w / sw).min(view_h / sh).clamp(crate::model::ZOOM_MIN, crate::model::ZOOM_MAX);
    Some((factor, (sel.x + sel.w / 2.0, sel.y + sel.h / 2.0)))
}

/// One cell of the contact sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub page: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Contact Sheet: thumbnails in a grid of the width of the canvas. Pages keep their aspect ratio inside square cells
/// of `thumb` pixels with `gap` between them; returns the cells and the total height.
pub fn contact_sheet(pages: &[(f32, f32)], canvas_w: f32, thumb: f32, gap: f32) -> (Vec<Cell>, f32) {
    let columns = (((canvas_w - gap) / (thumb + gap)).floor() as usize).max(1);
    let mut cells = vec![];
    for (i, &(pw, ph)) in pages.iter().enumerate() {
        let (col, row) = (i % columns, i / columns);
        let s = (thumb / pw.max(1.0)).min(thumb / ph.max(1.0));
        let (w, h) = (pw * s, ph * s);
        let x = gap + col as f32 * (thumb + gap) + (thumb - w) / 2.0;
        let y = gap + row as f32 * (thumb + gap) + (thumb - h) / 2.0;
        cells.push(Cell { page: i, x, y, w, h });
    }
    let rows = pages.len().div_ceil(columns);
    (cells, gap + rows as f32 * (thumb + gap))
}

/// Slideshow stepping over `count` pages: the page after `current` (wrapping when `looped`), or None at the end.
pub fn slideshow_next(current: usize, count: usize, looped: bool) -> Option<usize> {
    if count == 0 {
        None
    } else if current + 1 < count {
        Some(current + 1)
    } else if looped {
        Some(0)
    } else {
        None
    }
}

pub fn slideshow_prev(current: usize, count: usize, looped: bool) -> Option<usize> {
    if count == 0 {
        None
    } else if current > 0 {
        Some(current - 1)
    } else if looped {
        Some(count - 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> u32) -> Image {
        let mut px = vec![];
        for y in 0..h {
            for x in 0..w {
                px.push(f(x, y));
            }
        }
        Image { w, h, px }
    }

    const WHITE: u32 = 0xffff_ffff;
    const RED: u32 = 0xffff_0000;

    #[test]
    fn flips() {
        let i = img(3, 2, |x, y| (y * 3 + x) as u32);
        assert_eq!(flipped(&i, Flip { horizontal: true, vertical: false }).px, vec![2, 1, 0, 5, 4, 3]);
        assert_eq!(flipped(&i, Flip { horizontal: false, vertical: true }).px, vec![3, 4, 5, 0, 1, 2]);
        let both = Flip::default().toggled_horizontal().toggled_vertical();
        assert_eq!(flipped(&i, both).px, vec![5, 4, 3, 2, 1, 0]);
        assert_eq!(flipped(&flipped(&i, both), both), i);
    }

    #[test]
    fn crop_and_redact() {
        let i = img(4, 4, |x, y| (y * 4 + x) as u32);
        let sel = Norm { x: 0.25, y: 0.5, w: 0.5, h: 0.5 };
        let c = crop(&i, sel).unwrap();
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.px, vec![9, 10, 13, 14]);
        assert!(crop(&i, Norm { x: 0.5, y: 0.5, w: 0.0, h: 0.2 }).is_none());
        // a selection running past the page is clamped
        assert_eq!(crop(&i, Norm { x: 0.75, y: 0.75, w: 0.9, h: 0.9 }).unwrap().px, vec![15]);
        let mut r = i.clone();
        assert!(redact(&mut r, sel));
        assert_eq!(r.px[9], 0xff00_0000);
        assert_eq!(r.px[0], 0);
    }

    #[test]
    fn instant_alpha_clears_the_connected_region_only() {
        // white background with a red column in the middle splitting it in two
        let mut i = img(5, 3, |x, _| if x == 2 { RED } else { WHITE });
        let n = instant_alpha(&mut i, 0, 0, 10);
        assert_eq!(n, 6);
        assert_eq!(i.px[0] >> 24, 0);
        assert_eq!(i.px[4], WHITE); // the other side is not connected
        assert_eq!(i.px[2], RED);
        assert_eq!(instant_alpha(&mut i, 9, 9, 10), 0);
    }

    #[test]
    fn tolerance_widens_the_region() {
        let mut i = img(2, 1, |x, _| if x == 0 { 0xff80_8080 } else { 0xff88_8888 });
        assert_eq!(instant_alpha(&mut i.clone(), 0, 0, 2), 1);
        assert_eq!(instant_alpha(&mut i, 0, 0, 8), 2);
    }

    #[test]
    fn remove_background_keeps_an_enclosed_subject() {
        // white border, red 3x3 subject with a white hole inside it
        let mut i = img(7, 7, |x, y| {
            if (2..=4).contains(&x) && (2..=4).contains(&y) && !(x == 3 && y == 3) { RED } else { WHITE }
        });
        let n = remove_background(&mut i, 10);
        assert_eq!(n, 49 - 9);
        assert_eq!(i.px[3 * 7 + 3], WHITE); // the enclosed hole is not reached from the border
        assert_eq!(i.px[2 * 7 + 2], RED);
        assert_eq!(i.px[0] >> 24, 0);
    }

    #[test]
    fn magnifier_stays_inside_the_image() {
        assert_eq!(magnifier_source(50.0, 50.0, 100.0, 2.0, 200, 200), (25.0, 25.0, 50.0, 50.0));
        assert_eq!(magnifier_source(0.0, 0.0, 100.0, 2.0, 200, 200), (0.0, 0.0, 50.0, 50.0));
        assert_eq!(magnifier_source(199.0, 199.0, 100.0, 2.0, 200, 200), (150.0, 150.0, 50.0, 50.0));
        let (_, _, side, _) = magnifier_source(5.0, 5.0, 100.0, 1.0, 40, 30);
        assert_eq!(side, 30.0);
    }

    #[test]
    fn zoom_to_selection_fits_the_larger_side() {
        let (f, c) = zoom_to_selection(Norm { x: 0.2, y: 0.2, w: 0.5, h: 0.25 }, 1000.0, 1000.0, 1000.0, 500.0).unwrap();
        assert_eq!(f, 2.0);
        assert!((c.0 - 0.45).abs() < 1e-5 && (c.1 - 0.325).abs() < 1e-5);
        assert!(zoom_to_selection(Norm { x: 0.0, y: 0.0, w: 0.0, h: 0.1 }, 10.0, 10.0, 10.0, 10.0).is_none());
    }

    #[test]
    fn contact_sheet_wraps_into_rows() {
        let pages = vec![(100.0, 200.0); 5];
        let (cells, height) = contact_sheet(&pages, 330.0, 100.0, 10.0);
        // (330 - 10) / 110 = 2 columns
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0].h, 100.0);
        assert_eq!(cells[0].w, 50.0);
        assert_eq!(cells[1].page, 1);
        assert!(cells[2].y > cells[0].y);
        assert_eq!(height, 10.0 + 3.0 * 110.0);
    }

    #[test]
    fn slideshow_steps() {
        assert_eq!(slideshow_next(0, 3, false), Some(1));
        assert_eq!(slideshow_next(2, 3, false), None);
        assert_eq!(slideshow_next(2, 3, true), Some(0));
        assert_eq!(slideshow_prev(0, 3, false), None);
        assert_eq!(slideshow_prev(0, 3, true), Some(2));
        assert_eq!(slideshow_next(0, 0, true), None);
    }
}

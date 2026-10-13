//! Pixel edits of a picture document: flip, crop, instant alpha and background removal, plus the writers that need the
//! transparent result (a PNG keeps the alpha channel, a JPEG is flattened onto white). The functions take and return
//! whole pictures, so an edit is one undoable replacement of the document's picture. Marks follow the edit: flipping
//! mirrors them and cropping moves them into the cropped page. Everything here is plain data and runs without a window.

use crate::model::{Mark, Norm};
use bifrost_app::kit::Image;

/// The default colour tolerance of Instant Alpha and Remove Background, as the largest allowed difference in any
/// channel (0 to 255).
pub const TOLERANCE: u8 = 24;

/// The picture mirrored left to right (`horizontal`) or top to bottom.
pub fn flip(img: &Image, horizontal: bool) -> Image {
    let (w, h) = (img.w, img.h);
    let mut px = Vec::with_capacity(img.px.len());
    for y in 0..h {
        let sy = if horizontal { y } else { h - 1 - y };
        let row = &img.px[sy * w..(sy + 1) * w];
        if horizontal {
            px.extend(row.iter().rev());
        } else {
            px.extend_from_slice(row);
        }
    }
    Image { w, h, px }
}

/// Which way to flip the unrotated picture so the shown picture is mirrored `horizontal`ly or vertically, given the
/// quarter turns it is shown at. A picture turned by an odd number of quarters shows its columns as rows.
pub fn source_horizontal(horizontal: bool, quarters: u8) -> bool {
    if quarters % 2 == 0 {
        horizontal
    } else {
        !horizontal
    }
}

/// The marks of `page` mirrored with the picture (all marks of other pages are returned as they were).
pub fn flip_marks(marks: &[Mark], page: usize, horizontal: bool) -> Vec<Mark> {
    marks
        .iter()
        .map(|m| {
            if m.page != page {
                return m.clone();
            }
            let mirror = |(u, v): (f32, f32)| if horizontal { (1.0 - u, v) } else { (u, 1.0 - v) };
            let r = m.rect;
            let rect = if horizontal { Norm { x: 1.0 - r.x - r.w, ..r } } else { Norm { y: 1.0 - r.y - r.h, ..r } };
            Mark { rect, ink: m.ink.iter().copied().map(mirror).collect(), ..m.clone() }
        })
        .collect()
}

/// The pixel rectangle (x, y, w, h) of a normalised rectangle on a `w` by `h` picture: rounded to whole pixels and kept
/// inside the picture. None when it would be empty.
pub fn pixel_rect(r: Norm, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    if w == 0 || h == 0 {
        return None;
    }
    let edge = |v: f32, size: usize| ((v * size as f32).round().max(0.0) as usize).min(size);
    let (x0, x1) = (edge(r.x, w), edge(r.x + r.w, w));
    let (y0, y1) = (edge(r.y, h), edge(r.y + r.h, h));
    (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

/// The part of the picture inside `r`, or None when `r` holds no pixel.
pub fn crop(img: &Image, r: Norm) -> Option<Image> {
    let (x, y, w, h) = pixel_rect(r, img.w, img.h)?;
    Some(img.crop(x, y, w, h))
}

/// The marks of `page` moved into the page cropped to `r`. A mark that lies wholly outside is dropped; one that
/// reaches over the edge is clipped to it. Marks of other pages are returned as they were.
pub fn crop_marks(marks: &[Mark], page: usize, r: Norm) -> Vec<Mark> {
    if r.w <= 0.0 || r.h <= 0.0 {
        return marks.to_vec();
    }
    let to_x = |u: f32| ((u - r.x) / r.w).clamp(0.0, 1.0);
    let to_y = |v: f32| ((v - r.y) / r.h).clamp(0.0, 1.0);
    marks
        .iter()
        .filter_map(|m| {
            if m.page != page {
                return Some(m.clone());
            }
            let b = m.rect;
            let outside = b.x + b.w < r.x || b.x > r.x + r.w || b.y + b.h < r.y || b.y > r.y + r.h;
            if outside {
                return None;
            }
            let ink: Vec<(f32, f32)> = m.ink.iter().map(|&(u, v)| (to_x(u), to_y(v))).collect();
            let rect = if ink.is_empty() {
                Norm::from_corners((to_x(b.x), to_y(b.y)), (to_x(b.x + b.w), to_y(b.y + b.h)))
            } else {
                bounds_of(&ink)
            };
            Some(Mark { rect, ink, ..m.clone() })
        })
        .collect()
}

/// The box around some points.
fn bounds_of(points: &[(f32, f32)]) -> Norm {
    let mut a = (f32::MAX, f32::MAX);
    let mut b = (f32::MIN, f32::MIN);
    for &(x, y) in points {
        a = (a.0.min(x), a.1.min(y));
        b = (b.0.max(x), b.1.max(y));
    }
    Norm::from_corners(a, b)
}

/// Whether two pixels differ by at most `tol` in every channel.
fn close(a: u32, b: u32, tol: u8) -> bool {
    (0..4).all(|k| {
        let (x, y) = ((a >> (8 * k)) & 0xff, (b >> (8 * k)) & 0xff);
        x.abs_diff(y) <= tol as u32
    })
}

/// Clears to transparent every pixel that can be reached from a `seeds` pixel through pixels within `tol` of
/// `reference` (4-connected). Pixels already transparent are passed through and left as they are.
fn clear_connected(img: &Image, seeds: &[(usize, usize)], reference: u32, tol: u8) -> Image {
    let (w, h) = (img.w, img.h);
    let mut out = img.clone();
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for &(x, y) in seeds {
        if x < w && y < h {
            stack.push(y * w + x);
        }
    }
    while let Some(i) = stack.pop() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let p = img.px[i];
        if p >> 24 != 0 && !close(p, reference, tol) {
            continue;
        }
        out.px[i] = 0;
        let (x, y) = (i % w, i / w);
        if x > 0 {
            stack.push(i - 1);
        }
        if x + 1 < w {
            stack.push(i + 1);
        }
        if y > 0 {
            stack.push(i - w);
        }
        if y + 1 < h {
            stack.push(i + w);
        }
    }
    out
}

/// Instant Alpha: the region around pixel (`x`, `y`) of one colour (within `tol`) becomes transparent.
pub fn instant_alpha(img: &Image, x: usize, y: usize, tol: u8) -> Image {
    if x >= img.w || y >= img.h {
        return img.clone();
    }
    clear_connected(img, &[(x, y)], img.px[y * img.w + x], tol)
}

/// The colour most of the picture's four corners share (within `tol`), or the top left corner when no two agree.
pub fn background_colour(img: &Image, tol: u8) -> Option<u32> {
    if img.w == 0 || img.h == 0 {
        return None;
    }
    let (r, b) = (img.w - 1, img.h - 1);
    let corners = [img.px[0], img.px[r], img.px[b * img.w], img.px[b * img.w + r]];
    let votes = |c: u32| corners.iter().filter(|&&o| close(c, o, tol)).count();
    // the first corner wins a tie
    let mut best = corners[0];
    for &c in &corners[1..] {
        if votes(c) > votes(best) {
            best = c;
        }
    }
    Some(best)
}

/// Remove Background: the region that touches the picture's edge and matches the background colour (see
/// `background_colour`) becomes transparent. A picture whose subject reaches the edge keeps those pixels only when they
/// differ from the background.
pub fn remove_background(img: &Image, tol: u8) -> Image {
    let Some(reference) = background_colour(img, tol) else { return img.clone() };
    let (w, h) = (img.w, img.h);
    let mut seeds = Vec::with_capacity(2 * (w + h));
    for x in 0..w {
        seeds.push((x, 0));
        seeds.push((x, h - 1));
    }
    for y in 0..h {
        seeds.push((0, y));
        seeds.push((w - 1, y));
    }
    clear_connected(img, &seeds, reference, tol)
}

/// Whether any pixel is not fully opaque.
pub fn has_alpha(img: &Image) -> bool {
    img.px.iter().any(|&p| p >> 24 != 255)
}

/// The picture as straight (not premultiplied) 8-bit RGBA bytes, the layout PNG stores.
pub fn to_straight_rgba(img: &Image) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.px.len() * 4);
    for &p in &img.px {
        let a = p >> 24;
        let (r, g, b) = (p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff);
        match a {
            0 => out.extend([0, 0, 0, 0]),
            255 => out.extend([r as u8, g as u8, b as u8, 255]),
            _ => out.extend([(r * 255 / a).min(255) as u8, (g * 255 / a).min(255) as u8, (b * 255 / a).min(255) as u8, a as u8]),
        }
    }
    out
}

/// The picture laid over white, fully opaque: what a format without alpha stores.
pub fn flatten_on_white(img: &Image) -> Image {
    let px = img
        .px
        .iter()
        .map(|&p| {
            let a = p >> 24;
            let lift = 255 - a;
            let ch = |shift: u32| (((p >> shift) & 0xff) + lift).min(255) << shift;
            ch(0) | ch(8) | ch(16) | (255 << 24)
        })
        .collect();
    Image { w: img.w, h: img.h, px }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MarkKind;
    use bifrost_app::kit::pack;

    const WHITE: u32 = pack(255, 255, 255, 255);
    const RED: u32 = pack(255, 0, 0, 255);

    fn img(w: usize, h: usize) -> Image {
        Image { w, h, px: (0..w * h).map(|i| pack(i as u8, 0, 0, 255)).collect() }
    }

    fn mark(kind: MarkKind, rect: Norm, ink: Vec<(f32, f32)>) -> Mark {
        Mark { kind, page: 0, rect, color: RED, text: String::new(), ink }
    }

    #[test]
    fn flipping_twice_returns_the_picture_and_each_axis_moves_the_corners() {
        let src = img(3, 2);
        assert_eq!(flip(&flip(&src, true), true), src);
        assert_eq!(flip(&flip(&src, false), false), src);
        let h = flip(&src, true);
        assert_eq!(h.px[0], src.px[2]);
        assert_eq!(h.px[3], src.px[5]);
        let v = flip(&src, false);
        assert_eq!(v.px[0], src.px[3]);
        assert_eq!(v.px[5], src.px[2]);
    }

    #[test]
    fn a_flip_on_a_turned_picture_mirrors_the_other_source_axis() {
        assert!(source_horizontal(true, 0));
        assert!(source_horizontal(true, 2));
        assert!(!source_horizontal(true, 1));
        assert!(source_horizontal(false, 3));
    }

    #[test]
    fn flipped_marks_land_on_the_mirrored_spot() {
        let m = mark(MarkKind::Rect, Norm { x: 0.1, y: 0.2, w: 0.3, h: 0.1 }, Vec::new());
        let f = flip_marks(&[m.clone()], 0, true);
        assert!((f[0].rect.x - 0.6).abs() < 1e-6 && (f[0].rect.y - 0.2).abs() < 1e-6 && (f[0].rect.w - 0.3).abs() < 1e-6);
        let line = mark(MarkKind::Line, Norm { x: 0.1, y: 0.1, w: 0.4, h: 0.4 }, vec![(0.1, 0.1), (0.5, 0.5)]);
        let f = flip_marks(&[line], 0, false);
        assert!((f[0].ink[0].1 - 0.9).abs() < 1e-6 && (f[0].ink[1].1 - 0.5).abs() < 1e-6 && (f[0].ink[0].0 - 0.1).abs() < 1e-6);
        assert!((f[0].rect.y - 0.5).abs() < 1e-6);
        let other = Mark { page: 1, ..m.clone() };
        assert_eq!(flip_marks(&[other.clone()], 0, true), vec![other]);
    }

    #[test]
    fn a_crop_takes_whole_pixels_and_rejects_an_empty_rectangle() {
        let src = img(10, 10);
        let c = crop(&src, Norm { x: 0.2, y: 0.5, w: 0.3, h: 0.2 }).expect("a crop");
        assert_eq!((c.w, c.h), (3, 2));
        assert_eq!(c.px[0], src.px[5 * 10 + 2]);
        assert!(crop(&src, Norm { x: 0.2, y: 0.5, w: 0.0, h: 0.2 }).is_none());
        // a rectangle reaching past the edge is cut to it
        let edge = crop(&src, Norm { x: 0.9, y: 0.9, w: 0.5, h: 0.5 }).expect("the corner");
        assert_eq!((edge.w, edge.h), (1, 1));
    }

    #[test]
    fn cropped_marks_move_into_the_new_page_and_outside_ones_go() {
        let r = Norm { x: 0.5, y: 0.5, w: 0.5, h: 0.5 };
        let inside = mark(MarkKind::Rect, Norm { x: 0.6, y: 0.6, w: 0.2, h: 0.2 }, Vec::new());
        let outside = mark(MarkKind::Rect, Norm { x: 0.0, y: 0.0, w: 0.2, h: 0.2 }, Vec::new());
        let over = mark(MarkKind::Highlight, Norm { x: 0.4, y: 0.6, w: 0.3, h: 0.1 }, Vec::new());
        let stroke = mark(MarkKind::Ink, Norm { x: 0.4, y: 0.4, w: 0.4, h: 0.4 }, vec![(0.4, 0.4), (0.8, 0.8)]);
        let got = crop_marks(&[inside, outside, over, stroke], 0, r);
        assert_eq!(got.len(), 3);
        assert!((got[0].rect.x - 0.2).abs() < 1e-6 && (got[0].rect.w - 0.4).abs() < 1e-6);
        // the highlight is clipped at the new left edge
        assert!(got[1].rect.x.abs() < 1e-6 && (got[1].rect.w - 0.4).abs() < 1e-6);
        assert!(got[2].ink[0] == (0.0, 0.0) && (got[2].ink[1].0 - 0.6).abs() < 1e-6 && (got[2].ink[1].1 - 0.6).abs() < 1e-6);
        let other_page = Mark { page: 3, ..mark(MarkKind::Rect, Norm { x: 0.0, y: 0.0, w: 0.1, h: 0.1 }, Vec::new()) };
        assert_eq!(crop_marks(&[other_page.clone()], 0, r), vec![other_page]);
    }

    #[test]
    fn instant_alpha_clears_the_connected_colour_only() {
        // a white 4 by 3 picture with a red column in the middle and a white pocket past it
        let mut src = Image { w: 5, h: 3, px: vec![WHITE; 15] };
        for y in 0..3 {
            src.px[y * 5 + 2] = RED;
        }
        let out = instant_alpha(&src, 0, 0, 0);
        assert_eq!(out.px[0], 0);
        assert_eq!(out.px[1 * 5 + 1], 0);
        assert_eq!(out.px[2], RED);
        assert_eq!(out.px[4], WHITE, "the white beyond the red column is not connected");
        assert_eq!(instant_alpha(&src, 9, 9, 0), src);
    }

    #[test]
    fn tolerance_takes_in_near_colours() {
        let mut src = Image { w: 3, h: 1, px: vec![WHITE; 3] };
        src.px[1] = pack(250, 250, 250, 255);
        assert_eq!(instant_alpha(&src, 0, 0, 0).px[1], pack(250, 250, 250, 255));
        assert_eq!(instant_alpha(&src, 0, 0, 8).px[1], 0);
    }

    #[test]
    fn removing_the_background_keeps_the_subject_and_an_enclosed_patch() {
        // white border, a red ring and a white centre: the centre is enclosed and stays
        let mut src = Image { w: 7, h: 7, px: vec![WHITE; 49] };
        for i in 1..6 {
            for &(x, y) in &[(i, 1), (i, 5), (1, i), (5, i)] {
                src.px[y * 7 + x] = RED;
            }
        }
        let out = remove_background(&src, 0);
        assert_eq!(out.px[0], 0);
        assert_eq!(out.px[6 * 7 + 6], 0);
        assert_eq!(out.px[1 * 7 + 1], RED);
        assert_eq!(out.px[3 * 7 + 3], WHITE);
    }

    #[test]
    fn the_background_is_the_colour_most_corners_share() {
        let mut src = Image { w: 3, h: 3, px: vec![RED; 9] };
        src.px[0] = WHITE;
        src.px[2] = WHITE;
        src.px[6] = WHITE;
        assert_eq!(background_colour(&src, 0), Some(WHITE));
        assert_eq!(background_colour(&Image { w: 0, h: 0, px: Vec::new() }, 0), None);
    }

    #[test]
    fn a_png_gets_straight_alpha_and_a_jpeg_gets_white_behind_it() {
        let half = pack(100, 0, 0, 128);
        let src = Image { w: 3, h: 1, px: vec![0, half, RED] };
        let rgba = to_straight_rgba(&src);
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(rgba[7], 128);
        assert_eq!(rgba[4], (100u32 * 255 / 128) as u8);
        assert_eq!(&rgba[8..12], &[255, 0, 0, 255]);
        assert!(has_alpha(&src));
        let flat = flatten_on_white(&src);
        assert_eq!(flat.px[0], WHITE);
        assert_eq!(flat.px[2], RED);
        assert_eq!(flat.px[1] >> 24, 255);
        assert!(!has_alpha(&flat));
    }
}

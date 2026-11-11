//! The drag image of a Files drag: the icon of the first item and, when the drag carries more than one item, a count badge.
//!
//! Two images go to the shell. The one it draws is the item's icon as the view shows it, at device pixels
//! ([`drag_image`], sent with `dnd::set_image`): the size of the icon that was lifted, so nothing is scaled up. The
//! other is the small preview every session begins with ([`drag_preview`], straight RGBA on a square canvas, sampled
//! down by `dnd::preview_from_rgba`); the shell shows it only when the image does not arrive.
use bifrost_app::dnd::{preview_from_rgba, DragPreview};
use springboard::files::fs::Kind;

/// The canvas side in pixels; the shell's preview limit is half of it.
const CANVAS: i32 = 64;

/// The most device pixels a side of the drag image has (the shell's limit for one, `proto::rich::MAX_DRAG_IMAGE`).
pub(crate) const MAX_IMAGE_PX: u32 = bifrost_app::proto::rich::MAX_DRAG_IMAGE;
/// The least side of the drag image, in points: a list or column row's small icon is lifted at this size, so the
/// image is not lost under the pointer.
pub(crate) const MIN_IMAGE_PT: f32 = 32.;

/// The side of the drag image in device pixels for an icon `icon_pt` points wide on a display of `scale`: the icon's
/// own size, `MIN_IMAGE_PT` at least and `MAX_IMAGE_PX` at most.
pub(crate) fn image_side(icon_pt: f32, scale: f32) -> u32 {
    ((icon_pt.max(MIN_IMAGE_PT) * scale.max(0.1)).round() as u32).clamp(1, MAX_IMAGE_PX)
}

/// The drag image at device pixels, as straight RGBA: `icon` is the item's icon drawn on a clear square of `side`
/// pixels (premultiplied, as a `Surface` holds it), and a drag of several items gets the count badge at its corner.
pub(crate) fn drag_image(icon: &[u32], side: usize, count: usize) -> Vec<u8> {
    let mut px = Vec::with_capacity(side * side * 4);
    for &p in icon.iter().take(side * side) {
        let a = p >> 24;
        // a `Surface` pixel is premultiplied; the wire takes straight colour
        let straight = |c: u32| if a == 0 { 0 } else { ((c & 255) * 255 / a).min(255) as u8 };
        px.extend_from_slice(&[straight(p), straight(p >> 8), straight(p >> 16), a as u8]);
    }
    px.resize(side * side * 4, 0);
    if count > 1 {
        badge(&mut px, side as i32, &count_label(count));
    }
    px
}

/// The badge's text: the count, or "99+" above 99 items.
fn count_label(count: usize) -> String {
    if count > 99 { "99+".to_string() } else { count.to_string() }
}

const FOLDER: [u8; 4] = [74, 144, 226, 255];
const FOLDER_TAB: [u8; 4] = [58, 118, 200, 255];
const PAGE_EDGE: [u8; 4] = [140, 140, 140, 255];
const PAGE: [u8; 4] = [250, 250, 250, 255];
const LINE: [u8; 4] = [170, 170, 170, 255];
const BADGE: [u8; 4] = [220, 40, 40, 255];
const BADGE_TEXT: [u8; 4] = [255, 255, 255, 255];

/// Digit and plus glyphs, five rows of three bits each (the high bit is the left column).
const GLYPHS: [(char, [u8; 5]); 11] = [
    ('0', [7, 5, 5, 5, 7]),
    ('1', [2, 6, 2, 2, 7]),
    ('2', [7, 1, 7, 4, 7]),
    ('3', [7, 1, 7, 1, 7]),
    ('4', [5, 5, 7, 1, 1]),
    ('5', [7, 4, 7, 1, 7]),
    ('6', [7, 4, 7, 5, 7]),
    ('7', [7, 1, 1, 1, 1]),
    ('8', [7, 5, 7, 5, 7]),
    ('9', [7, 5, 7, 1, 7]),
    ('+', [0, 2, 7, 2, 0]),
];

/// The drag image for `count` items whose first one is of `kind`. Above 99 items the badge reads "99+".
pub(crate) fn drag_preview(kind: Kind, count: usize) -> DragPreview {
    let mut px = vec![0u8; (CANVAS * CANVAS * 4) as usize];
    if kind == Kind::Folder {
        folder(&mut px);
    } else {
        page(&mut px);
    }
    if count > 1 {
        badge(&mut px, CANVAS, &count_label(count));
    }
    preview_from_rgba(CANVAS as u32, CANVAS as u32, &px)
}

/// Sets one pixel of a square canvas of `side` pixels, ignoring what is off it.
fn put_at(px: &mut [u8], side: i32, x: i32, y: i32, c: [u8; 4]) {
    if (0..side).contains(&x) && (0..side).contains(&y) {
        let i = ((y * side + x) * 4) as usize;
        px[i..i + 4].copy_from_slice(&c);
    }
}

/// Fills the rectangle `x0..x1` by `y0..y1` (exclusive ends) of a square canvas of `side` pixels.
fn rect_at(px: &mut [u8], side: i32, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    for y in y0..y1 {
        for x in x0..x1 {
            put_at(px, side, x, y, c);
        }
    }
}

/// Fills a rectangle of the preview canvas.
fn rect(px: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    rect_at(px, CANVAS, x0, y0, x1, y1, c);
}

fn folder(px: &mut [u8]) {
    rect(px, 8, 12, 28, 20, FOLDER_TAB);
    rect(px, 8, 18, 56, 52, FOLDER);
}

fn page(px: &mut [u8]) {
    rect(px, 14, 6, 50, 58, PAGE_EDGE);
    rect(px, 15, 7, 49, 57, PAGE);
    rect(px, 20, 20, 44, 22, LINE);
    rect(px, 20, 28, 44, 30, LINE);
    rect(px, 20, 36, 38, 38, LINE);
}

/// The red disc at the top right of a square canvas of `side` pixels, with the label centred in it. It is laid out
/// on the 64 pixel preview canvas (a glyph cell is three pixels a side there) and grows with the canvas.
fn badge(px: &mut [u8], side: i32, label: &str) {
    let unit = |v: i32| (v * side + CANVAS / 2) / CANVAS;
    let (cx, cy, r) = (unit(48), unit(16), unit(14).max(1));
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= r * r {
                put_at(px, side, x, y, BADGE);
            }
        }
    }
    let scale = unit(3).max(1);
    let chars: Vec<char> = label.chars().collect();
    let width = chars.len() as i32 * 4 * scale - scale;
    let mut x0 = cx - width / 2;
    let y0 = cy - 5 * scale / 2;
    for ch in chars {
        if let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) {
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if (bits >> (2 - col)) & 1 == 1 {
                        rect_at(px, side, x0 + col * scale, y0 + row as i32 * scale, x0 + (col + 1) * scale, y0 + (row as i32 + 1) * scale, BADGE_TEXT);
                    }
                }
            }
        }
        x0 += 4 * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_of(p: &DragPreview, c: [u8; 4]) -> usize {
        p.rgba.chunks_exact(4).filter(|px| px[..] == c[..]).count()
    }

    #[test]
    fn a_single_item_has_an_icon_and_no_badge() {
        let p = drag_preview(Kind::Other, 1);
        assert!(p.valid());
        assert_eq!((p.w, p.h), (32, 32), "the canvas is sampled down to the largest drag image");
        assert_eq!(count_of(&p, BADGE), 0);
        assert!(count_of(&p, PAGE) > 0);
    }

    #[test]
    fn a_folder_is_drawn_in_the_folder_colour() {
        let p = drag_preview(Kind::Folder, 1);
        assert!(p.valid());
        assert!(count_of(&p, FOLDER) > 0);
        assert_eq!(count_of(&p, PAGE), 0);
    }

    #[test]
    fn several_items_get_a_badge_with_their_count() {
        let three = drag_preview(Kind::Other, 3);
        assert!(three.valid());
        assert!(count_of(&three, BADGE) > 0);
        assert!(count_of(&three, BADGE_TEXT) > 0, "the count is drawn in the badge");
        let four = drag_preview(Kind::Other, 4);
        assert_ne!(three.rgba, four.rgba, "different counts draw different digits");
    }

    #[test]
    fn the_image_is_the_icons_size_at_device_pixels_within_its_limits() {
        assert_eq!(image_side(64., 2.0), 128, "a 64 pt icon on a display of scale 2");
        assert_eq!(image_side(64., 1.0), 64);
        assert_eq!(image_side(64., 1.5625), 100);
        assert_eq!(image_side(16., 2.0), 64, "a row's small icon is lifted at the least size");
        assert_eq!(image_side(128., 2.0), MAX_IMAGE_PX, "never more than the shell takes");
        assert_eq!(MAX_IMAGE_PX, 128);
    }

    #[test]
    fn the_image_keeps_the_icons_pixels_as_straight_colour_and_badges_several_items() {
        let side = 128usize;
        // half-transparent red, premultiplied as a surface holds it, and a clear pixel
        let mut icon = vec![0u32; side * side];
        icon[side * 100 + 10] = 128 | 128 << 24;
        let one = drag_image(&icon, side, 1);
        assert_eq!(one.len(), side * side * 4);
        let at = (side * 100 + 10) * 4;
        assert_eq!(one[at..at + 4], [255, 0, 0, 128], "straight colour, its alpha kept");
        assert_eq!(one[..4], [0, 0, 0, 0], "a clear pixel stays clear");
        assert!(!one.chunks_exact(4).any(|p| p == BADGE), "one item has no badge");
        let three = drag_image(&icon, side, 3);
        let disc = three.chunks_exact(4).filter(|p| *p == BADGE).count();
        assert!(disc > 1500, "the badge grows with the image: {disc} pixels");
        assert!(three.chunks_exact(4).any(|p| p == BADGE_TEXT), "with the count in it");
        assert_eq!(three[at..at + 4], [255, 0, 0, 128], "the icon is under it");
        assert_ne!(three, drag_image(&icon, side, 4));
        // the badge is in the top right quarter
        let (x, y) = (side * 3 / 4, side / 4);
        assert_eq!(three[(y * side + x) * 4 + 3], 255);
    }

    #[test]
    fn long_counts_read_as_99_plus() {
        assert_eq!(drag_preview(Kind::Other, 150).rgba, drag_preview(Kind::Other, 1000).rgba);
        assert_ne!(drag_preview(Kind::Other, 99).rgba, drag_preview(Kind::Other, 150).rgba);
    }
}

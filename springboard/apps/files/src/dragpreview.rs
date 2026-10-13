//! The drag image of a Files drag: the icon of the first item and, when the drag carries more than one item, a count badge.
//! It is drawn as straight RGBA on a square canvas and handed to the shell through `dnd::preview_from_rgba`, which samples
//! it down to the largest size a drag image may have. The shell draws it at the pointer (`dnd_draw`).
use bifrost_app::dnd::{preview_from_rgba, DragPreview};
use springboard::files::fs::Kind;

/// The canvas side in pixels; the shell's drag image limit is half of it.
const CANVAS: i32 = 64;

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
        let label = if count > 99 { "99+".to_string() } else { count.to_string() };
        badge(&mut px, &label);
    }
    preview_from_rgba(CANVAS as u32, CANVAS as u32, &px)
}

/// Sets one pixel, ignoring what is off the canvas.
fn put(px: &mut [u8], x: i32, y: i32, c: [u8; 4]) {
    if (0..CANVAS).contains(&x) && (0..CANVAS).contains(&y) {
        let i = ((y * CANVAS + x) * 4) as usize;
        px[i..i + 4].copy_from_slice(&c);
    }
}

/// Fills the rectangle `x0..x1` by `y0..y1` (exclusive ends).
fn rect(px: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    for y in y0..y1 {
        for x in x0..x1 {
            put(px, x, y, c);
        }
    }
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

/// The red disc at the top right with the label centred in it. Each glyph cell is three pixels a side.
fn badge(px: &mut [u8], label: &str) {
    let (cx, cy, r) = (48, 16, 14);
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= r * r {
                put(px, x, y, BADGE);
            }
        }
    }
    let scale = 3;
    let chars: Vec<char> = label.chars().collect();
    let width = chars.len() as i32 * 4 * scale - scale;
    let mut x0 = cx - width / 2;
    let y0 = cy - 5 * scale / 2;
    for ch in chars {
        if let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) {
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if (bits >> (2 - col)) & 1 == 1 {
                        rect(px, x0 + col * scale, y0 + row as i32 * scale, x0 + (col + 1) * scale, y0 + (row as i32 + 1) * scale, BADGE_TEXT);
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
    fn long_counts_read_as_99_plus() {
        assert_eq!(drag_preview(Kind::Other, 150).rgba, drag_preview(Kind::Other, 1000).rgba);
        assert_ne!(drag_preview(Kind::Other, 99).rgba, drag_preview(Kind::Other, 150).rgba);
    }
}

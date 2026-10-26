//! Vector icons for the Files app in the the system symbol set idiom: the blue folder
//! and the white document with a folded corner for listings, the sidebar
//! location glyphs, and the outline symbols of the nav bar and menus.

use super::fs::{Glyph as Loc, Kind};
use super::menu::Glyph as MenuGlyph;
use crate::gfx::{pack, rgba, Surface};
use crate::shell::glyphs::{disc, either, paint, ring, segment};
use crate::shell::ui::WHITE;

/// bifrost 15 Files folder blue.
pub const FOLDER: u32 = pack(52, 160, 250, 255);
const FOLDER_TAB: u32 = pack(33, 128, 230, 255);
/// The document body of a file icon.
pub const PAGE: u32 = pack(235, 235, 240, 255);
/// The flap of the folded corner: a darker shade of the page.
const FOLD: u32 = pack(200, 200, 208, 255);
/// The hairline along the crease of the fold, a step darker than the flap.
const FOLD_EDGE: u32 = pack(172, 172, 182, 255);
const INK: u32 = pack(120, 120, 128, 255);
/// systemBlue
pub const BLUE: u32 = pack(10, 132, 255, 255);

/// A folder filling `(x, y, w, h)` (the tab takes the top eighth).
pub fn folder(s: &mut Surface, (x, y, w, h): (f32, f32, f32, f32), color: u32) {
    let r = (w * 0.08).max(1.5);
    let tab_h = h * 0.2;
    s.fill_rrect(x, y, w * 0.42, tab_h * 1.6, r, if color == FOLDER { FOLDER_TAB } else { color });
    s.fill_rrect(x, y + tab_h * 0.7, w, h - tab_h * 0.7, r, color);
}

/// Signed distance to the rounded rectangle `(x, y, w, h)` of corner radius `r`: negative inside, for `paint`.
fn rrect_dist((x, y, w, h): (f32, f32, f32, f32), r: f32) -> impl Fn(f32, f32) -> f32 {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (mx, my) = (x + hw, y + hh);
    let r = r.min(hw).min(hh);
    move |px, py| {
        let (qx, qy) = ((px - mx).abs() - hw + r, (py - my).abs() - hh + r);
        (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
    }
}

/// A document with a folded top-right corner, with a kind mark inside.
pub fn document(s: &mut Surface, (x, y, w, h): (f32, f32, f32, f32), kind: Kind) {
    let r = (w * 0.1).max(1.5);
    let fold = w * 0.3;
    let (cx, cy) = (x + w, y);
    // The page is its rounded rectangle less the top-right corner (a triangle with legs `fold`). The corner is left
    // unpainted, so what is behind the icon (either appearance's content colour, or a selection plate) shows through
    // the cut; painting it a colour drew a black wedge.
    let body = rrect_dist((x, y, w, h), r);
    let kept = move |px: f32, py: f32| (fold - (cx - px) - (py - cy)) * std::f32::consts::FRAC_1_SQRT_2;
    paint(s, (x - 1.0, y - 1.0, w + 2.0, h + 2.0), PAGE, move |px, py| body(px, py).max(kept(px, py)));
    // the flap folded over the cut: a darker shade of the page, with a hairline along its crease
    let flap = move |px: f32, py: f32| ((cx - fold) - px).max(py - (cy + fold)).max(fold - (cx - px) - (py - cy));
    paint(s, (cx - fold, cy, fold, fold), FOLD, flap);
    let hair = (w * 0.012).max(1.0);
    let crease = segment((cx - fold, cy), (cx, cy + fold), 2.0 * hair);
    paint(s, (cx - fold - 1.0, cy - 1.0, fold + 2.0, fold + 2.0), FOLD_EDGE, move |px, py| crease(px, py).max(flap(px, py)));
    let (mx, my, mw) = (x + w * 0.2, y + h * 0.45, w * 0.6);
    let t = (w * 0.06).max(1.0);
    match kind {
        Kind::Text => {
            for i in 0..3 {
                let ly = my + i as f32 * h * 0.13;
                let lw = if i == 2 { mw * 0.6 } else { mw };
                paint(s, (mx, ly - t, lw + t, 2.0 * t), INK, segment((mx, ly), (mx + lw, ly), t));
            }
        }
        Kind::Image => {
            let (bx, by, bw, bh) = (mx, my - h * 0.05, mw, h * 0.38);
            s.fill_rrect(bx, by, bw, bh, t, pack(90, 170, 240, 255));
            paint(s, (bx, by, bw, bh), pack(70, 150, 90, 255), move |px, py| {
                let hill = |cx: f32, k: f32| (by + bh - bh * 0.6 + k * (px - cx).abs()) - py;
                hill(bx + bw * 0.35, 1.1).min(hill(bx + bw * 0.75, 1.4)).max(by - py).max(px - bx - bw).max(bx - px)
            });
            paint(s, (bx, by, bw, bh), pack(255, 230, 120, 255), disc((bx + bw * 0.78, by + bh * 0.28), bh * 0.12));
        }
        Kind::Video => {
            let (cx, cy, rr) = (x + w / 2.0, y + h * 0.58, w * 0.24);
            paint(s, (cx - rr - 1.0, cy - rr - 1.0, 2.0 * rr + 2.0, 2.0 * rr + 2.0), INK, disc((cx, cy), rr));
            play_mark(s, (cx + rr * 0.1, cy), rr * 0.55, PAGE);
        }
        Kind::Audio => {
            let (nx, ny) = (x + w * 0.58, y + h * 0.42);
            paint(s, (x, y, w, h), INK, either(segment((nx, ny), (nx, ny + h * 0.28), t * 1.2), disc((nx - w * 0.1, ny + h * 0.3), w * 0.11)));
            paint(s, (x, y, w, h), INK, segment((nx, ny), (nx + w * 0.16, ny + h * 0.06), t * 1.2));
        }
        Kind::Archive => {
            let zx = x + w / 2.0;
            for i in 0..5 {
                let zy = my - h * 0.1 + i as f32 * h * 0.09;
                let dx = if i % 2 == 0 { -t } else { t };
                s.fill_rrect(zx + dx - t, zy, 2.0 * t, h * 0.06, 0.0, INK);
            }
        }
        Kind::Application => {
            // an app tile: the rounded square every app icon is, in the accent, with a lighter inner mark
            let side = mw;
            let (tx, ty) = (x + (w - side) / 2.0, my - h * 0.08);
            s.fill_rrect(tx, ty, side, side, side * 0.22, BLUE);
            s.fill_rrect(tx + side * 0.3, ty + side * 0.3, side * 0.4, side * 0.4, side * 0.1, WHITE);
        }
        Kind::Folder | Kind::Other => {}
    }
}

/// A right-pointing triangle of "radius" `r` centred at `c`.
pub fn play_mark(s: &mut Surface, (cx, cy): (f32, f32), r: f32, color: u32) {
    paint(s, (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), color, move |px, py| {
        let (dx, dy) = (px - cx, py - cy);
        (-r * 0.8 - dx).max((dy.abs() * 0.9 + dx * 0.5) - r * 0.5)
    });
}

/// The icon of a listing entry in `(x, y, w, h)`.
pub fn entry(s: &mut Surface, rect: (f32, f32, f32, f32), kind: Kind) {
    match kind {
        Kind::Folder => {
            let (x, y, w, h) = rect;
            folder(s, (x, y + h * 0.1, w, h * 0.8), FOLDER);
        }
        k => {
            let (x, y, w, h) = rect;
            let pw = h * 0.78;
            document(s, (x + (w - pw) / 2.0, y, pw, h), k);
        }
    }
}

/// The icon of a listing entry on the accent selection plate: a folder turns white, because its blue would vanish into
/// the plate; a document keeps its pale page, which already stands out.
pub fn entry_on_accent(s: &mut Surface, rect: (f32, f32, f32, f32), kind: Kind) {
    match kind {
        Kind::Folder => {
            let (x, y, w, h) = rect;
            folder(s, (x, y + h * 0.1, w, h * 0.8), WHITE);
        }
        k => entry(s, rect, k),
    }
}

/// A sidebar location glyph, `size` px square at `(x, y)`, in blue.
pub fn location(s: &mut Surface, (x, y): (f32, f32), size: f32, glyph: Loc) {
    let t = (size * 0.09).max(1.5);
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    match glyph {
        Loc::Computer => {
            s.stroke_rrect(x + size * 0.08, y + size * 0.12, size * 0.84, size * 0.58, size * 0.08, t, BLUE);
            paint(s, (x, y, size, size), BLUE, disc((cx, y + size * 0.86), t * 0.6));
        }
        Loc::Download => {
            paint(s, (x, y, size, size), BLUE, ring((cx, cy), size * 0.44, t));
            paint(s, (x, y, size, size), BLUE, segment((cx, cy - size * 0.22), (cx, cy + size * 0.18), t));
            paint(s, (x, y, size, size), BLUE, either(segment((cx, cy + size * 0.2), (cx - size * 0.16, cy + size * 0.04), t), segment((cx, cy + size * 0.2), (cx + size * 0.16, cy + size * 0.04), t)));
        }
        Loc::Play => {
            paint(s, (x, y, size, size), BLUE, ring((cx, cy), size * 0.44, t));
            play_mark(s, (cx + size * 0.03, cy), size * 0.2, BLUE);
        }
        Loc::Photo => {
            s.stroke_rrect(x + size * 0.08, y + size * 0.15, size * 0.84, size * 0.7, size * 0.1, t, BLUE);
            let (bx, by, bw, bh) = (x + size * 0.08 + t, y + size * 0.15 + t, size * 0.84 - 2.0 * t, size * 0.7 - 2.0 * t);
            paint(s, (bx, by, bw, bh), BLUE, move |px, py| {
                let hill = |hx: f32, k: f32| (by + bh - bh * 0.55 + k * (px - hx).abs()) - py;
                hill(bx + bw * 0.35, 1.0).min(hill(bx + bw * 0.72, 1.3)).max(by - py).max(px - bx - bw).max(bx - px)
            });
            paint(s, (x, y, size, size), BLUE, disc((bx + bw * 0.74, by + bh * 0.3), size * 0.07));
        }
        Loc::Clock => {
            paint(s, (x, y, size, size), BLUE, ring((cx, cy), size * 0.44, t));
            paint(s, (x, y, size, size), BLUE, either(segment((cx, cy), (cx, cy - size * 0.26), t), segment((cx, cy), (cx + size * 0.18, cy + size * 0.08), t)));
        }
        Loc::Bin => {
            s.stroke_rrect(x + size * 0.2, y + size * 0.28, size * 0.6, size * 0.64, size * 0.08, t, BLUE);
            paint(s, (x, y, size, size), BLUE, segment((x + size * 0.1, y + size * 0.24), (x + size * 0.9, y + size * 0.24), t));
            s.fill_rrect(cx - size * 0.12, y + size * 0.1, size * 0.24, t * 1.4, t * 0.5, BLUE);
        }
        Loc::Root => {
            let r = size * 0.1;
            s.stroke_rrect(x + size * 0.05, y + size * 0.2, size * 0.9, size * 0.62, r, t, BLUE);
            s.fill_rrect(x + size * 0.05, y + size * 0.12, size * 0.4, size * 0.16, r * 0.6, BLUE);
        }
        Loc::Tag(tag) => tag_dot(s, (cx, cy), size * 0.3, tag),
    }
}

/// A filled colour-tag dot of radius `r` centred on `c`.
pub fn tag_dot(s: &mut Surface, (cx, cy): (f32, f32), r: f32, tag: super::tags::Tag) {
    let c = tag.rgb();
    s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, pack((c >> 16) as u8, (c >> 8) as u8, c as u8, 255));
}

/// The select-mode mark: a ring, or a blue disc with a check when `on`.
pub fn check_circle(s: &mut Surface, (x, y): (f32, f32), size: f32, on: bool) {
    let t = (size * 0.09).max(1.5);
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    let b = (x, y, size, size);
    if on {
        paint(s, b, BLUE, disc((cx, cy), size * 0.5));
        let white = pack(255, 255, 255, 255);
        paint(s, b, white, either(segment((cx - size * 0.22, cy + size * 0.02), (cx - size * 0.06, cy + size * 0.18), t * 1.2), segment((cx - size * 0.06, cy + size * 0.18), (cx + size * 0.24, cy - size * 0.16), t * 1.2)));
    } else {
        paint(s, b, pack(142, 142, 147, 255), ring((cx, cy), size * 0.46, t));
    }
}

/// A menu row glyph, `size` px square at `(x, y)`.
pub fn menu(s: &mut Surface, (x, y): (f32, f32), size: f32, glyph: MenuGlyph, color: u32) {
    let t = (size * 0.09).max(1.5);
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    let box_ = (x, y, size, size);
    match glyph {
        MenuGlyph::Pencil => {
            paint(s, box_, color, segment((x + size * 0.2, y + size * 0.8), (x + size * 0.78, y + size * 0.22), t * 2.6));
            paint(s, box_, pack(0, 0, 0, 255), segment((x + size * 0.2, y + size * 0.8), (x + size * 0.7, y + size * 0.3), t * 0.9));
        }
        MenuGlyph::Folder => {
            s.stroke_rrect(x + size * 0.05, y + size * 0.22, size * 0.9, size * 0.6, size * 0.1, t, color);
            s.fill_rrect(x + size * 0.05, y + size * 0.14, size * 0.4, size * 0.16, size * 0.06, color);
        }
        MenuGlyph::Doc | MenuGlyph::Clipboard => {
            let (dx, dy, dw, dh) = (x + size * 0.22, y + size * 0.1, size * 0.56, size * 0.8);
            s.stroke_rrect(dx, dy, dw, dh, size * 0.08, t, color);
            if glyph == MenuGlyph::Clipboard {
                s.fill_rrect(cx - size * 0.16, dy - t, size * 0.32, t * 2.0, t, color);
            } else {
                paint(s, box_, color, segment((dx + size * 0.14, cy), (dx + dw - size * 0.14, cy), t));
                paint(s, box_, color, segment((dx + size * 0.14, cy + size * 0.18), (dx + dw - size * 0.14, cy + size * 0.18), t));
            }
        }
        MenuGlyph::Share => {
            let (bx, by, bw, bh) = (x + size * 0.18, y + size * 0.36, size * 0.64, size * 0.56);
            s.stroke_rrect(bx, by, bw, bh, size * 0.08, t, color);
            s.fill_rrect(cx - size * 0.14, by - t, size * 0.28, 2.5 * t, 0.0, pack(0, 0, 0, 255));
            paint(s, box_, color, segment((cx, y + size * 0.06), (cx, cy + size * 0.1), t));
            paint(s, box_, color, either(segment((cx, y + size * 0.06), (cx - size * 0.16, y + size * 0.22), t), segment((cx, y + size * 0.06), (cx + size * 0.16, y + size * 0.22), t)));
        }
        MenuGlyph::Terminal => {
            s.stroke_rrect(x + size * 0.05, y + size * 0.15, size * 0.9, size * 0.7, size * 0.1, t, color);
            paint(s, box_, color, either(segment((x + size * 0.25, cy - size * 0.14), (x + size * 0.4, cy), t), segment((x + size * 0.4, cy), (x + size * 0.25, cy + size * 0.14), t)));
            paint(s, box_, color, segment((x + size * 0.48, cy + size * 0.14), (x + size * 0.72, cy + size * 0.14), t));
        }
        MenuGlyph::Info => {
            paint(s, box_, color, ring((cx, cy), size * 0.44, t));
            paint(s, box_, color, segment((cx, cy - size * 0.05), (cx, cy + size * 0.22), t * 1.2));
            paint(s, box_, color, disc((cx, cy - size * 0.2), t * 0.75));
        }
        MenuGlyph::Trash => {
            s.stroke_rrect(x + size * 0.2, y + size * 0.26, size * 0.6, size * 0.66, size * 0.08, t, color);
            s.fill_rrect(x + size * 0.1, y + size * 0.18, size * 0.8, t, t / 2.0, color);
            s.fill_rrect(cx - size * 0.14, y + size * 0.08, size * 0.28, t, t / 2.0, color);
            for k in [-0.14, 0.0, 0.14] {
                paint(s, box_, color, segment((cx + size * k, y + size * 0.38), (cx + size * k, y + size * 0.78), t * 0.8));
            }
        }
        MenuGlyph::FolderPlus => {
            s.stroke_rrect(x + size * 0.05, y + size * 0.22, size * 0.9, size * 0.6, size * 0.1, t, color);
            s.fill_rrect(x + size * 0.05, y + size * 0.14, size * 0.4, size * 0.16, size * 0.06, color);
            paint(s, box_, color, either(segment((cx - size * 0.14, cy + size * 0.06), (cx + size * 0.14, cy + size * 0.06), t), segment((cx, cy - size * 0.08), (cx, cy + size * 0.2), t)));
        }
    }
}

/// The nav-bar "new folder" button: a folder outline with a plus.
pub fn new_folder_button(s: &mut Surface, (x, y): (f32, f32), size: f32) {
    menu(s, (x, y), size, MenuGlyph::FolderPlus, BLUE);
}

/// The nav-bar view toggle: four small squares (grid) or three lines (list).
pub fn view_button(s: &mut Surface, (x, y): (f32, f32), size: f32, grid: bool) {
    let t = (size * 0.09).max(1.5);
    if grid {
        let (sq, gap) = (size * 0.38, size * 0.1);
        for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            s.stroke_rrect(x + size * 0.07 + i * (sq + gap), y + size * 0.07 + j * (sq + gap), sq, sq, size * 0.08, t, BLUE);
        }
    } else {
        for k in 0..3 {
            let ly = y + size * (0.22 + 0.28 * k as f32);
            paint(s, (x, y, size, size), BLUE, segment((x + size * 0.1, ly), (x + size * 0.9, ly), t));
        }
    }
}

/// The search field's magnifying glass.
pub fn magnifier(s: &mut Surface, (x, y): (f32, f32), size: f32, color: u32) {
    let t = (size * 0.11).max(1.5);
    let (cx, cy, r) = (x + size * 0.42, y + size * 0.42, size * 0.3);
    paint(s, (x, y, size, size), color, ring((cx, cy), r, t));
    paint(s, (x, y, size, size), color, segment((cx + r * 0.75, cy + r * 0.75), (x + size * 0.92, y + size * 0.92), t));
}

/// Which way a chevron points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    Left,
    Right,
    Down,
}

/// A chevron centred at `(cx, cy)`, `size` px tall, stroked `t` px.
pub fn chevron(s: &mut Surface, (cx, cy): (f32, f32), size: f32, t: f32, point: Point, color: u32) {
    let h = size / 2.0;
    let (a, b, c) = match point {
        Point::Down => ((cx - h, cy - h * 0.5), (cx, cy + h * 0.5), (cx + h, cy - h * 0.5)),
        Point::Left => ((cx + h * 0.5, cy - h), (cx - h * 0.5, cy), (cx + h * 0.5, cy + h)),
        Point::Right => ((cx - h * 0.5, cy - h), (cx + h * 0.5, cy), (cx - h * 0.5, cy + h)),
    };
    paint(s, (cx - size, cy - size, 2.0 * size, 2.0 * size), color, either(segment(a, b, t), segment(b, c, t)));
}

/// A faint circle with an "x": clears the search field.
pub fn clear_button(s: &mut Surface, (cx, cy): (f32, f32), r: f32) {
    paint(s, (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), rgba(142, 142, 147, 255), disc((cx, cy), r));
    let k = r * 0.4;
    paint(s, (cx - r, cy - r, 2.0 * r, 2.0 * r), WHITE, either(segment((cx - k, cy - k), (cx + k, cy + k), r * 0.2), segment((cx - k, cy + k), (cx + k, cy - k), r * 0.2)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Image;

    fn canvas() -> Image {
        Image::new(120, 120)
    }

    fn painted(img: &Image) -> usize {
        img.px.iter().filter(|&&p| p != 0).count()
    }

    #[test]
    fn every_icon_draws_something() {
        for kind in [Kind::Folder, Kind::Text, Kind::Image, Kind::Video, Kind::Audio, Kind::Archive, Kind::Application, Kind::Other] {
            let mut img = canvas();
            entry(&mut Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 }, (10.0, 10.0, 100.0, 80.0), kind);
            assert!(painted(&img) > 500, "{kind:?}");
            if kind == Kind::Folder {
                assert!(img.px.contains(&FOLDER));
            } else {
                assert!(img.px.contains(&PAGE));
            }
        }
        for g in [Loc::Computer, Loc::Download, Loc::Play, Loc::Photo, Loc::Root] {
            let mut img = canvas();
            location(&mut Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 }, (10.0, 10.0), 100.0, g);
            assert!(painted(&img) > 100, "{g:?}");
        }
        for g in [MenuGlyph::Pencil, MenuGlyph::Folder, MenuGlyph::Doc, MenuGlyph::Share, MenuGlyph::Terminal, MenuGlyph::Info, MenuGlyph::Trash, MenuGlyph::FolderPlus, MenuGlyph::Clipboard] {
            let mut img = canvas();
            menu(&mut Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 }, (10.0, 10.0), 100.0, g, WHITE);
            assert!(painted(&img) > 100, "{g:?}");
        }
        let mut img = canvas();
        let mut s = Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 };
        view_button(&mut s, (0.0, 0.0), 40.0, true);
        view_button(&mut s, (40.0, 0.0), 40.0, false);
        new_folder_button(&mut s, (80.0, 0.0), 40.0);
        magnifier(&mut s, (0.0, 40.0), 40.0, WHITE);
        chevron(&mut s, (60.0, 60.0), 20.0, 3.0, Point::Left, WHITE);
        chevron(&mut s, (90.0, 60.0), 20.0, 3.0, Point::Down, WHITE);
        chevron(&mut s, (90.0, 90.0), 20.0, 3.0, Point::Right, WHITE);
        clear_button(&mut s, (20.0, 100.0), 10.0);
        assert!(painted(&img) > 300);
    }

    #[test]
    fn on_the_accent_plate_a_folder_is_white_and_a_document_is_unchanged() {
        let mut img = canvas();
        entry_on_accent(&mut Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 }, (10.0, 10.0, 100.0, 80.0), Kind::Folder);
        assert!(painted(&img) > 500);
        assert!(img.px.contains(&WHITE), "the folder is drawn white");
        assert!(!img.px.contains(&FOLDER), "its blue would vanish into the plate");
        let (mut plain, mut on_plate) = (canvas(), canvas());
        entry(&mut Surface { w: 120, h: 120, stride: 120, px: &mut plain.px, y0: 0 }, (10.0, 10.0, 100.0, 80.0), Kind::Text);
        entry_on_accent(&mut Surface { w: 120, h: 120, stride: 120, px: &mut on_plate.px, y0: 0 }, (10.0, 10.0, 100.0, 80.0), Kind::Text);
        assert_eq!(plain.px, on_plate.px, "a document looks the same on any plate");
    }

    #[test]
    fn the_folded_corner_of_a_document_is_cut_away_not_black() {
        let black = pack(0, 0, 0, 255);
        // the flap is a darker shade of the page, and its hairline a step darker again
        assert!((FOLD & 0xff) < (PAGE & 0xff) && (FOLD_EDGE & 0xff) < (FOLD & 0xff));
        // a light plate (Light appearance) and a dark one (Dark): the cut shows whichever is behind the icon
        for plate in [pack(255, 255, 255, 255), pack(30, 30, 32, 255)] {
            // the large grid icon, then the small list icon: its rectangle and a pixel in the cut corner, one on the
            // flap and one on the plain page
            let icons = [((10.0, 10.0, 80.0, 100.0), (88, 12), (70, 30), (20, 80)), ((10.0, 10.0, 20.0, 26.0), (29, 11), (25, 14), (14, 28))];
            for (rect, cut, flap, page) in icons {
                let mut img = canvas();
                img.px.fill(plate);
                document(&mut Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 }, rect, Kind::Other);
                let at = |(px, py): (usize, usize)| img.px[py * 120 + px];
                assert_eq!(at(cut), plate, "the cut corner of {rect:?} is not painted, the plate shows");
                assert_eq!(at(flap), FOLD, "the flap of {rect:?} is its own shade");
                assert_eq!(at(page), PAGE, "the page of {rect:?} is unchanged");
                assert!(!img.px.contains(&black), "no black pixel in {rect:?}");
            }
            // every kind of entry icon, whole: nothing black anywhere
            for kind in [Kind::Text, Kind::Image, Kind::Video, Kind::Audio, Kind::Archive, Kind::Application, Kind::Other] {
                let mut img = canvas();
                img.px.fill(plate);
                entry(&mut Surface { w: 120, h: 120, stride: 120, px: &mut img.px, y0: 0 }, (10.0, 10.0, 100.0, 80.0), kind);
                assert!(!img.px.contains(&black), "{kind:?} has a black pixel");
            }
        }
    }
}

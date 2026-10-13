//! Glyph outlines and direct glyph drawing for apps. [`glyph_path`] turns a glyph of a font into a
//! [`Path`] the shape layer understands (curves flattened, y down, origin on the baseline); [`text_path`] does it for a
//! string with advances; [`draw_glyphs`] paints glyphs at given positions into a surface; [`fill_path`] paints any
//! path with a solid colour (hard edged, for masks and large shapes).
use bifrost_render::bezier::{BezierPath, WindingRule};
use bifrost_render::font::{Font, Style, Var};
use bifrost_render::gfx::{pack, Image, Surface};
use bifrost_ui::shape::{Path, Point};

/// Joins flat segments `[x0, y0, x1, y1]` (consecutive segments of one contour share an end point) into a path.
/// A contour whose last point returns to its first is closed.
pub fn path_from_segments(segs: &[[f32; 4]], origin: (f32, f32)) -> Path {
    let mut path = Path::new();
    let mut start: Option<(f32, f32)> = None;
    let mut cur: Option<(f32, f32)> = None;
    let pt = |x: f32, y: f32| Point { x: x + origin.0, y: y + origin.1 };
    for s in segs {
        let a = (s[0], s[1]);
        let b = (s[2], s[3]);
        if cur != Some(a) {
            path.move_to(pt(a.0, a.1));
            start = Some(a);
        }
        if start == Some(b) {
            path.close();
            cur = None;
            start = None;
            continue;
        }
        path.line_to(pt(b.0, b.1));
        cur = Some(b);
    }
    path
}

/// The outline of glyph `gid` at size `px`, with the glyph origin (on the baseline) at `origin`. `vars` selects a
/// variable-font instance. A glyph without ink (a space) gives an empty path.
pub fn glyph_path(font: &Font, gid: u16, px: f32, vars: &[Var], origin: (f32, f32)) -> Path {
    path_from_segments(&font.glyph_segments(gid, px, vars), origin)
}

/// The outline of a string set on one line, the baseline origin at `origin`, using the font's advances. Missing
/// characters use the font's missing glyph. Returns the path and the advance of the string.
pub fn text_path(font: &Font, text: &str, px: f32, vars: &[Var], origin: (f32, f32)) -> (Path, f32) {
    let k = px / font.upem;
    let mut x = 0.0_f32;
    let mut all = Path::new();
    for ch in text.chars() {
        let gid = font.glyph(ch);
        let p = glyph_path(font, gid, px, vars, (origin.0 + x, origin.1));
        for sp in p.subpaths() {
            let mut it = sp.points.iter();
            if let Some(first) = it.next() {
                all.move_to(*first);
                for q in it {
                    all.line_to(*q);
                }
                if sp.closed {
                    all.close();
                }
            }
        }
        x += font.advance(gid) * k;
    }
    (all, x)
}

/// Draws glyphs at baseline positions (`positions[i]` is the origin of `gids[i]`, in surface pixels) in `color`
/// (premultiplied, as `gfx::rgba` makes it). Extra gids or positions are ignored.
pub fn draw_glyphs(surface: &mut Surface, font: &Font, gids: &[u16], positions: &[(f32, f32)], px: f32, color: u32) {
    let white = pack(255, 255, 255, 255);
    let st = Style { px, embolden: 0.0, color: white, shadow: false };
    // The rendered image has a 2 px margin and puts the baseline at margin + ascent.
    let top = 2.0 + font.ascent * px / font.upem;
    for (&gid, &(x, y)) in gids.iter().zip(positions) {
        let img = font.render_glyph(gid, &st);
        surface.blit_colored(&img, (x - 2.0).round() as i32, (y - top).round() as i32, color, 255);
    }
}

/// Fills a path with a solid colour (premultiplied), non-zero winding, hard edged, at the path's own coordinates
/// in the surface.
pub fn fill_path(surface: &mut Surface, path: &Path, color: u32) {
    let Some(b) = path.bounds() else { return };
    let (x0, y0) = (b.x.floor(), b.y.floor());
    let w = (b.w.ceil() as usize + 2).min(8192);
    let h = (b.h.ceil() as usize + 2).min(8192);
    let mut bp = BezierPath::new();
    for sp in path.subpaths() {
        let mut it = sp.points.iter();
        if let Some(p) = it.next() {
            bp.move_to((p.x - x0, p.y - y0));
            for q in it {
                bp.line_to((q.x - x0, q.y - y0));
            }
            bp.close();
        }
    }
    let mask = bp.fill_mask(w, h, WindingRule::NonZero);
    let mut img = Image::new(w, h);
    for (p, &m) in img.px.iter_mut().zip(&mask) {
        if m != 0 {
            *p = color;
        }
    }
    surface.blit(&img, x0 as i32, y0 as i32, 255, 255);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_become_a_closed_contour() {
        let segs = [[0.0, 0.0, 4.0, 0.0], [4.0, 0.0, 4.0, 4.0], [4.0, 4.0, 0.0, 0.0]];
        let p = path_from_segments(&segs, (1.0, 2.0));
        assert_eq!(p.subpaths().len(), 1);
        let sp = &p.subpaths()[0];
        assert!(sp.closed);
        assert_eq!(sp.points.len(), 3);
        assert_eq!(sp.points[0], Point { x: 1.0, y: 2.0 });
    }

    #[test]
    fn disjoint_contours_make_separate_subpaths() {
        let segs = [[0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 0.0, 0.0], [5.0, 5.0, 6.0, 5.0], [6.0, 5.0, 5.0, 5.0]];
        let p = path_from_segments(&segs, (0.0, 0.0));
        assert_eq!(p.subpaths().len(), 2);
        assert!(path_from_segments(&[], (0.0, 0.0)).is_empty());
    }

    #[test]
    fn fill_path_paints_inside_pixels() {
        let mut p = Path::new();
        p.move_to(Point { x: 1.0, y: 1.0 }).line_to(Point { x: 5.0, y: 1.0 }).line_to(Point { x: 5.0, y: 5.0 }).line_to(Point { x: 1.0, y: 5.0 }).close();
        let mut px = vec![0u32; 8 * 8];
        let mut s = Surface { w: 8, h: 8, stride: 8, px: &mut px, y0: 0 };
        fill_path(&mut s, &p, 0xff00_00ff);
        assert_ne!(px[3 * 8 + 3], 0);
        assert_eq!(px[7 * 8 + 7], 0);
    }
}

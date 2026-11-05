//! Glyph outlines and direct glyph drawing for apps. [`glyph_path`] turns a glyph of a font into a
//! [`Path`] the shape layer understands (curves flattened, y down, origin on the baseline); [`text_path`] does it for a
//! string with advances; [`draw_glyphs`] paints glyphs at given positions into a surface; [`fill_path`] paints any
//! path with a solid colour (hard edged, for masks and large shapes).
//!
//! Shape painters (the shell's glyph functions: [`paint`] with [`disc`], [`ring`], [`segment`], [`arc`] and [`either`]
//! as signed distances, plus the named icons [`globe`], [`mic`], [`dismiss_keyboard`], [`alarm_clock`], [`stopwatch`]
//! and [`timer`]) are re-exported from `bifrost_render::sdf_glyphs`, so apps draw through the SDK and the shell draws the
//! same pixels.
use bifrost_render::bezier::{BezierPath, WindingRule};
use bifrost_render::font::{Font, Style, Var};
use bifrost_render::gfx::{pack, Image, Surface};
use bifrost_render::cg::path::Path;

pub use bifrost_render::sdf_glyphs::{
    alarm_clock, arc, dismiss_keyboard, disc, either, ellipse, globe, mic, paint, ring, segment, stopwatch, timer,
};

/// Joins flat segments `[x0, y0, x1, y1]` (consecutive segments of one contour share an end point) into a path.
/// A contour whose last point returns to its first is closed.
pub fn path_from_segments(segs: &[[f32; 4]], origin: (f32, f32)) -> Path {
    let mut path = Path::new();
    let mut start: Option<(f32, f32)> = None;
    let mut cur: Option<(f32, f32)> = None;
    let pt = |x: f32, y: f32| (x + origin.0, y + origin.1);
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
        // default-ignorable characters (zero-width spaces, joiners, the soft hyphen) take no room and draw nothing
        if bifrost_render::font::is_default_ignorable(ch) {
            continue;
        }
        let gid = font.glyph(ch);
        let p = glyph_path(font, gid, px, vars, (origin.0 + x, origin.1));
        for sp in p.flatten_subpaths(0.1) {
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
    for sp in path.flatten_subpaths(0.1) {
        let mut it = sp.points.iter();
        if let Some(p) = it.next() {
            bp.move_to((p.0 - x0, p.1 - y0));
            for q in it {
                bp.line_to((q.0 - x0, q.1 - y0));
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
        let subs = p.flatten_subpaths(0.1);
        assert_eq!(subs.len(), 1);
        let sp = &subs[0];
        assert!(sp.closed);
        assert_eq!(sp.points.len(), 3);
        assert_eq!(sp.points[0], (1.0, 2.0));
    }

    #[test]
    fn disjoint_contours_make_separate_subpaths() {
        let segs = [[0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 0.0, 0.0], [5.0, 5.0, 6.0, 5.0], [6.0, 5.0, 5.0, 5.0]];
        let p = path_from_segments(&segs, (0.0, 0.0));
        assert_eq!(p.flatten_subpaths(0.1).len(), 2);
        assert!(path_from_segments(&[], (0.0, 0.0)).is_empty());
    }

    #[test]
    fn fill_path_paints_inside_pixels() {
        let mut p = Path::new();
        p.move_to((1.0, 1.0));
        p.line_to((5.0, 1.0));
        p.line_to((5.0, 5.0));
        p.line_to((1.0, 5.0));
        p.close();
        let mut px = vec![0u32; 8 * 8];
        let mut s = Surface { w: 8, h: 8, stride: 8, px: &mut px, y0: 0 };
        fill_path(&mut s, &p, 0xff00_00ff);
        assert_ne!(px[3 * 8 + 3], 0);
        assert_eq!(px[7 * 8 + 7], 0);
    }

    /// Runs `draw` on a fresh 64 x 64 surface and returns its pixels.
    fn with_surface(draw: impl FnOnce(&mut Surface)) -> Vec<u32> {
        let mut buf = vec![0u32; 64 * 64];
        {
            let mut s = Surface { w: 64, h: 64, stride: 64, px: &mut buf, y0: 0 };
            draw(&mut s);
        }
        buf
    }

    #[test]
    fn sdk_painters_draw_the_same_pixels_as_the_shell() {
        let white = pack(255, 255, 255, 255);
        let sdk = with_surface(|s| {
            paint(s, (0.0, 0.0, 64.0, 64.0), white, either(disc((20.0, 24.0), 9.0), segment((30.0, 10.0), (50.0, 40.0), 3.0)));
        });
        let shell = with_surface(|s| {
            use springboard::shell::glyphs as shell_glyphs;
            shell_glyphs::paint(
                s,
                (0.0, 0.0, 64.0, 64.0),
                white,
                shell_glyphs::either(shell_glyphs::disc((20.0, 24.0), 9.0), shell_glyphs::segment((30.0, 10.0), (50.0, 40.0), 3.0)),
            );
        });
        assert_eq!(sdk, shell);
        assert!(sdk.iter().any(|&p| p >> 24 == 255), "the glyph drew ink");
    }

    #[test]
    fn sdk_named_icons_match_the_shell() {
        let white = pack(255, 255, 255, 255);
        let sdk = with_surface(|s| globe(s, (32.0, 32.0), 14.0, 2.0, white));
        let shell = with_surface(|s| springboard::shell::glyphs::globe(s, (32.0, 32.0), 14.0, 2.0, white));
        assert_eq!(sdk, shell);
    }

    #[test]
    fn disc_covers_its_radius_at_two_sizes() {
        let white = pack(255, 255, 255, 255);
        for r in [6.0_f32, 20.0] {
            let buf = with_surface(|s| paint(s, (0.0, 0.0, 64.0, 64.0), white, disc((32.0, 32.0), r)));
            // row 32 (its pixel centres sit half a pixel below the centre): the last pixel inside is solid, the first past the edge is empty
            let inside = 32 + r as usize - 1;
            let outside = 32 + r as usize;
            assert!(buf[32 * 64 + inside] >> 24 > 240, "r {r}: last pixel inside the disc is solid");
            assert_eq!(buf[32 * 64 + outside] >> 24, 0, "r {r}: pixel past the radius is empty");
            let area: f32 = buf.iter().map(|&p| (p >> 24) as f32 / 255.0).sum();
            let want = std::f32::consts::PI * r * r;
            assert!((area - want).abs() < want * 0.05, "r {r}: coverage {area} within 5% of {want}");
        }
    }

    #[test]
    fn segment_keeps_its_width_at_two_lengths() {
        let white = pack(255, 255, 255, 255);
        for (len, t) in [(8.0_f32, 2.0_f32), (40.0, 6.0)] {
            let (x0, x1) = (12.0_f32, 12.0 + len);
            let buf = with_surface(|s| paint(s, (0.0, 0.0, 64.0, 64.0), white, segment((x0, 32.0), (x1, 32.0), t)));
            let mid = ((x0 + x1) / 2.0) as usize;
            assert!(buf[32 * 64 + mid] >> 24 > 240, "len {len}: the middle of the line is solid");
            assert_eq!(buf[(32 + t as usize) * 64 + mid] >> 24, 0, "len {len}: one width below the line is empty");
            assert_eq!(buf[32 * 64 + (x1 + t) as usize + 1] >> 24, 0, "len {len}: nothing past the round cap");
        }
    }
}

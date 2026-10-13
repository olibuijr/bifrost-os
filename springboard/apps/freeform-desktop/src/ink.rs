//! Pure stroke helpers for Freeform: painting strokes of the drawing model (`bifrost_app::controls::drawing`) into
//! the pixel canvas, the spacing of the discs along a stroke, and the outlines the shape tools draw. Nothing here
//! touches the screen, so the host tests cover it.
use bifrost_app::controls::cg::path::Affine;
use bifrost_app::controls::drawing::{Drawing, Ink, InkKind, Stroke};
use bifrost_app::controls::cg::path::Path;
use bifrost_app::kit::cg::color::Color;
use bifrost_app::kit::cg::context::{DrawContext, LineCap, LineJoin, Rect as CgRect};
use bifrost_app::kit::cg::path::Path as CgPath;
use bifrost_app::controls::Rect;

/// Marker ink covers at most this alpha (96 of 255), as the live marker does.
pub const MARKER_ALPHA: f32 = 96.0 / 255.0;

/// Linear blend of two packed pixels, byte by byte (`a` = 0..=255 of `src`).
pub fn mix(dst: u32, src: u32, a: u32) -> u32 {
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let d = (dst >> shift) & 255;
        let s = (src >> shift) & 255;
        out |= ((d * (255 - a) + s * a + 127) / 255) << shift;
    }
    out
}

/// Strokes the polyline `pts` (canvas pixels) into `px`, a `cw` x `ch` canvas, with round caps and joins: `width` is the
/// line width in pixels, `dash` the on and off lengths in pixels (solid when `None`), `color` the opaque canvas pixel
/// and `alpha` (0..=1) the most it covers of each pixel. A single point, or points that all coincide, paints a disc.
#[allow(clippy::too_many_arguments)]
/// A packed premultiplied pixel colour as a drawing colour (straight alpha).
fn color_of(c: u32) -> Color {
    let [r, g, b, a] = c.to_le_bytes();
    let un = |v: u8| if a == 0 { 0 } else { ((v as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
    Color::from_rgba8(un(r), un(g), un(b), a)
}

pub fn stroke_polyline(px: &mut [u32], cw: usize, ch: usize, pts: &[(f32, f32)], width: f32, dash: Option<[f32; 2]>, color: u32, alpha: f32) {
    let Some(&(x0, y0)) = pts.first() else { return };
    if cw == 0 || ch == 0 {
        return;
    }
    let mut cx = DrawContext::over_pixels(cw, ch, cw, px, 0);
    cx.set_alpha(alpha.clamp(0.0, 1.0));
    let paint = color_of(color);
    if pts.iter().all(|&p| p == (x0, y0)) {
        let r = width / 2.0;
        cx.set_fill_color(paint);
        cx.fill_ellipse_in_rect(CgRect::new(x0 - r, y0 - r, width, width));
        return;
    }
    cx.set_stroke_color(paint);
    cx.set_line_width(width);
    cx.set_line_cap(LineCap::Round);
    cx.set_line_join(LineJoin::Round);
    if let Some(d) = dash {
        cx.set_line_dash(0.0, &d);
    }
    let mut path = CgPath::new();
    path.move_to((x0, y0));
    for &(x, y) in &pts[1..] {
        path.line_to((x, y));
    }
    cx.stroke(&path);
}

/// The disc centres that paint the segment from `a` to `b` at radius `r`, spaced like the live pen (the last one is
/// `b`). A zero-length segment still gives one centre.
pub fn trail(a: (f32, f32), b: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    let dist = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let steps = (dist / (r * 0.4).max(1.0)).ceil().max(1.0) as usize;
    (1..=steps)
        .map(|i| {
            let f = i as f32 / steps as f32;
            (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
        })
        .collect()
}

/// Paints a whole stroke into `px`. `scale` converts the ink width (points) to canvas pixels. Marker ink is
/// translucent; eraser strokes are paper-coloured pen strokes, so they paint like the live eraser.
pub fn paint_stroke(px: &mut [u32], cw: usize, ch: usize, s: &Stroke, scale: f32) {
    let alpha = if s.ink.kind == InkKind::Marker { MARKER_ALPHA } else { 1.0 };
    let pts: Vec<(f32, f32)> = s.points.iter().map(|p| (p.x, p.y)).collect();
    stroke_polyline(px, cw, ch, &pts, s.ink.width * scale, None, s.ink.color, alpha);
}

/// The box between two drag points, whichever corners they are.
pub fn rect_between(a: (f32, f32), b: (f32, f32)) -> Rect {
    Rect { x: a.0.min(b.0), y: a.1.min(b.1), w: (a.0 - b.0).abs(), h: (a.1 - b.1).abs() }
}

/// The outline of a rectangle or an oval dragged from `a` to `b`, as one closed stroke per subpath of the path
/// (the path import of the model). Ovals are cubic curves flattened within half a pixel.
pub fn shape_drawing(a: (f32, f32), b: (f32, f32), oval: bool, ink: Ink) -> Drawing {
    let r = rect_between(a, b);
    let mut p = Path::new();
    if oval {
        p.add_ellipse(r.into());
    } else {
        p.add_rect(r.into());
    }
    Drawing::from_path(&p, 0.5, ink)
}

/// The canvas-space transform that moves a selection by (`dx`, `dy`).
pub fn nudge(dx: f32, dy: f32) -> Affine {
    Affine::translate(dx, dy)
}

/// Whole-number factor for a crisp export: how many device pixels the display has per canvas pixel. A display that
/// is no denser than the canvas gives 1; the factor is capped at 4 so a huge export stays bounded.
pub fn export_factor(backing: f32, ui_s: f32) -> u32 {
    if !(backing > 0.0) || !(ui_s > 0.0) {
        return 1;
    }
    (backing / ui_s).round().clamp(1.0, 4.0) as u32
}

/// Margin around a copied drawing, in canvas pixels per point of `scale`: the widest ink reaches 16 points from its
/// centre line.
const COPY_MARGIN_PT: f32 = 16.0;

/// A PNG of the strokes alone, cropped to their box on paper, at `scale` pixels per point. An empty drawing gives a
/// one-pixel paper PNG.
pub fn render_png(drawing: &Drawing, scale: f32) -> Vec<u8> {
    use bifrost_app::controls::cg::path::Affine as T;
    let paper = bifrost_app::kit::ui::WHITE;
    let Some(b) = drawing.bounds() else {
        return springboard::pngenc::encode_rgb(1, 1, &[paper]);
    };
    let pad = COPY_MARGIN_PT * scale;
    let w = (b.w + 2.0 * pad).ceil().max(1.0) as usize;
    let h = (b.h + 2.0 * pad).ceil().max(1.0) as usize;
    let mut moved = drawing.clone();
    moved.transform(T::translate(pad - b.x, pad - b.y));
    let mut px = vec![paper; w * h];
    for s in moved.strokes() {
        paint_stroke(&mut px, w, h, s, scale);
    }
    springboard::pngenc::encode_rgb(w, h, &px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::drawing::StrokePoint;

    const PAPER: u32 = 0xFFFF_FFFF;
    const BLACK: u32 = 0xFF00_0000;

    /// Edge pixels are antialiased, so a painted centre is judged dark, not exactly black.
    fn dark(p: u32) -> bool {
        p & 255 < 128
    }

    fn ink(kind: InkKind, color: u32, width: f32) -> Ink {
        Ink { kind, color, width }
    }

    fn stroke(kind: InkKind, color: u32, width: f32, pts: &[(f32, f32)]) -> Stroke {
        Stroke { ink: ink(kind, color, width), points: pts.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect() }
    }

    #[test]
    fn mix_returns_either_colour_at_the_ends() {
        assert_eq!(mix(0x1122_3344, 0xAABB_CCDD, 0), 0x1122_3344);
        assert_eq!(mix(0x1122_3344, 0xAABB_CCDD, 255), 0xAABB_CCDD);
    }

    #[test]
    fn mix_at_half_is_the_byte_midpoint() {
        assert_eq!(mix(0x0000_0000, 0xFFFF_FFFF, 128), 0x8080_8080);
    }

    #[test]
    fn trail_spacing_ends_on_the_target_point() {
        // radius 1 gives a step of max(0.4, 1) = 1 px, so 10 px is 10 centres.
        let t = trail((0.0, 0.0), (10.0, 0.0), 1.0);
        assert_eq!(t.len(), 10);
        assert_eq!(*t.last().unwrap(), (10.0, 0.0));
        assert_eq!(trail((3.0, 3.0), (3.0, 3.0), 1.0).len(), 1);
    }

    #[test]
    fn a_pen_stroke_paints_along_its_points() {
        let (cw, ch) = (20usize, 5usize);
        let mut px = vec![PAPER; cw * ch];
        let s = stroke(InkKind::Pen, BLACK, 2.0, &[(2.0, 2.0), (17.0, 2.0)]);
        paint_stroke(&mut px, cw, ch, &s, 1.0);
        assert!(dark(px[2 * cw + 2]));
        assert!(dark(px[2 * cw + 10]), "the middle of the segment is painted");
        assert!(dark(px[2 * cw + 17]));
    }

    #[test]
    fn marker_ink_is_translucent_and_eraser_paper_paints_out_black() {
        let (cw, ch) = (6usize, 6usize);
        let mut m = vec![BLACK; cw * ch];
        paint_stroke(&mut m, cw, ch, &stroke(InkKind::Marker, PAPER, 6.0, &[(3.0, 3.0)]), 1.0);
        let centre = m[3 * cw + 3];
        assert_ne!(centre, BLACK, "marker lightens");
        assert_ne!(centre, PAPER, "marker is translucent, not opaque paper");

        let mut e = vec![BLACK; cw * ch];
        paint_stroke(&mut e, cw, ch, &stroke(InkKind::Pen, PAPER, 6.0, &[(3.0, 3.0)]), 1.0);
        assert_eq!(e[3 * cw + 3], PAPER, "an eraser stroke paints paper");
    }

    #[test]
    fn a_dashed_polyline_leaves_gaps_and_a_dot_is_a_disc() {
        let (cw, ch) = (40usize, 5usize);
        let mut px = vec![PAPER; cw * ch];
        stroke_polyline(&mut px, cw, ch, &[(0.0, 2.5), (40.0, 2.5)], 2.0, Some([6.0, 6.0]), BLACK, 1.0);
        assert!(dark(px[2 * cw + 3]), "inside the first dash");
        assert!(!dark(px[2 * cw + 9]), "inside the first gap");
        let mut dot = vec![PAPER; 25];
        stroke_polyline(&mut dot, 5, 5, &[(2.5, 2.5)], 3.0, None, BLACK, 1.0);
        assert!(dark(dot[2 * 5 + 2]));
        assert_eq!(dot[0], PAPER);
    }

    #[test]
    fn an_empty_stroke_paints_nothing() {
        let mut px = vec![PAPER; 4];
        paint_stroke(&mut px, 2, 2, &stroke(InkKind::Pen, BLACK, 2.0, &[]), 1.0);
        assert!(px.iter().all(|&p| p == PAPER));
    }

    #[test]
    fn rect_between_normalises_a_reversed_drag() {
        let r = rect_between((10.0, 8.0), (2.0, 3.0));
        assert_eq!((r.x, r.y, r.w, r.h), (2.0, 3.0, 8.0, 5.0));
    }

    #[test]
    fn a_rectangle_is_one_closed_stroke_with_its_bounds() {
        let d = shape_drawing((2.0, 3.0), (10.0, 8.0), false, ink(InkKind::Pen, BLACK, 2.0));
        assert_eq!(d.strokes().len(), 1);
        assert_eq!(d.strokes()[0].points.len(), 5, "four corners and the closing point");
        let b = d.bounds().unwrap();
        assert_eq!((b.x, b.y, b.w, b.h), (2.0, 3.0, 8.0, 5.0));
    }

    #[test]
    fn an_oval_stays_inside_its_box() {
        let d = shape_drawing((0.0, 0.0), (40.0, 20.0), true, ink(InkKind::Pen, BLACK, 2.0));
        assert_eq!(d.strokes().len(), 1);
        let b = d.bounds().unwrap();
        assert!((b.x - 0.0).abs() < 0.01 && (b.y - 0.0).abs() < 0.01);
        assert!((b.w - 40.0).abs() < 0.05 && (b.h - 20.0).abs() < 0.05, "bounds {b:?}");
    }

    #[test]
    fn nudge_moves_points_by_the_offset() {
        let mut s = stroke(InkKind::Pen, BLACK, 2.0, &[(1.0, 1.0)]);
        s.transform(nudge(4.0, -1.0));
        assert_eq!((s.points[0].x, s.points[0].y), (5.0, 0.0));
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use bifrost_app::controls::drawing::StrokePoint;

    /// (width, height) from the IHDR chunk of a PNG.
    fn png_size(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A], "a PNG signature");
        let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        (w, h)
    }

    #[test]
    fn the_export_factor_follows_the_display_density_in_whole_steps() {
        assert_eq!(export_factor(2.0, 1.0), 2);
        assert_eq!(export_factor(1.0, 1.0), 1);
        assert_eq!(export_factor(1.4, 1.0), 1, "a little denser rounds down to the canvas");
        assert_eq!(export_factor(8.0, 1.0), 4, "capped at 4");
        assert_eq!(export_factor(0.0, 1.0), 1, "no screen known: no change");
        assert_eq!(export_factor(2.0, 0.0), 1);
    }

    #[test]
    fn a_copied_drawing_renders_a_png_cropped_to_its_strokes_with_a_margin() {
        let line = Stroke {
            ink: Ink { kind: InkKind::Pen, color: 0xFF00_0000, width: 2.0 },
            points: vec![StrokePoint::at(100.0, 100.0), StrokePoint::at(140.0, 100.0)],
        };
        let png = render_png(&Drawing::from_strokes(vec![line]), 1.0);
        let (w, h) = png_size(&png);
        assert_eq!(w, (40.0 + 2.0 * COPY_MARGIN_PT).ceil() as u32);
        assert_eq!(h, (2.0 * COPY_MARGIN_PT).ceil() as u32, "a flat line only needs the margins");
    }

    #[test]
    fn an_empty_drawing_still_gives_a_valid_png() {
        assert_eq!(png_size(&render_png(&Drawing::new(), 1.0)), (1, 1));
    }
}

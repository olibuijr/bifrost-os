//! The board as a vector PDF page, for File > Export as PDF and File > Print. Each stroke becomes a path in the
//! page's content stream with its colour, width, dash and fill, so the printout stays sharp at any size. The page is
//! as large as the board in points. A drop shadow is not drawn; opacity is blended towards the paper.
use crate::style::{Fill, Style};
use bifrost_app::controls::drawing::{InkKind, Stroke};
use bifrost_app::print::PdfPage;

/// Dash on and off lengths, in points (the same as the board's dashed outline).
const DASH_PT: [f32; 2] = [6.0, 4.0];
/// Marker ink covers this much of the paper, as on the board.
const MARKER_ALPHA: f32 = 96.0 / 255.0;

/// The three colour components of a packed 0xAARRGGBB colour blended towards white by `alpha`, as PDF operands.
fn rgb(color: u32, alpha: f32) -> String {
    let chan = |shift: u32| {
        let c = ((color >> shift) & 255) as f32 / 255.0;
        1.0 - alpha.clamp(0.0, 1.0) * (1.0 - c)
    };
    format!("{:.3} {:.3} {:.3}", chan(16), chan(8), chan(0))
}

/// One stroke's operators. `scale` converts canvas pixels to points; `height_pt` flips the page's y axis.
fn stroke_ops(out: &mut String, s: &Stroke, st: &Style, scale: f32, height_pt: f32) {
    if s.points.is_empty() {
        return;
    }
    let at = |x: f32, y: f32| format!("{:.2} {:.2}", x / scale, height_pt - y / scale);
    let mut path = String::new();
    for (i, p) in s.points.iter().enumerate() {
        path.push_str(&at(p.x, p.y));
        path.push_str(if i == 0 { " m " } else { " l " });
    }
    // a single point is a dot: a zero-length segment with round caps
    if s.points.len() == 1 {
        path.push_str(&at(s.points[0].x, s.points[0].y));
        path.push_str(" l ");
    }
    let alpha = st.opacity * if s.ink.kind == InkKind::Marker { MARKER_ALPHA } else { 1.0 };
    out.push_str("q 1 J 1 j ");
    out.push_str(&format!("{:.2} w ", s.ink.width));
    if st.dashed {
        out.push_str(&format!("[{} {}] 0 d ", DASH_PT[0], DASH_PT[1]));
    }
    out.push_str(&format!("{} RG ", rgb(s.ink.color, alpha)));
    let fill = if st.shape {
        match st.fill {
            Fill::None => None,
            Fill::Solid(c) | Fill::Gradient(c, _) => Some(c),
        }
    } else {
        None
    };
    match fill {
        Some(c) => out.push_str(&format!("{} rg {path}h B Q\n", rgb(c, st.opacity))),
        None => out.push_str(&format!("{path}S Q\n")),
    }
}

/// A page of `w_px` by `h_px` canvas pixels (`scale` pixels per point) with every stroke drawn in order.
pub fn board_page(strokes: &[Stroke], styles: &[Style], w_px: usize, h_px: usize, scale: f32) -> PdfPage {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (w, h) = (w_px as f32 / scale, h_px as f32 / scale);
    let mut content = String::new();
    for (i, s) in strokes.iter().enumerate() {
        let st = styles.get(i).copied().unwrap_or_default();
        stroke_ops(&mut content, s, &st, scale, h);
    }
    PdfPage { width: w as f64, height: h as f64, content: content.into_bytes(), images: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::drawing::{Ink, StrokePoint};

    fn seg(color: u32, kind: InkKind, pts: &[(f32, f32)]) -> Stroke {
        Stroke { ink: Ink { kind, color, width: 4.0 }, points: pts.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect() }
    }

    fn text(p: &PdfPage) -> String {
        String::from_utf8(p.content.clone()).unwrap()
    }

    #[test]
    fn the_page_is_the_board_size_in_points() {
        let p = board_page(&[], &[], 800, 600, 2.0);
        assert_eq!((p.width, p.height), (400.0, 300.0));
        assert!(p.content.is_empty());
    }

    #[test]
    fn a_stroke_is_a_flipped_path_with_its_colour_and_width() {
        let s = seg(0xFFFF_0000, InkKind::Pen, &[(0.0, 0.0), (20.0, 10.0)]);
        let t = text(&board_page(&[s], &[], 100, 50, 1.0));
        assert!(t.contains("1.000 0.000 0.000 RG"), "{t}");
        assert!(t.contains("4.00 w"));
        assert!(t.contains("0.00 50.00 m 20.00 40.00 l S"), "y is flipped: {t}");
    }

    #[test]
    fn a_dashed_shape_with_a_solid_fill_closes_and_fills() {
        let s = seg(0xFF00_0000, InkKind::Pen, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
        let st = Style { dashed: true, shape: true, fill: Fill::Solid(0xFF00_FF00), ..Style::default() };
        let t = text(&board_page(&[s], &[st], 20, 20, 1.0));
        assert!(t.contains("[6 4] 0 d"));
        assert!(t.contains("0.000 1.000 0.000 rg"));
        assert!(t.contains(" h B Q"));
    }

    #[test]
    fn a_fill_is_ignored_on_a_stroke_that_is_not_a_shape() {
        let s = seg(0xFF00_0000, InkKind::Pen, &[(0.0, 0.0), (10.0, 0.0)]);
        let st = Style { fill: Fill::Solid(0xFFFF_0000), ..Style::default() };
        assert!(!text(&board_page(&[s], &[st], 20, 20, 1.0)).contains(" rg "));
    }

    #[test]
    fn marker_ink_and_opacity_lighten_towards_the_paper() {
        let s = seg(0xFF00_0000, InkKind::Marker, &[(0.0, 0.0), (10.0, 0.0)]);
        let t = text(&board_page(&[s], &[], 20, 20, 1.0));
        assert!(t.contains("0.624 0.624 0.624 RG"), "{t}");
        let s = seg(0xFF00_0000, InkKind::Pen, &[(0.0, 0.0), (10.0, 0.0)]);
        let half = Style { opacity: 0.5, ..Style::default() };
        assert!(text(&board_page(&[s], &[half], 20, 20, 1.0)).contains("0.500 0.500 0.500 RG"));
    }

    #[test]
    fn a_single_point_is_a_dot_and_an_empty_stroke_is_skipped() {
        let dot = seg(0xFF00_0000, InkKind::Pen, &[(5.0, 5.0)]);
        let t = text(&board_page(&[dot, seg(0xFF00_0000, InkKind::Pen, &[])], &[], 20, 20, 1.0));
        assert!(t.contains("5.00 15.00 m 5.00 15.00 l S"), "{t}");
        assert_eq!(t.matches(" Q").count(), 1);
    }

    #[test]
    fn the_page_writes_as_a_pdf_document() {
        let s = seg(0xFF00_0000, InkKind::Pen, &[(0.0, 0.0), (10.0, 0.0)]);
        let page = board_page(&[s], &[], 20, 20, 1.0);
        let op = bifrost_app::print::PrintOperation::pages("Board", vec![page], Vec::new());
        let (format, bytes) = op.document().expect("the pages write");
        assert_eq!(format, "application/pdf");
        assert!(bytes.starts_with(b"%PDF"));
    }
}

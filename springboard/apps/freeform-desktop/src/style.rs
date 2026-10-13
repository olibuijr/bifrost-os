//! The look of each stroke for Freeform's shape inspector: a fill (solid, or a two-colour gradient, for rectangles
//! and ovals), a dashed outline, a drop shadow and opacity. The look lives beside the drawing model, one `Style` per
//! stroke at the same index, so the shared model crate keeps its own shape. Painting builds on the discs and trails of
//! `ink`, and on the models `cg::paint` (the ramp), `layer_style` (the shadow) and `cg::raster` (the fill and the dash pieces).
//! `Bitmap` is the bitmap drawing context a picture is exported through: a pixel buffer that is painted and then
//! written as a PNG.

use crate::ink;
use bifrost_app::controls::drawing::{InkKind, Stroke};
use bifrost_app::kit::cg::paint::{ColorRamp, LinearGradient};
use bifrost_app::controls::layer_style::Shadow;
use bifrost_app::controls::cg::path::{Affine, Path};
use bifrost_app::controls::cg::raster::{self, dash_polyline, FillRule};

/// The opacity steps the inspector offers, as fractions of full opacity.
pub const OPACITY_STEPS: [f32; 3] = [1.0, 0.5, 0.25];
/// Dash on and off lengths, in points.
const DASH_PT: [f32; 2] = [6.0, 4.0];
/// The shadow: its opacity, its offset (points, right and down) and its blur radius (points).
const SHADOW_OPACITY: f32 = 0.35;
const SHADOW_OFFSET_PT: (f32, f32) = (3.0, 4.0);
const SHADOW_BLUR_PT: f32 = 3.0;

/// What fills a closed shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    None,
    Solid(u32),
    /// A linear gradient from the first colour (left edge) to the second (right edge). Colours are packed 0xAARRGGBB.
    Gradient(u32, u32),
}

/// The inspector's settings for one stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fill: Fill,
    pub dashed: bool,
    pub shadow: bool,
    /// Fraction of full opacity, 0..=1.
    pub opacity: f32,
    /// A closed shape drawn by the rectangle or oval tool. Only a shape takes a fill.
    pub shape: bool,
}

impl Default for Style {
    /// A solid, unfilled, opaque, shadowless stroke that is not a shape.
    fn default() -> Self {
        Style { fill: Fill::None, dashed: false, shadow: false, opacity: 1.0, shape: false }
    }
}

/// A packed 0xAARRGGBB colour as the opaque pixel the canvas stores.
fn opaque(c: u32) -> u32 {
    c | 0xFF00_0000
}

/// Straight RGBA bytes of a packed colour, for the gradient ramp.
fn rgba_of(c: u32) -> [u8; 4] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]
}

/// The two-stop ramp of a gradient fill.
fn ramp(a: u32, b: u32) -> ColorRamp {
    ColorRamp::from_rgba8(&[(0.0, rgba_of(a)), (1.0, rgba_of(b))]).expect("two stops")
}

/// The shadow the inspector draws: the model's shadow, with the offset, opacity and blur radius above.
fn shadow_model(blur_pt: f32) -> Shadow {
    Shadow { opacity: SHADOW_OPACITY, radius: blur_pt, offset: SHADOW_OFFSET_PT }
}

/// The offsets (pixels) of the copies a blurred shadow is painted as: the centre, then eight points on a ring of
/// the blur radius. Each copy takes an equal share of the shadow's opacity, so the copies together spread the shadow
/// over the ring and the total stays the same. A radius that is not positive gives the centre alone (a hard shadow).
fn blur_taps(radius_px: f32) -> Vec<(f32, f32)> {
    if !(radius_px > 0.0 && radius_px.is_finite()) {
        return vec![(0.0, 0.0)];
    }
    let mut taps = vec![(0.0, 0.0)];
    for k in 0..8 {
        let angle = k as f32 * std::f32::consts::FRAC_PI_4;
        taps.push((radius_px * angle.cos(), radius_px * angle.sin()));
    }
    taps
}

/// Paints the outline of a polyline: a disc at each point and along each segment, or, when dashed, only along the
/// dash pieces. `width` is in points, `scale` converts points to pixels, `alpha` covers at most that fraction.
#[allow(clippy::too_many_arguments)]
fn outline(px: &mut [u32], cw: usize, ch: usize, pts: &[(f32, f32)], width: f32, dashed: bool, scale: f32, color: u32, alpha: f32) {
    let r = width / 2.0 * scale;
    let dashed = dashed && pts.len() >= 2;
    let pieces: Vec<Vec<(f32, f32)>> = if dashed { dash_polyline(pts, &[DASH_PT[0] * scale, DASH_PT[1] * scale], 0.0) } else { vec![pts.to_vec()] };
    for piece in &pieces {
        let Some(first) = piece.first() else { continue };
        ink::paint_disc(px, cw, ch, first.0, first.1, r, color, alpha);
        for w in piece.windows(2) {
            for (x, y) in ink::trail(w[0], w[1], r) {
                ink::paint_disc(px, cw, ch, x, y, r, color, alpha);
            }
        }
    }
}

/// Fills the polygon through `pts` (closed back to its first point) with the even-odd rule, antialiased at the
/// edges by the shared scan converter. A gradient runs left to right across the polygon's box.
fn fill_polygon(px: &mut [u32], cw: usize, ch: usize, pts: &[(f32, f32)], fill: Fill, alpha: f32) {
    if pts.len() < 3 || cw == 0 || ch == 0 {
        return;
    }
    let (solid, gradient) = match fill {
        Fill::None => return,
        Fill::Solid(c) => (opaque(c), None),
        Fill::Gradient(a, b) => (opaque(a), Some(ramp(a, b))),
    };
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let cy = (y0 + y1) / 2.0;
    let gradient = gradient.map(|r| LinearGradient::new((x0, cy), (x1, cy), r).extended(true, true));
    let a = (255.0 * alpha).round().clamp(0.0, 255.0) as u32;
    let mut path = Path::new();
    path.add_lines(pts);
    path.close();
    let cov = raster::fill(&path, FillRule::EvenOdd, &Affine::IDENTITY, None, cw, ch);
    for row in 0..cov.h {
        let py = cov.y as usize + row;
        for col in 0..cov.w {
            let c = cov.a[row * cov.w + col] as u32;
            if c == 0 {
                continue;
            }
            let px_x = cov.x as usize + col;
            let colour = match &gradient {
                Some(g) => {
                    // a zero-width box has no axis: it takes the start colour
                    let [r, gg, b, _] = g.shade_rgba(px_x as f32 + 0.5, py as f32 + 0.5).unwrap_or_else(|| g.ramp.color_at(0.0));
                    0xFF00_0000 | (r as u32) << 16 | (gg as u32) << 8 | b as u32
                }
                None => solid,
            };
            if let Some(p) = px.get_mut(py * cw + px_x) {
                *p = ink::mix(*p, colour, (a * c + 127) / 255);
            }
        }
    }
}

/// Paints one stroke with its style into `px`, a `cw` x `ch` canvas. `scale` converts points to pixels. Without a
/// style (the default) the result is the same as `ink::paint_stroke`.
pub fn paint(px: &mut [u32], cw: usize, ch: usize, s: &Stroke, st: &Style, scale: f32) {
    paint_with_blur(px, cw, ch, s, st, scale, SHADOW_BLUR_PT);
}

/// `paint`, with the shadow's blur radius (points) given. A radius of zero paints the shadow hard.
fn paint_with_blur(px: &mut [u32], cw: usize, ch: usize, s: &Stroke, st: &Style, scale: f32, blur_pt: f32) {
    let pts: Vec<(f32, f32)> = s.points.iter().map(|p| (p.x, p.y)).collect();
    if pts.is_empty() {
        return;
    }
    let opacity = st.opacity.clamp(0.0, 1.0);
    if st.shadow {
        let sh = shadow_model(blur_pt);
        let (dx, dy) = (sh.offset.0 * scale, sh.offset.1 * scale);
        let taps = blur_taps(sh.radius * scale);
        let each = sh.effective_opacity() * opacity / taps.len() as f32;
        for (ox, oy) in taps {
            let moved: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (x + dx + ox, y + dy + oy)).collect();
            outline(px, cw, ch, &moved, s.ink.width, st.dashed, scale, 0xFF00_0000, each);
        }
    }
    if st.shape {
        fill_polygon(px, cw, ch, &pts, st.fill, opacity);
    }
    let kind_alpha = if s.ink.kind == InkKind::Marker { ink::MARKER_ALPHA } else { 1.0 };
    outline(px, cw, ch, &pts, s.ink.width, st.dashed, scale, opaque(s.ink.color), kind_alpha * opacity);
}

/// Paints every stroke with its style (`styles` may be shorter than `strokes`; a missing style is the default).
pub fn paint_strokes(px: &mut [u32], cw: usize, ch: usize, strokes: &[Stroke], styles: &[Style], scale: f32) {
    for (i, s) in strokes.iter().enumerate() {
        let st = styles.get(i).copied().unwrap_or_default();
        paint(px, cw, ch, s, &st, scale);
    }
}

/// A bitmap drawing context: a `w` x `h` pixel buffer, cleared to `paper`, that strokes are painted into and that is
/// written out as a PNG.
pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Bitmap {
    pub fn new(w: usize, h: usize, paper: u32) -> Self {
        Bitmap { w, h, px: vec![paper; w * h] }
    }

    /// The buffer as an RGB PNG.
    pub fn png(&self) -> Vec<u8> {
        springboard::pngenc::encode_rgb(self.w, self.h, &self.px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::drawing::{Ink, StrokePoint};

    const WHITE: u32 = 0xFFFF_FFFF;
    const BLACK: u32 = 0xFF00_0000;

    #[test]
    fn a_solid_fill_covers_the_rows_inside_the_polygon_only() {
        // a 10 x 10 square in a 20 x 10 canvas: row 5 is painted from x 0 to x 9 (ten pixels)
        let mut px = vec![WHITE; 20 * 10];
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        fill_polygon(&mut px, 20, 10, &sq, Fill::Solid(BLACK), 1.0);
        let row: Vec<u32> = px[5 * 20..6 * 20].to_vec();
        assert_eq!(row.iter().filter(|&&p| p == BLACK).count(), 10);
        assert_eq!(row[10], WHITE, "the pixel just right of the square is untouched");
    }

    #[test]
    fn half_opacity_leaves_a_mid_grey_pixel() {
        // alpha 0.5 gives a = round(127.5) = 128; mix(255, 0, 128) = (255 * 127 + 127) / 255 = 127
        let mut px = vec![WHITE; 20 * 10];
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        fill_polygon(&mut px, 20, 10, &sq, Fill::Solid(BLACK), 0.5);
        assert_eq!(px[5 * 20] & 0xFF, 127);
    }

    #[test]
    fn a_dashed_line_leaves_a_gap_where_the_dash_is_off() {
        // a 2-point-wide line of length 20 at y = 5: dashes are on 0..6 and 10..16, off 6..10 and 16..20
        let stroke = Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 2.0 },
            points: vec![StrokePoint::at(0.0, 5.0), StrokePoint::at(20.0, 5.0)],
        };
        let st = Style { dashed: true, ..Style::default() };
        let mut px = vec![WHITE; 20 * 10];
        paint(&mut px, 20, 10, &stroke, &st, 1.0);
        assert!(px[5 * 20 + 3] & 255 < 128, "inside the first dash is dark");
        assert_eq!(px[5 * 20 + 8], WHITE, "inside the gap stays paper");
    }

    #[test]
    fn a_default_style_paints_a_solid_line() {
        let stroke = Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 2.0 },
            points: vec![StrokePoint::at(0.0, 5.0), StrokePoint::at(20.0, 5.0)],
        };
        let mut px = vec![WHITE; 20 * 10];
        paint(&mut px, 20, 10, &stroke, &Style::default(), 1.0);
        assert!(px[5 * 20 + 8] & 255 < 128, "no gap without the dash");
    }

    #[test]
    fn a_blur_radius_gives_nine_copies_and_none_gives_one() {
        assert_eq!(blur_taps(0.0), vec![(0.0, 0.0)], "no blur is one hard copy");
        let taps = blur_taps(3.0);
        assert_eq!(taps.len(), 9, "the centre plus eight points on the ring");
        assert_eq!(taps[1], (3.0, 0.0), "the first ring point is to the right");
        assert!((taps[3].0).abs() < 1e-4 && (taps[3].1 - 3.0).abs() < 1e-4, "the ring point at 90 degrees is down");
    }

    #[test]
    fn a_blurred_shadow_reaches_past_the_hard_shadow_edge() {
        // The stroke runs along y = 30 from x 5 to 25 (2 pt wide). The hard shadow is offset (3, 4): y = 34.
        // The blur copy at (0, 3) runs along y = 37, so pixel (15, 37) is painted only when it is blurred.
        let stroke = Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 2.0 },
            points: vec![StrokePoint::at(5.0, 30.0), StrokePoint::at(25.0, 30.0)],
        };
        let st = Style { shadow: true, ..Style::default() };
        let (cw, ch) = (40, 50);
        let mut hard = vec![WHITE; cw * ch];
        paint_with_blur(&mut hard, cw, ch, &stroke, &st, 1.0, 0.0);
        assert_eq!(hard[37 * cw + 15], WHITE, "a hard shadow stops at its own edge");
        let mut soft = vec![WHITE; cw * ch];
        paint_with_blur(&mut soft, cw, ch, &stroke, &st, 1.0, 3.0);
        assert_ne!(soft[37 * cw + 15], WHITE, "the blur spreads the shadow to the ring");
        assert!(soft[37 * cw + 15] & 255 > 200, "the spread is faint: one ninth of the shadow's opacity");
    }

    #[test]
    fn the_bitmap_context_writes_a_png_of_its_size() {
        let bmp = Bitmap::new(7, 3, WHITE);
        let png = bmp.png();
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), 7);
        assert_eq!(u32::from_be_bytes([png[20], png[21], png[22], png[23]]), 3);
    }
}

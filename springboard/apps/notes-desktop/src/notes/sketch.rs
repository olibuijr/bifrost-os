//! Sketch attachments for notes: a pen drawing made on the note's sketch pad. A sketch is stored twice beside the
//! note: `<id>-<n>.sketch`, the drawing model's own text form (so the strokes can be read back), and `<id>-<n>.png`,
//! a picture of the strokes that the note shows inline like a photo. The strokes are kept in a `Drawing` (the
//! `bifrost_app::controls::drawing` model); the dab rules and the picture are pure functions here.

use super::folders;
use bifrost_app::controls::drawing::{Drawing, Ink, InkKind, Stroke, StrokePoint};
use springboard::gfx::pack;
use std::io;
use std::path::Path;

/// Ink of a new sketch line (0xAARRGGBB): opaque black.
pub const INK_COLOR: u32 = 0xFF00_0000;
/// Width of a new sketch line, in points.
pub const LINE_PT: f32 = 2.5;
/// Margin around the strokes in the picture (px).
pub const PAD_PX: f32 = 16.0;
/// Longest side of the picture (px). A bigger drawing is scaled down to fit.
pub const MAX_SIDE: f32 = 1600.0;
/// Extension of the drawing's text form, beside its picture.
pub const EXT: &str = "sketch";

/// The sketch pad's state: the finished strokes and the stroke being drawn.
pub struct Pad {
    drawing: Drawing,
    current: Option<Stroke>,
}

impl Default for Pad {
    fn default() -> Self {
        Self::new()
    }
}

impl Pad {
    pub fn new() -> Self {
        Pad { drawing: Drawing::new(), current: None }
    }

    /// Starts a stroke at `(x, y)` with a line `width` px wide (the pad's coordinates).
    pub fn begin(&mut self, x: f32, y: f32, width: f32) {
        let ink = Ink { kind: InkKind::Pen, color: INK_COLOR, width };
        self.current = Some(Stroke { ink, points: vec![StrokePoint::at(x, y)] });
    }

    /// Adds a point to the stroke being drawn; ignored when no stroke is open.
    pub fn extend(&mut self, x: f32, y: f32) {
        if let Some(s) = &mut self.current {
            s.points.push(StrokePoint::at(x, y));
        }
    }

    /// Finishes the stroke being drawn and keeps it. A stroke with no points is dropped.
    pub fn end(&mut self) {
        if let Some(s) = self.current.take() {
            if !s.points.is_empty() {
                self.drawing.push(s);
            }
        }
    }

    /// Removes the newest finished stroke (an open stroke is dropped first). True when a stroke was removed.
    pub fn undo(&mut self) -> bool {
        self.current = None;
        let mut strokes = self.drawing.strokes().to_vec();
        if strokes.pop().is_none() {
            return false;
        }
        self.drawing = Drawing::from_strokes(strokes);
        true
    }

    /// Every stroke on the pad, finished ones first, then the one being drawn.
    pub fn all(&self) -> impl Iterator<Item = &Stroke> {
        self.drawing.strokes().iter().chain(self.current.iter())
    }

    /// True while a stroke is being drawn (touch is down and not yet lifted).
    pub fn is_drawing(&self) -> bool {
        self.current.is_some()
    }

    /// True when nothing has been drawn.
    pub fn is_blank(&self) -> bool {
        self.drawing.is_empty() && self.current.is_none()
    }

    /// The drawing, with an open stroke finished first.
    pub fn into_drawing(mut self) -> Drawing {
        self.end();
        self.drawing
    }
}

/// The centres of the dabs that paint a line from `a` to `b` with a brush `radius` px across: one every half radius
/// (at least one px), so neighbouring dabs overlap and leave no gaps. Both ends are included, so a single point gives
/// one dab.
pub fn dabs(a: (f32, f32), b: (f32, f32), radius: f32) -> Vec<(i32, i32)> {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let step = (radius * 0.5).max(1.0);
    let n = (len / step).ceil().max(0.0) as usize;
    (0..=n)
        .map(|i| {
            let t = if n == 0 { 0.0 } else { i as f32 / n as f32 };
            ((a.0 + (b.0 - a.0) * t).round() as i32, (a.1 + (b.1 - a.1) * t).round() as i32)
        })
        .collect()
}

/// Opaque pixel (RGBA layout of the springboard surfaces) of a packed 0xAARRGGBB colour.
fn rgba(argb: u32) -> u32 {
    pack(((argb >> 16) & 0xff) as u8, ((argb >> 8) & 0xff) as u8, (argb & 0xff) as u8, 255)
}

/// The square dabs that paint stroke `s`: each is `(centre x, centre y, half side, colour)` in picture pixels. `to_px`
/// maps a drawing point to a picture pixel, and `scale` shrinks the line width the same way the picture is scaled.
pub fn stroke_dabs(s: &Stroke, to_px: &dyn Fn(f32, f32) -> (f32, f32), scale: f32) -> Vec<(i32, i32, i32, u32)> {
    let r = ((s.ink.width * scale) * 0.5).round().max(1.0);
    let color = rgba(s.ink.color);
    let pts: Vec<(f32, f32)> = s.points.iter().map(|p| to_px(p.x, p.y)).collect();
    let pairs: Vec<((f32, f32), (f32, f32))> = match pts.len() {
        0 => Vec::new(),
        1 => vec![(pts[0], pts[0])],
        _ => pts.windows(2).map(|w| (w[0], w[1])).collect(),
    };
    pairs.into_iter().flat_map(|(a, b)| dabs(a, b, r).into_iter().map(move |(x, y)| (x, y, r as i32, color))).collect()
}

/// The picture of a drawing: white paper with every stroke, cropped to the strokes plus `PAD_PX` and scaled down so the
/// longest side is at most `MAX_SIDE`. Returns `(width, height, pixels)`, or None for a drawing with no points.
pub fn render(drawing: &Drawing) -> Option<(usize, usize, Vec<u32>)> {
    let b = drawing.bounds()?;
    let (ox, oy) = (b.x - PAD_PX, b.y - PAD_PX);
    let (bw, bh) = (b.w + 2.0 * PAD_PX, b.h + 2.0 * PAD_PX);
    let scale = (MAX_SIDE / bw.max(bh)).min(1.0);
    let w = ((bw * scale).ceil() as usize).max(1);
    let h = ((bh * scale).ceil() as usize).max(1);
    let mut px = vec![rgba(0xFFFF_FFFF); w * h];
    let to_px = |x: f32, y: f32| ((x - ox) * scale, (y - oy) * scale);
    for s in drawing.strokes() {
        for (cx, cy, r, color) in stroke_dabs(s, &to_px, scale) {
            for y in (cy - r)..=(cy + r) {
                for x in (cx - r)..=(cx + r) {
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        px[y as usize * w + x as usize] = color;
                    }
                }
            }
        }
    }
    Some((w, h, px))
}

/// The PNG of a drawing (see `render`); None for a drawing with no points.
pub fn encode(drawing: &Drawing) -> Option<Vec<u8>> {
    render(drawing).map(|(w, h, px)| springboard::pngenc::encode_rgb(w, h, &px))
}

/// Stores a sketch beside note `id` in `dir`: the `.sketch` text form and the PNG, numbered like the other
/// attachments. Returns the PNG's file name, the name the note's image line uses. An empty drawing is refused.
pub fn save(dir: &Path, id: &str, drawing: &Drawing) -> io::Result<String> {
    let png = encode(drawing).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Nothing drawn"))?;
    std::fs::create_dir_all(dir)?;
    let name = folders::next_attachment(dir, id, "png");
    let stem = name.trim_end_matches(".png");
    springboard::shell::fsutil::write_atomic(&dir.join(format!("{stem}.{EXT}")), drawing.to_text())?;
    springboard::shell::fsutil::write_atomic(&dir.join(&name), png)?;
    Ok(name)
}

/// The drawing behind a sketch picture, read from its `.sketch` file. None for a photo or a missing or damaged file.
pub fn load(dir: &Path, png: &str) -> Option<Drawing> {
    let stem = png.strip_suffix(".png")?;
    let text = std::fs::read_to_string(dir.join(format!("{stem}.{EXT}"))).ok()?;
    Drawing::from_text(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes-sketch")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn line(a: (f32, f32), b: (f32, f32)) -> Drawing {
        let mut p = Pad::new();
        p.begin(a.0, a.1, 2.0);
        p.extend(b.0, b.1);
        p.end();
        p.into_drawing()
    }

    #[test]
    fn dabs_overlap_and_include_both_ends() {
        let d = dabs((0.0, 0.0), (10.0, 0.0), 2.0);
        assert_eq!(d.len(), 11, "step is one px for a radius of 2");
        assert_eq!((d[0], d[10]), ((0, 0), (10, 0)));
        assert_eq!(dabs((0.0, 0.0), (10.0, 0.0), 4.0).len(), 6, "step is two px for a radius of 4");
        assert_eq!(dabs((3.0, 4.0), (3.0, 4.0), 2.0), vec![(3, 4)], "a point is one dab");
    }

    #[test]
    fn pad_draws_undoes_and_refuses_blank() {
        let mut p = Pad::new();
        assert!(p.is_blank());
        p.begin(1.0, 1.0, 2.0);
        assert!(!p.is_blank());
        p.extend(5.0, 5.0);
        assert_eq!(p.all().count(), 1, "an open stroke is shown");
        p.end();
        p.begin(9.0, 9.0, 2.0);
        p.end();
        p.begin(20.0, 20.0, 2.0); // left open
        assert!(p.undo(), "undo drops the open stroke, then the newest finished one");
        assert_eq!(p.all().count(), 1, "the first stroke remains");
        assert!(p.undo());
        assert!(!p.undo(), "nothing left to undo");
        assert!(p.is_blank());
        let mut q = Pad::new();
        q.extend(1.0, 1.0);
        assert!(q.is_blank(), "extend without an open stroke is ignored");
    }

    #[test]
    fn a_picture_is_cropped_to_the_strokes_with_a_margin() {
        // a 40 px line: 40 + 2 * 16 = 72 wide, 0 + 2 * 16 = 32 tall (the line is flat)
        let (w, h, px) = render(&line((10.0, 20.0), (50.0, 20.0))).unwrap();
        assert_eq!((w, h), (72, 32));
        assert_eq!(px.len(), 72 * 32);
        assert_eq!(px[0], rgba(0xFFFF_FFFF), "paper is white");
        assert!(px.contains(&rgba(INK_COLOR)), "the stroke is inked");
        assert!(render(&Drawing::new()).is_none());
    }

    #[test]
    fn a_huge_drawing_is_scaled_to_the_longest_side_limit() {
        let (w, h, _) = render(&line((0.0, 0.0), (4000.0, 0.0))).unwrap();
        assert!(w.max(h) <= MAX_SIDE as usize + 1, "longest side {w}x{h}");
        assert!(w > h);
    }

    #[test]
    fn the_picture_is_a_png_that_the_attachment_rules_accept() {
        let png = encode(&line((0.0, 0.0), (30.0, 30.0))).unwrap();
        assert!(super::super::attach::acceptable(&png));
        assert_eq!(super::super::attach::extension(&png), Some("png"));
    }

    #[test]
    fn save_writes_the_picture_and_its_drawing_and_load_reads_the_strokes_back() {
        let dir = scratch("save");
        let d = line((5.0, 5.0), (25.0, 9.0));
        let name = save(&dir, "300", &d).unwrap();
        assert_eq!(name, "300-1.png");
        assert!(dir.join("300-1.png").exists() && dir.join("300-1.sketch").exists());
        let back = load(&dir, &name).expect("the drawing reads back");
        assert_eq!(back.strokes(), d.strokes(), "the text form round-trips the strokes");
        assert_eq!(save(&dir, "300", &d).unwrap(), "300-2.png", "numbered after the first sketch");
        assert_eq!(folders::attachments(&dir, "300").len(), 4, "pictures and drawings both count as attachments");
        assert!(save(&dir, "300", &Drawing::new()).is_err(), "an empty sketch is refused");
        assert!(load(&dir, "300-9.png").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

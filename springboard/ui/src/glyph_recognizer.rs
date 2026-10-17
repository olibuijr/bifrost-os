//! A handwriting recogniser for single-stroke letters and digits. Each character is written in one stroke in a fixed
//! way (the chart is [`template`]): `a` is an inverted V, `l` is a stem with a foot, `o` is a circle drawn
//! counter-clockwise from the top and `0` the same circle drawn clockwise. A stroke is scaled to a unit box (an axis the
//! stroke barely uses stays centred), resampled to 32 points, and compared point by point with every template, so
//! position, size and the width-to-height ratio do not matter but the start, the direction and the shape do.
//!
//! [`GlyphRecognizer`] reads one stroke with [`GlyphRecognizer::recognize_stroke`], and implements
//! `handwriting::Recognizer` so a drawing's lines can be read into text: one stroke is one character, the strokes of a
//! line are read left to right, and a gap wider than most of a character's size is a space. A line with a stroke that
//! matches nothing is left unread, as the trait asks. The templates are an alphabet that works without any trained
//! model; a person's own way of writing a character can be added with [`GlyphRecognizer::add_template`].
//! Multi-stroke writing (a dotted i, a crossed t) is not read.

use crate::drawing::Stroke;
use crate::handwriting::Recognizer;
use crate::stroke_recognizer::resample;

type Pt = (f32, f32);

/// Points a stroke is resampled to before it is compared.
const SAMPLES: usize = 32;
/// An axis whose extent is under this share of the longer axis is not stretched (a straight stem keeps its place).
const THIN: f32 = 0.2;
/// The mean distance, in unit-box lengths, beyond which a stroke matches no template.
pub const MAX_DISTANCE: f32 = 0.25;
/// A stroke whose longer side is under this many drawing units is a dot, not a character.
const MIN_EXTENT: f32 = 1.0;
/// A gap between two strokes wider than this share of the average stroke size is a space.
const SPACE_GAP: f32 = 0.8;

/// One piece of a character's shape, in a box 10 wide and 10 tall with y growing downwards. Arc angles are in degrees,
/// 0 pointing right and 90 up, growing counter-clockwise on the page.
enum Part {
    Line(&'static [Pt]),
    Arc { cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32 },
}

static GLYPHS: &[(char, &[Part])] = &[
    ('a', &[Part::Line(&[(0.0, 10.0), (5.0, 0.0), (10.0, 10.0)])]),
    ('b', &[Part::Line(&[(0.0, 0.0), (0.0, 10.0), (0.0, 5.0)]), Part::Arc { cx: 0.0, cy: 7.5, rx: 5.0, ry: 2.5, from: 90.0, to: -90.0 }]),
    ('c', &[Part::Arc { cx: 5.0, cy: 5.0, rx: 5.0, ry: 5.0, from: 60.0, to: 300.0 }]),
    ('d', &[Part::Line(&[(10.0, 0.0), (10.0, 10.0), (10.0, 5.0)]), Part::Arc { cx: 10.0, cy: 7.5, rx: 5.0, ry: 2.5, from: 90.0, to: 270.0 }]),
    ('e', &[Part::Line(&[(1.0, 5.0), (9.0, 5.0)]), Part::Arc { cx: 5.0, cy: 5.0, rx: 4.0, ry: 5.0, from: 0.0, to: 315.0 }]),
    ('f', &[Part::Line(&[(9.0, 0.0), (1.0, 0.0), (1.0, 10.0)])]),
    ('g', &[Part::Arc { cx: 5.0, cy: 4.0, rx: 5.0, ry: 4.0, from: 0.0, to: 360.0 }, Part::Line(&[(10.0, 4.0), (10.0, 9.0), (4.0, 10.0), (0.0, 8.0)])]),
    ('h', &[Part::Line(&[(0.0, 0.0), (0.0, 10.0), (0.0, 5.0), (2.0, 3.0), (5.0, 3.0), (7.0, 5.0), (7.0, 10.0)])]),
    ('i', &[Part::Line(&[(5.0, 0.0), (5.0, 10.0)])]),
    ('j', &[Part::Line(&[(8.0, 0.0), (8.0, 8.0), (4.0, 10.0), (0.0, 8.0)])]),
    ('k', &[Part::Line(&[(9.0, 0.0), (1.0, 5.0), (9.0, 10.0)])]),
    ('l', &[Part::Line(&[(0.0, 0.0), (0.0, 10.0), (6.0, 10.0)])]),
    ('m', &[Part::Line(&[(0.0, 10.0), (0.0, 0.0), (5.0, 6.0), (10.0, 0.0), (10.0, 10.0)])]),
    ('n', &[Part::Line(&[(0.0, 10.0), (0.0, 0.0), (10.0, 10.0), (10.0, 0.0)])]),
    ('o', &[Part::Arc { cx: 5.0, cy: 5.0, rx: 5.0, ry: 5.0, from: 90.0, to: 450.0 }]),
    ('p', &[Part::Line(&[(0.0, 10.0), (0.0, 0.0)]), Part::Arc { cx: 0.0, cy: 2.5, rx: 5.0, ry: 2.5, from: 90.0, to: -90.0 }]),
    ('q', &[Part::Arc { cx: 5.0, cy: 4.0, rx: 5.0, ry: 4.0, from: 0.0, to: 360.0 }, Part::Line(&[(10.0, 4.0), (10.0, 10.0)])]),
    ('r', &[Part::Line(&[(0.0, 10.0), (0.0, 0.0)]), Part::Arc { cx: 0.0, cy: 1.5, rx: 4.0, ry: 1.5, from: 90.0, to: 0.0 }]),
    ('s', &[Part::Line(&[(9.0, 1.0), (5.0, 0.0), (1.0, 2.0), (5.0, 5.0), (9.0, 8.0), (5.0, 10.0), (1.0, 9.0)])]),
    ('t', &[Part::Line(&[(0.0, 2.0), (10.0, 2.0), (5.0, 2.0), (5.0, 10.0)])]),
    ('u', &[Part::Line(&[(0.0, 0.0), (0.0, 6.0)]), Part::Arc { cx: 5.0, cy: 6.0, rx: 5.0, ry: 4.0, from: 180.0, to: 360.0 }, Part::Line(&[(10.0, 6.0), (10.0, 0.0)])]),
    ('v', &[Part::Line(&[(0.0, 0.0), (5.0, 10.0), (10.0, 0.0)])]),
    ('w', &[Part::Line(&[(0.0, 0.0), (2.5, 10.0), (5.0, 3.0), (7.5, 10.0), (10.0, 0.0)])]),
    ('x', &[Part::Line(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)])]),
    ('y', &[Part::Line(&[(0.0, 0.0), (5.0, 6.0), (10.0, 0.0), (5.0, 6.0), (2.0, 10.0)])]),
    ('z', &[Part::Line(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (10.0, 10.0)])]),
    ('0', &[Part::Arc { cx: 5.0, cy: 5.0, rx: 5.0, ry: 5.0, from: 90.0, to: -270.0 }]),
    ('1', &[Part::Line(&[(2.0, 3.0), (6.0, 0.0), (6.0, 10.0)])]),
    ('2', &[Part::Line(&[(0.0, 4.0), (3.0, 0.0), (7.0, 0.0), (10.0, 4.0), (0.0, 10.0), (10.0, 10.0)])]),
    ('3', &[Part::Line(&[(1.0, 0.0), (9.0, 1.0), (5.0, 4.0), (9.0, 6.0), (8.0, 9.0), (1.0, 10.0)])]),
    ('4', &[Part::Line(&[(8.0, 10.0), (8.0, 0.0), (0.0, 7.0), (10.0, 7.0)])]),
    ('5', &[Part::Line(&[(1.0, 0.0), (1.0, 5.0), (6.0, 4.0), (9.0, 7.0), (7.0, 10.0), (0.0, 9.0)])]),
    ('6', &[Part::Line(&[(8.0, 0.0), (1.0, 6.0), (1.0, 9.0), (5.0, 10.0), (9.0, 9.0), (9.0, 7.0), (5.0, 5.0), (1.0, 7.0)])]),
    ('7', &[Part::Line(&[(0.0, 3.0), (0.0, 0.0), (10.0, 0.0), (4.0, 10.0)])]),
    ('8', &[Part::Line(&[(5.0, 5.0), (1.0, 2.0), (5.0, 0.0), (9.0, 2.0), (5.0, 5.0), (1.0, 8.0), (5.0, 10.0), (9.0, 8.0), (5.0, 5.0)])]),
    ('9', &[Part::Arc { cx: 5.0, cy: 3.0, rx: 4.0, ry: 3.0, from: 0.0, to: -360.0 }, Part::Line(&[(9.0, 3.0), (9.0, 10.0)])]),
];

fn glyph_points(parts: &[Part]) -> Vec<Pt> {
    let mut out = Vec::new();
    for part in parts {
        match part {
            Part::Line(points) => out.extend_from_slice(points),
            Part::Arc { cx, cy, rx, ry, from, to } => {
                let (cx, cy, rx, ry, from, to) = (*cx, *cy, *rx, *ry, *from, *to);
                let steps = (((to - from).abs() / 15.0) as usize).max(4);
                for i in 0..=steps {
                    let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
                    out.push((cx + rx * a.cos(), cy - ry * a.sin()));
                }
            }
        }
    }
    out
}

/// The characters the built-in chart covers: `a` to `z`, then `0` to `9`.
pub fn alphabet() -> impl Iterator<Item = char> {
    GLYPHS.iter().map(|g| g.0)
}

/// The path of the built-in way to write `ch`, in a box 10 wide and 10 tall with y growing downwards. `None` for a
/// character the chart does not have. An app can draw it as a guide.
pub fn template(ch: char) -> Option<Vec<(f32, f32)>> {
    GLYPHS.iter().find(|g| g.0 == ch).map(|g| glyph_points(g.1))
}

/// Scales a stroke to the unit box and resamples it. `None` for fewer than two points, a point that is not finite, or
/// a stroke whose longer side is under `min_extent`.
fn prepare(points: &[Pt], min_extent: f32) -> Option<Vec<Pt>> {
    if points.len() < 2 || points.iter().any(|p| !(p.0.is_finite() && p.1.is_finite())) {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let longest = w.max(h);
    if longest < min_extent.max(f32::EPSILON) {
        return None;
    }
    let unit: Vec<Pt> = points
        .iter()
        .map(|p| (if w >= THIN * longest { (p.0 - x0) / w } else { 0.5 }, if h >= THIN * longest { (p.1 - y0) / h } else { 0.5 }))
        .collect();
    Some(resample(&unit, SAMPLES))
}

/// The mean distance between corresponding points of two prepared strokes.
fn mean_distance(a: &[Pt], b: &[Pt]) -> f32 {
    let n = a.len().min(b.len()).max(1);
    a.iter().zip(b).map(|(p, q)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()).sum::<f32>() / n as f32
}

/// Which characters a recogniser may answer with. A field that takes numbers reads digits only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charset {
    Letters,
    Digits,
    Both,
}

impl Charset {
    pub fn allows(self, ch: char) -> bool {
        match self {
            Charset::Letters => ch.is_ascii_alphabetic(),
            Charset::Digits => ch.is_ascii_digit(),
            Charset::Both => ch.is_ascii_alphanumeric(),
        }
    }
}

/// The character a stroke was read as and how well it fits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphMatch {
    pub glyph: char,
    /// 1.0 for an exact fit, falling to 0.0 at [`MAX_DISTANCE`].
    pub score: f32,
}

/// Reads single-stroke characters against the built-in chart and any templates added.
#[derive(Clone, Debug)]
pub struct GlyphRecognizer {
    templates: Vec<(char, Vec<Pt>)>,
    charset: Charset,
}

impl Default for GlyphRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphRecognizer {
    /// A recogniser with the built-in chart, answering with letters and digits.
    pub fn new() -> Self {
        let templates = GLYPHS.iter().filter_map(|(ch, parts)| prepare(&glyph_points(parts), 0.0).map(|t| (*ch, t))).collect();
        GlyphRecognizer { templates, charset: Charset::Both }
    }

    /// A recogniser with no templates; only what is added with [`GlyphRecognizer::add_template`] is known.
    pub fn empty() -> Self {
        GlyphRecognizer { templates: Vec::new(), charset: Charset::Both }
    }

    /// Limits the answers to one kind of character.
    pub fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = charset;
        self
    }

    /// Adds another way of writing `ch`, such as the person's own. False when the stroke is too short to use.
    pub fn add_template(&mut self, ch: char, points: &[(f32, f32)]) -> bool {
        match prepare(points, MIN_EXTENT) {
            Some(t) => {
                self.templates.push((ch, t));
                true
            }
            None => false,
        }
    }

    /// The number of templates.
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Reads one stroke. `None` for a dot, a stroke of fewer than two points, or a stroke that fits no template within
    /// [`MAX_DISTANCE`].
    pub fn recognize_stroke(&self, points: &[(f32, f32)]) -> Option<GlyphMatch> {
        let stroke = prepare(points, MIN_EXTENT)?;
        let (glyph, distance) = self
            .templates
            .iter()
            .filter(|(ch, _)| self.charset.allows(*ch))
            .map(|(ch, t)| (*ch, mean_distance(&stroke, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        (distance <= MAX_DISTANCE).then(|| GlyphMatch { glyph, score: 1.0 - distance / MAX_DISTANCE })
    }
}

impl Recognizer for GlyphRecognizer {
    fn recognize(&self, strokes: &[&Stroke]) -> Option<String> {
        if strokes.is_empty() {
            return None;
        }
        let mut read = Vec::with_capacity(strokes.len());
        for stroke in strokes {
            let b = stroke.bounds()?;
            let points: Vec<Pt> = stroke.points.iter().map(|p| (p.x, p.y)).collect();
            read.push((b, self.recognize_stroke(&points)?.glyph));
        }
        let average = read.iter().map(|(b, _)| b.w.max(b.h)).sum::<f32>() / read.len() as f32;
        let mut text = String::new();
        let mut right = f32::MIN;
        for (i, (b, glyph)) in read.iter().enumerate() {
            if i > 0 && b.x - right > SPACE_GAP * average {
                text.push(' ');
            }
            text.push(*glyph);
            right = right.max(b.x + b.w);
        }
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Drawing, Ink, InkKind, StrokePoint};
    use crate::handwriting::recognize_lines;

    fn scaled(ch: char, sx: f32, sy: f32, ox: f32, oy: f32) -> Vec<Pt> {
        template(ch).unwrap().into_iter().map(|(x, y)| (x * sx + ox, y * sy + oy)).collect()
    }

    fn stroke_of(points: &[Pt]) -> Stroke {
        Stroke {
            ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 },
            points: points.iter().map(|p| StrokePoint::at(p.0, p.1)).collect(),
        }
    }

    #[test]
    fn the_chart_has_letters_and_digits_once_each() {
        let all: Vec<char> = alphabet().collect();
        assert_eq!(all.len(), 36);
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 36);
        assert!(all.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        assert!(template('?').is_none());
        assert_eq!(GlyphRecognizer::new().len(), 36);
    }

    #[test]
    fn every_character_is_read_at_another_size_and_place() {
        let r = GlyphRecognizer::new();
        for ch in alphabet() {
            let m = r.recognize_stroke(&scaled(ch, 7.0, 7.0, 120.0, 40.0)).unwrap_or_else(|| panic!("{ch} unread"));
            assert_eq!(m.glyph, ch);
            assert!(m.score > 0.9, "{ch}: {}", m.score);
            let wide = r.recognize_stroke(&scaled(ch, 3.0, 5.0, -50.0, 900.0)).unwrap();
            assert_eq!(wide.glyph, ch, "{ch} stretched");
        }
    }

    #[test]
    fn shaky_writing_is_still_read() {
        let r = GlyphRecognizer::new();
        for ch in alphabet() {
            let shaky: Vec<Pt> = template(ch)
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, (x, y))| {
                    let (dx, dy) = (0.3 * (i as f32 * 12.9898).sin(), 0.3 * (i as f32 * 78.233).sin());
                    ((x + dx) * 4.0 + 7.0, (y + dy) * 4.0 + 9.0)
                })
                .collect();
            let m = r.recognize_stroke(&shaky).unwrap_or_else(|| panic!("{ch} unread"));
            assert_eq!(m.glyph, ch, "{ch} shaky");
            assert!(m.score > 0.4, "{ch}: {}", m.score);
        }
    }

    #[test]
    fn no_two_characters_are_drawn_alike() {
        let prepared: Vec<(char, Vec<Pt>)> = alphabet().map(|c| (c, prepare(&template(c).unwrap(), 0.0).unwrap())).collect();
        for (i, (a, pa)) in prepared.iter().enumerate() {
            for (b, pb) in &prepared[i + 1..] {
                assert!(mean_distance(pa, pb) > 0.1, "{a} and {b} are too alike");
            }
        }
    }

    #[test]
    fn direction_tells_o_from_zero_and_a_stem_from_a_hook() {
        let r = GlyphRecognizer::new();
        let mut backwards = scaled('o', 5.0, 5.0, 0.0, 0.0);
        backwards.reverse();
        assert_eq!(r.recognize_stroke(&backwards).map(|m| m.glyph), Some('0'));
        assert_eq!(r.recognize_stroke(&scaled('i', 5.0, 5.0, 0.0, 0.0)).map(|m| m.glyph), Some('i'));
        assert_eq!(r.recognize_stroke(&scaled('j', 5.0, 5.0, 0.0, 0.0)).map(|m| m.glyph), Some('j'));
    }

    #[test]
    fn a_charset_limits_the_answers() {
        let digits = GlyphRecognizer::new().with_charset(Charset::Digits);
        assert_eq!(digits.recognize_stroke(&scaled('7', 6.0, 6.0, 0.0, 0.0)).map(|m| m.glyph), Some('7'));
        assert!(digits.recognize_stroke(&scaled('a', 6.0, 6.0, 0.0, 0.0)).map_or(true, |m| m.glyph.is_ascii_digit()));
        let letters = GlyphRecognizer::new().with_charset(Charset::Letters);
        assert_eq!(letters.recognize_stroke(&scaled('k', 6.0, 6.0, 0.0, 0.0)).map(|m| m.glyph), Some('k'));
        assert!(letters.recognize_stroke(&scaled('4', 6.0, 6.0, 0.0, 0.0)).map_or(true, |m| m.glyph.is_ascii_alphabetic()));
        assert!(Charset::Both.allows('q') && Charset::Both.allows('3') && !Charset::Both.allows('-'));
    }

    #[test]
    fn dots_and_short_strokes_are_not_characters() {
        let r = GlyphRecognizer::new();
        assert_eq!(r.recognize_stroke(&[(3.0, 3.0)]), None);
        assert_eq!(r.recognize_stroke(&[(3.0, 3.0), (3.2, 3.2)]), None);
        assert_eq!(r.recognize_stroke(&[(0.0, 0.0), (f32::NAN, 1.0)]), None);
        assert_eq!(r.recognize_stroke(&[]), None);
    }

    #[test]
    fn a_stroke_far_from_every_template_is_refused() {
        let mut r = GlyphRecognizer::empty();
        assert!(r.is_empty());
        assert_eq!(r.recognize_stroke(&scaled('a', 5.0, 5.0, 0.0, 0.0)), None);
        assert!(r.add_template('a', &scaled('a', 5.0, 5.0, 0.0, 0.0)));
        assert!(!r.add_template('b', &[(1.0, 1.0)]));
        assert_eq!(r.len(), 1);
        // A circle is nothing like an inverted V.
        assert_eq!(r.recognize_stroke(&scaled('o', 5.0, 5.0, 0.0, 0.0)), None);
    }

    #[test]
    fn a_persons_own_template_is_read() {
        let mut r = GlyphRecognizer::empty();
        let mine = [(0.0, 0.0), (10.0, 10.0), (0.0, 20.0), (10.0, 30.0)];
        assert!(r.add_template('s', &mine));
        let again: Vec<Pt> = mine.iter().map(|p| (p.0 * 3.0 + 40.0, p.1 * 2.0 + 5.0)).collect();
        assert_eq!(r.recognize_stroke(&again).map(|m| m.glyph), Some('s'));
    }

    #[test]
    fn a_line_of_strokes_reads_left_to_right_with_spaces_at_wide_gaps() {
        let r = GlyphRecognizer::new();
        let mut d = Drawing::new();
        d.push(stroke_of(&scaled('i', 4.0, 4.0, 34.0, 0.0)));
        d.push(stroke_of(&scaled('h', 4.0, 4.0, 0.0, 0.0)));
        let lines = recognize_lines(&d, &r);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].1, "hi");

        let mut spaced = Drawing::new();
        spaced.push(stroke_of(&scaled('a', 4.0, 4.0, 0.0, 0.0)));
        spaced.push(stroke_of(&scaled('b', 4.0, 4.0, 100.0, 0.0)));
        assert_eq!(recognize_lines(&spaced, &r)[0].1, "a b");
    }

    #[test]
    fn a_line_with_an_unreadable_stroke_is_left_unread() {
        let r = GlyphRecognizer::new();
        let good = stroke_of(&scaled('a', 4.0, 4.0, 0.0, 0.0));
        let dot = stroke_of(&[(60.0, 10.0), (60.1, 10.1)]);
        assert_eq!(r.recognize(&[&good, &dot]), None);
        assert_eq!(r.recognize(&[]), None);
        assert_eq!(r.recognize(&[&good]), Some("a".to_string()));
    }
}

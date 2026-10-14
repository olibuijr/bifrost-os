//! Handwriting input for a drawing: the strokes are grouped into text lines, the lines are put in reading order, and
//! each line is handed to a recogniser. This module does the layout work only. It does not recognise letters: a
//! shell or app supplies a `Recognizer` that turns the strokes of one line into text.
//!
//! Rule: a stroke joins the first line whose vertical band it overlaps by at least half of the smaller of the two
//! heights. Lines read top to bottom, and the strokes of a line read left to right by the left edge of their boxes.
use crate::drawing::{union, Drawing, Stroke};
use crate::Rect;

/// One line of handwriting: the indices of its strokes in drawing order, and the box that holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub strokes: Vec<usize>,
    pub bounds: Rect,
}

/// Turns the strokes of one line into text. An implementation may return `None` when it cannot read the line.
pub trait Recognizer {
    fn recognize(&self, strokes: &[&Stroke]) -> Option<String>;
}

/// Groups the drawing's strokes into lines in reading order. Strokes with no points are left out.
pub fn segment_lines(drawing: &Drawing) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for (index, stroke) in drawing.strokes().iter().enumerate() {
        let Some(b) = stroke.bounds() else {
            continue;
        };
        let home = lines.iter().position(|line| overlaps_band(line.bounds, b));
        match home {
            Some(i) => {
                let line = &mut lines[i];
                line.strokes.push(index);
                line.bounds = union(line.bounds, b);
            }
            None => lines.push(Line { strokes: vec![index], bounds: b }),
        }
    }
    lines.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
    for line in &mut lines {
        line.strokes.sort_by(|&i, &j| {
            let li = drawing.strokes()[i].bounds().map_or(0.0, |r| r.x);
            let lj = drawing.strokes()[j].bounds().map_or(0.0, |r| r.x);
            li.total_cmp(&lj).then(i.cmp(&j))
        });
    }
    lines
}

/// True when the stroke box `b` overlaps the line band `line` by at least half of the smaller height.
fn overlaps_band(line: Rect, b: Rect) -> bool {
    let top = line.y.max(b.y);
    let bottom = (line.y + line.h).min(b.y + b.h);
    let overlap = bottom - top;
    let smaller = line.h.min(b.h);
    overlap > 0.0 && overlap >= 0.5 * smaller
}

/// Segments the drawing and recognises each line. Lines the recogniser cannot read are left out. Each entry is the
/// line's box and its text.
pub fn recognize_lines(drawing: &Drawing, recognizer: &dyn Recognizer) -> Vec<(Rect, String)> {
    segment_lines(drawing)
        .into_iter()
        .filter_map(|line| {
            let strokes: Vec<&Stroke> = line.strokes.iter().map(|&i| &drawing.strokes()[i]).collect();
            recognizer.recognize(&strokes).map(|text| (line.bounds, text))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Ink, InkKind, StrokePoint};

    fn stroke_at(x0: f32, y0: f32, x1: f32, y1: f32) -> Stroke {
        Stroke {
            ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 },
            points: vec![StrokePoint::at(x0, y0), StrokePoint::at(x1, y1)],
        }
    }

    struct CountingRecognizer;

    impl Recognizer for CountingRecognizer {
        fn recognize(&self, strokes: &[&Stroke]) -> Option<String> {
            if strokes.is_empty() {
                None
            } else {
                Some(format!("{} strokes", strokes.len()))
            }
        }
    }

    #[test]
    fn strokes_on_one_band_form_one_line_read_left_to_right() {
        let mut d = Drawing::new();
        d.push(stroke_at(100.0, 10.0, 120.0, 30.0)); // index 0, right
        d.push(stroke_at(10.0, 12.0, 30.0, 28.0)); // index 1, left
        let lines = segment_lines(&d);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].strokes, vec![1, 0]);
        assert_eq!(lines[0].bounds, Rect { x: 10.0, y: 10.0, w: 110.0, h: 20.0 });
    }

    #[test]
    fn separate_bands_become_lines_in_top_to_bottom_order() {
        let mut d = Drawing::new();
        d.push(stroke_at(10.0, 200.0, 40.0, 220.0)); // lower line, drawn first
        d.push(stroke_at(10.0, 10.0, 40.0, 30.0)); // upper line
        let lines = segment_lines(&d);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].strokes, vec![1]);
        assert_eq!(lines[1].strokes, vec![0]);
    }

    #[test]
    fn a_small_overlap_does_not_join_a_line() {
        // Band 0..20 and stroke 18..40 overlap by 2, which is under half of the smaller height (10).
        let mut d = Drawing::new();
        d.push(stroke_at(0.0, 0.0, 10.0, 20.0));
        d.push(stroke_at(20.0, 18.0, 30.0, 40.0));
        assert_eq!(segment_lines(&d).len(), 2);
    }

    #[test]
    fn strokes_without_points_are_left_out() {
        let mut d = Drawing::new();
        d.push(Stroke { ink: Ink { kind: InkKind::Pen, color: 0, width: 1.0 }, points: Vec::new() });
        d.push(stroke_at(0.0, 0.0, 5.0, 5.0));
        let lines = segment_lines(&d);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].strokes, vec![1]);
    }

    #[test]
    fn empty_drawing_has_no_lines() {
        assert!(segment_lines(&Drawing::new()).is_empty());
    }

    #[test]
    fn recognizer_runs_once_per_line_and_skips_unreadable_lines() {
        struct Skip;
        impl Recognizer for Skip {
            fn recognize(&self, _: &[&Stroke]) -> Option<String> {
                None
            }
        }
        let mut d = Drawing::new();
        d.push(stroke_at(0.0, 0.0, 10.0, 10.0));
        d.push(stroke_at(0.0, 50.0, 10.0, 60.0));
        d.push(stroke_at(20.0, 52.0, 30.0, 58.0));
        let out = recognize_lines(&d, &CountingRecognizer);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1, "1 strokes");
        assert_eq!(out[1].1, "2 strokes");
        assert!(recognize_lines(&d, &Skip).is_empty());
    }
}

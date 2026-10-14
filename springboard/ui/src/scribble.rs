//! Scribble on views that are not text fields: a pen stroke written over a view is read as one of three gestures. A
//! tap is a short stroke that stays in place and selects the view under it. A scratch-out is a tight zigzag that
//! deletes the views it crosses. Anything else is writing for the view under the stroke, which the shell hands to a
//! recogniser. This is pure geometry over `drawing::Stroke`. The input device, the rendering and the recogniser
//! belong to the shell and apps.
//!
//! Targets are passed in z-order, bottom first, so the last target that matches is the topmost one.
use crate::drawing::Stroke;
use crate::Rect;

/// A stroke whose longest side is at most this many points is a tap.
pub const TAP_EXTENT: f32 = 12.0;
/// A scratch-out has at least this many direction reversals along one axis.
pub const SCRATCH_MIN_REVERSALS: usize = 3;
/// A scratch-out travels at least this many times its own extent.
pub const SCRATCH_MIN_LENGTH_RATIO: f32 = 3.0;
/// Movement smaller than this, in points, is jitter and does not count as a direction.
pub const JITTER: f32 = 2.0;

/// The gesture a stroke makes over a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Tap,
    ScratchOut,
    Write,
}

/// What a gesture does to the views it touched. Target ids are the ones the caller gave in `resolve`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The topmost view under the tap.
    Tap(u32),
    /// Every view the scratch-out crosses, bottom first.
    Delete(Vec<u32>),
    /// The topmost view under the centre of the writing.
    Write(u32),
    /// The gesture touched no view.
    Ignored,
}

/// Reads a stroke as a gesture. A stroke with no points has no gesture.
pub fn classify(stroke: &Stroke) -> Option<Gesture> {
    let bounds = stroke.bounds()?;
    let extent = bounds.w.max(bounds.h);
    if extent <= TAP_EXTENT {
        return Some(Gesture::Tap);
    }
    let reversals = direction_reversals(stroke);
    let travelled = stroke.length();
    if reversals >= SCRATCH_MIN_REVERSALS && travelled >= SCRATCH_MIN_LENGTH_RATIO * extent {
        return Some(Gesture::ScratchOut);
    }
    Some(Gesture::Write)
}

/// The number of times the stroke turns back on itself along its busier axis. Steps smaller than `JITTER` are
/// ignored, so a shaky hand does not count as a turn.
fn direction_reversals(stroke: &Stroke) -> usize {
    let xs: Vec<f32> = stroke.points.iter().map(|p| p.x).collect();
    let ys: Vec<f32> = stroke.points.iter().map(|p| p.y).collect();
    count_reversals(&xs).max(count_reversals(&ys))
}

fn count_reversals(values: &[f32]) -> usize {
    let mut reversals = 0;
    let mut last_sign = 0i8;
    for pair in values.windows(2) {
        let step = pair[1] - pair[0];
        if step.abs() < JITTER {
            continue;
        }
        let sign = if step > 0.0 { 1 } else { -1 };
        if last_sign != 0 && sign != last_sign {
            reversals += 1;
        }
        last_sign = sign;
    }
    reversals
}

/// Decides what a stroke does, given the views on screen as `(id, rect)` pairs in z-order, bottom first.
pub fn resolve(stroke: &Stroke, targets: &[(u32, Rect)]) -> Action {
    let Some(gesture) = classify(stroke) else {
        return Action::Ignored;
    };
    match gesture {
        Gesture::Tap => {
            let p = stroke.points[0];
            topmost_at(targets, p.x, p.y).map_or(Action::Ignored, Action::Tap)
        }
        Gesture::ScratchOut => {
            let hit: Vec<u32> = targets
                .iter()
                .filter(|(_, rect)| stroke.points.iter().any(|p| rect.contains(p.x, p.y)))
                .map(|(id, _)| *id)
                .collect();
            if hit.is_empty() {
                Action::Ignored
            } else {
                Action::Delete(hit)
            }
        }
        Gesture::Write => {
            let Some(b) = stroke.bounds() else {
                return Action::Ignored;
            };
            topmost_at(targets, b.x + b.w / 2.0, b.y + b.h / 2.0).map_or(Action::Ignored, Action::Write)
        }
    }
}

fn topmost_at(targets: &[(u32, Rect)], x: f32, y: f32) -> Option<u32> {
    targets.iter().rev().find(|(_, rect)| rect.contains(x, y)).map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Ink, InkKind, StrokePoint};

    fn stroke(points: &[(f32, f32)]) -> Stroke {
        Stroke {
            ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 },
            points: points.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect(),
        }
    }

    fn zigzag(amplitude: f32, turns: usize) -> Stroke {
        let mut pts = Vec::new();
        for i in 0..=turns * 2 {
            let x = if i % 2 == 0 { 0.0 } else { amplitude };
            pts.push((x, i as f32 * 5.0));
        }
        stroke(&pts)
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn empty_stroke_has_no_gesture() {
        assert_eq!(classify(&stroke(&[])), None);
    }

    #[test]
    fn short_stroke_is_a_tap() {
        assert_eq!(classify(&stroke(&[(10.0, 10.0), (12.0, 11.0)])), Some(Gesture::Tap));
        assert_eq!(classify(&stroke(&[(10.0, 10.0)])), Some(Gesture::Tap));
    }

    #[test]
    fn straight_line_is_writing() {
        let s = stroke(&[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]);
        assert_eq!(classify(&s), Some(Gesture::Write));
    }

    #[test]
    fn tight_zigzag_is_a_scratch_out() {
        assert_eq!(classify(&zigzag(30.0, 4)), Some(Gesture::ScratchOut));
    }

    #[test]
    fn jitter_is_not_a_turn() {
        // Every step is under JITTER, so nothing reverses even though the sign flips.
        let s = stroke(&[(0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (40.0, 0.0)]);
        assert_eq!(count_reversals(&s.points.iter().map(|p| p.x).collect::<Vec<_>>()), 0);
    }

    #[test]
    fn a_long_zigzag_with_few_turns_is_writing() {
        // Two reversals is below the minimum, even though the stroke is long.
        let s = stroke(&[(0.0, 0.0), (40.0, 0.0), (0.0, 0.0), (40.0, 0.0)]);
        assert_eq!(classify(&s), Some(Gesture::Write));
    }

    #[test]
    fn tap_selects_the_topmost_view_under_it() {
        let targets = [(1, rect(0.0, 0.0, 100.0, 100.0)), (2, rect(20.0, 20.0, 40.0, 40.0))];
        let s = stroke(&[(30.0, 30.0)]);
        assert_eq!(resolve(&s, &targets), Action::Tap(2));
    }

    #[test]
    fn tap_outside_every_view_is_ignored() {
        let targets = [(1, rect(0.0, 0.0, 10.0, 10.0))];
        assert_eq!(resolve(&stroke(&[(50.0, 50.0)]), &targets), Action::Ignored);
    }

    #[test]
    fn scratch_out_deletes_each_view_it_crosses_in_order() {
        let targets = [(7, rect(-5.0, 0.0, 20.0, 60.0)), (8, rect(200.0, 200.0, 10.0, 10.0)), (9, rect(25.0, 0.0, 20.0, 60.0))];
        assert_eq!(resolve(&zigzag(30.0, 4), &targets), Action::Delete(vec![7, 9]));
    }

    #[test]
    fn scratch_out_over_empty_space_is_ignored() {
        let targets = [(1, rect(500.0, 500.0, 10.0, 10.0))];
        assert_eq!(resolve(&zigzag(30.0, 4), &targets), Action::Ignored);
    }

    #[test]
    fn writing_goes_to_the_view_under_its_centre() {
        let targets = [(3, rect(0.0, 0.0, 200.0, 40.0))];
        let s = stroke(&[(10.0, 10.0), (50.0, 12.0), (90.0, 14.0)]);
        assert_eq!(resolve(&s, &targets), Action::Write(3));
    }

    #[test]
    fn empty_targets_ignore_everything() {
        assert_eq!(resolve(&stroke(&[(1.0, 1.0)]), &[]), Action::Ignored);
    }
}

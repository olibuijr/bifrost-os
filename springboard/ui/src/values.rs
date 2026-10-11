//! Value controls as pure models: segmented control, stepper, slider, page control and progress indicator
//! (`NativeSegmentedControl`, `NativeStepper`, `NativeSlider`, `NativePageControl`, `NativeProgressIndicator`). They hold state and map
//! pointer positions to values; drawing is left to the renderer adapter.
use crate::Rect;

/// A row of equal-width segments, one selected (`NativeSegmentedControl` in select-one mode).
#[derive(Clone, Debug, PartialEq)]
pub struct Segmented {
    enabled: Vec<bool>,
    selected: Option<usize>,
    pressed: Option<usize>,
}

impl Segmented {
    pub fn new(segments: usize, selected: Option<usize>) -> Self {
        Segmented { enabled: vec![true; segments], selected: selected.filter(|&i| i < segments), pressed: None }
    }
    pub fn len(&self) -> usize {
        self.enabled.len()
    }
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    pub fn pressed(&self) -> Option<usize> {
        self.pressed
    }
    pub fn set_enabled(&mut self, i: usize, on: bool) {
        if let Some(e) = self.enabled.get_mut(i) {
            *e = on;
        }
    }
    pub fn is_enabled(&self, i: usize) -> bool {
        self.enabled.get(i).copied().unwrap_or(false)
    }
    /// Selects segment `i` if it exists and is enabled; true when the selection changed.
    pub fn select(&mut self, i: usize) -> bool {
        if !self.is_enabled(i) || self.selected == Some(i) {
            return false;
        }
        self.selected = Some(i);
        true
    }
    /// The segment under `(x, y)` in `bounds`.
    pub fn segment_at(&self, bounds: Rect, x: f32, y: f32) -> Option<usize> {
        if self.enabled.is_empty() || !bounds.contains(x, y) {
            return None;
        }
        let i = (((x - bounds.x) / bounds.w) * self.enabled.len() as f32) as usize;
        Some(i.min(self.enabled.len() - 1))
    }
    /// Pointer down: remembers the segment (for the pressed look). Disabled segments do not press.
    pub fn press(&mut self, bounds: Rect, x: f32, y: f32) {
        self.pressed = self.segment_at(bounds, x, y).filter(|&i| self.is_enabled(i));
    }
    /// Pointer up: selects the pressed segment only if the pointer is still over it. True when the selection changed.
    pub fn release(&mut self, bounds: Rect, x: f32, y: f32) -> bool {
        let down = self.pressed.take();
        match (down, self.segment_at(bounds, x, y)) {
            (Some(d), Some(u)) if d == u => self.select(d),
            _ => false,
        }
    }
    /// Arrow keys: moves the selection by `delta` to the next enabled segment, without wrapping.
    pub fn step(&mut self, delta: isize) -> bool {
        let Some(cur) = self.selected else { return false };
        let mut i = cur as isize + delta;
        while i >= 0 && (i as usize) < self.enabled.len() {
            if self.enabled[i as usize] {
                return self.select(i as usize);
            }
            i += delta.signum();
        }
        false
    }
}

/// Which half of a stepper was hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepperPart {
    Decrement,
    Increment,
}

/// A numeric stepper (`NativeStepper`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stepper {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// Wraps from max to min and back (`valueWraps`).
    pub wraps: bool,
}

impl Stepper {
    pub fn new(value: f32, min: f32, max: f32, step: f32) -> Self {
        Stepper { value: value.clamp(min, max), min, max, step: step.abs().max(f32::EPSILON), wraps: false }
    }
    /// Moves by `dir` steps; true when the value changed.
    pub fn nudge(&mut self, dir: i32) -> bool {
        let raw = self.value + self.step * dir as f32;
        let new = if raw > self.max {
            if self.wraps { self.min } else { self.max }
        } else if raw < self.min {
            if self.wraps { self.max } else { self.min }
        } else {
            raw
        };
        let changed = new != self.value;
        self.value = new;
        changed
    }
    /// A stepper is a stacked pair: the upper half increments, the lower decrements.
    pub fn part_at(bounds: Rect, x: f32, y: f32) -> Option<StepperPart> {
        bounds.contains(x, y).then(|| if y < bounds.y + bounds.h / 2.0 { StepperPart::Increment } else { StepperPart::Decrement })
    }
    pub fn press(&mut self, part: StepperPart) -> bool {
        self.nudge(if part == StepperPart::Increment { 1 } else { -1 })
    }
}

/// A horizontal slider (`NativeSlider`). With `step` the value snaps to `min + k * step`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slider {
    pub min: f32,
    pub max: f32,
    value: f32,
    pub step: Option<f32>,
}

impl Slider {
    pub fn new(min: f32, max: f32, value: f32, step: Option<f32>) -> Self {
        let mut s = Slider { min, max: max.max(min), value: min, step: step.filter(|s| *s > 0.0) };
        s.set(value);
        s
    }
    pub fn value(&self) -> f32 {
        self.value
    }
    /// Sets the value, clamped and snapped; true when it changed.
    pub fn set(&mut self, v: f32) -> bool {
        let mut v = v.clamp(self.min, self.max);
        if let Some(step) = self.step {
            v = (self.min + ((v - self.min) / step).round() * step).min(self.max);
        }
        let changed = v != self.value;
        self.value = v;
        changed
    }
    /// 0..=1 position along the track.
    pub fn fraction(&self) -> f32 {
        if self.max > self.min { (self.value - self.min) / (self.max - self.min) } else { 0.0 }
    }
    /// The value for a pointer at `x` on `track` (what dragging the knob sets).
    pub fn value_at(&self, track: Rect, x: f32) -> f32 {
        let f = if track.w > 0.0 { ((x - track.x) / track.w).clamp(0.0, 1.0) } else { 0.0 };
        self.min + f * (self.max - self.min)
    }
    /// Pointer drag: set the value under `x`.
    pub fn drag(&mut self, track: Rect, x: f32) -> bool {
        self.set(self.value_at(track, x))
    }
    /// Arrow keys move one step (a tenth of the range when no step is set).
    pub fn nudge(&mut self, dir: i32) -> bool {
        let step = self.step.unwrap_or((self.max - self.min) / 10.0);
        self.set(self.value + step * dir as f32)
    }
    /// x of the knob centre on `track`.
    pub fn knob_x(&self, track: Rect) -> f32 {
        track.x + track.w * self.fraction()
    }
}

/// Page dots (`NativePageControl`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageControl {
    pub pages: usize,
    current: usize,
}

impl PageControl {
    pub fn new(pages: usize, current: usize) -> Self {
        PageControl { pages, current: current.min(pages.saturating_sub(1)) }
    }
    pub fn current(&self) -> usize {
        self.current
    }
    pub fn set_current(&mut self, i: usize) -> bool {
        let i = i.min(self.pages.saturating_sub(1));
        let changed = i != self.current;
        self.current = i;
        changed
    }
    pub fn step(&mut self, delta: isize) -> bool {
        self.set_current((self.current as isize + delta).max(0) as usize)
    }
    /// One `dot` x `dot` rectangle per page, `gap` apart, centred in `bounds`.
    pub fn dots(&self, bounds: Rect, dot: f32, gap: f32) -> Vec<Rect> {
        let total = self.pages as f32 * dot + self.pages.saturating_sub(1) as f32 * gap;
        let x0 = bounds.x + (bounds.w - total) / 2.0;
        let y = bounds.y + (bounds.h - dot) / 2.0;
        (0..self.pages).map(|i| Rect { x: x0 + i as f32 * (dot + gap), y, w: dot, h: dot }).collect()
    }
    /// The page whose dot, widened by half the gap on each side and to the full control height, is under `(x, y)`.
    pub fn page_at(&self, bounds: Rect, x: f32, y: f32, dot: f32, gap: f32) -> Option<usize> {
        if !bounds.contains(x, y) {
            return None;
        }
        self.dots(bounds, dot, gap).iter().position(|d| x >= d.x - gap / 2.0 && x < d.right() + gap / 2.0)
    }
}

/// A progress bar (`NativeProgressIndicator`): determinate with a fraction, indeterminate without one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    fraction: Option<f32>,
}

impl Progress {
    pub fn determinate(f: f32) -> Self {
        Progress { fraction: Some(f.clamp(0.0, 1.0)) }
    }
    pub fn indeterminate() -> Self {
        Progress { fraction: None }
    }
    pub fn fraction(&self) -> Option<f32> {
        self.fraction
    }
    pub fn set(&mut self, f: f32) {
        self.fraction = Some(f.clamp(0.0, 1.0));
    }
    /// The filled part of `track`; for an indeterminate bar a segment a third of the width sweeping left to right
    /// once every `period` seconds at time `t`.
    pub fn filled(&self, track: Rect, t: f64, period: f64) -> Rect {
        match self.fraction {
            Some(f) => Rect { w: track.w * f, ..track },
            None => {
                let seg = track.w / 3.0;
                let phase = if period > 0.0 { ((t / period).fract() + 1.0).fract() as f32 } else { 0.0 };
                Rect { x: track.x + (track.w + seg) * phase - seg, w: seg, ..track }.intersection(track).unwrap_or(Rect { w: 0.0, ..track })
            }
        }
    }
    /// "42%" for a determinate bar, "" otherwise.
    pub fn label(&self) -> String {
        self.fraction.map(|f| format!("{}%", (f * 100.0).round() as i32)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const B: Rect = Rect { x: 0.0, y: 0.0, w: 300.0, h: 20.0 };

    #[test]
    fn segments_select_on_release_over_the_same_segment_only() {
        let mut s = Segmented::new(3, Some(0));
        assert_eq!(s.segment_at(B, 150.0, 10.0), Some(1));
        assert_eq!(s.segment_at(B, 299.9, 10.0), Some(2));
        assert_eq!(s.segment_at(B, 300.0, 10.0), None);
        s.press(B, 150.0, 10.0);
        assert_eq!(s.pressed(), Some(1));
        assert!(!s.release(B, 250.0, 10.0), "released over another segment: no change");
        assert_eq!(s.selected(), Some(0));
        s.press(B, 150.0, 10.0);
        assert!(s.release(B, 160.0, 10.0));
        assert_eq!((s.selected(), s.pressed()), (Some(1), None));
    }

    #[test]
    fn disabled_segments_cannot_be_pressed_selected_or_stepped_onto() {
        let mut s = Segmented::new(4, Some(0));
        s.set_enabled(1, false);
        s.press(B, 100.0, 10.0);
        assert_eq!(s.pressed(), None);
        assert!(!s.select(1));
        assert!(s.step(1), "skips the disabled segment");
        assert_eq!(s.selected(), Some(2));
        assert!(s.step(-1));
        assert_eq!(s.selected(), Some(0));
        assert!(!s.step(-1), "no wrapping at the ends");
        assert_eq!(Segmented::new(2, Some(9)).selected(), None, "an out-of-range initial selection is dropped");
    }

    #[test]
    fn stepper_clamps_or_wraps() {
        let mut s = Stepper::new(9.0, 0.0, 10.0, 1.0);
        assert!(s.nudge(1));
        assert!(!s.nudge(1), "at max");
        assert_eq!(s.value, 10.0);
        s.wraps = true;
        assert!(s.nudge(1));
        assert_eq!(s.value, 0.0);
        assert!(s.nudge(-1));
        assert_eq!(s.value, 10.0);
        let b = Rect { x: 0.0, y: 0.0, w: 20.0, h: 30.0 };
        assert_eq!(Stepper::part_at(b, 5.0, 5.0), Some(StepperPart::Increment));
        assert_eq!(Stepper::part_at(b, 5.0, 25.0), Some(StepperPart::Decrement));
        assert_eq!(Stepper::part_at(b, 50.0, 5.0), None);
    }

    #[test]
    fn slider_maps_pointer_to_value_and_snaps() {
        let track = Rect { x: 10.0, y: 0.0, w: 200.0, h: 4.0 };
        let mut s = Slider::new(0.0, 100.0, 0.0, Some(10.0));
        assert!(s.drag(track, 10.0 + 200.0 * 0.37));
        assert_eq!(s.value(), 40.0, "37 snaps to the nearest step");
        assert_eq!(s.fraction(), 0.4);
        assert_eq!(s.knob_x(track), 90.0);
        assert!(s.drag(track, 5000.0));
        assert_eq!(s.value(), 100.0, "dragging past the end clamps");
        assert!(s.drag(track, -50.0));
        assert_eq!(s.value(), 0.0);
        assert!(s.nudge(1));
        assert_eq!(s.value(), 10.0);
        let mut free = Slider::new(0.0, 1.0, 0.5, None);
        assert!(free.nudge(1));
        assert!((free.value() - 0.6).abs() < 1e-6);
        assert_eq!(Slider::new(5.0, 5.0, 9.0, None).fraction(), 0.0, "a degenerate range does not divide by zero");
    }

    #[test]
    fn page_control_dots_are_centred_and_hit_testable() {
        let mut p = PageControl::new(5, 9);
        assert_eq!(p.current(), 4, "clamped to the last page");
        let b = Rect { x: 0.0, y: 0.0, w: 100.0, h: 10.0 };
        let dots = p.dots(b, 6.0, 4.0);
        assert_eq!(dots.len(), 5);
        let (l, r) = (dots[0].x - b.x, b.right() - dots[4].right());
        assert!((l - r).abs() < 1e-4, "centred");
        assert_eq!(p.page_at(b, dots[2].x + 1.0, 5.0, 6.0, 4.0), Some(2));
        assert_eq!(p.page_at(b, -1.0, 5.0, 6.0, 4.0), None);
        assert!(p.step(-2));
        assert_eq!(p.current(), 2);
        assert!(!PageControl::new(0, 0).set_current(3));
        assert!(PageControl::new(0, 0).dots(b, 6.0, 4.0).is_empty());
    }

    #[test]
    fn progress_fills_proportionally_and_sweeps_when_indeterminate() {
        let t = Rect { x: 0.0, y: 0.0, w: 90.0, h: 6.0 };
        let mut p = Progress::determinate(2.0);
        assert_eq!(p.fraction(), Some(1.0));
        p.set(0.42);
        assert_eq!(p.filled(t, 0.0, 1.0).w, 90.0 * 0.42);
        assert_eq!(p.label(), "42%");
        let i = Progress::indeterminate();
        assert_eq!(i.label(), "");
        for k in 0..20 {
            let seg = i.filled(t, k as f64 * 0.05, 1.0);
            assert!(seg.x >= t.x && seg.right() <= t.right() + 1e-3, "stays on the track at {k}: {seg:?}");
        }
        assert!(i.filled(t, 0.5, 1.0).w > 0.0);
        assert_eq!(i.filled(t, 0.5, 0.0).w.is_nan(), false, "a zero period does not produce NaN");
    }
}

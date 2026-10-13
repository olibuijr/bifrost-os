//! Screen-edge pan: a pan that must start at an edge of the screen and move away from it.

use super::pan::Pan;
use super::*;
use std::ops::{BitOr, Deref, DerefMut};

/// A screen edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn mask(self) -> EdgeMask {
        match self {
            Edge::Left => EdgeMask::LEFT,
            Edge::Right => EdgeMask::RIGHT,
            Edge::Top => EdgeMask::TOP,
            Edge::Bottom => EdgeMask::BOTTOM,
        }
    }

    /// Splits a translation into the distance moved away from this edge and the absolute distance moved along it.
    pub fn split(self, tx: f32, ty: f32) -> (f32, f32) {
        match self {
            Edge::Left => (tx, ty.abs()),
            Edge::Right => (-tx, ty.abs()),
            Edge::Top => (ty, tx.abs()),
            Edge::Bottom => (-ty, tx.abs()),
        }
    }
}

/// A set of screen edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeMask(u8);

impl EdgeMask {
    pub const NONE: EdgeMask = EdgeMask(0);
    pub const LEFT: EdgeMask = EdgeMask(1);
    pub const RIGHT: EdgeMask = EdgeMask(2);
    pub const TOP: EdgeMask = EdgeMask(4);
    pub const BOTTOM: EdgeMask = EdgeMask(8);
    pub const ALL: EdgeMask = EdgeMask(15);

    pub fn contains(self, edge: Edge) -> bool {
        self.0 & edge.mask().0 != 0
    }
}

impl BitOr for EdgeMask {
    type Output = EdgeMask;
    fn bitor(self, rhs: EdgeMask) -> EdgeMask {
        EdgeMask(self.0 | rhs.0)
    }
}

/// Where a touch must start for an edge pan, and which edge it did start at.
#[derive(Clone, Copy, Debug)]
pub(super) struct EdgeRule {
    pub edges: EdgeMask,
    pub screen: (f32, f32),
    pub margin: f32,
    pub hit: Option<Edge>,
}

impl EdgeRule {
    /// The allowed edge nearest to `p` that `p` lies within `margin` of.
    pub fn edge_at(&self, p: (f32, f32)) -> Option<Edge> {
        let (w, h) = self.screen;
        [(Edge::Left, p.0), (Edge::Right, w - p.0), (Edge::Top, p.1), (Edge::Bottom, h - p.1)]
            .into_iter()
            .filter(|(e, d)| self.edges.contains(*e) && *d <= self.margin)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(e, _)| e)
    }
}

/// Continuous: a one finger pan that starts within `margin` points of an allowed screen edge and begins once the
/// finger moved `slop` points away from it, mostly perpendicular to the edge. Starting elsewhere, moving along the
/// edge or back towards it fails the gesture. All of [`Pan`]'s accessors are available.
#[derive(Debug)]
pub struct ScreenEdgePan {
    pan: Pan,
}

impl ScreenEdgePan {
    /// `screen` is the size of the surface the touch positions are relative to.
    pub fn new(edges: EdgeMask, screen: (f32, f32)) -> Self {
        let mut pan = Pan::new().with_touches(1, 1);
        pan.edge = Some(EdgeRule { edges, screen, margin: 20.0, hit: None });
        ScreenEdgePan { pan }
    }

    /// Sets how close to an edge a touch must start (points).
    pub fn with_margin(mut self, margin: f32) -> Self {
        if let Some(rule) = &mut self.pan.edge {
            rule.margin = margin;
        }
        self
    }

    /// Updates the surface size, for example after a rotation.
    pub fn set_screen(&mut self, screen: (f32, f32)) {
        if let Some(rule) = &mut self.pan.edge {
            rule.screen = screen;
        }
    }

    /// The edge the current touch started at.
    pub fn edge(&self) -> Option<Edge> {
        self.pan.edge_hit()
    }
}

impl Deref for ScreenEdgePan {
    type Target = Pan;
    fn deref(&self) -> &Pan {
        &self.pan
    }
}

impl DerefMut for ScreenEdgePan {
    fn deref_mut(&mut self) -> &mut Pan {
        &mut self.pan
    }
}

impl Recognizer for ScreenEdgePan {
    fn core(&self) -> &Core {
        &self.pan.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.pan.core
    }
    fn feed(&mut self, s: &TouchSample) {
        self.pan.feed(s);
    }
    fn tick(&mut self, t: f64) {
        self.pan.tick(t);
    }
    fn deadline(&self) -> Option<f64> {
        self.pan.deadline()
    }
    fn reset(&mut self) {
        self.pan.reset();
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (f32, f32) = (800.0, 600.0);

    fn swipe_in(r: &mut ScreenEdgePan, from: (f32, f32), to: (f32, f32)) {
        r.feed(&TouchSample::began(1, from.0, from.1, 0.0));
        r.feed(&TouchSample::moved(1, to.0, to.1, 0.05));
    }

    #[test]
    fn begins_from_the_left_edge_moving_inward() {
        let mut r = ScreenEdgePan::new(EdgeMask::LEFT, SCREEN);
        swipe_in(&mut r, (5.0, 300.0), (40.0, 305.0));
        assert_eq!(r.state(), GestureState::Began);
        assert_eq!(r.edge(), Some(Edge::Left));
        assert_eq!(r.translation(), (35.0, 5.0));
        r.feed(&TouchSample::moved(1, 90.0, 310.0, 0.1));
        assert_eq!(r.state(), GestureState::Changed);
        r.feed(&TouchSample::ended(1, 90.0, 310.0, 0.12));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn works_from_every_edge() {
        let mut right = ScreenEdgePan::new(EdgeMask::RIGHT, SCREEN);
        swipe_in(&mut right, (795.0, 300.0), (760.0, 300.0));
        assert_eq!(right.state(), GestureState::Began);
        assert_eq!(right.edge(), Some(Edge::Right));

        let mut top = ScreenEdgePan::new(EdgeMask::TOP, SCREEN);
        swipe_in(&mut top, (400.0, 3.0), (400.0, 40.0));
        assert_eq!(top.state(), GestureState::Began);

        let mut bottom = ScreenEdgePan::new(EdgeMask::BOTTOM, SCREEN);
        swipe_in(&mut bottom, (400.0, 598.0), (400.0, 560.0));
        assert_eq!(bottom.state(), GestureState::Began);
        assert_eq!(bottom.edge(), Some(Edge::Bottom));
    }

    #[test]
    fn starting_away_from_the_edge_fails_at_once() {
        let mut r = ScreenEdgePan::new(EdgeMask::LEFT, SCREEN);
        r.feed(&TouchSample::began(1, 100.0, 300.0, 0.0));
        assert_eq!(r.state(), GestureState::Failed);
        r.feed(&TouchSample::moved(1, 200.0, 300.0, 0.05));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn a_disallowed_edge_does_not_count() {
        let mut r = ScreenEdgePan::new(EdgeMask::RIGHT, SCREEN);
        r.feed(&TouchSample::began(1, 5.0, 300.0, 0.0));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn moving_along_the_edge_or_back_towards_it_fails() {
        let mut along = ScreenEdgePan::new(EdgeMask::LEFT, SCREEN);
        swipe_in(&mut along, (5.0, 300.0), (6.0, 340.0));
        assert_eq!(along.state(), GestureState::Failed);

        let mut back = ScreenEdgePan::new(EdgeMask::LEFT, SCREEN);
        swipe_in(&mut back, (15.0, 300.0), (2.0, 300.0));
        assert_eq!(back.state(), GestureState::Failed);
    }

    #[test]
    fn a_diagonal_start_waits_until_it_is_mostly_inward() {
        let mut r = ScreenEdgePan::new(EdgeMask::LEFT, SCREEN);
        r.feed(&TouchSample::began(1, 5.0, 300.0, 0.0));
        r.feed(&TouchSample::moved(1, 12.0, 306.0, 0.02));
        assert_eq!(r.state(), GestureState::Possible, "neither direction passed the slop");
        r.feed(&TouchSample::moved(1, 30.0, 312.0, 0.04));
        assert_eq!(r.state(), GestureState::Began);
    }

    #[test]
    fn margin_and_screen_size_are_adjustable() {
        let mut r = ScreenEdgePan::new(EdgeMask::RIGHT, SCREEN).with_margin(40.0);
        swipe_in(&mut r, (765.0, 300.0), (720.0, 300.0));
        assert_eq!(r.state(), GestureState::Began);

        let mut rotated = ScreenEdgePan::new(EdgeMask::RIGHT, SCREEN);
        rotated.set_screen((600.0, 800.0));
        swipe_in(&mut rotated, (595.0, 300.0), (550.0, 300.0));
        assert_eq!(rotated.state(), GestureState::Began);
    }

    #[test]
    fn a_corner_picks_the_nearest_allowed_edge() {
        let rule = EdgeRule { edges: EdgeMask::LEFT | EdgeMask::TOP, screen: SCREEN, margin: 20.0, hit: None };
        assert_eq!(rule.edge_at((4.0, 10.0)), Some(Edge::Left));
        assert_eq!(rule.edge_at((10.0, 4.0)), Some(Edge::Top));
        assert_eq!(rule.edge_at((400.0, 300.0)), None);
        assert_eq!(rule.edge_at((790.0, 590.0)), None, "right and bottom are not in the mask");
        let all = EdgeRule { edges: EdgeMask::ALL, ..rule };
        assert_eq!(all.edge_at((790.0, 400.0)), Some(Edge::Right));
    }

    #[test]
    fn a_second_finger_fails_it_and_the_next_touch_starts_afresh() {
        let mut r = ScreenEdgePan::new(EdgeMask::LEFT, SCREEN);
        r.feed(&TouchSample::began(1, 5.0, 300.0, 0.0));
        r.feed(&TouchSample::began(2, 300.0, 300.0, 0.01));
        assert_eq!(r.state(), GestureState::Failed);
        r.feed(&TouchSample::ended(1, 5.0, 300.0, 0.02));
        r.feed(&TouchSample::ended(2, 300.0, 300.0, 0.02));
        swipe_in(&mut r, (4.0, 100.0), (50.0, 100.0));
        assert_eq!(r.state(), GestureState::Began);
    }

    #[test]
    fn a_pointer_can_drive_it() {
        let mut p = PointerAdapter::new();
        let mut r = ScreenEdgePan::new(EdgeMask::TOP, SCREEN);
        r.feed(&p.press(100.0, 0.0, 0.0));
        r.feed(&p.motion(100.0, 30.0, 0.05).unwrap());
        assert_eq!(r.state(), GestureState::Began);
        assert!(r.velocity().1 > 0.0);
    }
}

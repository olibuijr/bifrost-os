//! Scroll position (`ScrollPosition`, `scrollPosition(_:anchor:)`, `scrollTargetLayout`): the semantic place a
//! vertical scroll view is scrolled to, kept in app state. Pure model; the SDK `ScrollView` applies it and reports
//! user scrolling back into it.
//!
//! A position is one of: automatic (nothing chosen yet), an edge, a point (an offset) or a view identity with an
//! optional anchor. Scroll targets are the identified items of the document (`ScrollTarget`: id, top, height in
//! document points), what the reference calls the views of the scroll target layout.
//!
//! Behaviour, following the reference documentation:
//! - Scrolling to an identity without an anchor scrolls the minimal amount that shows the target; with an anchor the
//!   target's edge or middle meets the same place of the viewport (`scroll::offset_for_rect`).
//! - When the user scrolls, the position is marked as positioned by the user, the edge and point are cleared, and the
//!   identity becomes the top-most target in the visible region (the bottom-most when the anchor is `Trailing`).
//! - Stability when the content changes (`stable_offset`): an edge stays at its edge (a bottom-anchored log keeps
//!   showing its end as it grows), a point is left alone, an identity the app chose stays visible, and an identity
//!   the user scrolled to keeps the same distance from the viewport's top.
use crate::scroll::{offset_for_rect, Anchor};

/// An edge of a vertical scroll view's content (`Edge.top`, `Edge.bottom`). Horizontal edges are not modelled: the SDK
/// scroll view scrolls vertically only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

/// An identified item of the scrolled document, in document points.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollTarget {
    pub id: String,
    pub top: f32,
    pub height: f32,
}

impl ScrollTarget {
    pub fn new(id: &str, top: f32, height: f32) -> Self {
        ScrollTarget { id: id.into(), top, height }
    }
    /// Targets for rows of one height `row_h`, `spacing` apart, the first at `top` (a list or a lazy stack).
    pub fn rows<S: AsRef<str>>(ids: &[S], top: f32, row_h: f32, spacing: f32) -> Vec<ScrollTarget> {
        ids.iter().enumerate().map(|(i, id)| ScrollTarget::new(id.as_ref(), top + i as f32 * (row_h + spacing), row_h)).collect()
    }
    fn bottom(&self) -> f32 {
        self.top + self.height
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Place {
    Automatic,
    Edge(Edge),
    Point(f32),
    /// `delta`: for a position the user scrolled to, the viewport top's distance below the target's top
    Id { id: String, anchor: Option<Anchor>, delta: Option<f32> },
}

/// Where a scroll view is scrolled, as app state (`ScrollPosition`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollPosition {
    place: Place,
    by_user: bool,
    /// the anchor that chooses which visible target the user's scrolling reports
    anchor: Option<Anchor>,
}

impl Default for ScrollPosition {
    fn default() -> Self {
        ScrollPosition::automatic()
    }
}

fn max_offset(viewport_h: f32, content_h: f32) -> f32 {
    (content_h - viewport_h).max(0.0)
}

fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

impl ScrollPosition {
    /// `init(idType:)`: nothing chosen; the scroll view keeps its own offset.
    pub fn automatic() -> Self {
        ScrollPosition { place: Place::Automatic, by_user: false, anchor: None }
    }
    /// `init(id:anchor:)`.
    pub fn id(id: &str, anchor: Option<Anchor>) -> Self {
        ScrollPosition { place: Place::Id { id: id.into(), anchor, delta: None }, by_user: false, anchor }
    }
    /// `init(idType:edge:)`.
    pub fn edge(edge: Edge) -> Self {
        ScrollPosition { place: Place::Edge(edge), by_user: false, anchor: None }
    }
    /// `init(idType:y:)` (and `point:` with x ignored).
    pub fn y(y: f32) -> Self {
        ScrollPosition { place: Place::Point(finite_or_zero(y)), by_user: false, anchor: None }
    }

    /// `edge`: the edge when the position is one.
    pub fn edge_value(&self) -> Option<Edge> {
        match self.place {
            Place::Edge(e) => Some(e),
            _ => None,
        }
    }
    /// `y` / `point`: the offset when the position is a point.
    pub fn y_value(&self) -> Option<f32> {
        match self.place {
            Place::Point(y) => Some(y),
            _ => None,
        }
    }
    /// `viewID(type:)`: the identity the position names, or the one the user scrolled to.
    pub fn view_id(&self) -> Option<&str> {
        match &self.place {
            Place::Id { id, .. } => Some(id),
            _ => None,
        }
    }
    /// `isPositionedByUser`.
    pub fn is_positioned_by_user(&self) -> bool {
        self.by_user
    }

    /// `scrollTo(id:anchor:)`.
    pub fn scroll_to_id(&mut self, id: &str, anchor: Option<Anchor>) {
        *self = ScrollPosition::id(id, anchor);
    }
    /// `scrollTo(edge:)`.
    pub fn scroll_to_edge(&mut self, edge: Edge) {
        let anchor = self.anchor;
        *self = ScrollPosition { anchor, ..ScrollPosition::edge(edge) };
    }
    /// `scrollTo(y:)` / `scrollTo(point:)`.
    pub fn scroll_to_y(&mut self, y: f32) {
        let anchor = self.anchor;
        *self = ScrollPosition { anchor, ..ScrollPosition::y(y) };
    }
    /// The `anchor:` of `scrollPosition(_:anchor:)`: which visible target the user's scrolling reports (`Trailing`
    /// picks the bottom-most) and how an identity is placed when it has no anchor of its own.
    pub fn with_anchor(mut self, anchor: Option<Anchor>) -> Self {
        self.anchor = anchor;
        if let Place::Id { anchor: a, .. } = &mut self.place {
            if a.is_none() {
                *a = anchor;
            }
        }
        self
    }

    /// The offset a programmatic scroll to this position goes to, for a viewport `viewport_h` tall now scrolled to
    /// `current`, over a document `content_h` tall with `targets`. `None` when the position names nothing to scroll
    /// to (automatic, or an identity no target has). The result is clamped to `0..=content_h - viewport_h`.
    pub fn resolve(&self, current: f32, viewport_h: f32, content_h: f32, targets: &[ScrollTarget]) -> Option<f32> {
        let max = max_offset(viewport_h, content_h);
        let current = finite_or_zero(current).clamp(0.0, max);
        match &self.place {
            Place::Automatic => None,
            Place::Edge(Edge::Top) => Some(0.0),
            Place::Edge(Edge::Bottom) => Some(max),
            Place::Point(y) => Some(y.clamp(0.0, max)),
            Place::Id { id, anchor, .. } => {
                let t = targets.iter().find(|t| &t.id == id)?;
                Some(match anchor {
                    Some(a) => offset_for_rect(t.top, t.height, viewport_h, content_h, *a),
                    // the minimal scroll: a target above the viewport comes in at the top, one below at the bottom
                    // (a target taller than the viewport shows its top)
                    None if t.top < current => t.top.clamp(0.0, max),
                    None if t.bottom() > current + viewport_h => (t.bottom() - viewport_h).min(t.top).clamp(0.0, max),
                    None => current,
                })
            }
        }
    }

    /// The offset that keeps this position stable after the content or viewport changed (a relayout), or `None` to
    /// leave the scroll view's offset alone (automatic, a point, or an identity that is gone).
    pub fn stable_offset(&self, current: f32, viewport_h: f32, content_h: f32, targets: &[ScrollTarget]) -> Option<f32> {
        match &self.place {
            Place::Point(_) | Place::Automatic => None,
            Place::Id { id, delta: Some(d), .. } => {
                let t = targets.iter().find(|t| &t.id == id)?;
                Some((t.top + d).clamp(0.0, max_offset(viewport_h, content_h)))
            }
            _ => self.resolve(current, viewport_h, content_h, targets),
        }
    }

    /// The user scrolled the view to `offset`: the position becomes the top-most target in the visible region (the
    /// bottom-most with a `Trailing` anchor), positioned by the user. With no target visible it becomes automatic.
    pub fn user_scrolled(&mut self, offset: f32, viewport_h: f32, targets: &[ScrollTarget]) {
        let offset = finite_or_zero(offset).max(0.0);
        let (lo, hi) = (offset, offset + viewport_h.max(0.0));
        let visible = targets.iter().filter(|t| t.bottom() > lo && t.top < hi);
        let pick = if self.anchor == Some(Anchor::Trailing) {
            visible.max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
        } else {
            visible.min_by(|a, b| a.top.total_cmp(&b.top))
        };
        self.by_user = true;
        self.place = match pick {
            Some(t) => Place::Id { id: t.id.clone(), anchor: self.anchor, delta: Some(offset - t.top) },
            None => Place::Automatic,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<ScrollTarget> {
        // ten rows 40 tall, no spacing: a 400 point document
        ScrollTarget::rows(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"], 0.0, 40.0, 0.0)
    }

    #[test]
    fn edges_and_points_resolve_to_clamped_offsets() {
        let t = rows();
        assert_eq!(ScrollPosition::edge(Edge::Top).resolve(120.0, 100.0, 400.0, &t), Some(0.0));
        assert_eq!(ScrollPosition::edge(Edge::Bottom).resolve(0.0, 100.0, 400.0, &t), Some(300.0));
        assert_eq!(ScrollPosition::y(1000.0).resolve(0.0, 100.0, 400.0, &t), Some(300.0));
        assert_eq!(ScrollPosition::y(f32::NAN).y_value(), Some(0.0));
        assert_eq!(ScrollPosition::automatic().resolve(50.0, 100.0, 400.0, &t), None);
    }

    #[test]
    fn an_identity_without_anchor_scrolls_the_minimal_amount() {
        let t = rows();
        let p = ScrollPosition::id("5", None);
        // row 5 is 200..240; from the top it comes in at the bottom of a 100 point viewport
        assert_eq!(p.resolve(0.0, 100.0, 400.0, &t), Some(140.0));
        // from below it comes in at the top
        assert_eq!(p.resolve(260.0, 100.0, 400.0, &t), Some(200.0));
        // already visible: no scroll
        assert_eq!(p.resolve(180.0, 100.0, 400.0, &t), Some(180.0));
        assert_eq!(ScrollPosition::id("missing", None).resolve(0.0, 100.0, 400.0, &t), None);
    }

    #[test]
    fn an_identity_with_an_anchor_meets_that_place_of_the_viewport() {
        let t = rows();
        assert_eq!(ScrollPosition::id("5", Some(Anchor::Leading)).resolve(0.0, 100.0, 400.0, &t), Some(200.0));
        assert_eq!(ScrollPosition::id("5", Some(Anchor::Center)).resolve(0.0, 100.0, 400.0, &t), Some(170.0));
        assert_eq!(ScrollPosition::id("9", Some(Anchor::Leading)).resolve(0.0, 100.0, 400.0, &t), Some(300.0));
    }

    #[test]
    fn user_scrolling_reports_the_top_most_visible_target_and_clears_edges() {
        let t = rows();
        let mut p = ScrollPosition::edge(Edge::Top);
        p.user_scrolled(130.0, 100.0, &t);
        assert!(p.is_positioned_by_user());
        assert_eq!((p.view_id(), p.edge_value(), p.y_value()), (Some("3"), None, None));
        // a bottom anchor reports the bottom-most visible target instead
        let mut p = ScrollPosition::automatic().with_anchor(Some(Anchor::Trailing));
        p.user_scrolled(130.0, 100.0, &t);
        assert_eq!(p.view_id(), Some("5"));
        // nothing visible: automatic
        let mut p = ScrollPosition::automatic();
        p.user_scrolled(0.0, 100.0, &[]);
        assert_eq!(p.view_id(), None);
        assert!(p.is_positioned_by_user());
    }

    #[test]
    fn programmatic_scrolls_clear_the_user_flag() {
        let t = rows();
        let mut p = ScrollPosition::automatic();
        p.user_scrolled(50.0, 100.0, &t);
        p.scroll_to_id("7", Some(Anchor::Leading));
        assert!(!p.is_positioned_by_user());
        assert_eq!(p.view_id(), Some("7"));
        p.scroll_to_edge(Edge::Bottom);
        assert_eq!((p.edge_value(), p.view_id()), (Some(Edge::Bottom), None));
        p.scroll_to_y(12.0);
        assert_eq!(p.y_value(), Some(12.0));
    }

    #[test]
    fn positions_stay_stable_when_the_content_changes() {
        let t = rows();
        // a bottom edge follows a growing document
        assert_eq!(ScrollPosition::edge(Edge::Bottom).stable_offset(300.0, 100.0, 800.0, &t), Some(700.0));
        // a point is left alone
        assert_eq!(ScrollPosition::y(40.0).stable_offset(40.0, 100.0, 800.0, &t), None);
        // the user's row keeps its distance from the viewport top when rows are inserted above it
        let mut p = ScrollPosition::automatic();
        p.user_scrolled(130.0, 100.0, &t);
        assert_eq!(p.view_id(), Some("3"));
        let shifted: Vec<ScrollTarget> = t.iter().map(|r| ScrollTarget::new(&r.id, r.top + 80.0, r.height)).collect();
        assert_eq!(p.stable_offset(130.0, 100.0, 480.0, &shifted), Some(210.0));
        // an identity the app chose stays visible
        assert_eq!(ScrollPosition::id("9", None).stable_offset(0.0, 100.0, 400.0, &t), Some(300.0));
    }
}

//! Reserved regions: parts of a view that another entity occupies, such as the on-screen keyboard or the window controls
//! (the UIKit reserved region API).
//!
//! A view declares its regions in [`ReservedRegions`]. Layout asks for the area it may use with
//! [`ReservedRegions::free_area`], and a pointer or touch event asks which region it lands in with
//! [`ReservedRegions::region_at`]. The on-screen keyboard declares its region while it is raised; on a desktop with a
//! hardware keyboard it is never raised, so its region stays inactive and the window area is not reduced.

use crate::geometry::EdgeInsets;
use crate::Rect;

/// What occupies a reserved region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReservedRegionKind {
    /// The on-screen keyboard. Declared active only while it is raised.
    Keyboard,
    /// The window controls (close, minimise, zoom) at a window corner.
    WindowControls,
    /// Any other system user interface, such as a status or menu bar.
    System,
}

/// A rectangle a view leaves clear for system UI.
#[derive(Clone, Debug, PartialEq)]
pub struct ReservedRegion {
    /// Names the region so a view can replace or remove it later.
    pub identifier: String,
    pub kind: ReservedRegionKind,
    /// The rectangle in the view's coordinate space. It includes `margins`.
    pub frame: Rect,
    /// Space inside `frame` around the occupied area for interactive content, such as a drag handle or a touch slop.
    pub margins: EdgeInsets,
    /// An inactive region is declared but does not occupy space (the keyboard before it is raised).
    pub active: bool,
}

impl ReservedRegion {
    /// An active region with no margins.
    pub fn new(identifier: &str, kind: ReservedRegionKind, frame: Rect) -> Self {
        ReservedRegion {
            identifier: identifier.to_string(),
            kind,
            frame,
            margins: EdgeInsets::ZERO,
            active: true,
        }
    }

    /// The same region with the given margins.
    pub fn with_margins(mut self, margins: EdgeInsets) -> Self {
        self.margins = margins;
        self
    }

    /// The same region, active or inactive.
    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// The area the system occupies: the frame without its margins.
    pub fn occupied(&self) -> Rect {
        self.frame.inset(self.margins)
    }
}

/// Options for [`ReservedRegions::query`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryOptions {
    /// Whether inactive regions are included in the result.
    pub include_inactive: bool,
}

/// The regions a view has declared, in the order they were first declared.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReservedRegions {
    regions: Vec<ReservedRegion>,
}

impl ReservedRegions {
    /// No regions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a region. A region with the same identifier is replaced in place.
    pub fn declare(&mut self, region: ReservedRegion) {
        match self.regions.iter_mut().find(|r| r.identifier == region.identifier) {
            Some(slot) => *slot = region,
            None => self.regions.push(region),
        }
    }

    /// Removes the region with `identifier`. Returns whether one was removed.
    pub fn remove(&mut self, identifier: &str) -> bool {
        let before = self.regions.len();
        self.regions.retain(|r| r.identifier != identifier);
        self.regions.len() != before
    }

    /// Activates or deactivates the region with `identifier`. Returns whether the region exists.
    pub fn set_active(&mut self, identifier: &str, active: bool) -> bool {
        match self.regions.iter_mut().find(|r| r.identifier == identifier) {
            Some(region) => {
                region.active = active;
                true
            }
            None => false,
        }
    }

    /// The declared regions, filtered by `options`, in declaration order.
    pub fn query(&self, options: QueryOptions) -> Vec<&ReservedRegion> {
        self.regions.iter().filter(|r| options.include_inactive || r.active).collect()
    }

    /// The first active region whose frame holds the point `(x, y)`, or `None`. Used to route a pointer or touch
    /// event that lands on system UI.
    pub fn region_at(&self, x: f32, y: f32) -> Option<&ReservedRegion> {
        self.regions.iter().find(|r| r.active && r.frame.contains(x, y))
    }

    /// The largest rectangle inside `bounds` that the active regions do not occupy.
    ///
    /// Regions are applied in declaration order. A region that spans the full width of the remaining area removes the
    /// top or the bottom part, whichever is larger; a region that spans the full height removes the left or the right part.
    /// A region that only crosses a corner or the middle of the area is ignored, because removing it would leave a shape
    /// that is not a rectangle. Views that need that case must lay out their own content around the region.
    pub fn free_area(&self, bounds: Rect) -> Rect {
        self.query(QueryOptions::default())
            .into_iter()
            .fold(bounds, |area, region| cut(area, region.frame))
    }
}

/// Removes `hole` from `area` along the one edge the hole spans. See [`ReservedRegions::free_area`].
fn cut(area: Rect, hole: Rect) -> Rect {
    let Some(overlap) = area.intersection(hole) else {
        return area;
    };
    if overlap.w >= area.w {
        let above = overlap.y - area.y;
        let below = area.bottom() - overlap.bottom();
        if above >= below {
            Rect { h: above, ..area }
        } else {
            Rect { y: overlap.bottom(), h: below, ..area }
        }
    } else if overlap.h >= area.h {
        let left = overlap.x - area.x;
        let right = area.right() - overlap.right();
        if left >= right {
            Rect { w: left, ..area }
        } else {
            Rect { x: overlap.right(), w: right, ..area }
        }
    } else {
        area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 800.0, h: 1000.0 };

    fn keyboard(h: f32) -> ReservedRegion {
        ReservedRegion::new("keyboard", ReservedRegionKind::Keyboard, Rect { x: 0.0, y: 1000.0 - h, w: 800.0, h })
    }

    #[test]
    fn declare_replaces_a_region_with_the_same_identifier() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0));
        regions.declare(keyboard(400.0));
        let all = regions.query(QueryOptions::default());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].frame.h, 400.0);
    }

    #[test]
    fn remove_and_set_active_report_whether_the_region_exists() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0));
        assert!(regions.set_active("keyboard", false));
        assert!(!regions.set_active("missing", false));
        assert!(regions.remove("keyboard"));
        assert!(!regions.remove("keyboard"));
    }

    #[test]
    fn query_hides_inactive_regions_unless_asked() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0).with_active(false));
        assert!(regions.query(QueryOptions::default()).is_empty());
        assert_eq!(regions.query(QueryOptions { include_inactive: true }).len(), 1);
    }

    #[test]
    fn inactive_region_does_not_reduce_the_free_area() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0).with_active(false));
        assert_eq!(regions.free_area(SCREEN), SCREEN);
    }

    #[test]
    fn raised_keyboard_at_the_bottom_leaves_the_top_part() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0));
        assert_eq!(regions.free_area(SCREEN), Rect { x: 0.0, y: 0.0, w: 800.0, h: 700.0 });
    }

    #[test]
    fn a_band_at_the_top_leaves_the_bottom_part() {
        let mut regions = ReservedRegions::new();
        regions.declare(ReservedRegion::new("bar", ReservedRegionKind::System, Rect { x: 0.0, y: 0.0, w: 800.0, h: 100.0 }));
        assert_eq!(regions.free_area(SCREEN), Rect { x: 0.0, y: 100.0, w: 800.0, h: 900.0 });
    }

    #[test]
    fn a_full_height_column_leaves_the_larger_side() {
        let mut regions = ReservedRegions::new();
        regions.declare(ReservedRegion::new("controls", ReservedRegionKind::WindowControls, Rect { x: 0.0, y: 0.0, w: 100.0, h: 1000.0 }));
        assert_eq!(regions.free_area(SCREEN), Rect { x: 100.0, y: 0.0, w: 700.0, h: 1000.0 });
    }

    #[test]
    fn a_partial_region_in_the_middle_is_ignored() {
        let mut regions = ReservedRegions::new();
        regions.declare(ReservedRegion::new("corner", ReservedRegionKind::WindowControls, Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }));
        assert_eq!(regions.free_area(SCREEN), SCREEN);
    }

    #[test]
    fn regions_apply_in_declaration_order() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0));
        regions.declare(ReservedRegion::new("bar", ReservedRegionKind::System, Rect { x: 0.0, y: 0.0, w: 800.0, h: 100.0 }));
        // The keyboard leaves y 0..700; the bar then leaves y 100..700.
        assert_eq!(regions.free_area(SCREEN), Rect { x: 0.0, y: 100.0, w: 800.0, h: 600.0 });
    }

    #[test]
    fn region_at_routes_points_to_active_regions_only() {
        let mut regions = ReservedRegions::new();
        regions.declare(keyboard(300.0));
        assert_eq!(regions.region_at(400.0, 900.0).map(|r| r.kind), Some(ReservedRegionKind::Keyboard));
        assert!(regions.region_at(400.0, 100.0).is_none());
        regions.set_active("keyboard", false);
        assert!(regions.region_at(400.0, 900.0).is_none());
    }

    #[test]
    fn occupied_area_excludes_the_margins() {
        let r = ReservedRegion::new("handle", ReservedRegionKind::System, Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 })
            .with_margins(EdgeInsets::symmetric(8.0, 4.0));
        assert_eq!(r.occupied(), Rect { x: 8.0, y: 4.0, w: 84.0, h: 32.0 });
    }
}

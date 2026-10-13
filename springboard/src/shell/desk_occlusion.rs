//! Built-in windows that a window in front of them hides completely (pure geometry, no drawing).
//!
//! A window is hidden when everything it draws, its shadow reach included, lies inside the opaque interior of a
//! window in front of it. The front window paints every pixel of it, so the shell does not draw it. Windows that
//! cannot cover (external windows, minimizing windows, windows with an effect running) pass `None` and neither hide
//! nor get hidden.

use super::desk::Rect;

/// Shadow reach around a built-in window, pt: `draw_window` draws the key window's shadow 40 pt to the sides and
/// 52 pt below, so 56 pt covers it on every side.
pub(super) const SHADOW_REACH_PT: f32 = 56.0;

/// The rectangles of one window: `shown` is everything it draws, `opaque` is the part that is solid paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cover {
    pub shown: Rect,
    pub opaque: Rect,
}

/// `r` with each side pulled in by `radius` px. A rectangle inset by the corner radius on every side lies inside
/// the rounded shape, so it is solid paint.
pub(super) fn opaque_interior(r: Rect, radius: i32) -> Rect {
    let k = radius.max(0);
    Rect { x: r.x + k, y: r.y + k, w: (r.w - 2 * k).max(0), h: (r.h - 2 * k).max(0) }
}

/// `r` grown by `by` px on every side.
pub(super) fn grown(r: Rect, by: i32) -> Rect {
    Rect { x: r.x - by, y: r.y - by, w: r.w + 2 * by, h: r.h + 2 * by }
}

/// Whether `inner` is non-empty and lies inside `outer`.
fn inside(inner: Rect, outer: Rect) -> bool {
    inner.w > 0
        && inner.h > 0
        && inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.w <= outer.x + outer.w
        && inner.y + inner.h <= outer.y + outer.h
}

/// For each window, back to front, whether a window in front of it covers all of it.
///
/// Testing against every window in front, hidden ones included, gives the same answer as testing only the visible
/// ones: a hidden window's opaque part lies inside its own `shown` area, which lies inside the opaque part of the
/// window that hides it.
pub(super) fn hidden(windows: &[Option<Cover>]) -> Vec<bool> {
    (0..windows.len())
        .map(|i| match windows[i] {
            Some(c) => windows[i + 1..].iter().flatten().any(|front| inside(c.shown, front.opaque)),
            None => false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn cover(shown: Rect, opaque: Rect) -> Option<Cover> {
        Some(Cover { shown, opaque })
    }

    #[test]
    fn opaque_interior_is_pulled_in_by_the_radius() {
        assert_eq!(opaque_interior(r(0, 0, 200, 300), 17), r(17, 17, 166, 266));
        assert_eq!(opaque_interior(r(0, 0, 10, 10), 17), r(17, 17, 0, 0), "a window smaller than its corners has no interior");
        assert_eq!(opaque_interior(r(5, 5, 40, 40), 0), r(5, 5, 40, 40));
    }

    #[test]
    fn grown_adds_the_reach_on_every_side() {
        assert_eq!(grown(r(100, 50, 300, 200), 56), r(44, -6, 412, 312));
    }

    #[test]
    fn a_window_under_a_larger_opaque_window_is_hidden() {
        let back = cover(r(120, 120, 100, 100), r(130, 130, 80, 80));
        let front = cover(r(0, 0, 400, 400), r(17, 17, 366, 366));
        assert_eq!(hidden(&[back, front]), vec![true, false]);
    }

    #[test]
    fn a_window_that_sticks_out_is_not_hidden() {
        let back = cover(r(300, 300, 200, 200), r(310, 310, 180, 180));
        let front = cover(r(0, 0, 400, 400), r(17, 17, 366, 366));
        assert_eq!(hidden(&[back, front]), vec![false, false]);
    }

    #[test]
    fn only_windows_in_front_hide_a_window() {
        let back = cover(r(120, 120, 100, 100), r(130, 130, 80, 80));
        let front = cover(r(0, 0, 400, 400), r(17, 17, 366, 366));
        assert_eq!(hidden(&[front, back]), vec![false, false], "the larger window is behind the smaller");
    }

    #[test]
    fn none_neither_hides_nor_is_hidden() {
        let back = cover(r(120, 120, 100, 100), r(130, 130, 80, 80));
        assert_eq!(hidden(&[back, None]), vec![false, false]);
        assert_eq!(hidden(&[None, back]), vec![false, false]);
    }

    #[test]
    fn a_chain_of_covers_hides_every_window_under_the_front_one() {
        let a = cover(r(150, 150, 50, 50), r(155, 155, 40, 40));
        let b = cover(r(100, 100, 200, 200), r(110, 110, 180, 180));
        let c = cover(r(0, 0, 400, 400), r(17, 17, 366, 366));
        assert_eq!(hidden(&[a, b, c]), vec![true, true, false]);
    }

    #[test]
    fn a_window_inside_a_hidden_one_is_hidden_by_the_same_front_window() {
        // b is hidden by c; a lies inside b's opaque part, so c covers a too
        let a = cover(r(150, 150, 50, 50), r(155, 155, 40, 40));
        let b = cover(r(100, 100, 200, 200), r(110, 110, 180, 180));
        let c = cover(r(0, 0, 400, 400), r(17, 17, 366, 366));
        assert!(hidden(&[a, None, c])[0]);
        assert!(hidden(&[None, b, c])[1]);
    }
}

//! Keyboard focus loop (`NativeWindow.keyViewLoop`): Tab and Shift-Tab move through focusable views in reading order
//! (top to bottom, then left to right), skipping disabled ones and wrapping around. Arrow keys pick the nearest
//! view in a direction (the focus engine).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focusable {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

fn reading_order(items: &[Focusable]) -> Vec<&Focusable> {
    let mut v: Vec<&Focusable> = items.iter().filter(|f| f.enabled).collect();
    // Rows are bands: two views whose vertical centres are within half a row height share a row.
    v.sort_by(|a, b| {
        let (ay, by) = (a.y + a.h / 2.0, b.y + b.h / 2.0);
        let tol = a.h.min(b.h) / 2.0;
        if (ay - by).abs() <= tol { a.x.total_cmp(&b.x) } else { ay.total_cmp(&by) }
    });
    v
}

/// The view after (`forward`) or before `current`; with no current view, the first (or last).
pub fn next(items: &[Focusable], current: Option<u64>, forward: bool) -> Option<u64> {
    let order = reading_order(items);
    if order.is_empty() {
        return None;
    }
    let at = current.and_then(|c| order.iter().position(|f| f.id == c));
    let i = match (at, forward) {
        (None, true) => 0,
        (None, false) => order.len() - 1,
        (Some(i), true) => (i + 1) % order.len(),
        (Some(i), false) => (i + order.len() - 1) % order.len(),
    };
    Some(order[i].id)
}

/// The nearest enabled view in direction `dir` from `current` (no wrapping). Distance along the axis counts once,
/// off-axis offset counts double, so a view straight ahead beats a nearer one far to the side.
pub fn nearest(items: &[Focusable], current: u64, dir: Dir) -> Option<u64> {
    let c = items.iter().find(|f| f.id == current)?;
    let (cx, cy) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
    items
        .iter()
        .filter(|f| f.enabled && f.id != current)
        .filter_map(|f| {
            let (fx, fy) = (f.x + f.w / 2.0 - cx, f.y + f.h / 2.0 - cy);
            let (along, across) = match dir {
                Dir::Right => (fx, fy),
                Dir::Left => (-fx, fy),
                Dir::Down => (fy, fx),
                Dir::Up => (-fy, fx),
            };
            (along > 0.0).then(|| (f.id, along + 2.0 * across.abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|f| f.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(id: u64, x: f32, y: f32, enabled: bool) -> Focusable {
        Focusable { id, x, y, w: 50.0, h: 20.0, enabled }
    }
    #[test]
    fn tab_follows_reading_order_and_wraps() {
        let v = [f(3, 0.0, 40.0, true), f(2, 100.0, 1.0, true), f(1, 0.0, 0.0, true)];
        assert_eq!(next(&v, None, true), Some(1));
        assert_eq!(next(&v, Some(1), true), Some(2));
        assert_eq!(next(&v, Some(2), true), Some(3));
        assert_eq!(next(&v, Some(3), true), Some(1));
        assert_eq!(next(&v, Some(1), false), Some(3));
        assert_eq!(next(&v, None, false), Some(3));
    }
    #[test]
    fn disabled_views_are_skipped() {
        let v = [f(1, 0.0, 0.0, true), f(2, 60.0, 0.0, false), f(3, 120.0, 0.0, true)];
        assert_eq!(next(&v, Some(1), true), Some(3));
        assert_eq!(next(&[f(1, 0.0, 0.0, false)], None, true), None);
    }
    #[test]
    fn arrows_prefer_the_view_straight_ahead() {
        let v = [f(1, 0.0, 0.0, true), f(2, 60.0, 100.0, true), f(3, 0.0, 60.0, true), f(4, 60.0, 0.0, true)];
        assert_eq!(nearest(&v, 1, Dir::Down), Some(3));
        assert_eq!(nearest(&v, 1, Dir::Right), Some(4));
        assert_eq!(nearest(&v, 1, Dir::Up), None);
        assert_eq!(nearest(&v, 3, Dir::Up), Some(1));
    }
}

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

// --- focused values: a view publishes a value under a key; the focused view or its nearest ancestor answers --------------

/// Focused values: a view publishes a value under a key, and a reader asks the focused view for it. When the focused view
/// publishes nothing under that key, the nearest ancestor that does answers, so a menu can show the document a focused
/// text field belongs to. Parents are set with [`FocusedValues::set_parent`]; a view without a parent is a root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusedValues {
    /// (child view, parent view): one parent per child
    parents: Vec<(u64, u64)>,
    /// (view, key, value)
    values: Vec<(u64, String, String)>,
}

impl FocusedValues {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `parent` the ancestor of `view`, replacing an earlier parent of `view`.
    pub fn set_parent(&mut self, view: u64, parent: u64) {
        self.parents.retain(|(c, _)| *c != view);
        self.parents.push((view, parent));
    }

    /// Publishes `value` under `key` from `view`, replacing what that view published under the key before.
    pub fn publish(&mut self, view: u64, key: &str, value: &str) {
        match self.values.iter_mut().find(|(v, k, _)| *v == view && k == key) {
            Some(slot) => slot.2 = value.into(),
            None => self.values.push((view, key.into(), value.into())),
        }
    }

    /// Removes what `view` published under `key`.
    pub fn withdraw(&mut self, view: u64, key: &str) {
        self.values.retain(|(v, k, _)| !(*v == view && k == key));
    }

    /// Removes a view: its values and its parent link. Views that named it as their parent keep that link and simply find
    /// nothing above them.
    pub fn remove_view(&mut self, view: u64) {
        self.values.retain(|(v, _, _)| *v != view);
        self.parents.retain(|(c, _)| *c != view);
    }

    /// The value under `key` for the focused view, or for its nearest ancestor that publishes one. `None` when nothing has focus
    /// or no view on the chain publishes the key.
    pub fn value(&self, focused: Option<u64>, key: &str) -> Option<&str> {
        let mut view = focused?;
        // a loop in the parent links ends the walk: it can take at most one step per link
        for _ in 0..=self.parents.len() {
            if let Some((_, _, v)) = self.values.iter().find(|(w, k, _)| *w == view && k == key) {
                return Some(v.as_str());
            }
            view = self.parents.iter().find(|(c, _)| *c == view).map(|(_, p)| *p)?;
        }
        None
    }
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

    /// A window (1) holds a sheet (2) holds a text field (3).
    fn chain() -> FocusedValues {
        let mut fv = FocusedValues::new();
        fv.set_parent(3, 2);
        fv.set_parent(2, 1);
        fv
    }

    #[test]
    fn the_focused_view_answers_before_its_ancestors() {
        let mut fv = chain();
        fv.publish(1, "document", "Report");
        fv.publish(3, "document", "Draft");
        assert_eq!(fv.value(Some(3), "document"), Some("Draft"));
        assert_eq!(fv.value(Some(1), "document"), Some("Report"));
    }

    #[test]
    fn a_focused_view_without_a_value_falls_back_to_the_nearest_ancestor() {
        let mut fv = chain();
        fv.publish(1, "document", "Report");
        fv.publish(2, "document", "Sheet copy");
        assert_eq!(fv.value(Some(3), "document"), Some("Sheet copy"), "the sheet is nearer than the window");
        assert_eq!(fv.value(Some(2), "document"), Some("Sheet copy"));
    }

    #[test]
    fn nothing_focused_or_an_unknown_key_gives_nothing_and_keys_do_not_leak() {
        let mut fv = chain();
        fv.publish(1, "document", "Report");
        assert_eq!(fv.value(None, "document"), None);
        assert_eq!(fv.value(Some(3), "selection"), None);
        assert_eq!(fv.value(Some(3), "document"), Some("Report"));
        fv.publish(3, "selection", "word");
        assert_eq!(fv.value(Some(3), "document"), Some("Report"), "a second key does not replace the first");
        assert_eq!(fv.value(Some(1), "selection"), None, "a key stays on the view that published it");
    }

    #[test]
    fn publishing_again_replaces_and_withdrawing_falls_back() {
        let mut fv = chain();
        fv.publish(1, "document", "Report");
        fv.publish(3, "document", "Draft");
        fv.publish(3, "document", "Final");
        assert_eq!(fv.value(Some(3), "document"), Some("Final"));
        fv.withdraw(3, "document");
        assert_eq!(fv.value(Some(3), "document"), Some("Report"));
    }

    #[test]
    fn a_loop_in_the_parent_links_ends_the_walk() {
        let mut fv = FocusedValues::new();
        fv.set_parent(1, 2);
        fv.set_parent(2, 1);
        fv.publish(9, "document", "elsewhere");
        assert_eq!(fv.value(Some(1), "document"), None);
        assert_eq!(fv.value(Some(2), "document"), None);
    }

    #[test]
    fn a_new_parent_replaces_the_old_one_and_a_removed_view_takes_its_values() {
        let mut fv = chain();
        fv.publish(1, "document", "Report");
        fv.publish(5, "document", "Other window");
        fv.set_parent(3, 5);
        assert_eq!(fv.value(Some(3), "document"), Some("Other window"), "the field now hangs under the other window");
        fv.remove_view(5);
        assert_eq!(fv.value(Some(3), "document"), None, "the removed window's value is gone");
        assert_eq!(fv.value(Some(2), "document"), Some("Report"), "the sheet still reaches the window it was given");
    }
}

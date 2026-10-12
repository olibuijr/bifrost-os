//! Window list: the windows on the desktop as a pure model, front to back. The window server owns the windows and
//! reports one record per window; this module holds those records and applies the list options an app can ask for:
//! every window or only the ones on screen, the on-screen windows stacked above or below one window, and the
//! exclusion of desktop elements. Records keep the stacking order they were reported in, front first. Nothing here
//! talks to the window server, so the model can be tested on its own.

use crate::Rect;

/// The layer a normal window sits on. A window on a layer below this one is a desktop element (the wallpaper and
/// the icons drawn on it).
pub const NORMAL_LAYER: i32 = 0;

/// One window record: identity, owner and title, layer, bounds, visibility and opacity.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub id: u32,
    pub owner: String,
    pub title: String,
    pub layer: i32,
    pub bounds: Rect,
    pub on_screen: bool,
    pub alpha: f32,
}

/// Which windows a list includes. The on-screen above and below scopes compare stacking positions with the window
/// named by the id; that window is never in its own list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowScope {
    /// Every window, on screen or not.
    All,
    /// Only the windows that are on screen.
    OnScreenOnly,
    /// The on-screen windows stacked above the window with this id. An unknown id gives an empty list.
    OnScreenAbove(u32),
    /// The on-screen windows stacked below the window with this id. An unknown id gives an empty list.
    OnScreenBelow(u32),
}

/// The options of a window list: a scope and whether desktop elements are left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowListOptions {
    pub scope: WindowScope,
    pub exclude_desktop_elements: bool,
}

impl WindowListOptions {
    /// Every window, desktop elements included.
    pub fn all() -> Self {
        WindowListOptions { scope: WindowScope::All, exclude_desktop_elements: false }
    }
}

/// A desktop element: a window on a layer below the normal layer.
pub fn is_desktop_element(window: &WindowInfo) -> bool {
    window.layer < NORMAL_LAYER
}

/// The position of the window with this id in the stacking order (0 is the front-most), or `None` when no record has
/// the id.
pub fn stacking_index(windows: &[WindowInfo], id: u32) -> Option<usize> {
    windows.iter().position(|w| w.id == id)
}

/// Whether window `a` is stacked above window `b`. `None` when either id is unknown.
pub fn is_above(windows: &[WindowInfo], a: u32, b: u32) -> Option<bool> {
    let ia = stacking_index(windows, a)?;
    let ib = stacking_index(windows, b)?;
    Some(ia < ib)
}

/// The records the options select, in the same stacking order as `windows`.
pub fn window_list(windows: &[WindowInfo], options: WindowListOptions) -> Vec<WindowInfo> {
    let reference = match options.scope {
        WindowScope::OnScreenAbove(id) | WindowScope::OnScreenBelow(id) => stacking_index(windows, id),
        _ => None,
    };
    let mut out = Vec::new();
    for (i, w) in windows.iter().enumerate() {
        if options.exclude_desktop_elements && is_desktop_element(w) {
            continue;
        }
        let keep = match options.scope {
            WindowScope::All => true,
            WindowScope::OnScreenOnly => w.on_screen,
            WindowScope::OnScreenAbove(_) => w.on_screen && reference.map_or(false, |r| i < r),
            WindowScope::OnScreenBelow(_) => w.on_screen && reference.map_or(false, |r| i > r),
        };
        if keep {
            out.push(w.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u32, layer: i32, on_screen: bool) -> WindowInfo {
        WindowInfo {
            id,
            owner: format!("app{}", id),
            title: String::new(),
            layer,
            bounds: Rect { x: 0.0, y: 0.0, w: 100.0, h: 80.0 },
            on_screen,
            alpha: 1.0,
        }
    }

    fn ids(list: &[WindowInfo]) -> Vec<u32> {
        list.iter().map(|w| w.id).collect()
    }

    /// Front to back: 1 (on screen), 2 (off screen), 3 (desktop element, on screen), 4 (on screen).
    fn stack() -> Vec<WindowInfo> {
        vec![win(1, 0, true), win(2, 0, false), win(3, -20, true), win(4, 0, true)]
    }

    #[test]
    fn all_scope_keeps_every_window_in_stacking_order() {
        assert_eq!(ids(&window_list(&stack(), WindowListOptions::all())), vec![1, 2, 3, 4]);
    }

    #[test]
    fn on_screen_only_drops_hidden_windows() {
        let o = WindowListOptions { scope: WindowScope::OnScreenOnly, exclude_desktop_elements: false };
        assert_eq!(ids(&window_list(&stack(), o)), vec![1, 3, 4]);
    }

    #[test]
    fn excluding_desktop_elements_drops_windows_below_the_normal_layer() {
        let o = WindowListOptions { scope: WindowScope::All, exclude_desktop_elements: true };
        assert_eq!(ids(&window_list(&stack(), o)), vec![1, 2, 4]);
        assert!(is_desktop_element(&stack()[2]));
        assert!(!is_desktop_element(&stack()[0]));
    }

    #[test]
    fn above_and_below_scopes_use_the_stacking_position_of_the_reference() {
        let above = WindowListOptions { scope: WindowScope::OnScreenAbove(4), exclude_desktop_elements: false };
        assert_eq!(ids(&window_list(&stack(), above)), vec![1, 3]);
        let below = WindowListOptions { scope: WindowScope::OnScreenBelow(1), exclude_desktop_elements: false };
        assert_eq!(ids(&window_list(&stack(), below)), vec![3, 4]);
    }

    #[test]
    fn unknown_reference_id_gives_an_empty_list() {
        let o = WindowListOptions { scope: WindowScope::OnScreenAbove(99), exclude_desktop_elements: false };
        assert!(window_list(&stack(), o).is_empty());
    }

    #[test]
    fn stacking_index_and_is_above_follow_the_reported_order() {
        let s = stack();
        assert_eq!(stacking_index(&s, 3), Some(2));
        assert_eq!(stacking_index(&s, 9), None);
        assert_eq!(is_above(&s, 1, 4), Some(true));
        assert_eq!(is_above(&s, 4, 1), Some(false));
        assert_eq!(is_above(&s, 4, 9), None);
    }
}

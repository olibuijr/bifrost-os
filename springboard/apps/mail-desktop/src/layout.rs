//! The three-pane window: mailboxes, message list and message view. Widths come from the window width alone, so
//! the layout holds at any size; the list gives way first, then the sidebar, when the view would get too narrow.

pub const SIDEBAR_MIN: f32 = 180.0;
pub const SIDEBAR_MAX: f32 = 260.0;
pub const LIST_MIN: f32 = 240.0;
pub const LIST_MAX: f32 = 380.0;
pub const VIEW_MIN: f32 = 320.0;

/// Widths of the sidebar, the list and the view, in points. They sum to `width` when `width` is not negative.
pub fn panes(width: f32) -> [f32; 3] {
    let width = width.max(0.0);
    let mut sidebar = (width * 0.18).clamp(SIDEBAR_MIN, SIDEBAR_MAX).min(width);
    let mut list = (width * 0.28).clamp(LIST_MIN, LIST_MAX).min(width - sidebar);
    let mut view = width - sidebar - list;
    if view < VIEW_MIN {
        let short = VIEW_MIN - view;
        let from_list = short.min((list - LIST_MIN).max(0.0));
        list -= from_list;
        let short = short - from_list;
        sidebar = (sidebar - short).max(0.0);
        view = width - sidebar - list;
    }
    [sidebar, list, view]
}

/// Like [`panes`], with the sidebar shown or hidden (View > Show Sidebar). Hidden, the sidebar is 0 wide and the list
/// and the view share the window.
pub fn panes_for(width: f32, sidebar_shown: bool) -> [f32; 3] {
    if sidebar_shown {
        return panes(width);
    }
    let width = width.max(0.0);
    let mut list = (width * 0.34).clamp(LIST_MIN, LIST_MAX).min(width);
    let mut view = width - list;
    if view < VIEW_MIN {
        list = (list - (VIEW_MIN - view)).max(LIST_MIN.min(list));
        view = width - list;
    }
    [0.0, list, view]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_fill_the_width_at_any_size() {
        for width in [0.0, 100.0, 600.0, 900.0, 1200.0, 1920.0, 3840.0] {
            let [a, b, c] = panes(width);
            assert!(a >= 0.0 && b >= 0.0 && c >= 0.0, "width {width}");
            assert!((a + b + c - width).abs() < 0.01, "width {width}");
        }
    }

    #[test]
    fn a_hidden_sidebar_gives_its_room_to_the_list_and_the_view() {
        for width in [0.0, 100.0, 600.0, 900.0, 1200.0, 3840.0] {
            let [a, b, c] = panes_for(width, false);
            assert_eq!(a, 0.0, "width {width}");
            assert!(b >= 0.0 && c >= 0.0, "width {width}");
            assert!((a + b + c - width).abs() < 0.01, "width {width}");
        }
        assert_eq!(panes_for(1200.0, true), panes(1200.0));
        let [_, list, view] = panes_for(1200.0, false);
        assert!(list >= LIST_MIN && view >= VIEW_MIN);
    }

    #[test]
    fn wide_windows_keep_the_preferred_widths() {
        let [a, b, c] = panes(1200.0);
        assert!((a - 216.0).abs() < 0.01, "18% of 1200");
        assert!((b - 336.0).abs() < 0.01, "28% of 1200");
        assert!(c >= VIEW_MIN);
    }
}

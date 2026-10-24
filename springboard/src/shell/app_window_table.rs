//! The shell's table of the extra windows each app has open (`ToShell::WindowOpen`, `WindowClose`, `WindowTitle`,
//! `WindowResize`). The app's main window is not here: it is the app's own desk window. Each entry is one window the
//! desk draws as a separate window with its own traffic lights, and the Window menu and the Dock list them.
//!
//! The table keeps each app's windows front to back, so focusing one raises it to the front of that app. Window ids
//! are the app's own: the shell checks that an id is not already open for the app, and it never invents one.
//!
//! The desk follows the table: `desk_changes` says which windows to open or close on the desk, and `menu_items` is
//! what the Window menu and the Dock's app menu list for the frontmost app.
#![cfg_attr(not(test), allow(dead_code))]

/// The most extra windows one app may have open in the shell.
pub const MAX_PER_APP: usize = 7;

/// One extra window of an app, as the shell holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraWindow {
    pub app: String,
    pub id: u32,
    pub title: String,
    pub w: f32,
    pub h: f32,
    /// The window floats above the app's normal windows (`ToShell::WindowLevelOf`); a new window is normal.
    pub floating: bool,
    /// The window's position in points relative to the visible desktop area, as the app asked or the person dragged it.
    pub position: Option<(f32, f32)>,
    /// A position the app asked for that the desk has not applied yet (`take_pending_move`).
    pending: Option<(f32, f32)>,
    /// The window is collapsed to its title bar (`ToShell::WindowCollapse`).
    pub collapsed: bool,
    /// The app wants `ToApp::WindowTitleDoubleClick` instead of the shell's zoom (`window_flags::TITLE_DOUBLE_CLICK`).
    pub title_double_click: bool,
    /// The app asked for no position and for the window to cascade from its other windows (`window_flags::CASCADE`).
    pub cascade: bool,
}

/// The part of a title bar, in points, that stays reachable on screen: a window keeps this much of its width inside
/// the visible area on each side, so its title bar can always be grabbed.
pub const GRAB_KEEP_PT: f32 = 48.0;

/// The height of a title bar in points: the top edge of a window stays at or above `area_h - TITLE_PT`.
pub const TITLE_PT: f32 = 28.0;

/// Clamps a window's top left corner (points, relative to the visible desktop area of `area_w` x `area_h` points) so the
/// window stays on screen. None for a non-finite value or an empty area.
pub fn clamp_to_screen(x: f32, y: f32, area_w: f32, area_h: f32, win_w: f32) -> Option<(f32, f32)> {
    if ![x, y, area_w, area_h, win_w].iter().all(|v| v.is_finite()) || area_w <= 0.0 || area_h <= 0.0 {
        return None;
    }
    let lo_x = GRAB_KEEP_PT - win_w;
    let hi_x = (area_w - GRAB_KEEP_PT).max(lo_x);
    let hi_y = (area_h - TITLE_PT).max(0.0);
    Some((x.max(lo_x).min(hi_x), y.max(0.0).min(hi_y)))
}

/// Why a window request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError {
    /// The app already has a window with this id.
    Duplicate(u32),
    /// The app already has `MAX_PER_APP` extra windows.
    TooMany,
    /// The size is not a positive, finite size in points.
    BadSize,
}

fn good_size(w: f32, h: f32) -> bool {
    w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0
}

/// Every app's extra windows, front to back within each app.
#[derive(Debug, Default)]
pub struct WindowTable {
    windows: Vec<ExtraWindow>,
}

impl WindowTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a window for `app` in front of its other windows.
    pub fn open(&mut self, app: &str, id: u32, title: &str, w: f32, h: f32) -> Result<(), TableError> {
        if !good_size(w, h) {
            return Err(TableError::BadSize);
        }
        if self.windows.iter().any(|x| x.app == app && x.id == id) {
            return Err(TableError::Duplicate(id));
        }
        if self.of_app(app).len() >= MAX_PER_APP {
            return Err(TableError::TooMany);
        }
        let window = ExtraWindow {
            app: app.to_string(),
            id,
            title: title.to_string(),
            w,
            h,
            floating: false,
            position: None,
            pending: None,
            collapsed: false,
            title_double_click: false,
            cascade: false,
        };
        let at = self.windows.iter().position(|x| x.app == app).unwrap_or(self.windows.len());
        self.windows.insert(at, window);
        Ok(())
    }

    /// Closes a window and returns it. Unknown windows give `None`.
    pub fn close(&mut self, app: &str, id: u32) -> Option<ExtraWindow> {
        let index = self.index_of(app, id)?;
        Some(self.windows.remove(index))
    }

    /// Closes every extra window of an app (the app quit), and returns them front to back.
    pub fn close_app(&mut self, app: &str) -> Vec<ExtraWindow> {
        let (gone, kept): (Vec<_>, Vec<_>) = self.windows.drain(..).partition(|x| x.app == app);
        self.windows = kept;
        gone
    }

    /// Changes a window's title. False when the window is not open.
    pub fn retitle(&mut self, app: &str, id: u32, title: &str) -> bool {
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].title = title.to_string();
                true
            }
            None => false,
        }
    }

    /// Changes a window's size. Refused for an unknown window or a bad size.
    pub fn resize(&mut self, app: &str, id: u32, w: f32, h: f32) -> Result<(), TableError> {
        if !good_size(w, h) {
            return Err(TableError::BadSize);
        }
        let index = self.index_of(app, id).ok_or(TableError::Duplicate(id))?;
        self.windows[index].w = w;
        self.windows[index].h = h;
        Ok(())
    }

    /// Sets whether a window floats. False when the window is not open (or is the main window, id 0).
    pub fn set_floating(&mut self, app: &str, id: u32, floating: bool) -> bool {
        if id == 0 {
            return false;
        }
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].floating = floating;
                true
            }
            None => false,
        }
    }

    /// The ids of one app's windows that float, front to back.
    pub fn floating_ids(&self, app: &str) -> Vec<u32> {
        self.of_app(app).iter().filter(|w| w.floating).map(|w| w.id).collect()
    }

    /// Raises a window to the front of its app's windows. False when the window is not open.
    pub fn focus(&mut self, app: &str, id: u32) -> bool {
        match self.close(app, id) {
            Some(window) => {
                let at = self.windows.iter().position(|x| x.app == app).unwrap_or(self.windows.len());
                self.windows.insert(at, window);
                true
            }
            None => false,
        }
    }

    /// The windows of one app, front to back: the Window menu and the Dock list show them in this order.
    /// The app asked for window `id` to move to (x, y) points. The desk applies it once (`take_pending_move`). False for a
    /// window that is not open or a non-finite position.
    pub fn request_position(&mut self, app: &str, id: u32, x: f32, y: f32) -> bool {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].position = Some((x, y));
                self.windows[index].pending = Some((x, y));
                true
            }
            None => false,
        }
    }

    /// The person dragged window `id` to (x, y) points: the position is kept, and nothing is pending. False when not open.
    pub fn note_position(&mut self, app: &str, id: u32, x: f32, y: f32) -> bool {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].position = Some((x, y));
                true
            }
            None => false,
        }
    }

    /// The app's requested position for window `id`, once: the desk applies it and it is not given again.
    pub fn take_pending_move(&mut self, app: &str, id: u32) -> Option<(f32, f32)> {
        let index = self.index_of(app, id)?;
        self.windows[index].pending.take()
    }

    /// Collapses (or expands) window `id`. False when the window is not open.
    pub fn set_collapsed(&mut self, app: &str, id: u32, collapsed: bool) -> bool {
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].collapsed = collapsed;
                true
            }
            None => false,
        }
    }

    /// Whether window `id` gets the title bar double-click message. False when the window is not open.
    pub fn set_title_double_click(&mut self, app: &str, id: u32, on: bool) -> bool {
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].title_double_click = on;
                true
            }
            None => false,
        }
    }

    /// Whether window `id` cascades from the app's other windows when the desk first places it. False when the window is
    /// not open.
    pub fn set_cascade(&mut self, app: &str, id: u32, on: bool) -> bool {
        match self.index_of(app, id) {
            Some(index) => {
                self.windows[index].cascade = on;
                true
            }
            None => false,
        }
    }

    pub fn of_app(&self, app: &str) -> Vec<&ExtraWindow> {
        self.windows.iter().filter(|x| x.app == app).collect()
    }

    /// The Window menu and the Dock's app menu list one app's extra windows: their ids and titles, front to back.
    pub fn menu_items(&self, app: &str) -> Vec<(u32, String)> {
        self.of_app(app).iter().map(|w| (w.id, w.title.clone())).collect()
    }

    /// What the desk must change so its windows of `app` match the table: the ids to open (in the table, not yet on
    /// the desk, front to back) and the ids to close (on the desk, gone from the table). `shown` is the ids the desk
    /// draws for the app now.
    pub fn desk_changes(&self, app: &str, shown: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let wanted: Vec<u32> = self.of_app(app).iter().map(|w| w.id).collect();
        let open = wanted.iter().copied().filter(|id| !shown.contains(id)).collect();
        let close = shown.iter().copied().filter(|id| !wanted.contains(id)).collect();
        (open, close)
    }

    fn index_of(&self, app: &str, id: u32) -> Option<usize> {
        self.windows.iter().position(|x| x.app == app && x.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(t: &WindowTable, app: &str) -> Vec<u32> {
        t.of_app(app).iter().map(|x| x.id).collect()
    }

    #[test]
    fn a_new_window_goes_in_front_of_its_app_and_keeps_its_fields() {
        let mut t = WindowTable::new();
        t.open("is.olibuijr.contacts.desktop", 1, "Ada", 480.0, 560.0).unwrap();
        t.open("is.olibuijr.contacts.desktop", 2, "Grace", 480.0, 560.0).unwrap();
        assert_eq!(ids(&t, "is.olibuijr.contacts.desktop"), vec![2, 1]);
        let first = t.of_app("is.olibuijr.contacts.desktop")[0];
        assert_eq!((first.title.as_str(), first.w, first.h), ("Grace", 480.0, 560.0));
    }

    #[test]
    fn windows_of_two_apps_do_not_mix() {
        let mut t = WindowTable::new();
        t.open("a", 1, "one", 10.0, 10.0).unwrap();
        t.open("b", 1, "two", 10.0, 10.0).unwrap();
        assert_eq!(ids(&t, "a"), vec![1]);
        assert_eq!(t.of_app("b")[0].title, "two");
        assert!(t.of_app("c").is_empty());
    }

    #[test]
    fn an_id_is_unique_per_app_and_a_bad_size_is_refused() {
        let mut t = WindowTable::new();
        t.open("a", 1, "x", 10.0, 10.0).unwrap();
        assert_eq!(t.open("a", 1, "y", 10.0, 10.0), Err(TableError::Duplicate(1)));
        assert!(t.open("b", 1, "y", 10.0, 10.0).is_ok(), "another app may use the same id");
        assert_eq!(t.open("a", 2, "z", 0.0, 10.0), Err(TableError::BadSize));
        assert_eq!(t.open("a", 3, "z", f32::NAN, 10.0), Err(TableError::BadSize));
    }

    #[test]
    fn an_app_is_held_to_its_window_limit() {
        let mut t = WindowTable::new();
        for id in 1..=MAX_PER_APP as u32 {
            t.open("a", id, "w", 10.0, 10.0).unwrap();
        }
        assert_eq!(t.open("a", 99, "w", 10.0, 10.0), Err(TableError::TooMany));
        assert!(t.open("b", 99, "w", 10.0, 10.0).is_ok());
    }

    #[test]
    fn close_removes_one_window_and_close_app_removes_all_of_one_app() {
        let mut t = WindowTable::new();
        t.open("a", 1, "x", 10.0, 10.0).unwrap();
        t.open("a", 2, "y", 10.0, 10.0).unwrap();
        t.open("b", 1, "z", 10.0, 10.0).unwrap();
        assert_eq!(t.close("a", 2).map(|x| x.title), Some("y".to_string()));
        assert_eq!(t.close("a", 2), None);
        let gone = t.close_app("a");
        assert_eq!(gone.len(), 1);
        assert!(t.of_app("a").is_empty());
        assert_eq!(ids(&t, "b"), vec![1]);
    }

    #[test]
    fn a_window_floats_only_once_it_is_set_and_the_main_window_is_refused() {
        let mut t = WindowTable::new();
        t.open("calc", 1, "CPU", 300.0, 200.0).unwrap();
        t.open("calc", 2, "Other", 300.0, 200.0).unwrap();
        assert!(t.floating_ids("calc").is_empty(), "a new window is normal");
        assert!(t.set_floating("calc", 1, true));
        assert_eq!(t.floating_ids("calc"), vec![1]);
        assert!(!t.set_floating("calc", 0, true), "the main window is not in the table");
        assert!(!t.set_floating("calc", 9, true), "an unknown window is not set");
        assert!(!t.set_floating("other", 1, true), "windows of another app do not mix");
        assert!(t.set_floating("calc", 1, false));
        assert!(t.floating_ids("calc").is_empty());
    }

    #[test]
    fn retitle_and_resize_change_only_an_open_window() {
        let mut t = WindowTable::new();
        t.open("a", 1, "x", 10.0, 10.0).unwrap();
        assert!(t.retitle("a", 1, "renamed"));
        assert!(!t.retitle("a", 9, "nope"));
        t.resize("a", 1, 320.0, 240.0).unwrap();
        assert_eq!(t.resize("a", 9, 1.0, 1.0), Err(TableError::Duplicate(9)));
        let w = t.of_app("a")[0];
        assert_eq!((w.title.as_str(), w.w, w.h), ("renamed", 320.0, 240.0));
    }

    #[test]
    fn focus_raises_a_window_to_the_front_of_its_app() {
        let mut t = WindowTable::new();
        t.open("a", 1, "x", 10.0, 10.0).unwrap();
        t.open("a", 2, "y", 10.0, 10.0).unwrap();
        assert!(t.focus("a", 1));
        assert_eq!(ids(&t, "a"), vec![1, 2]);
        assert!(!t.focus("a", 9));
    }

    #[test]
    fn the_window_menu_lists_the_apps_windows_front_to_back() {
        let mut t = WindowTable::new();
        t.open("a", 1, "Ada", 10.0, 10.0).unwrap();
        t.open("a", 2, "Grace", 10.0, 10.0).unwrap();
        t.open("b", 5, "other", 10.0, 10.0).unwrap();
        assert_eq!(t.menu_items("a"), vec![(2, "Grace".to_string()), (1, "Ada".to_string())]);
        assert!(t.menu_items("c").is_empty());
        assert!(t.focus("a", 1));
        assert_eq!(t.menu_items("a")[0], (1, "Ada".to_string()), "focusing one moves it to the top of the menu");
    }

    #[test]
    fn the_desk_opens_and_closes_windows_to_follow_the_table() {
        let mut t = WindowTable::new();
        t.open("a", 1, "Ada", 10.0, 10.0).unwrap();
        t.open("a", 2, "Grace", 10.0, 10.0).unwrap();
        // nothing on the desk yet: both windows open, the newest first
        assert_eq!(t.desk_changes("a", &[]), (vec![2, 1], vec![]));
        // the desk already shows window 1: only 2 opens
        assert_eq!(t.desk_changes("a", &[1]), (vec![2], vec![]));
        // the app closed window 2 and the desk still shows it: it is closed on the desk
        t.close("a", 2);
        assert_eq!(t.desk_changes("a", &[1, 2]), (vec![], vec![2]));
        // in sync: nothing to do
        assert_eq!(t.desk_changes("a", &[1]), (vec![], vec![]));
        // another app's windows are not the desk's concern
        assert_eq!(t.desk_changes("b", &[1]), (vec![], vec![1]));
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    const APP: &str = "is.olibuijr.notes.desktop";

    #[test]
    fn a_requested_position_is_pending_once_and_the_drag_is_kept_without_pending() {
        let mut t = WindowTable::new();
        t.open(APP, 1, "Note", 200.0, 150.0).unwrap();
        assert!(t.request_position(APP, 1, 12.0, 40.0));
        assert_eq!(t.take_pending_move(APP, 1), Some((12.0, 40.0)));
        assert_eq!(t.take_pending_move(APP, 1), None, "a position is applied once");
        assert!(t.note_position(APP, 1, 300.0, 90.0));
        assert_eq!(t.take_pending_move(APP, 1), None, "a drag is not sent back to the desk");
        assert_eq!(t.of_app(APP)[0].position, Some((300.0, 90.0)));
    }

    #[test]
    fn a_non_finite_or_unknown_position_is_refused() {
        let mut t = WindowTable::new();
        t.open(APP, 1, "Note", 200.0, 150.0).unwrap();
        assert!(!t.request_position(APP, 1, f32::NAN, 0.0));
        assert!(!t.note_position(APP, 1, 0.0, f32::INFINITY));
        assert!(!t.request_position(APP, 9, 1.0, 1.0), "an unknown window");
        assert_eq!(t.take_pending_move(APP, 1), None);
    }

    #[test]
    fn collapse_and_the_double_click_opt_in_are_per_window() {
        let mut t = WindowTable::new();
        t.open(APP, 1, "A", 200.0, 150.0).unwrap();
        t.open(APP, 2, "B", 200.0, 150.0).unwrap();
        assert!(t.set_collapsed(APP, 1, true));
        assert!(t.set_title_double_click(APP, 2, true));
        assert!(!t.set_collapsed(APP, 9, true));
        let windows = t.of_app(APP);
        let one = windows.iter().find(|w| w.id == 1).unwrap();
        let two = windows.iter().find(|w| w.id == 2).unwrap();
        assert!(one.collapsed && !one.title_double_click);
        assert!(!two.collapsed && two.title_double_click);
    }

    #[test]
    fn a_position_is_clamped_to_the_visible_area() {
        // a window far off the right and bottom edges comes back to the last place its grab strip stays visible
        let (x, y) = clamp_to_screen(5000.0, 5000.0, 1280.0, 800.0, 480.0).unwrap();
        assert_eq!(x, 1280.0 - GRAB_KEEP_PT);
        assert_eq!(y, 800.0 - TITLE_PT);
        // off the left edge: only the grab strip of the window must stay on screen
        let (x, _) = clamp_to_screen(-5000.0, 10.0, 1280.0, 800.0, 480.0).unwrap();
        assert_eq!(x, GRAB_KEEP_PT - 480.0);
        // a point already on screen is kept
        assert_eq!(clamp_to_screen(100.0, 50.0, 1280.0, 800.0, 480.0), Some((100.0, 50.0)));
        // above the top edge the window goes back to the top
        assert_eq!(clamp_to_screen(100.0, -40.0, 1280.0, 800.0, 480.0), Some((100.0, 0.0)));
    }

    #[test]
    fn a_clamp_refuses_a_non_finite_value_or_an_empty_area() {
        assert_eq!(clamp_to_screen(f32::NAN, 0.0, 1280.0, 800.0, 480.0), None);
        assert_eq!(clamp_to_screen(0.0, 0.0, 0.0, 800.0, 480.0), None);
        // a very narrow area never gives an inverted range
        assert!(clamp_to_screen(10.0, 10.0, 20.0, 800.0, 480.0).is_some());
    }
}

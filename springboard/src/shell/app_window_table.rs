//! The shell's table of the extra windows each app has open (`ToShell::WindowOpen`, `WindowClose`, `WindowTitle`,
//! `WindowResize`). The app's main window is not here: it is the app's own desk window. Each entry is one window the
//! desk draws as a separate window with its own traffic lights, and the Window menu and the Dock list them.
//!
//! The table keeps each app's windows front to back, so focusing one raises it to the front of that app. Window ids
//! are the app's own: the shell checks that an id is not already open for the app, and it never invents one.
//!
//! Not drawn yet: the desk does not read this table. The module is the model the desk will use.
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
        let window = ExtraWindow { app: app.to_string(), id, title: title.to_string(), w, h };
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
    pub fn of_app(&self, app: &str) -> Vec<&ExtraWindow> {
        self.windows.iter().filter(|x| x.app == app).collect()
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
}

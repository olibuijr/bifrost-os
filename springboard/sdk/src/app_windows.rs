//! Several windows for one app: the app's window registry ([`AppWindows`]) and the window each event and each frame
//! belongs to ([`WindowId`]).
//!
//! The main window is [`WindowId::MAIN`]. It always exists, and `App::draw` and `App::event` serve it, so an app with
//! one window changes nothing. An app opens another window with [`AppWindows::open`] and gets its id back. The trait
//! methods `App::window_event`, `App::draw_window` and `App::window_closed` then take that id, and their defaults send
//! the main window to `event` and `draw`, and every other window to nothing until the app overrides them.
//!
//! The registry keeps the windows front to back: the focused window is first. The shell's `scenes` service owns the
//! window of each scene on the desk (`scene_session`); this registry is the app-side half, and it is the only part
//! that exists for now. The runtime does not yet give a second window its own surface, so a window opened here is not
//! on screen until the shell hosts it.
//!
//! ```ignore
//! let mut windows = AppWindows::new(DEFAULT_LIMIT);
//! windows.set_frame(WindowId::MAIN, "Contacts", 960.0, 640.0)?;
//! let card = windows.open(WindowSpec::new("Ada Lovelace", 480.0, 560.0))?;
//! windows.close(card)?; // the card's window is gone; its id is never reused
//! ```

/// The id of one window of the app. The main window is [`WindowId::MAIN`]; other ids count up from 1 and are not reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

impl WindowId {
    /// The app's main window, which is open from the start and cannot be closed.
    pub const MAIN: WindowId = WindowId(0);

    pub fn is_main(self) -> bool {
        self == WindowId::MAIN
    }
}

/// The most windows an app holds at once, the main window included, unless the app asks for another limit.
pub const DEFAULT_LIMIT: usize = 8;

/// What an app asks for when it opens a window: the title and the size in points.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSpec {
    pub title: String,
    pub width: f32,
    pub height: f32,
}

impl WindowSpec {
    pub fn new(title: &str, width: f32, height: f32) -> Self {
        Self { title: title.to_string(), width, height }
    }
}

/// One window of the app as the registry holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppWindow {
    pub id: WindowId,
    pub title: String,
    pub width: f32,
    pub height: f32,
}

/// Why a window request failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowError {
    /// No open window has this id (it was never opened, or it was closed).
    Unknown(WindowId),
    /// The main window cannot be closed.
    MainWindow,
    /// The app already holds the most windows it allows.
    TooMany(usize),
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowError::Unknown(id) => write!(f, "no window {}", id.0),
            WindowError::MainWindow => write!(f, "the main window cannot be closed"),
            WindowError::TooMany(max) => write!(f, "an app holds at most {max} windows"),
        }
    }
}

/// The windows of one app, front to back. The main window is always there.
#[derive(Debug)]
pub struct AppWindows {
    windows: Vec<AppWindow>,
    limit: usize,
    next: u32,
}

impl Default for AppWindows {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl AppWindows {
    /// A registry with the main window only, holding at most `limit` windows.
    pub fn new(limit: usize) -> Self {
        let main = AppWindow { id: WindowId::MAIN, title: String::new(), width: 0.0, height: 0.0 };
        Self { windows: vec![main], limit: limit.max(1), next: 1 }
    }

    /// The open windows, front to back. The first one is focused.
    pub fn list(&self) -> &[AppWindow] {
        &self.windows
    }

    pub fn get(&self, id: WindowId) -> Option<&AppWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// The focused window, which is the front one.
    pub fn focused(&self) -> WindowId {
        self.windows[0].id
    }

    /// Opens a window in front of the others and focuses it. Returns its id.
    pub fn open(&mut self, spec: WindowSpec) -> Result<WindowId, WindowError> {
        if self.windows.len() >= self.limit {
            return Err(WindowError::TooMany(self.limit));
        }
        let id = WindowId(self.next);
        self.next += 1;
        self.windows.insert(0, AppWindow { id, title: spec.title, width: spec.width, height: spec.height });
        Ok(id)
    }

    /// Brings an open window to the front and focuses it.
    pub fn focus(&mut self, id: WindowId) -> Result<(), WindowError> {
        let index = self.index_of(id)?;
        let window = self.windows.remove(index);
        self.windows.insert(0, window);
        Ok(())
    }

    /// Closes a window and returns it. The main window cannot be closed. The window behind it becomes the focused one.
    pub fn close(&mut self, id: WindowId) -> Result<AppWindow, WindowError> {
        if id.is_main() {
            return Err(WindowError::MainWindow);
        }
        let index = self.index_of(id)?;
        Ok(self.windows.remove(index))
    }

    /// Sets the title and the size of an open window (the main window's size follows the shell's configure).
    pub fn set_frame(&mut self, id: WindowId, title: &str, width: f32, height: f32) -> Result<(), WindowError> {
        let index = self.index_of(id)?;
        let window = &mut self.windows[index];
        window.title = title.to_string();
        window.width = width;
        window.height = height;
        Ok(())
    }

    /// Where an event or a frame tagged with `window` goes: the window itself while it is open, and nothing once it is
    /// closed or when it never existed. A closed window's late events are dropped rather than sent to the main window.
    pub fn route(&self, window: WindowId) -> Option<WindowId> {
        self.get(window).map(|w| w.id)
    }

    fn index_of(&self, id: WindowId) -> Result<usize, WindowError> {
        self.windows.iter().position(|w| w.id == id).ok_or(WindowError::Unknown(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(title: &str) -> WindowSpec {
        WindowSpec::new(title, 480.0, 560.0)
    }

    fn ids(w: &AppWindows) -> Vec<u32> {
        w.list().iter().map(|x| x.id.0).collect()
    }

    #[test]
    fn a_new_registry_holds_only_the_main_window_and_focuses_it() {
        let w = AppWindows::default();
        assert_eq!(ids(&w), vec![0]);
        assert_eq!(w.focused(), WindowId::MAIN);
        assert!(WindowId::MAIN.is_main());
    }

    #[test]
    fn open_gives_a_new_id_with_its_title_and_size_and_puts_it_in_front() {
        let mut w = AppWindows::default();
        let a = w.open(WindowSpec::new("Ada", 320.0, 240.0)).unwrap();
        let b = w.open(spec("Grace")).unwrap();
        assert_eq!((a, b), (WindowId(1), WindowId(2)));
        assert_eq!(ids(&w), vec![2, 1, 0]);
        assert_eq!(w.focused(), b);
        let ada = w.get(a).unwrap();
        assert_eq!((ada.title.as_str(), ada.width, ada.height), ("Ada", 320.0, 240.0));
    }

    #[test]
    fn focus_raises_a_window_and_an_unknown_id_is_refused() {
        let mut w = AppWindows::default();
        let a = w.open(spec("a")).unwrap();
        let _b = w.open(spec("b")).unwrap();
        w.focus(a).unwrap();
        assert_eq!(w.focused(), a);
        assert_eq!(ids(&w), vec![1, 2, 0]);
        assert_eq!(w.focus(WindowId(9)), Err(WindowError::Unknown(WindowId(9))));
    }

    #[test]
    fn close_removes_the_window_and_the_one_behind_it_takes_focus() {
        let mut w = AppWindows::default();
        let a = w.open(spec("a")).unwrap();
        let b = w.open(spec("b")).unwrap();
        let gone = w.close(b).unwrap();
        assert_eq!(gone.title, "b");
        assert_eq!(w.focused(), a);
        assert_eq!(ids(&w), vec![1, 0]);
        assert_eq!(w.close(b), Err(WindowError::Unknown(b)));
    }

    #[test]
    fn the_main_window_cannot_be_closed() {
        let mut w = AppWindows::default();
        assert_eq!(w.close(WindowId::MAIN), Err(WindowError::MainWindow));
        assert_eq!(ids(&w), vec![0]);
    }

    #[test]
    fn ids_are_never_reused_after_a_window_closes() {
        let mut w = AppWindows::default();
        let a = w.open(spec("a")).unwrap();
        w.close(a).unwrap();
        let b = w.open(spec("b")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn the_limit_counts_the_main_window() {
        let mut w = AppWindows::new(2);
        w.open(spec("a")).unwrap();
        assert_eq!(w.open(spec("b")), Err(WindowError::TooMany(2)));
        w.close(WindowId(1)).unwrap();
        assert!(w.open(spec("c")).is_ok());
    }

    #[test]
    fn set_frame_changes_the_title_and_size_of_an_open_window_only() {
        let mut w = AppWindows::default();
        w.set_frame(WindowId::MAIN, "Contacts", 960.0, 640.0).unwrap();
        assert_eq!(w.get(WindowId::MAIN).unwrap().title, "Contacts");
        let a = w.open(spec("a")).unwrap();
        w.close(a).unwrap();
        assert_eq!(w.set_frame(a, "x", 1.0, 1.0), Err(WindowError::Unknown(a)));
    }

    #[test]
    fn events_route_to_an_open_window_and_are_dropped_after_it_closes() {
        let mut w = AppWindows::default();
        let a = w.open(spec("a")).unwrap();
        assert_eq!(w.route(WindowId::MAIN), Some(WindowId::MAIN));
        assert_eq!(w.route(a), Some(a));
        w.close(a).unwrap();
        assert_eq!(w.route(a), None, "a late event for a closed window is not sent to the main window");
        assert_eq!(w.route(WindowId(42)), None);
    }

    #[test]
    fn a_window_error_names_the_problem() {
        assert_eq!(WindowError::Unknown(WindowId(3)).to_string(), "no window 3");
        assert_eq!(WindowError::TooMany(8).to_string(), "an app holds at most 8 windows");
    }
}

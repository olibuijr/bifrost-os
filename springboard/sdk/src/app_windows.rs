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
    /// Where the window goes, in points relative to the visible desktop area; the shell clamps it to the screen.
    pub position: Option<(f32, f32)>,
    /// The app wants `App::window_title_double_click` when the person double-clicks the title bar (the shell zooms otherwise).
    pub title_double_click: bool,
}

impl WindowSpec {
    pub fn new(title: &str, width: f32, height: f32) -> Self {
        Self { title: title.to_string(), width, height, position: None, title_double_click: false }
    }

    /// Asks for the window at (x, y) points relative to the visible desktop area (a restored position).
    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.position = Some((x, y));
        self
    }

    /// Asks for `App::window_title_double_click` instead of the shell's zoom.
    pub fn with_title_double_click(mut self) -> Self {
        self.title_double_click = true;
        self
    }
}

/// One window of the app as the registry holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppWindow {
    pub id: WindowId,
    pub title: String,
    pub width: f32,
    pub height: f32,
    /// The last position the window was asked for or dragged to, in points relative to the visible desktop area.
    pub position: Option<(f32, f32)>,
    /// The window is collapsed to its title bar.
    pub collapsed: bool,
    pub title_double_click: bool,
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
        let main = AppWindow { id: WindowId::MAIN, title: String::new(), width: 0.0, height: 0.0, position: None, collapsed: false, title_double_click: false };
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
        let window = AppWindow {
            id,
            title: spec.title,
            width: spec.width,
            height: spec.height,
            position: spec.position,
            collapsed: false,
            title_double_click: spec.title_double_click,
        };
        self.windows.insert(0, window);
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

    /// Records where an open window is: the app asked for a position, or the shell reported a drag. The main window has
    /// no shell position.
    pub fn set_position(&mut self, id: WindowId, x: f32, y: f32) -> Result<(), WindowError> {
        if id.is_main() {
            return Err(WindowError::MainWindow);
        }
        let index = self.index_of(id)?;
        self.windows[index].position = Some((x, y));
        Ok(())
    }

    /// Records whether an open window is collapsed to its title bar.
    pub fn set_collapsed(&mut self, id: WindowId, collapsed: bool) -> Result<(), WindowError> {
        if id.is_main() {
            return Err(WindowError::MainWindow);
        }
        let index = self.index_of(id)?;
        self.windows[index].collapsed = collapsed;
        Ok(())
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

/// Routes one window message from the shell (`ToApp::WindowClosed`, `WindowConfigure` or `WindowTouch`) to the registry
/// and to the app's window callbacks. Returns `None` for any other message, otherwise whether the app needs a frame.
/// The registry is never borrowed across a call into the app, so the app may open or close windows from a callback.
/// A window that is closed (or never existed) gets no touch: the event is dropped, never sent to the main window.
pub(crate) fn deliver_window<A: crate::App>(app: &mut A, ctx: &crate::Ctx, msg: &crate::proto::ToApp, now: f64) -> Option<bool> {
    use crate::proto::ToApp;
    Some(match msg {
        ToApp::WindowClosed { id } => {
            let window = WindowId(*id);
            let closed = !window.is_main() && ctx.windows.borrow_mut().close(window).is_ok();
            closed && app.window_closed(ctx, window)
        }
        ToApp::WindowMoved { window, x, y } => {
            let id = WindowId(*window);
            let moved = ctx.windows.borrow_mut().set_position(id, *x, *y).is_ok();
            moved && app.window_moved(ctx, id, *x, *y)
        }
        ToApp::WindowTitleDoubleClick { window } => {
            let id = WindowId(*window);
            let wanted = ctx.windows.borrow().get(id).is_some_and(|w| w.title_double_click);
            wanted && app.window_title_double_click(ctx, id)
        }
        ToApp::WindowConfigure { id, w, h } => {
            let window = WindowId(*id);
            let title = ctx.windows.borrow().get(window).map(|x| x.title.clone());
            match title {
                Some(title) => ctx.windows.borrow_mut().set_frame(window, &title, *w, *h).is_ok(),
                None => false,
            }
        }
        ToApp::WindowTouch { id, touch, kind, x, y, t_ms } => {
            let routed = ctx.windows.borrow().route(WindowId(*id));
            match routed {
                Some(window) => {
                    let t = crate::runtime::event_time(now, *t_ms);
                    app.window_event(ctx, window, crate::Input::Touch { kind: *kind, id: *touch, x: *x, y: *y, t })
                }
                None => false,
            }
        }
        // the shell moved key focus: the window is key while it is focused, and the main window is key again once the
        // extra window that was key loses focus (the shell sends that pair)
        ToApp::WindowFocus { window, focused } => {
            apply_focus(&ctx.key_window_id, *window, *focused);
            true
        }
        // the key window's keys, commands, menu items and text edits; a closed window gets none of them
        ToApp::WindowInput { window, input } => {
            let routed = ctx.windows.borrow().route(WindowId(*window));
            match routed {
                Some(window) => app.window_input(ctx, window, input.clone()),
                None => false,
            }
        }
        _ => return None,
    })
}

/// What `App::window_input` does unless the app overrides it: keys and commands go to `window_event` of the window
/// (as `Input::Key` and `Input::Command`); a menu item and a text edit go to the main window's `menu_action` and
/// `text_input`, and drop for an extra window. An app that handles its own keys calls this for the rest.
pub fn default_window_input<A: crate::App + ?Sized>(app: &mut A, ctx: &crate::Ctx, window: WindowId, input: crate::proto::WindowInput) -> bool {
    use crate::proto::WindowInput;
    match input {
        WindowInput::Key(k) => app.window_event(ctx, window, crate::Input::Key(k)),
        WindowInput::Command(c) => app.window_event(ctx, window, crate::Input::Command(c)),
        WindowInput::MenuAction(id) if window.is_main() => app.menu_action(ctx, id),
        text @ (WindowInput::SetMarkedText { .. } | WindowInput::UnmarkText { .. } | WindowInput::DeleteSurrounding { .. }) if window.is_main() => {
            app.text_input(ctx, text.to_app(0))
        }
        _ => false,
    }
}

/// Cmd+W (evdev W with the Meta chord): the key that closes the window it is typed in.
pub fn is_close_window_key(key: &crate::proto::KeyInput) -> bool {
    key.as_chord() == Some((17, crate::proto::CHORD_META))
}

/// Records a focus change in the key window cell. Focusing window `window` makes it key; losing focus only clears it when
/// it was the key window, so a late "lost" for an old window cannot take the key from the window that is key now.
pub(crate) fn apply_focus(key: &std::cell::Cell<u32>, window: u32, focused: bool) {
    if focused {
        key.set(window);
    } else if key.get() == window {
        key.set(WindowId::MAIN.0);
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

    use crate::proto::{ToApp, ToShell, TouchKind};
    use crate::App;

    /// Records what the window callbacks receive.
    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
    }

    impl crate::App for Fake {
        fn id(&self) -> &str {
            "is.test.windows"
        }
        fn draw(&mut self, _: &crate::Ctx, s: &mut crate::kit::Surface, _: f64) -> crate::kit::Damage {
            crate::kit::Damage::new(s.w, s.h)
        }
        fn window_event(&mut self, _: &crate::Ctx, window: WindowId, ev: crate::Input) -> bool {
            if let crate::Input::Touch { id, .. } = ev {
                self.log.push(format!("touch {id} on {}", window.0));
            }
            true
        }
        fn window_closed(&mut self, _: &crate::Ctx, window: WindowId) -> bool {
            self.log.push(format!("closed {}", window.0));
            true
        }
        fn window_moved(&mut self, _: &crate::Ctx, window: WindowId, x: f32, y: f32) -> bool {
            self.log.push(format!("moved {} {x} {y}", window.0));
            true
        }
        fn window_title_double_click(&mut self, _: &crate::Ctx, window: WindowId) -> bool {
            self.log.push(format!("double {}", window.0));
            true
        }
    }

    /// An app that logs what each window and the main window receive. Keys and commands reach `window_event` on an
    /// extra window (the trait default of `window_input`), menu items and text edits reach the main window only.
    #[derive(Default)]
    struct KeyFake {
        log: Vec<String>,
    }

    impl crate::App for KeyFake {
        fn id(&self) -> &str {
            "is.test.keys"
        }
        fn draw(&mut self, _: &crate::Ctx, s: &mut crate::kit::Surface, _: f64) -> crate::kit::Damage {
            crate::kit::Damage::new(s.w, s.h)
        }
        fn event(&mut self, _: &crate::Ctx, ev: crate::Input) -> bool {
            if let crate::Input::Key(crate::proto::KeyInput::Code(c)) = ev {
                self.log.push(format!("main key {c}"));
            }
            true
        }
        fn window_event(&mut self, _: &crate::Ctx, window: WindowId, ev: crate::Input) -> bool {
            match ev {
                crate::Input::Key(crate::proto::KeyInput::Code(c)) => self.log.push(format!("key {c} on {}", window.0)),
                crate::Input::Command(c) => self.log.push(format!("command {c:?} on {}", window.0)),
                _ => {}
            }
            true
        }
        fn menu_action(&mut self, _: &crate::Ctx, id: u32) -> bool {
            self.log.push(format!("menu {id}"));
            true
        }
        fn text_input(&mut self, _: &crate::Ctx, ev: ToApp) -> bool {
            self.log.push(format!("text {ev:?}"));
            true
        }
        fn window_closed(&mut self, _: &crate::Ctx, window: WindowId) -> bool {
            self.log.push(format!("closed {}", window.0));
            true
        }
    }

    /// The fake transport: the message goes through its wire bytes and back before the app sees it.
    fn over_the_wire(app: &mut KeyFake, ctx: &crate::Ctx, m: &ToApp) -> Option<bool> {
        let decoded = ToApp::decode(&m.encode()).expect("the message decodes");
        deliver_window(app, ctx, &decoded, 0.0)
    }

    #[test]
    fn the_main_window_is_key_until_the_shell_makes_an_extra_window_key() {
        let ctx = crate::runtime::test_ctx();
        let mut app = KeyFake::default();
        let id = ctx.open_window(WindowSpec::new("Card", 480.0, 560.0)).unwrap();
        assert_eq!(ctx.key_window(), WindowId::MAIN);
        assert_eq!(over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: id.0, focused: true }), Some(true));
        assert_eq!(ctx.key_window(), id);
        assert_eq!(over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: id.0, focused: false }), Some(true));
        assert_eq!(ctx.key_window(), WindowId::MAIN, "losing focus hands the key back to the main window");
        assert_eq!(over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: 0, focused: true }), Some(true));
        assert_eq!(ctx.key_window(), WindowId::MAIN);
    }

    #[test]
    fn a_late_lost_focus_for_an_old_window_does_not_take_the_key_from_the_new_one() {
        let ctx = crate::runtime::test_ctx();
        let mut app = KeyFake::default();
        let a = ctx.open_window(WindowSpec::new("A", 100.0, 100.0)).unwrap();
        let b = ctx.open_window(WindowSpec::new("B", 100.0, 100.0)).unwrap();
        over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: a.0, focused: true });
        over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: b.0, focused: true });
        over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: a.0, focused: false });
        assert_eq!(ctx.key_window(), b);
    }

    #[test]
    fn keys_and_commands_go_to_the_key_window_and_not_to_the_main_window() {
        let ctx = crate::runtime::test_ctx();
        let mut app = KeyFake::default();
        let id = ctx.open_window(WindowSpec::new("Card", 480.0, 560.0)).unwrap();
        over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: id.0, focused: true });
        let key = ToApp::WindowInput { window: id.0, input: crate::proto::WindowInput::Key(crate::proto::KeyInput::Code(30)) };
        assert_eq!(over_the_wire(&mut app, &ctx, &key), Some(true));
        let cmd = ToApp::WindowInput { window: id.0, input: crate::proto::WindowInput::Command(crate::proto::Command::SelectAll) };
        over_the_wire(&mut app, &ctx, &cmd);
        assert_eq!(app.log, vec!["key 30 on 1".to_string(), format!("command {:?} on 1", crate::proto::Command::SelectAll)]);
    }

    #[test]
    fn the_main_window_key_arrives_through_the_window_path_as_a_plain_key() {
        let ctx = crate::runtime::test_ctx();
        let mut app = KeyFake::default();
        let key = ToApp::WindowInput { window: 0, input: crate::proto::WindowInput::Key(crate::proto::KeyInput::Code(9)) };
        assert_eq!(over_the_wire(&mut app, &ctx, &key), Some(true));
        assert_eq!(app.log, vec!["key 9 on 0".to_string()]);
    }

    #[test]
    fn menu_items_and_text_edits_reach_only_the_main_window() {
        let ctx = crate::runtime::test_ctx();
        let mut app = KeyFake::default();
        let id = ctx.open_window(WindowSpec::new("Card", 480.0, 560.0)).unwrap();
        over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: id.0, focused: true });
        let menu = ToApp::WindowInput { window: id.0, input: crate::proto::WindowInput::MenuAction(4) };
        over_the_wire(&mut app, &ctx, &menu);
        let text = ToApp::WindowInput { window: id.0, input: crate::proto::WindowInput::UnmarkText { commit: true } };
        over_the_wire(&mut app, &ctx, &text);
        assert!(app.log.is_empty(), "an extra window gets no menu item or text edit by default: {:?}", app.log);
        // the main window's menu item and text edit go to the main window's callbacks
        let main_menu = ToApp::WindowInput { window: 0, input: crate::proto::WindowInput::MenuAction(4) };
        over_the_wire(&mut app, &ctx, &main_menu);
        assert_eq!(app.log, vec!["menu 4".to_string()]);
    }

    #[test]
    fn input_for_a_closed_window_is_dropped() {
        let ctx = crate::runtime::test_ctx();
        let mut app = KeyFake::default();
        let id = ctx.open_window(WindowSpec::new("Card", 480.0, 560.0)).unwrap();
        over_the_wire(&mut app, &ctx, &ToApp::WindowFocus { window: id.0, focused: true });
        ctx.close_window(id).unwrap();
        let key = ToApp::WindowInput { window: id.0, input: crate::proto::WindowInput::Key(crate::proto::KeyInput::Code(30)) };
        assert_eq!(over_the_wire(&mut app, &ctx, &key), Some(false));
        assert_eq!(ctx.key_window(), WindowId::MAIN, "a closed key window falls back to the main window");
        assert!(app.log.is_empty());
    }

    #[test]
    fn the_menus_for_a_window_default_to_the_main_menus() {
        let ctx = crate::runtime::test_ctx();
        let app = KeyFake::default();
        assert!(app.menus_for(&ctx, WindowId(1)) == app.main_menu(&ctx));
    }

    #[test]
    fn only_cmd_w_is_the_close_window_key() {
        use crate::proto::{KeyInput, CHORD_CTRL, CHORD_META};
        assert!(is_close_window_key(&KeyInput::chord(17, CHORD_META)));
        assert!(!is_close_window_key(&KeyInput::chord(17, CHORD_CTRL)));
        assert!(!is_close_window_key(&KeyInput::Code(17)));
        assert!(!is_close_window_key(&KeyInput::Text("w".into())));
    }

    fn touch_down(id: u32) -> ToApp {
        ToApp::WindowTouch { id, touch: 7, kind: TouchKind::Down, x: 1.0, y: 2.0, t_ms: 0 }
    }

    #[test]
    fn the_open_window_sends_its_open_message_with_the_app_id() {
        let ctx = crate::runtime::test_ctx();
        let id = ctx.open_window(WindowSpec::new("Ada", 480.0, 560.0)).unwrap();
        assert_eq!(id, WindowId(1));
        assert_eq!(ctx.take_sent(), vec![ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 480.0, h: 560.0, position: None, flags: 0 }]);
    }

    #[test]
    fn a_touch_on_an_open_window_reaches_its_window_callback_with_the_window_id() {
        let ctx = crate::runtime::test_ctx();
        let mut app = Fake::default();
        let id = ctx.open_window(WindowSpec::new("Ada", 480.0, 560.0)).unwrap();
        assert_eq!(deliver_window(&mut app, &ctx, &touch_down(id.0), 0.0), Some(true));
        assert_eq!(app.log, vec!["touch 7 on 1".to_string()]);
    }

    #[test]
    fn a_touch_on_a_closed_or_unknown_window_is_dropped_not_sent_to_the_main_window() {
        let ctx = crate::runtime::test_ctx();
        let mut app = Fake::default();
        let id = ctx.open_window(WindowSpec::new("Ada", 480.0, 560.0)).unwrap();
        ctx.close_window(id).unwrap();
        assert_eq!(deliver_window(&mut app, &ctx, &touch_down(id.0), 0.0), Some(false));
        assert_eq!(deliver_window(&mut app, &ctx, &touch_down(42), 0.0), Some(false));
        assert!(app.log.is_empty());
    }

    #[test]
    fn a_window_the_shell_closes_is_removed_once_and_the_app_is_told() {
        let ctx = crate::runtime::test_ctx();
        let mut app = Fake::default();
        let id = ctx.open_window(WindowSpec::new("Ada", 480.0, 560.0)).unwrap();
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowClosed { id: id.0 }, 0.0), Some(true));
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowClosed { id: id.0 }, 0.0), Some(false));
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowClosed { id: 0 }, 0.0), Some(false), "the main window never closes");
        assert_eq!(app.log, vec!["closed 1".to_string()]);
        assert!(ctx.windows().get(id).is_none());
    }

    #[test]
    fn a_window_moves_collapses_and_only_an_opted_in_window_gets_the_double_click() {
        let ctx = crate::runtime::test_ctx();
        let mut app = Fake::default();
        let spec = WindowSpec::new("Note", 200.0, 150.0).at(12.0, 40.0).with_title_double_click();
        let id = ctx.open_window(spec).unwrap();
        assert_eq!(
            ctx.take_sent(),
            vec![ToShell::WindowOpen { id: 1, title: "Note".into(), w: 200.0, h: 150.0, position: Some((12.0, 40.0)), flags: crate::proto::window_flags::TITLE_DOUBLE_CLICK }]
        );
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowMoved { window: id.0, x: 30.0, y: 60.0 }, 0.0), Some(true));
        assert_eq!(ctx.windows().get(id).and_then(|w| w.position), Some((30.0, 60.0)));
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowTitleDoubleClick { window: id.0 }, 0.0), Some(true));
        ctx.set_window_collapsed(id, true).unwrap();
        assert!(ctx.windows().get(id).is_some_and(|w| w.collapsed));
        assert_eq!(ctx.take_sent(), vec![ToShell::WindowCollapse { window: 1, collapsed: true }]);
        let plain = ctx.open_window(WindowSpec::new("Plain", 10.0, 10.0)).unwrap();
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowTitleDoubleClick { window: plain.0 }, 0.0), Some(false));
        assert_eq!(app.log, vec!["moved 1 30 60".to_string(), "double 1".to_string()]);
        assert_eq!(ctx.move_window(WindowId::MAIN, 1.0, 1.0), Err(WindowError::MainWindow));
    }

    #[test]
    fn a_shell_resize_keeps_the_title_and_changes_the_size() {
        let ctx = crate::runtime::test_ctx();
        let mut app = Fake::default();
        let id = ctx.open_window(WindowSpec::new("Ada", 480.0, 560.0)).unwrap();
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowConfigure { id: id.0, w: 300.0, h: 200.0 }, 0.0), Some(true));
        let open = ctx.windows().get(id).cloned().unwrap();
        assert_eq!((open.title.as_str(), open.width, open.height), ("Ada", 300.0, 200.0));
    }

    #[test]
    fn the_app_changes_title_and_size_and_each_change_is_sent() {
        let ctx = crate::runtime::test_ctx();
        let id = ctx.open_window(WindowSpec::new("Ada", 480.0, 560.0)).unwrap();
        ctx.take_sent();
        ctx.set_window_title(id, "Grace").unwrap();
        ctx.resize_window(id, 320.0, 240.0).unwrap();
        assert_eq!(
            ctx.take_sent(),
            vec![ToShell::WindowTitle { id: 1, title: "Grace".into() }, ToShell::WindowResize { id: 1, w: 320.0, h: 240.0 }]
        );
        let open = ctx.windows().get(id).cloned().unwrap();
        assert_eq!((open.title.as_str(), open.width, open.height), ("Grace", 320.0, 240.0));
        assert_eq!(ctx.set_window_title(WindowId(9), "x"), Err(WindowError::Unknown(WindowId(9))));
        assert!(ctx.take_sent().is_empty(), "a refused change sends nothing");
    }

    #[test]
    fn messages_that_are_not_about_a_window_are_left_to_the_run_loop() {
        let ctx = crate::runtime::test_ctx();
        let mut app = Fake::default();
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::Resume, 0.0), None);
        assert_eq!(deliver_window(&mut app, &ctx, &ToApp::WindowRelease { id: 1, buf: 0, generation: 1 }, 0.0), None);
    }
}

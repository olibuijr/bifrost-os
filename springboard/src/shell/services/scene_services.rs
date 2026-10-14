//! The `scenes` service: the windows an app holds as scenes, opened, listed, focused, reopened and closed by the shell.
//! The app side is `bifrost_app::scene_session` (`SceneSessions`), which keeps the app's scene state; this file owns the
//! window of each scene: its persistent identifier, its window number, its frame and its stacking order.
//!
//! Placement is the shared window scene geometry (`bifrost_ui::window_scene`): a new window cascades from the top-left
//! corner of the desk, or from the top of the screen on the handheld, and stays inside the app's size restrictions.
//! The table keeps the windows front to back. Opening a window puts it in front, and focusing one raises it.
//!
//! Methods (JSON parameters):
//! - `request {title, width, height, min_width, min_height, max_width, max_height}` opens a window and answers a scene
//!   record (see below). The size defaults to 640 x 480 points; the restrictions default to none.
//! - `reopen {scene, title, frame, min_width, ...}` puts back a window the app saved before a relaunch with the same
//!   identifier, clamped into the screen. Reopening an identifier that is already open answers the open window.
//! - `list {}` answers `{scenes: [record]}` for the app, front first.
//! - `focus {scene}` and `close {scene}` answer `{}`.
//!
//! A scene record is `{scene, number, frame: {x, y, w, h}, title}`. Scene window numbers start at 20001, above the app
//! window numbers of the `windows` service.
//!
//! The table is in memory: the shell does not yet keep scene windows across a shell restart. The app's restorable state
//! (`restoration`) brings the scenes back through `reopen`.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::wire_json::*;
use bifrost_ui::window_scene::{self, SizeRestrictions};
use bifrost_ui::Rect;

pub const SERVICE: &str = "scenes";
/// The first window number a scene window gets.
pub const NUMBER_BASE: u32 = 20_000;
/// The most windows one app may hold as scenes.
pub const MAX_PER_APP: usize = 8;
/// The size a window asks for when the app gives none, in points.
pub const DEFAULT_SIZE: (f32, f32) = (640.0, 480.0);

/// One scene window in the shell.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneWindow {
    pub app: String,
    pub scene: String,
    pub number: u32,
    pub frame: Rect,
    pub title: String,
}

/// The scene windows of every app, front to back.
#[derive(Debug)]
pub struct SceneWindows {
    windows: Vec<SceneWindow>,
    next_scene: u32,
    next_number: u32,
}

impl Default for SceneWindows {
    fn default() -> Self {
        Self { windows: Vec::new(), next_scene: 1, next_number: NUMBER_BASE + 1 }
    }
}

/// The frame of `frame` moved and shrunk into `screen` and the restrictions.
fn fit(frame: Rect, screen: Rect, restrictions: SizeRestrictions) -> Rect {
    let (w, h) = restrictions.clamp(frame.w.min(screen.w.max(0.0)), frame.h.min(screen.h.max(0.0)));
    let x = frame.x.clamp(screen.x, (screen.x + screen.w - w).max(screen.x));
    let y = frame.y.clamp(screen.y, (screen.y + screen.h - h).max(screen.y));
    Rect { x, y, w, h }
}

impl SceneWindows {
    /// Windows of `app`, front first.
    pub fn list(&self, app: &str) -> Vec<SceneWindow> {
        self.windows.iter().filter(|w| w.app == app).cloned().collect()
    }

    pub fn find(&self, app: &str, scene: &str) -> Option<&SceneWindow> {
        self.windows.iter().find(|w| w.app == app && w.scene == scene)
    }

    fn index_of(&self, app: &str, scene: &str) -> Result<usize, String> {
        self.windows.iter().position(|w| w.app == app && w.scene == scene).ok_or_else(|| format!("scenes: no window {scene}"))
    }

    /// Opens a new window for `app` at the front, cascaded from its other windows.
    pub fn open(
        &mut self,
        app: &str,
        title: &str,
        size: (f32, f32),
        restrictions: SizeRestrictions,
        screen: Rect,
    ) -> Result<SceneWindow, String> {
        let existing: Vec<Rect> = self.windows.iter().filter(|w| w.app == app).map(|w| w.frame).collect();
        if existing.len() >= MAX_PER_APP {
            return Err(format!("scenes: an app holds at most {MAX_PER_APP} windows"));
        }
        let (w, h) = restrictions.clamp(size.0, size.1);
        let frame = window_scene::cascade(&existing, screen, (w, h));
        let scene = format!("window-{}", self.next_scene);
        self.next_scene += 1;
        let win = SceneWindow { app: app.to_string(), scene, number: self.next_number, frame, title: title.to_string() };
        self.next_number += 1;
        self.windows.insert(0, win.clone());
        Ok(win)
    }

    /// Puts back a window with its saved identifier. An identifier already open is answered as it is.
    pub fn reopen(
        &mut self,
        app: &str,
        scene: &str,
        title: &str,
        frame: Rect,
        restrictions: SizeRestrictions,
        screen: Rect,
    ) -> Result<SceneWindow, String> {
        if let Some(w) = self.find(app, scene) {
            return Ok(w.clone());
        }
        if self.windows.iter().filter(|w| w.app == app).count() >= MAX_PER_APP {
            return Err(format!("scenes: an app holds at most {MAX_PER_APP} windows"));
        }
        self.note_identifier(scene);
        let win = SceneWindow {
            app: app.to_string(),
            scene: scene.to_string(),
            number: self.next_number,
            frame: fit(frame, screen, restrictions),
            title: title.to_string(),
        };
        self.next_number += 1;
        self.windows.insert(0, win.clone());
        Ok(win)
    }

    /// Keeps generated identifiers clear of a reopened one of the form `window-N`.
    fn note_identifier(&mut self, scene: &str) {
        if let Some(n) = scene.strip_prefix("window-").and_then(|n| n.parse::<u32>().ok()) {
            self.next_scene = self.next_scene.max(n.saturating_add(1));
        }
    }

    /// Moves a window to the front.
    pub fn focus(&mut self, app: &str, scene: &str) -> Result<(), String> {
        let i = self.index_of(app, scene)?;
        let win = self.windows.remove(i);
        self.windows.insert(0, win);
        Ok(())
    }

    /// Closes a window and returns it.
    pub fn close(&mut self, app: &str, scene: &str) -> Result<SceneWindow, String> {
        let i = self.index_of(app, scene)?;
        Ok(self.windows.remove(i))
    }
}

/// The record a window answers with.
pub fn record(win: &SceneWindow) -> Json {
    obj(vec![
        ("scene", text(&win.scene)),
        ("number", num(win.number)),
        ("frame", rect_json(win.frame)),
        ("title", text(&win.title)),
    ])
}

/// The desk area in points: the whole screen on the handheld, and below the menu bar on the desktop.
fn screen_rect(shell: &Shell) -> Rect {
    let pt = shell.ui.pt(1.0).max(0.01);
    let top = if shell.desk.is_some() { shell.desk_metrics().menubar } else { 0 };
    Rect { x: 0.0, y: top as f32 / pt, w: shell.w as f32 / pt, h: (shell.h - top) as f32 / pt }
}

/// The size restrictions the app gave in `params`, each bound optional.
fn restrictions_of(params: &Json) -> SizeRestrictions {
    let d = SizeRestrictions::default();
    SizeRestrictions {
        min_w: f32_of(params, "min_width").unwrap_or(d.min_w),
        min_h: f32_of(params, "min_height").unwrap_or(d.min_h),
        max_w: f32_of(params, "max_width").unwrap_or(d.max_w),
        max_h: f32_of(params, "max_height").unwrap_or(d.max_h),
    }
}

pub struct SceneServices {
    pub windows: SceneWindows,
}

impl Default for SceneServices {
    fn default() -> Self {
        Self { windows: SceneWindows::default() }
    }
}

impl SystemService for SceneServices {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let screen = screen_rect(shell);
        let title = str_of(params, "title").unwrap_or_else(|| app_id.to_string());
        match method {
            "request" => {
                let size = (f32_of(params, "width").unwrap_or(DEFAULT_SIZE.0), f32_of(params, "height").unwrap_or(DEFAULT_SIZE.1));
                let win = self.windows.open(app_id, &title, size, restrictions_of(params), screen)?;
                Ok(record(&win))
            }
            "reopen" => {
                let scene = str_of(params, "scene").ok_or("scenes: reopen needs a scene")?;
                let frame = rect_of(params, "frame").ok_or("scenes: reopen needs a frame")?;
                let win = self.windows.reopen(app_id, &scene, &title, frame, restrictions_of(params), screen)?;
                Ok(record(&win))
            }
            "list" => {
                let items: Vec<Json> = self.windows.list(app_id).iter().map(record).collect();
                Ok(obj(vec![("scenes", Json::Arr(items))]))
            }
            "focus" => {
                let scene = str_of(params, "scene").ok_or("scenes: focus needs a scene")?;
                self.windows.focus(app_id, &scene)?;
                Ok(obj(vec![]))
            }
            "close" => {
                let scene = str_of(params, "scene").ok_or("scenes: close needs a scene")?;
                self.windows.close(app_id, &scene)?;
                Ok(obj(vec![]))
            }
            _ => Err(format!("scenes: unknown method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect { x: 0.0, y: 30.0, w: 1200.0, h: 800.0 }
    }

    #[test]
    fn opening_windows_cascades_them_and_puts_each_at_the_front() {
        let mut t = SceneWindows::default();
        let a = t.open("notes", "Notes", (400.0, 300.0), SizeRestrictions::default(), screen()).unwrap();
        let b = t.open("notes", "Notes", (400.0, 300.0), SizeRestrictions::default(), screen()).unwrap();
        assert_ne!(a.scene, b.scene);
        assert_eq!(a.number, NUMBER_BASE + 1);
        assert_eq!(b.number, NUMBER_BASE + 2);
        assert_eq!(b.frame.x, a.frame.x + window_scene::CASCADE_STEP);
        let listed = t.list("notes");
        assert_eq!(listed[0].scene, b.scene);
        assert_eq!(listed[1].scene, a.scene);
    }

    #[test]
    fn windows_of_other_apps_are_not_listed_or_cascaded_with() {
        let mut t = SceneWindows::default();
        t.open("notes", "Notes", (400.0, 300.0), SizeRestrictions::default(), screen()).unwrap();
        let other = t.open("mail", "Mail", (400.0, 300.0), SizeRestrictions::default(), screen()).unwrap();
        assert_eq!(other.frame.x, window_scene::CASCADE_MARGIN);
        assert_eq!(t.list("mail").len(), 1);
        assert_eq!(t.list("notes").len(), 1);
        assert!(t.list("nobody").is_empty());
    }

    #[test]
    fn the_restrictions_bound_the_size_of_a_new_window() {
        let mut t = SceneWindows::default();
        let r = SizeRestrictions { min_w: 500.0, min_h: 0.0, max_w: 600.0, max_h: 200.0 };
        let win = t.open("notes", "Notes", (100.0, 900.0), r, screen()).unwrap();
        assert_eq!((win.frame.w, win.frame.h), (500.0, 200.0));
    }

    #[test]
    fn an_app_is_held_to_its_window_limit() {
        let mut t = SceneWindows::default();
        for _ in 0..MAX_PER_APP {
            t.open("notes", "Notes", (300.0, 200.0), SizeRestrictions::default(), screen()).unwrap();
        }
        assert!(t.open("notes", "Notes", (300.0, 200.0), SizeRestrictions::default(), screen()).is_err());
        assert!(t.open("mail", "Mail", (300.0, 200.0), SizeRestrictions::default(), screen()).is_ok());
    }

    #[test]
    fn focus_raises_a_window_and_close_removes_it() {
        let mut t = SceneWindows::default();
        let a = t.open("notes", "A", (300.0, 200.0), SizeRestrictions::default(), screen()).unwrap();
        t.open("notes", "B", (300.0, 200.0), SizeRestrictions::default(), screen()).unwrap();
        t.focus("notes", &a.scene).unwrap();
        assert_eq!(t.list("notes")[0].scene, a.scene);
        let gone = t.close("notes", &a.scene).unwrap();
        assert_eq!(gone.title, "A");
        assert_eq!(t.list("notes").len(), 1);
        assert!(t.focus("notes", &a.scene).is_err());
        assert!(t.close("notes", &a.scene).is_err());
    }

    #[test]
    fn reopening_keeps_the_identifier_and_fits_the_frame_into_the_screen() {
        let mut t = SceneWindows::default();
        let frame = Rect { x: 5000.0, y: -40.0, w: 400.0, h: 300.0 };
        let win = t.reopen("notes", "window-7", "Notes", frame, SizeRestrictions::default(), screen()).unwrap();
        assert_eq!(win.scene, "window-7");
        assert!(window_scene::contains(screen(), win.frame));
        let again = t.reopen("notes", "window-7", "Notes", frame, SizeRestrictions::default(), screen()).unwrap();
        assert_eq!(again, win);
        assert_eq!(t.list("notes").len(), 1);
    }

    #[test]
    fn a_generated_identifier_skips_a_reopened_one() {
        let mut t = SceneWindows::default();
        let frame = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        t.reopen("notes", "window-4", "Notes", frame, SizeRestrictions::default(), screen()).unwrap();
        let fresh = t.open("notes", "Notes", (300.0, 200.0), SizeRestrictions::default(), screen()).unwrap();
        assert_eq!(fresh.scene, "window-5");
    }

    #[test]
    fn the_record_carries_the_window_fields() {
        let win = SceneWindow {
            app: "notes".into(),
            scene: "window-1".into(),
            number: NUMBER_BASE + 1,
            frame: Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 },
            title: "Notes".into(),
        };
        let j = record(&win);
        assert_eq!(str_of(&j, "scene").as_deref(), Some("window-1"));
        assert_eq!(u32_of(&j, "number"), Some(NUMBER_BASE + 1));
        assert_eq!(rect_of(&j, "frame"), Some(win.frame));
    }
}

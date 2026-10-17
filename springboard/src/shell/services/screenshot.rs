//! The `screenshot` service: a picture of the screen for an app, taken the way the screenshot toolbar takes one and
//! saved to Photos beside the toolbar's captures. The SDK client is `bifrost_app::screenshot`.
//!
//! Methods (the app needs the Screen permission for each):
//! - `selection()` opens the toolbar's selection: a rectangle dragged over the screen;
//! - `window()` opens the window pick: the window under the pointer, with its shadow;
//! - `screen()` takes the whole screen at once.
//!
//! Each replies `{started: true}` at once, or an error when the screen is locked, the Screen permission is missing or
//! another capture is open. When the picture's file has landed the shell sends the event `captured` to the app with
//! body `{path}`. A selection or window the person cancels sends nothing. Photos keeps the file, so the path is in the
//! Pictures folder (or the Photos library's first folder), not in the app's data folder.

use super::display_services::require_screen_recording;
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use std::path::{Path, PathBuf};

/// The service name, as the shell registers it.
pub const SERVICE: &str = "screenshot";
/// The event a finished picture is sent with.
pub const CAPTURED: &str = "captured";
/// A picture whose file has not landed this many seconds after it was saved is given up (a failed write never lands).
pub const WAIT_SECS: f64 = 10.0;

/// The request in progress and the pictures owed to apps. Pure state: the shell feeds it each tick.
#[derive(Default)]
pub struct Shots {
    /// The app whose request has not ended yet.
    asked: Option<String>,
    /// Saved pictures owed to apps: (app, path, time saved).
    waiting: Vec<(String, PathBuf, f64)>,
}

impl Shots {
    /// Records a request by `app`. Refused while another request is open or a capture session is on screen.
    pub fn ask(&mut self, app: &str, session_open: bool) -> Result<(), String> {
        if session_open || self.asked.is_some() {
            return Err("another screenshot is already being taken".into());
        }
        self.asked = Some(app.to_string());
        Ok(())
    }

    /// Drops the open request, for a request the shell refused to start.
    pub fn cancel(&mut self) {
        self.asked = None;
    }

    /// One tick. `saved` are the pictures the shell saved since the last tick and `session_open` says whether a capture
    /// session is still on screen. A request ends when its session closes: the pictures saved by then are owed to its
    /// app. Returns the (app, path) pairs whose files exist now; pictures not yet on disk wait, up to [`WAIT_SECS`].
    pub fn step(&mut self, saved: Vec<PathBuf>, session_open: bool, t: f64, landed: impl Fn(&Path) -> bool) -> Vec<(String, PathBuf)> {
        if let Some(app) = self.asked.clone() {
            if !session_open {
                self.asked = None;
                self.waiting.extend(saved.into_iter().map(|p| (app.clone(), p, t)));
            }
        }
        let mut ready = Vec::new();
        self.waiting.retain(|(app, path, since)| {
            if landed(path) {
                ready.push((app.clone(), path.clone()));
                false
            } else {
                t - since < WAIT_SECS
            }
        });
        ready
    }
}

/// The `captured` event for `path`.
fn captured(app: String, path: &Path) -> Event {
    Event { app: Some(app), event: CAPTURED.into(), body: Json::Obj(vec![("path".into(), Json::Str(path.display().to_string()))]) }
}

#[derive(Default)]
pub struct ScreenshotService {
    shots: Shots,
}

impl SystemService for ScreenshotService {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, _params: &Json, t: f64) -> Result<Json, String> {
        if !matches!(method, "selection" | "window" | "screen") {
            return Err(format!("screenshot has no method {method}"));
        }
        require_screen_recording(app_id)?;
        if !shell.screen_on || shell.is_locked() {
            return Err("locked".into());
        }
        self.shots.ask(app_id, shell.capture.is_some())?;
        let started = match method {
            "screen" => {
                shell.take_screenshot(t);
                Ok(())
            }
            "window" => shell.capture_request("window"),
            _ => shell.capture_request("select"),
        };
        if let Err(e) = started {
            self.shots.cancel();
            return Err(e);
        }
        Ok(Json::Obj(vec![("started".into(), Json::Bool(true))]))
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let saved = std::mem::take(&mut shell.shot_saved);
        let session_open = shell.capture.is_some();
        self.shots.step(saved, session_open, t, |p| p.exists()).into_iter().map(|(app, path)| captured(app, &path)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(format!("/pictures/{name}"))
    }

    #[test]
    fn a_second_request_is_refused_while_one_is_open_or_a_session_is_on_screen() {
        let mut s = Shots::default();
        assert!(s.ask("viewer", true).is_err(), "a capture session is on screen");
        s.ask("viewer", false).unwrap();
        assert!(s.ask("notes", false).is_err(), "one request at a time");
        s.cancel();
        assert!(s.ask("notes", false).is_ok(), "a cancelled request frees the slot");
    }

    #[test]
    fn the_picture_saved_by_a_selection_is_owed_to_the_app_that_asked_once_the_session_closes() {
        let mut s = Shots::default();
        s.ask("viewer", false).unwrap();
        // the session is still open: nothing is owed yet
        assert!(s.step(vec![], true, 0.0, |_| true).is_empty());
        // the person finishes: the save lands on the next tick
        let ready = s.step(vec![p("Screenshot-1.png")], false, 1.0, |_| true);
        assert_eq!(ready, vec![("viewer".to_string(), p("Screenshot-1.png"))]);
        // and it is delivered only once
        assert!(s.step(vec![], false, 2.0, |_| true).is_empty());
    }

    #[test]
    fn a_cancelled_selection_owes_nothing_and_a_toolbar_capture_is_not_the_apps() {
        let mut s = Shots::default();
        s.ask("viewer", false).unwrap();
        assert!(s.step(vec![], false, 1.0, |_| true).is_empty(), "cancelled: no picture");
        // a later toolbar capture while nothing is asked is dropped, not sent to an app
        assert!(s.step(vec![p("Screenshot-2.png")], false, 2.0, |_| true).is_empty());
    }

    #[test]
    fn a_picture_whose_file_has_not_landed_waits_then_is_given_up() {
        let mut s = Shots::default();
        s.ask("viewer", false).unwrap();
        assert!(s.step(vec![p("a.png")], false, 0.0, |_| false).is_empty(), "the file is still being written");
        assert_eq!(s.step(vec![], false, 1.0, |_| true).len(), 1, "it lands later");
        s.ask("viewer", false).unwrap();
        s.step(vec![p("b.png")], false, 5.0, |_| false);
        assert!(s.step(vec![], false, 5.0 + WAIT_SECS, |_| false).is_empty(), "a write that never lands is dropped");
    }

    #[test]
    fn the_captured_event_carries_the_path_for_the_app_only() {
        let e = captured("viewer".into(), Path::new("/pictures/x.png"));
        assert_eq!(e.app.as_deref(), Some("viewer"));
        assert_eq!(e.event, CAPTURED);
        assert_eq!(e.body.get("path").and_then(Json::as_str), Some("/pictures/x.png"));
    }
}

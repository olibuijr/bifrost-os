//! Screenshots over the shell's `screenshot` service (`springboard/src/shell/services/screenshot.rs`). An app asks for a
//! picture (no manifest permission; the person's screen-recording consent is asked on the first request); the shell saves it to Photos, as the screenshot toolbar does, and sends the
//! `captured` event with the file's path once the file is there. The app opens the path itself.
//!
//! ```ignore
//! screenshot::take(ctx, screenshot::Pick::Selection);   // or Window, Screen
//! // App::service_reply: screenshot::started_from_reply(&result)
//! // App::service_event: if let Some(path) = screenshot::captured_path(service, event, body) { open it }
//! ```
//! One request at a time: a second request, or a capture session already on screen, is refused. A selection or window
//! the person cancels sends no event. The picture is in the Photos library's folder, not the app's data folder.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use std::path::PathBuf;

/// The service name, as the shell registers it.
pub const SERVICE: &str = "screenshot";
/// The event a finished picture is sent with.
pub const CAPTURED: &str = "captured";

/// What a request takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A rectangle the person drags over the screen.
    Selection,
    /// The window under the pointer, with its shadow.
    Window,
    /// The whole screen, at once.
    Screen,
}

impl Pick {
    /// The service method for this pick.
    pub fn method(self) -> &'static str {
        match self {
            Pick::Selection => "selection",
            Pick::Window => "window",
            Pick::Screen => "screen",
        }
    }
}

/// Asks the shell for a picture. The reply is read with [`started_from_reply`]; the file arrives as [`captured_path`].
pub fn take(ctx: &Ctx, pick: Pick) -> u32 {
    service::call(ctx, SERVICE, pick.method(), &Json::Null)
}

/// Reads the reply to [`take`]: `Ok(())` once the shell has started the capture, else why it did not.
pub fn started_from_reply(result: &Result<Json, String>) -> Result<(), String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    match body.get("started") {
        Some(Json::Bool(true)) => Ok(()),
        _ => Err("screenshot: the reply does not say the capture started".into()),
    }
}

/// The file path from a `captured` event of the screenshot service; `None` for any other event.
pub fn captured_path(service: &str, event: &str, body: &Json) -> Option<PathBuf> {
    if service != SERVICE || event != CAPTURED {
        return None;
    }
    body.get("path").and_then(Json::as_str).filter(|p| !p.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Result<Json, String> {
        Ok(bifrost_core::json::parse(text).unwrap())
    }

    #[test]
    fn each_pick_calls_its_own_method() {
        assert_eq!(Pick::Selection.method(), "selection");
        assert_eq!(Pick::Window.method(), "window");
        assert_eq!(Pick::Screen.method(), "screen");
    }

    #[test]
    fn a_started_reply_is_ok_and_anything_else_is_an_error() {
        assert_eq!(started_from_reply(&reply(r#"{"started":true}"#)), Ok(()));
        assert!(started_from_reply(&reply(r#"{"started":false}"#)).is_err());
        assert!(started_from_reply(&reply(r#"{"stopped":true}"#)).is_err(), "another service's reply");
        assert_eq!(started_from_reply(&Err("locked".into())), Err("locked".into()));
    }

    #[test]
    fn the_captured_event_gives_the_path_and_nothing_else_does() {
        let body = reply(r#"{"path":"/home/u/Pictures/Screenshot-1.png"}"#).unwrap();
        assert_eq!(captured_path(SERVICE, CAPTURED, &body), Some(PathBuf::from("/home/u/Pictures/Screenshot-1.png")));
        assert_eq!(captured_path("speech", CAPTURED, &body), None, "another service");
        assert_eq!(captured_path(SERVICE, "other", &body), None, "another event");
        assert_eq!(captured_path(SERVICE, CAPTURED, &reply("{}").unwrap()), None, "no path");
    }
}

//! Developer-tools additions to the web view pool (`WebProcessPool`), used by the Browser's Web Inspector and its
//! control socket (`browser.*` methods; apps/browser/README.md "Web Inspector").
//!
//! The base pool in `webview.rs` stays as it is; this module adds, through the pool's `send_raw` and
//! `take_dev_lines` hooks, a token-correlated protocol with the engine's `devtools` module:
//!
//! app -> engine
//!   dev eval TOKEN ID JS              run JS in view ID (JS escaped like console text: `\n`, `\r`, `\\`)
//!   dev shot TOKEN ID MODE X Y W H MAXPX PATH
//!                                     PNG of the view into PATH. MODE view | rect (CSS px, viewport relative) | full
//!   dev collector JS                  a script the engine runs at the start of every document (network log)
//!   dev memory TOKEN                  native engine-wide memory report
//!   dev preferences TOKEN             effective native Servo preferences
//! engine -> app
//!   dev eval TOKEN ok TYPE JSON       TYPE undefined|null|boolean|number|string|array|object|element
//!   dev eval TOKEN err MESSAGE
//!   dev shot TOKEN ok W H | dev shot TOKEN err MESSAGE
//!   dev memory TOKEN ok JSON | dev preferences TOKEN ok JSON
//!                                     JSON is serialized directly; errors use escaped MESSAGE
//!
//! Desktop page input uses `mouse`, `text`, `key`, `chord`, and `wheel`; public coordinates are CSS pixels.

use bifrost_core::json::{self, Json};
use crate::webview::{WebProcessPool, WebViewId};

/// What a `dev screenshot` captures.
#[derive(Clone, Debug, PartialEq)]
pub enum ShotMode {
    /// the visible viewport
    View,
    /// a rectangle of the viewport, CSS pixels
    Rect { x: f32, y: f32, w: f32, h: f32 },
    /// the whole document, capped at `max_pixels`
    Full,
}

/// The value of a finished `dev_eval`.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalValue {
    /// `undefined`, `null`, `boolean`, `number`, `string`, `array`, `object` or `element`
    pub ty: String,
    /// the value as JSON text (`null` for undefined)
    pub json: String,
}

/// An answer to a `dev_*` request.
#[derive(Clone, Debug, PartialEq)]
pub enum DevEvent {
    Eval { token: u64, result: Result<EvalValue, String> },
    /// the PNG is at the path given in the request; `(width, height)` in pixels
    Shot { token: u64, result: Result<(u32, u32), String> },
    /// Servo's native report covers the engine and its content processes, without evaluating a page script.
    Memory { token: u64, result: Result<Json, String> },
    /// The effective native Servo preferences, including startup and runtime overrides.
    Preferences { token: u64, result: Result<Json, String> },
}

/// Escapes `\n`, `\r` and `\\` for the line protocol, with no length cap.
pub fn escape_full(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// Parses one `dev ...` line (without the leading `dev `) into an event.
pub fn parse_dev_line(line: &str) -> Option<DevEvent> {
    let mut f = line.splitn(4, ' ');
    let verb = f.next()?;
    let token: u64 = f.next()?.parse().ok()?;
    let status = f.next()?;
    let rest = f.next().unwrap_or("");
    match (verb, status) {
        ("eval", "ok") => {
            let (ty, json) = rest.split_once(' ').unwrap_or((rest, "null"));
            Some(DevEvent::Eval { token, result: Ok(EvalValue { ty: ty.to_string(), json: crate::webview::unescape_line(json) }) })
        }
        ("eval", _) => Some(DevEvent::Eval { token, result: Err(crate::webview::unescape_line(rest)) }),
        ("shot", "ok") => {
            let (w, h) = rest.split_once(' ')?;
            Some(DevEvent::Shot { token, result: Ok((w.parse().ok()?, h.parse().ok()?)) })
        }
        ("shot", _) => Some(DevEvent::Shot { token, result: Err(crate::webview::unescape_line(rest)) }),
        ("memory" | "preferences", _) => {
            // Native diagnostics serialize JSON directly rather than applying the eval line escaping.
            // Preserve JSON escapes (including backslashes in preference strings) for the JSON parser.
            let result = if status == "ok" {
                json::parse(rest).map_err(|error| format!("invalid {verb} response: {error}"))
            } else {
                Err(crate::webview::unescape_line(rest))
            };
            Some(if verb == "memory" { DevEvent::Memory { token, result } } else { DevEvent::Preferences { token, result } })
        }
        _ => None,
    }
}

impl WebProcessPool {
    /// Requests Servo's native engine-wide memory report; answered as `DevEvent::Memory`.
    /// Does not create, select, or revive a web view.
    pub fn dev_memory(&mut self, token: u64) {
        self.send_raw(&format!("dev memory {token}"));
    }

    /// Requests the effective Servo preferences; answered as `DevEvent::Preferences`.
    pub fn dev_preferences(&mut self, token: u64) {
        self.send_raw(&format!("dev preferences {token}"));
    }

    /// `evaluateJavaScript:` with a correlation token: the answer comes back from `dev_events` as `DevEvent::Eval`
    /// with the same token (including the page's thrown error text).
    pub fn dev_eval(&mut self, id: WebViewId, token: u64, js: &str) {
        self.send_raw(&format!("dev eval {token} {id} {}", escape_full(js)));
    }

    /// Takes a PNG of view `id` into `path`; the answer is `DevEvent::Shot`. `max_pixels` caps `ShotMode::Full`.
    pub fn dev_screenshot(&mut self, id: WebViewId, token: u64, mode: &ShotMode, max_pixels: u64, path: &str) {
        let (m, x, y, w, h) = match mode {
            ShotMode::View => ("view", 0.0, 0.0, 0.0, 0.0),
            ShotMode::Rect { x, y, w, h } => ("rect", *x, *y, *w, *h),
            ShotMode::Full => ("full", 0.0, 0.0, 0.0, 0.0),
        };
        self.send_raw(&format!("dev shot {token} {id} {m} {x} {y} {w} {h} {max_pixels} {path}"));
    }

    /// Tells the engine the app abandoned a screenshot: it restores view `id` to the window's rendering size.
    pub fn dev_screenshot_abort(&mut self, id: WebViewId) {
        self.send_raw(&format!("dev shotabort {id}"));
    }

    /// Installs a script that runs at the start of every document the engine loads from now on (the network log).
    pub fn dev_collector(&mut self, js: &str) {
        self.send_raw(&format!("dev collector {}", escape_full(js)));
    }

    /// The answers received since the last call.
    pub fn dev_events(&mut self) -> Vec<DevEvent> {
        self.take_dev_lines().iter().filter_map(|l| parse_dev_line(l)).collect()
    }

    /// The engine's process id.
    pub fn dev_pid(&self) -> u32 {
        self.engine_pid()
    }

    /// A left-button press or release (`kind` "d" or "u") or a move ("m") at page-local CSS pixels.
    pub fn dev_pointer(&mut self, kind: &str, x: f32, y: f32) {
        let k = match kind {
            "d" | "u" | "m" => kind,
            _ => return,
        };
        let s = self.device_scale();
        self.send_raw(&format!("mouse {k} {} {}", x * s, y * s));
    }

    /// Scrolls by `dx, dy` CSS pixels at page-local CSS pixels `(x, y)`.
    pub fn dev_wheel(&mut self, dx: f32, dy: f32, x: f32, y: f32) {
        let s = self.device_scale();
        self.send_raw(&format!("wheel {} {} {} {}", dx * s, dy * s, x * s, y * s));
    }

    /// Typed text (each character as a key press).
    pub fn dev_text(&mut self, text: &str) {
        self.send_raw(&format!("text {text}"));
    }

    /// A named key (`Enter`, `Backspace`, `Tab`, `Escape`, `Delete`, `Arrow*`, `Home`, `End`, `PageUp`, `PageDown`).
    pub fn dev_key(&mut self, name: &str) {
        self.send_raw(&format!("key {name}"));
    }

    /// A character with modifiers (bits: 1 ctrl, 2 shift, 4 alt, 8 meta).
    pub fn dev_chord(&mut self, mods: u32, ch: char) {
        self.send_raw(&format!("chord {mods} {ch}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_replies_parse() {
        assert_eq!(
            parse_dev_line("eval 7 ok string \"a\\nb\""),
            Some(DevEvent::Eval { token: 7, result: Ok(EvalValue { ty: "string".into(), json: "\"a\nb\"".into() }) })
        );
        assert_eq!(parse_dev_line("eval 8 err ReferenceError: x"), Some(DevEvent::Eval { token: 8, result: Err("ReferenceError: x".into()) }));
        assert_eq!(parse_dev_line("shot 9 ok 640 480"), Some(DevEvent::Shot { token: 9, result: Ok((640, 480)) }));
        assert_eq!(parse_dev_line("shot 9 err nope"), Some(DevEvent::Shot { token: 9, result: Err("nope".into()) }));
        assert_eq!(parse_dev_line("eval x ok"), None);
    }

    #[test]
    fn escape_round_trips_without_a_cap() {
        let long = "a\nb\\c".repeat(2000);
        assert_eq!(crate::webview::unescape_line(&escape_full(&long)), long);
    }
}

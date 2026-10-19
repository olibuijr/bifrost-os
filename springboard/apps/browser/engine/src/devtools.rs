//! Engine side of the Browser's developer-tools control (`browser.*` methods on the app control socket;
//! apps/browser/README.md "Web Inspector"). Every request carries a token the app chose; every answer repeats it.
//!
//! app -> engine (the verb `dev`, handled by `Engine::dev_command`)
//!   dev eval TOKEN ID JS              JS escaped like console text (`\n`, `\r`, `\\`), no length cap
//!   dev shot TOKEN ID MODE X Y W H MAXPX PATH
//!                                     PNG of the view written to PATH. MODE view | rect (CSS px, relative to the
//!                                     viewport, X Y W H) | full (viewport width x document height, at most MAXPX pixels)
//!   dev collector JS                  a script run at the start of every document loaded from now on (network log)
//!   dev memory TOKEN                  native Servo memory report for all engine processes
//! engine -> app
//!   dev eval TOKEN ok TYPE JSON       the value as JSON (TYPE undefined|null|boolean|number|string|array|object|element)
//!   dev eval TOKEN err MESSAGE        the page threw, or the script did not compile
//!   dev shot TOKEN ok W H | dev shot TOKEN err MESSAGE
//!
//! Call sites this module needs in `main.rs` (the stability track adds them):
//!   1. `mod devtools;` next to the other items, and in `Engine::command`: `"dev" => self.dev_command(rest),`
//!   2. in `Engine::build`, first line: `let builder = devtools::collector_builder(&self.servo, builder);`
//!      (installs the network collector script in every web view; without it the app injects the collector after
//!      the document head is parsed, which misses the very first requests of a page)
//!
//! Servo 0.7.0 facts this relies on (read in out/servo-source/servo-0.7.0): `WebView::evaluate_javascript` calls back
//! with `Result<JSValue, JavaScriptEvaluationError>` and does not await promises; `WebView::take_screenshot(rect, cb)`
//! reads the `RenderingContext` once layout and paint are up to date, `WebViewRect::Page` is in CSS pixels of the
//! viewport; `UserContentManager::add_script` runs a `UserScript` in every document of the views that share it.

use std::cell::RefCell;
use std::rc::Rc;

use super::{Engine, MAX_DIM, say};
use dpi::PhysicalSize;
use servo::{CSSPixel, JSValue, JavaScriptEvaluationError, Servo, UserContentManager, UserScript, WebViewBuilder, WebViewRect};

/// Longest reply the engine sends, bytes.
const MAX_REPLY: usize = 16 * 1024 * 1024;

thread_local! {
    static COLLECTOR: RefCell<Option<String>> = const { RefCell::new(None) };
    static MANAGER: RefCell<Option<Rc<UserContentManager>>> = const { RefCell::new(None) };
    /// App scripts by slot name (`userscript NAME JS`), so a slot can be replaced or removed.
    static SLOTS: RefCell<std::collections::HashMap<String, Rc<UserScript>>> = RefCell::new(std::collections::HashMap::new());
}

/// Most script slots the app may hold.
const MAX_SLOTS: usize = 8;

/// `userscript NAME JS`: keeps JS (escaped like `dev` text) as the document-start script `NAME` in every web view.
/// An existing slot is replaced; empty JS removes it.
pub(super) fn set_user_script(servo: &Servo, rest: &str) {
    let (name, js) = rest.split_once(' ').unwrap_or((rest, ""));
    if name.is_empty() || name.len() > 32 {
        return;
    }
    let Some(mgr) = manager(servo) else { return };
    if let Some(old) = SLOTS.with(|s| s.borrow_mut().remove(name)) {
        mgr.remove_script(old);
    }
    let js = unesc(js);
    if js.trim().is_empty() || SLOTS.with(|s| s.borrow().len() >= MAX_SLOTS) {
        return;
    }
    let script = Rc::new(UserScript::new(js, None));
    mgr.add_script(script.clone());
    SLOTS.with(|s| s.borrow_mut().insert(name.to_string(), script));
}

/// Escapes `\n`, `\r` and `\\`, with no length cap.
pub(super) fn esc(s: &str) -> String {
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

/// The inverse of `esc`.
fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn js_type(v: &JSValue) -> &'static str {
    match v {
        JSValue::Undefined => "undefined",
        JSValue::Null => "null",
        JSValue::Boolean(_) => "boolean",
        JSValue::Number(_) => "number",
        JSValue::String(_) => "string",
        JSValue::Array(_) => "array",
        JSValue::Object(_) => "object",
        _ => "element",
    }
}

/// Map Servo's tagged values into protocol JSON; serde_json owns escaping and number formatting.
fn js_json(value: &JSValue, depth: usize) -> serde_json::Value {
    use serde_json::Value;
    if depth > 32 { return Value::Null; }
    match value {
        JSValue::Undefined | JSValue::Null => Value::Null,
        JSValue::Boolean(value) => Value::Bool(*value),
        JSValue::Number(value) => serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number),
        JSValue::String(value) | JSValue::Element(value) | JSValue::ShadowRoot(value) | JSValue::Frame(value) | JSValue::Window(value) => Value::String(value.clone()),
        JSValue::Array(values) => Value::Array(values.iter().map(|value| js_json(value, depth + 1)).collect()),
        JSValue::Object(values) => Value::Object(values.iter().map(|(key, value)| (key.clone(), js_json(value, depth + 1))).collect()),
    }
}

fn error_text(e: &JavaScriptEvaluationError) -> String {
    use JavaScriptEvaluationError as E;
    match e {
        E::EvaluationFailure(Some(info)) => {
            if info.line_number > 0 {
                format!("{} ({}:{}:{})", info.message, info.filename, info.line_number, info.column)
            } else {
                info.message.clone()
            }
        }
        E::EvaluationFailure(None) => "evaluation failed".to_string(),
        E::CompilationFailure => "SyntaxError: the script could not be compiled".to_string(),
        E::DocumentNotFound => "the document no longer exists".to_string(),
        E::WebViewNotReady => "the web view is not ready (a load is changing the document)".to_string(),
        E::InternalError => "internal engine error".to_string(),
        E::SerializationError(e) => format!("the result cannot be serialized: {e:?}"),
    }
}

/// The one-line reply to a finished `dev eval`.
fn eval_reply(token: &str, r: &Result<JSValue, JavaScriptEvaluationError>) -> String {
    match r {
        Ok(v) => {
            let j = js_json(v, 0).to_string();
            if j.len() > MAX_REPLY {
                return format!("dev eval {token} err result larger than {MAX_REPLY} bytes");
            }
            format!("dev eval {token} ok {} {}", js_type(v), esc(&j))
        }
        Err(e) => format!("dev eval {token} err {}", esc(&error_text(e))),
    }
}

/// The user content manager that runs the network collector in every web view (None until the app sent `dev collector`).
fn manager(servo: &Servo) -> Option<Rc<UserContentManager>> {
    MANAGER.with(|m| {
        if m.borrow().is_none() {
            let mgr = Rc::new(UserContentManager::new(servo));
            if let Some(js) = COLLECTOR.with(|c| c.borrow().clone()) {
                mgr.add_script(Rc::new(UserScript::new(js, None)));
            }
            *m.borrow_mut() = Some(mgr);
        }
        m.borrow().clone()
    })
}

/// Adds the collector script to a web view being built (see the module docs, call site 2).
pub(super) fn collector_builder(servo: &Servo, builder: WebViewBuilder) -> WebViewBuilder {
    match manager(servo) {
        Some(m) => builder.user_content_manager(m),
        None => builder,
    }
}

fn write_png(img: &servo::RgbaImage, path: &str) -> Result<(u32, u32), String> {
    img.save(path).map_err(|e| format!("cannot write {path}: {e}"))?;
    Ok((img.width(), img.height()))
}

impl Engine {
    /// Handles one `dev ...` line (without the verb `dev`).
    pub(super) fn dev_command(&mut self, rest: &str) {
        let (verb, tail) = rest.split_once(' ').unwrap_or((rest, ""));
        match verb {
            "eval" => self.dev_eval(tail),
            "preferences" => {
                let token = tail.split(' ').next().unwrap_or("0");
                let preferences = serde_json::to_value(&*servo::prefs::get()).map(|mut preferences| {
                    // Embedder diagnostics are namespaced separately from native Servo preferences.
                    preferences.as_object_mut().expect("Servo preferences serialize as an object").insert(
                        "_bifrost_renderer".into(),
                        serde_json::json!({"mode": self.renderer_mode, "driver": self.renderer_driver}),
                    );
                    preferences.to_string()
                });
                match preferences {
                    Ok(json) => say(&format!("dev preferences {token} ok {json}")),
                    Err(error) => say(&format!("dev preferences {token} err {}", esc(&error.to_string()))),
                }
            }
            "memory" => {
                let token = tail.split(' ').next().unwrap_or("0").to_string();
                let error_token = token.clone();
                let callback = servo_base::generic_channel::GenericCallback::new(move |result| {
                    match result {
                        Ok(report) => match serde_json::to_string(&report) {
                            Ok(json) => say(&format!("dev memory {token} ok {json}")),
                            Err(error) => say(&format!("dev memory {token} err {}", esc(&error.to_string()))),
                        },
                        Err(error) => say(&format!("dev memory {token} err {}", esc(&format!("{error:?}")))),
                    }
                });
                match callback {
                    Ok(callback) => self.servo.create_memory_report(callback),
                    Err(error) => say(&format!("dev memory {error_token} err {}", esc(&format!("{error:?}")))),
                }
            }
            "shot" => self.dev_shot(tail),
            "shotabort" => self.dev_shot_abort(tail),
            "collector" => COLLECTOR.with(|c| *c.borrow_mut() = Some(unesc(tail))),
            _ => {}
        }
    }

    fn dev_eval(&mut self, tail: &str) {
        let mut f = tail.splitn(3, ' ');
        let token = f.next().unwrap_or("0").to_string();
        let id: u32 = f.next().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
        let js = unesc(f.next().unwrap_or(""));
        let Some(v) = self.views.get(&id) else {
            say(&format!("dev eval {token} err no such view"));
            return;
        };
        v.evaluate_javascript(js, move |r| say(&eval_reply(&token, &r)));
    }

    /// The app gave up on a screenshot (timeout): put the view back at the window's size whatever state a
    /// full-page capture left it in. Resizes through `WebView::resize` only.
    fn dev_shot_abort(&mut self, tail: &str) {
        let id: u32 = tail.split(' ').next().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
        if let Some(v) = self.views.get(&id) {
            v.resize(self.size);
        }
    }

    fn dev_shot(&mut self, tail: &str) {
        let mut f = tail.splitn(9, ' ');
        let token = f.next().unwrap_or("0").to_string();
        let id: u32 = f.next().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
        let mode = f.next().unwrap_or("view").to_string();
        let n: Vec<f32> = (0..4).map(|_| f.next().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0)).collect();
        let max_px: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(16_000_000).clamp(1, 64_000_000);
        let path = f.next().unwrap_or("").to_string();
        let Some(v) = self.views.get(&id).cloned() else {
            say(&format!("dev shot {token} err no such view"));
            return;
        };
        if path.is_empty() {
            say(&format!("dev shot {token} err no output path"));
            return;
        }
        let finish = move |t: String, p: String, r: Result<servo::RgbaImage, servo::ScreenshotCaptureError>, restore: Option<(servo::WebView, PhysicalSize<u32>)>| {
            if let Some((view, size)) = restore {
                view.resize(size);
            }
            match r {
                Ok(img) => match write_png(&img, &p) {
                    Ok((w, h)) => say(&format!("dev shot {t} ok {w} {h}")),
                    Err(e) => say(&format!("dev shot {t} err {}", esc(&e))),
                },
                Err(e) => say(&format!("dev shot {t} err {}", esc(&format!("screenshot failed: {e:?}")))),
            }
        };
        match mode.as_str() {
            "rect" => {
                let (x, y, w, h) = (n[0], n[1], n[2].max(1.0), n[3].max(1.0));
                let rect = WebViewRect::Page(euclid::Box2D::<f32, CSSPixel>::new(euclid::Point2D::new(x, y), euclid::Point2D::new(x + w, y + h)));
                v.take_screenshot(Some(rect), move |r| finish(token, path, r, None));
            }
            "full" => {
                let orig = self.size;
                let dpr = self.scale;
                let probe = "Math.max(document.documentElement.scrollHeight,document.body?document.body.scrollHeight:0)";
                let view = v.clone();
                v.evaluate_javascript(probe, move |r| {
                    let css_h = match r {
                        Ok(JSValue::Number(h)) if h.is_finite() && h >= 1.0 => h,
                        _ => {
                            say(&format!("dev shot {token} err could not measure the document"));
                            return;
                        }
                    };
                    let w = orig.width;
                    let mut h = ((css_h as f32) * dpr).ceil().max(orig.height as f32) as u64;
                    h = h.min(MAX_DIM as u64).min((max_px / w.max(1) as u64).max(orig.height as u64));
                    let size = PhysicalSize::new(w, h as u32);
                    view.resize(size);
                    let restore = (view.clone(), orig);
                    view.take_screenshot(None, move |r| finish(token, path, r, Some(restore)));
                });
            }
            _ => v.take_screenshot(None, move |r| finish(token, path, r, None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn escaping_round_trips() {
        let s = "a\nb\\c\r\"";
        assert_eq!(unesc(&esc(s)), s);
        assert!(!esc(s).contains('\n'));
    }

    #[test]
    fn values_become_json() {
        let mut m = HashMap::new();
        m.insert("b".to_string(), JSValue::Number(1.5));
        m.insert("a".to_string(), JSValue::Array(vec![JSValue::Null, JSValue::Boolean(true), JSValue::String("x\"y".into())]));
        let out = js_json(&JSValue::Object(m), 0).to_string();
        assert_eq!(out, r#"{"a":[null,true,"x\"y"],"b":1.5}"#);
        assert_eq!(js_type(&JSValue::Undefined), "undefined");
        let n = js_json(&JSValue::Number(f64::NAN), 0).to_string();
        assert_eq!(n, "null");
    }

    #[test]
    fn eval_replies_carry_the_token() {
        let ok = eval_reply("7", &Ok(JSValue::String("hi\nthere".into())));
        assert_eq!(ok, r#"dev eval 7 ok string "hi\\nthere""#);
        let err = eval_reply("8", &Err(JavaScriptEvaluationError::CompilationFailure));
        assert!(err.starts_with("dev eval 8 err SyntaxError"));
    }
}

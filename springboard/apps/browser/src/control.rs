//! The Browser's `browser.*` control methods, pure layer: parameter validation, JS snippets, result shaping.
//! Nothing here touches the engine or the app state (`browser_ctl.rs` runs the parsed requests), so every method
//! is unit-testable with params in and Json out (tests/control.rs includes this file).
//!
//! Errors are JSON-RPC errors: -32601 unknown method, -32602 invalid params, -32000 timeout, -32001 no such tab,
//! -32002 engine not running (with `data.reason`).
use std::collections::VecDeque;

use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::{self, Json};

/// Server error: no tab has that id.
pub const NO_TAB: i64 = -32001;
/// Server error: the web engine is not running.
pub const ENGINE_DOWN: i64 = -32002;

/// Default and largest `timeout_ms` of an async call.
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
pub const MAX_TIMEOUT_MS: u64 = 60_000;
/// Largest screenshot reply, bytes.
pub const MAX_SHOT_BYTES: usize = 16 * 1024 * 1024;
/// Default pixel cap of a full-page screenshot.
pub const MAX_SHOT_PIXELS: u64 = 16_000_000;

/// Every method, as `app.methods` lists them. `console` is kept as an alias of `browser.console`.
pub const METHODS: &[&str] = &[
    "console",
    "browser.state",
    "browser.tabs",
    "browser.tab.new",
    "browser.tab.select",
    "browser.tab.close",
    "browser.tab.reopen",
    "browser.navigate",
    "browser.back",
    "browser.forward",
    "browser.reload",
    "browser.eval",
    "browser.console",
    "browser.dom",
    "browser.screenshot",
    "browser.network",
    "browser.wait",
    "browser.viewport",
    "browser.zoom",
    "browser.find",
    "browser.user_agent",
    "browser.input",
    "browser.devtools",
    "browser.source",
    "browser.inspect",
    "browser.downloads",
    "browser.bookmarks",
    "browser.history",
    "browser.favicon",
];

// ---- small Json helpers ----

pub fn num(n: f64) -> Json {
    Json::Num(n)
}
pub fn text(s: &str) -> Json {
    Json::Str(s.to_string())
}
pub fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn invalid(msg: impl Into<String>) -> RpcError {
    RpcError::new(INVALID_PARAMS, msg)
}
pub fn no_tab(id: u32) -> RpcError {
    RpcError::new(NO_TAB, format!("no tab with id {id}"))
}
pub fn engine_down(reason: &str) -> RpcError {
    RpcError::new(ENGINE_DOWN, "engine not running").with_reason(reason)
}

// ---- parameters ----

fn field<'a>(p: &'a Json, key: &str) -> Option<&'a Json> {
    match p.get(key) {
        Some(Json::Null) | None => None,
        Some(v) => Some(v),
    }
}

fn opt_str(p: &Json, key: &str) -> Result<Option<String>, RpcError> {
    match field(p, key) {
        None => Ok(None),
        Some(Json::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(format!("{key} must be a string"))),
    }
}

fn req_str(p: &Json, key: &str) -> Result<String, RpcError> {
    match opt_str(p, key)? {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(invalid(format!("{key} is required (a non-empty string)"))),
    }
}

fn opt_num(p: &Json, key: &str) -> Result<Option<f64>, RpcError> {
    match field(p, key) {
        None => Ok(None),
        Some(Json::Num(n)) if n.is_finite() => Ok(Some(*n)),
        Some(_) => Err(invalid(format!("{key} must be a number"))),
    }
}

fn opt_bool(p: &Json, key: &str) -> Result<Option<bool>, RpcError> {
    match field(p, key) {
        None => Ok(None),
        Some(Json::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(invalid(format!("{key} must be true or false"))),
    }
}

fn opt_uint(p: &Json, key: &str, max: u64) -> Result<Option<u64>, RpcError> {
    match opt_num(p, key)? {
        None => Ok(None),
        Some(n) if n >= 0.0 && n.fract() == 0.0 && n <= max as f64 => Ok(Some(n as u64)),
        Some(_) => Err(invalid(format!("{key} must be an integer from 0 to {max}"))),
    }
}

/// The optional tab id of a call.
pub fn tab_id(p: &Json) -> Result<Option<u32>, RpcError> {
    Ok(opt_uint(p, "id", u32::MAX as u64)?.map(|v| v as u32))
}

fn req_tab(p: &Json) -> Result<u32, RpcError> {
    tab_id(p)?.ok_or_else(|| invalid("id is required"))
}

/// `timeout_ms`: default 10000, 1 to 60000.
pub fn timeout_ms(p: &Json) -> Result<u64, RpcError> {
    match opt_uint(p, "timeout_ms", u64::MAX / 2)? {
        None => Ok(DEFAULT_TIMEOUT_MS),
        Some(0) => Err(invalid("timeout_ms must be at least 1")),
        Some(t) if t > MAX_TIMEOUT_MS => Err(invalid(format!("timeout_ms must be at most {MAX_TIMEOUT_MS}"))),
        Some(t) => Ok(t),
    }
}

pub const LEVELS: [&str; 7] = ["error", "warn", "info", "log", "debug", "input", "result"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wait {
    None,
    Commit,
    Load,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DomMode {
    OuterHtml,
    Text,
    Attrs,
    Query,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WaitFor {
    Load,
    Selector(String),
    Js(String),
    Text(String),
    Console(Option<String>),
    Idle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Click { x: f32, y: f32 },
    Move { x: f32, y: f32 },
    Key { key: String, mods: u32 },
    Text(String),
    /// `x`, `y` are -1 when not given: the centre of the viewport
    Scroll { dx: f32, dy: f32, x: f32, y: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FindDir {
    New,
    Back,
    Forward,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Console,
    Network,
    Elements,
    Sources,
    Storage,
}

impl Panel {
    pub fn name(self) -> &'static str {
        match self {
            Panel::Console => "console",
            Panel::Network => "network",
            Panel::Elements => "elements",
            Panel::Sources => "sources",
            Panel::Storage => "storage",
        }
    }
    pub fn parse(s: &str) -> Option<Panel> {
        match s {
            "console" => Some(Panel::Console),
            "network" => Some(Panel::Network),
            "elements" => Some(Panel::Elements),
            "sources" => Some(Panel::Sources),
            "storage" => Some(Panel::Storage),
            _ => None,
        }
    }
}

/// A parsed, validated request.
#[derive(Clone, Debug, PartialEq)]
pub enum Req {
    State,
    Tabs,
    TabNew { url: Option<String>, background: bool },
    TabSelect { id: u32 },
    TabClose { id: u32 },
    TabReopen,
    Navigate { id: Option<u32>, url: String, wait: Wait, timeout_ms: u64 },
    Back { id: Option<u32> },
    Forward { id: Option<u32> },
    Reload { id: Option<u32>, hard: bool },
    Eval { id: Option<u32>, js: String, timeout_ms: u64 },
    Console { id: Option<u32>, level: Option<String>, limit: usize, since_seq: Option<u64>, clear: bool },
    Dom { id: Option<u32>, selector: String, mode: DomMode, limit: usize, max_bytes: usize, timeout_ms: u64 },
    Screenshot { id: Option<u32>, full_page: bool, selector: Option<String>, rect: Option<(f32, f32, f32, f32)>, scale: f32, max_pixels: u64, timeout_ms: u64 },
    Network { id: Option<u32>, since_seq: Option<u64>, limit: usize, clear: bool },
    Wait { id: Option<u32>, what: WaitFor, timeout_ms: u64 },
    Viewport { size: Option<(u32, u32)>, dpr: Option<f32>, reset: bool },
    Zoom { factor: Option<f32> },
    Find { id: Option<u32>, query: String, dir: FindDir },
    UserAgent { ua: Option<String> },
    Input { id: Option<u32>, input: Input },
    Devtools { open: Option<bool>, panel: Option<Panel> },
    Source { id: Option<u32>, timeout_ms: u64 },
    /// the element at a page point (CSS px of the viewport), selected in the Elements panel and highlighted
    Inspect { id: Option<u32>, x: f32, y: f32, timeout_ms: u64 },
    /// List downloads; `cancel` stops one by id, `clear` drops finished entries from the list.
    Downloads { cancel: Option<u32>, clear: bool },
    /// Bookmarks: `op` is list (default), add (url defaults to the current page), remove, rename or import (`path` is a
    /// bookmark export in HTML form).
    Bookmarks { op: String, url: Option<String>, title: Option<String>, query: Option<String>, limit: usize, path: Option<String> },
    /// History, newest first; `remove` deletes one URL, `clear` deletes everything, `clear_since` (epoch seconds)
    /// deletes the entries last visited at or after that time.
    History { query: Option<String>, limit: usize, remove: Option<String>, clear: bool, clear_since: Option<u64> },
    Favicon { id: Option<u32> },
}

/// CSS size and device pixel ratio of a viewport preset (`mobile`, `bifrost`, `desktop`).
pub fn preset_size(name: &str) -> Option<(u32, u32, f32)> {
    match name.to_ascii_lowercase().as_str() {
        "mobile" => Some((393, 852, 2.0)),
        "bifrost" => Some((820, 1180, 2.0)),
        "desktop" => Some((1280, 800, 1.0)),
        _ => None,
    }
}

fn parse_mods(p: &Json) -> Result<u32, RpcError> {
    match field(p, "mods") {
        None => Ok(0),
        Some(Json::Arr(items)) => {
            let mut m = 0;
            for it in items {
                m |= match it.as_str() {
                    Some("ctrl") | Some("control") => 1,
                    Some("shift") => 2,
                    Some("alt") | Some("option") => 4,
                    Some("meta") | Some("cmd") | Some("command") => 8,
                    _ => return Err(invalid("mods entries are ctrl, shift, alt or meta")),
                };
            }
            Ok(m)
        }
        Some(_) => Err(invalid("mods must be an array of ctrl, shift, alt, meta")),
    }
}

/// Keys the engine accepts by name (`key` verb).
pub const NAMED_KEYS: [&str; 13] = [
    "Enter", "Backspace", "Tab", "Escape", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown",
];

fn parse_input(p: &Json) -> Result<Input, RpcError> {
    let kind = req_str(p, "kind")?;
    let xy = |need: bool| -> Result<(f32, f32), RpcError> {
        let (x, y) = (opt_num(p, "x")?, opt_num(p, "y")?);
        match (x, y) {
            (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => Ok((x as f32, y as f32)),
            (None, None) if !need => Ok((-1.0, -1.0)), // -1: the centre of the viewport
            _ => Err(invalid("x and y are required (page-local CSS pixels, not negative)")),
        }
    };
    match kind.as_str() {
        "click" => {
            let (x, y) = xy(true)?;
            if let Some(b) = opt_str(p, "button")? {
                if b != "left" {
                    return Err(invalid("only the left button is supported (page input goes through the touch path)"));
                }
            }
            Ok(Input::Click { x, y })
        }
        "move" => {
            let (x, y) = xy(true)?;
            Ok(Input::Move { x, y })
        }
        "text" => Ok(Input::Text(req_str(p, "text")?)),
        "key" => {
            let key = req_str(p, "key")?;
            let mods = parse_mods(p)?;
            if key.chars().count() != 1 && !NAMED_KEYS.contains(&key.as_str()) {
                return Err(invalid(format!("key must be one character or one of {}", NAMED_KEYS.join(", "))));
            }
            Ok(Input::Key { key, mods })
        }
        "scroll" => {
            let (dx, dy) = (opt_num(p, "dx")?.unwrap_or(0.0), opt_num(p, "dy")?.unwrap_or(0.0));
            if dx == 0.0 && dy == 0.0 {
                return Err(invalid("scroll needs dx or dy"));
            }
            let (x, y) = xy(false)?;
            Ok(Input::Scroll { dx: dx as f32, dy: dy as f32, x, y })
        }
        _ => Err(invalid("kind must be click, move, key, text or scroll")),
    }
}

/// Validates `method` and `params`. `METHOD_NOT_FOUND` for an unknown method, `INVALID_PARAMS` for bad parameters.
pub fn parse(method: &str, p: &Json) -> Result<Req, RpcError> {
    if !matches!(p, Json::Obj(_) | Json::Null) {
        return Err(invalid("params must be an object"));
    }
    let method = if method == "console" { "browser.console" } else { method };
    Ok(match method {
        "browser.state" => Req::State,
        "browser.tabs" => Req::Tabs,
        "browser.tab.new" => Req::TabNew { url: opt_str(p, "url")?, background: opt_bool(p, "background")?.unwrap_or(false) },
        "browser.tab.select" => Req::TabSelect { id: req_tab(p)? },
        "browser.tab.close" => Req::TabClose { id: req_tab(p)? },
        "browser.tab.reopen" => Req::TabReopen,
        "browser.navigate" => {
            let url = req_str(p, "url")?;
            let wait = match opt_str(p, "wait")?.as_deref() {
                None | Some("load") => Wait::Load,
                Some("commit") => Wait::Commit,
                Some("none") => Wait::None,
                Some(_) => return Err(invalid("wait must be none, commit or load")),
            };
            Req::Navigate { id: tab_id(p)?, url, wait, timeout_ms: timeout_ms(p)? }
        }
        "browser.back" => Req::Back { id: tab_id(p)? },
        "browser.forward" => Req::Forward { id: tab_id(p)? },
        "browser.reload" => Req::Reload { id: tab_id(p)?, hard: opt_bool(p, "hard")?.unwrap_or(false) },
        "browser.eval" => Req::Eval { id: tab_id(p)?, js: req_str(p, "js")?, timeout_ms: timeout_ms(p)? },
        "browser.console" => {
            let level = opt_str(p, "level")?;
            if let Some(l) = &level {
                if !LEVELS.contains(&l.as_str()) {
                    return Err(invalid(format!("level must be one of {}", LEVELS.join(", "))));
                }
            }
            Req::Console {
                id: tab_id(p)?,
                level,
                limit: opt_uint(p, "limit", 100_000)?.unwrap_or(200) as usize,
                since_seq: opt_uint(p, "since_seq", u64::MAX / 2)?,
                clear: opt_bool(p, "clear")?.unwrap_or(false),
            }
        }
        "browser.dom" => {
            let mode = match opt_str(p, "mode")?.as_deref() {
                None | Some("outer_html") => DomMode::OuterHtml,
                Some("text") => DomMode::Text,
                Some("attrs") => DomMode::Attrs,
                Some("query") => DomMode::Query,
                Some(_) => return Err(invalid("mode must be outer_html, text, attrs or query")),
            };
            // `query` is the natural default when the caller asks for a list of elements
            let limit = opt_uint(p, "limit", 5000)?.unwrap_or(200) as usize;
            let max_bytes = opt_uint(p, "max_bytes", 16 * 1024 * 1024)?.unwrap_or(200_000) as usize;
            Req::Dom { id: tab_id(p)?, selector: opt_str(p, "selector")?.filter(|s| !s.is_empty()).unwrap_or_else(|| "html".into()), mode, limit, max_bytes, timeout_ms: timeout_ms(p)? }
        }
        "browser.screenshot" => {
            let full_page = opt_bool(p, "full_page")?.unwrap_or(false);
            let selector = opt_str(p, "selector")?.filter(|s| !s.is_empty());
            let rect = match field(p, "rect") {
                None => None,
                Some(r @ Json::Obj(_)) => {
                    let g = |k: &str| -> Result<f32, RpcError> {
                        match r.get(k) {
                            Some(Json::Num(n)) if n.is_finite() => Ok(*n as f32),
                            _ => Err(invalid("rect needs numeric x, y, w, h")),
                        }
                    };
                    let (x, y, w, h) = (g("x")?, g("y")?, g("w")?, g("h")?);
                    if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 {
                        return Err(invalid("rect needs x, y >= 0 and w, h > 0"));
                    }
                    Some((x, y, w, h))
                }
                Some(_) => return Err(invalid("rect must be an object {x, y, w, h}")),
            };
            if [full_page, selector.is_some(), rect.is_some()].iter().filter(|b| **b).count() > 1 {
                return Err(invalid("full_page, selector and rect are mutually exclusive"));
            }
            if let Some(f) = opt_str(p, "format")? {
                if f != "png" {
                    return Err(invalid("format must be png"));
                }
            }
            let scale = opt_num(p, "scale")?.unwrap_or(1.0);
            if !(0.1..=4.0).contains(&scale) {
                return Err(invalid("scale must be from 0.1 to 4"));
            }
            let max_pixels = opt_uint(p, "max_pixels", 64_000_000)?.unwrap_or(MAX_SHOT_PIXELS).max(1);
            Req::Screenshot { id: tab_id(p)?, full_page, selector, rect, scale: scale as f32, max_pixels, timeout_ms: timeout_ms(p)? }
        }
        "browser.network" => Req::Network {
            id: tab_id(p)?,
            since_seq: opt_uint(p, "since_seq", u64::MAX / 2)?,
            limit: opt_uint(p, "limit", 100_000)?.unwrap_or(200) as usize,
            clear: opt_bool(p, "clear")?.unwrap_or(false),
        },
        "browser.wait" => {
            let what = match req_str(p, "for")?.as_str() {
                "load" => WaitFor::Load,
                "idle" => WaitFor::Idle,
                "selector" => WaitFor::Selector(req_str(p, "selector")?),
                "js" => WaitFor::Js(req_str(p, "js")?),
                "text" => WaitFor::Text(req_str(p, "text")?),
                "console" => WaitFor::Console(opt_str(p, "text")?),
                _ => return Err(invalid("for must be load, selector, js, text, console or idle")),
            };
            Req::Wait { id: tab_id(p)?, what, timeout_ms: timeout_ms(p)? }
        }
        "browser.viewport" => {
            let reset = opt_bool(p, "reset")?.unwrap_or(false);
            let preset = opt_str(p, "preset")?;
            let (w, h) = (opt_uint(p, "width", 8192)?, opt_uint(p, "height", 8192)?);
            let mut dpr = opt_num(p, "dpr")?;
            let mut size = match (w, h) {
                (Some(w), Some(h)) if w > 0 && h > 0 => Some((w as u32, h as u32)),
                (None, None) => None,
                _ => return Err(invalid("width and height go together and must be from 1 to 8192")),
            };
            if let Some(name) = preset {
                let (pw, ph, pd) = preset_size(&name).ok_or_else(|| invalid("preset must be mobile, bifrost or desktop"))?;
                size = size.or(Some((pw, ph)));
                dpr = dpr.or(Some(pd as f64));
            }
            if let Some(d) = dpr {
                if !(0.5..=4.0).contains(&d) {
                    return Err(invalid("dpr must be from 0.5 to 4"));
                }
            }
            if reset && (size.is_some() || dpr.is_some()) {
                return Err(invalid("reset cannot be combined with a size, dpr or preset"));
            }
            Req::Viewport { size, dpr: dpr.map(|d| d as f32), reset }
        }
        "browser.zoom" => {
            let factor = opt_num(p, "factor")?;
            if let Some(f) = factor {
                if !(0.3..=5.0).contains(&f) {
                    return Err(invalid("factor must be from 0.3 to 5"));
                }
            }
            Req::Zoom { factor: factor.map(|f| f as f32) }
        }
        "browser.find" => {
            let dir = match opt_str(p, "dir")?.as_deref() {
                None | Some("n") => FindDir::New,
                Some("f") => FindDir::Forward,
                Some("b") => FindDir::Back,
                Some("c") => FindDir::Clear,
                Some(_) => return Err(invalid("dir must be n (new search), f (next), b (previous) or c (clear)")),
            };
            let query = if dir == FindDir::Clear { opt_str(p, "query")?.unwrap_or_default() } else { req_str(p, "query")? };
            Req::Find { id: tab_id(p)?, query, dir }
        }
        "browser.user_agent" => Req::UserAgent { ua: opt_str(p, "ua")? },
        "browser.input" => Req::Input { id: tab_id(p)?, input: parse_input(p)? },
        "browser.devtools" => {
            let panel = match opt_str(p, "panel")? {
                None => None,
                Some(s) => Some(Panel::parse(&s).ok_or_else(|| invalid("panel must be console, network, elements, sources or storage"))?),
            };
            Req::Devtools { open: opt_bool(p, "open")?, panel }
        }
        "browser.source" => Req::Source { id: tab_id(p)?, timeout_ms: timeout_ms(p)? },
        "browser.inspect" => {
            let (x, y) = (opt_num(p, "x")?, opt_num(p, "y")?);
            match (x, y) {
                (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => Req::Inspect { id: tab_id(p)?, x: x as f32, y: y as f32, timeout_ms: timeout_ms(p)? },
                _ => return Err(invalid("x and y are required (page-local CSS pixels, not negative)")),
            }
        }
        "browser.downloads" => Req::Downloads { cancel: opt_uint(p, "cancel", u32::MAX as u64)?.map(|v| v as u32), clear: opt_bool(p, "clear")?.unwrap_or(false) },
        "browser.bookmarks" => {
            let op = opt_str(p, "op")?.unwrap_or_else(|| "list".into());
            if !matches!(op.as_str(), "list" | "add" | "remove" | "rename" | "import") {
                return Err(invalid("op must be list, add, remove, rename or import"));
            }
            let (url, title, path) = (opt_str(p, "url")?, opt_str(p, "title")?, opt_str(p, "path")?);
            if op == "import" && path.is_none() {
                return Err(invalid("path is required for import"));
            }
            if matches!(op.as_str(), "remove" | "rename") && url.is_none() {
                return Err(invalid("url is required for remove and rename"));
            }
            if op == "rename" && title.is_none() {
                return Err(invalid("title is required for rename"));
            }
            Req::Bookmarks { op, url, title, query: opt_str(p, "query")?, limit: opt_uint(p, "limit", 5000)?.unwrap_or(200) as usize, path }
        }
        "browser.history" => Req::History {
            query: opt_str(p, "query")?,
            limit: opt_uint(p, "limit", 5000)?.unwrap_or(100) as usize,
            remove: opt_str(p, "remove")?,
            clear: opt_bool(p, "clear")?.unwrap_or(false),
            clear_since: opt_uint(p, "clear_since", i64::MAX as u64)?,
        },
        "browser.favicon" => Req::Favicon { id: tab_id(p)? },
        _ => return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}"))),
    })
}

// ---- console records ----

/// One console line (or a REPL input/result) of a tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Rec {
    pub seq: u64,
    pub ts_ms: u64,
    pub level: String,
    pub text: String,
    pub source: String,
    pub line: u32,
}

pub fn rec_json(r: &Rec) -> Json {
    obj(vec![
        ("seq", num(r.seq as f64)),
        ("ts_ms", num(r.ts_ms as f64)),
        ("level", text(&r.level)),
        ("text", text(&r.text)),
        ("source", text(&r.source)),
        ("line", num(r.line as f64)),
    ])
}

/// `browser.console`: entries newer than `since_seq`, filtered by `level`, the newest `limit` of them, oldest first.
/// `next_seq` is the highest sequence number assigned in the tab so far: pass it back as `since_seq`.
pub fn console_query(recs: &VecDeque<Rec>, last_seq: u64, level: Option<&str>, limit: usize, since_seq: Option<u64>) -> Json {
    let mut picked: Vec<&Rec> = recs.iter().rev().filter(|r| since_seq.is_none_or(|s| r.seq > s) && level.is_none_or(|l| r.level == l)).take(limit).collect();
    picked.reverse();
    obj(vec![("entries", Json::Arr(picked.into_iter().map(rec_json).collect())), ("next_seq", num(last_seq as f64))])
}

// ---- JavaScript snippets (one line each; no `//` comments) ----

/// A JavaScript string literal (JSON quoting is valid JavaScript).
pub fn js_str(s: &str) -> String {
    json::quote(s)
}

/// `browser.dom`: runs in the page, returns a JSON string.
pub fn js_dom(selector: &str, mode: DomMode, limit: usize, max_bytes: usize) -> String {
    let m = match mode {
        DomMode::OuterHtml => "outer_html",
        DomMode::Text => "text",
        DomMode::Attrs => "attrs",
        DomMode::Query => "query",
    };
    let limit = if mode == DomMode::Attrs { 1 } else { limit };
    format!(
        "(function(sel,mode,limit,maxb){{var els;try{{els=document.querySelectorAll(sel)}}catch(e){{return JSON.stringify({{error:String(e)}})}}\
         if(mode==='outer_html'||mode==='text'){{var first=els[0];if(!first)return JSON.stringify({{matches:0,truncated:false}});\
         var s=mode==='text'?(first.innerText||first.textContent||''):first.outerHTML;var tr=s.length>maxb;if(tr)s=s.slice(0,maxb);\
         var o={{matches:els.length,truncated:tr}};o[mode==='text'?'text':'html']=s;return JSON.stringify(o)}}\
         var items=[];for(var i=0;i<els.length&&i<limit;i++){{var e=els[i],r=e.getBoundingClientRect(),a={{}};\
         for(var j=0;j<e.attributes.length;j++)a[e.attributes[j].name]=e.attributes[j].value;\
         items.push({{tag:e.tagName.toLowerCase(),id:e.id||'',class:typeof e.className==='string'?e.className:'',\
         text:(e.innerText||e.textContent||'').trim().slice(0,200),attrs:a,rect:{{x:r.x,y:r.y,w:r.width,h:r.height}}}})}}\
         return JSON.stringify({{matches:els.length,items:items}})}})({},{},{limit},{max_bytes})",
        js_str(selector),
        js_str(m)
    )
}

/// `browser.source`: the document as text.
pub const JS_SOURCE: &str = "(document.doctype?'<!DOCTYPE '+document.doctype.name+'>\\n':'')+(document.documentElement?document.documentElement.outerHTML:'')";

/// Viewport size, scroll and ratio, for `browser.screenshot` and `browser.state`.
pub const JS_VIEW_INFO: &str = "JSON.stringify({dpr:window.devicePixelRatio,sx:window.scrollX,sy:window.scrollY,w:window.innerWidth,h:window.innerHeight,dw:Math.max(document.documentElement.scrollWidth,document.body?document.body.scrollWidth:0),dh:Math.max(document.documentElement.scrollHeight,document.body?document.body.scrollHeight:0)})";

/// The viewport rectangle (CSS px) of the first element matching `selector`, scrolled into view and clipped to the
/// viewport, plus the view info. `null` when nothing matches.
pub fn js_element_rect(selector: &str) -> String {
    format!(
        "(function(sel){{var e;try{{e=document.querySelector(sel)}}catch(x){{return JSON.stringify({{error:String(x)}})}}\
         if(!e)return null;if(e.scrollIntoView)e.scrollIntoView({{block:'center',inline:'center'}});\
         var r=e.getBoundingClientRect(),x=Math.max(0,r.left),y=Math.max(0,r.top),\
         w=Math.min(r.right,window.innerWidth)-x,h=Math.min(r.bottom,window.innerHeight)-y;\
         return JSON.stringify({{x:x,y:y,w:w,h:h,dpr:window.devicePixelRatio,sx:window.scrollX,sy:window.scrollY}})}})({})",
        js_str(selector)
    )
}

/// One poll of a `browser.wait` condition: evaluates to a boolean (or, for `idle`, a sample string).
pub fn js_wait_poll(what: &WaitFor) -> Option<String> {
    match what {
        WaitFor::Selector(s) => Some(format!("(function(s){{try{{return !!document.querySelector(s)}}catch(e){{return false}}}})({})", js_str(s))),
        WaitFor::Js(j) => Some(format!("(function(){{try{{return !!(0,eval)({})}}catch(e){{return false}}}})()", js_str(j))),
        WaitFor::Text(t) => Some(format!("((document.body&&document.body.innerText)||'').indexOf({})>=0", js_str(t))),
        WaitFor::Idle => Some("document.readyState+'|'+(performance.getEntriesByType?performance.getEntriesByType('resource').length:0)+'|'+document.getElementsByTagName('*').length".into()),
        WaitFor::Load | WaitFor::Console(_) => None,
    }
}

/// The Elements panel tree: nested `{t,p,i,c,a,x,k,m}` (tag, element-index path, id, class, attributes, text of a
/// leaf, children, matches the filter). With a non-empty `filter` only matching elements and their ancestors are included.
pub fn js_tree(filter: &str, max_nodes: usize) -> String {
    format!(
        "(function(f,max){{var n=0,sel=null;if(f){{try{{document.querySelectorAll(f);sel=f}}catch(e){{return JSON.stringify({{error:String(e)}})}}}}\
         function w(e,d,pp){{if(n>=max||d>40||e.id==='__bifrost_hl')return null;n++;var o={{t:e.nodeName.toLowerCase(),p:pp}};\
         if(e.nodeType===1){{if(e.id)o.i=e.id;if(typeof e.className==='string'&&e.className)o.c=e.className;\
         var a={{}},has=false;for(var j=0;j<e.attributes.length;j++){{var an=e.attributes[j].name;if(an!=='id'&&an!=='class'){{a[an]=String(e.attributes[j].value).slice(0,120);has=true}}}}if(has)o.a=a;\
         if(sel&&e.matches&&e.matches(sel))o.m=1;var k=[],tx='',ei=0;\
         for(var c=e.firstChild;c;c=c.nextSibling){{if(c.nodeType===1){{var r=w(c,d+1,pp+'.'+(ei++));if(r)k.push(r)}}else if(c.nodeType===3){{var s=c.nodeValue.trim();if(s)tx+=(tx?' ':'')+s}}}}\
         if(tx)o.x=tx.slice(0,160);if(k.length)o.k=k;if(sel&&!o.m&&!k.length)return null}}return o}}\
         return JSON.stringify({{root:w(document.documentElement,0,'0'),nodes:n,truncated:n>=max}})}})({},{max_nodes})",
        js_str(filter)
    )
}

// ---- results ----

/// `browser.eval` reply from an engine value.
pub fn eval_ok(ty: &str, json_text: &str) -> Json {
    let result = json::parse(json_text).unwrap_or_else(|_| Json::Str(json_text.to_string()));
    obj(vec![("ok", Json::Bool(true)), ("result", result), ("type", text(ty))])
}

pub fn eval_err(message: &str) -> Json {
    obj(vec![("ok", Json::Bool(false)), ("error", text(message))])
}

/// The page script of `browser.dom` / `browser.source` returned a JSON string; shape the reply.
pub fn dom_reply(raw: &str) -> Result<Json, RpcError> {
    let v = json::parse(raw).map_err(|e| RpcError::new(bifrost_app::control::INTERNAL_ERROR, format!("bad dom reply: {e}")))?;
    if let Some(msg) = v.get("error").and_then(Json::as_str) {
        return Err(invalid(format!("selector: {msg}")));
    }
    Ok(v)
}

/// What an `eval` of a JSON-returning snippet gave: the string inside (the engine sent it as a JSON string).
pub fn inner_string(ty: &str, json_text: &str) -> Option<String> {
    if ty != "string" {
        return None;
    }
    match json::parse(json_text) {
        Ok(Json::Str(s)) => Some(s),
        _ => None,
    }
}

// ---- base64 and PNG ----

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Width and height from a PNG's IHDR chunk.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Some((be(16), be(20)))
}

/// The `browser.screenshot` reply.
pub fn screenshot_reply(png: &[u8], dpr: f64, scroll: (f64, f64)) -> Result<Json, RpcError> {
    if png.len() > MAX_SHOT_BYTES {
        return Err(RpcError::new(bifrost_app::control::INTERNAL_ERROR, "screenshot larger than 16 MB: use rect, selector or a smaller viewport"));
    }
    let (w, h) = png_size(png).ok_or_else(|| RpcError::new(bifrost_app::control::INTERNAL_ERROR, "engine wrote an invalid PNG"))?;
    Ok(obj(vec![
        ("width", num(w as f64)),
        ("height", num(h as f64)),
        ("format", text("png")),
        ("encoding", text("base64")),
        ("data", Json::Str(base64(png))),
        ("dpr", num(dpr)),
        ("scroll", obj(vec![("x", num(scroll.0)), ("y", num(scroll.1))])),
    ]))
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ---- inspector rows (shared by the Network and Elements panels) ----

/// One text cell of an inspector row: `x` and `w` in points from the list's left edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub x: f32,
    pub w: f32,
    pub text: String,
    pub color: u32,
}

/// One row of an inspector list. A tap copies `copy`, or flips the `toggle` key when there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub copy: String,
    pub toggle: Option<String>,
    /// Elements: the node path a tap selects (and highlights in the page)
    pub select: Option<String>,
}

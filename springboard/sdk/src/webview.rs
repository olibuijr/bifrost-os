//! Web content in an SDK app (WebKit's `WebView`, `WebProcessPool` and `WebNavigationDelegate`).
//!
//! Pages run in a separate engine process (`bifrost-webview`: Servo 0.7.0 embedded through libservo),
//! like WebKit's WebContent process. One [`WebProcessPool`] owns that process and any number of web views in it,
//! named by [`WebViewId`]; the app shows one of them in a rectangle of its window. Each presented frame is copied
//! once from shared memory into the app's surface (`draw`). Navigation state arrives as [`WebViewEvent`]s from
//! `poll`, and the current values are on [`WebViewState`].
//!
//! ```ignore
//! let mut pool = WebProcessPool::spawn(&engine_binary, &profile_dir)?;
//! let tab = pool.create("https://example.com");
//! pool.show(tab);
//! // App::configure: pool.set_frame(rect_px, ctx.cfg.scale);
//! // App::wake_fd:   Some(pool.fd())
//! // App::tick:      for ev in pool.poll() { ... }   (true when a frame arrived)
//! // App::draw:      pool.draw(surface);
//! // App::event:     pool.touch(kind, id, x, y) / pool.key(&k)
//! ```
//!
//! parity and its gaps are listed in docs/bifrost-ui.md ("WebView").

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::proto::sys::{self, Map, Owned};
use crate::proto::{KeyInput, TouchKind};
use crate::Surface;

/// A web view in the pool (the app's handle; `WebView`).
pub type WebViewId = u32;

const MAX_PICKER_JSON: usize = 64 * 1024;
const MAX_PICKER_FILTERS: usize = 64;
const MAX_PICKER_FILTER_BYTES: usize = 1024;
const MAX_PICKER_PATHS: usize = 128;
const MAX_PICKER_PATH_BYTES: usize = 4096;
const MAX_STATUS_BYTES: usize = 4096;

fn control_token(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) { return None; }
    text.parse::<u32>().ok().filter(|token| *token != 0)
}

fn color_channel(text: &str) -> Option<u8> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) { return None; }
    text.parse().ok()
}

fn picker_request(text: &str) -> Option<(bool, Vec<String>)> {
    use bifrost_core::json::{self, Json};
    if text.len() > MAX_PICKER_JSON { return None; }
    let Json::Obj(fields) = json::parse(text).ok()? else { return None; };
    // Reject ambiguous duplicate fields as well as unrecognized request shapes.
    if fields.len() != 2 { return None; }
    let request = Json::Obj(fields);
    let Json::Bool(multiple) = request.get("multiple")? else { return None; };
    let filters = request.get("filters")?.as_array()?;
    if filters.len() > MAX_PICKER_FILTERS { return None; }
    let filters = filters.iter().map(|filter| {
        let value = filter.as_str()?;
        (value.len() <= MAX_PICKER_FILTER_BYTES && !value.contains('\0')).then(|| value.to_owned())
    }).collect::<Option<Vec<_>>>()?;
    Some((*multiple, filters))
}

/// The requested rendering backend for a new engine process. The engine reports initialization errors rather
/// than silently substituting CPU rendering when GPU initialization fails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WebRenderer {
    #[default]
    Gpu,
    /// Explicit software rendering for systems without a usable GPU context.
    Cpu,
}

impl WebRenderer {
    pub fn as_str(self) -> &'static str {
        match self { Self::Gpu => "gpu", Self::Cpu => "cpu" }
    }
}

/// True when a frame tagged `from` may replace the pixels: untagged frames (older engines) pass, tagged ones
/// must come from the visible view, so a frame in flight during a tab switch never shows the previous tab.
fn frame_is_current(visible: Option<WebViewId>, from: Option<WebViewId>) -> bool {
    match (visible, from) {
        (_, None) => true,
        (Some(v), Some(f)) => v == f,
        (None, Some(_)) => false,
    }
}

/// The current navigation values of one web view (`WebView` properties).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebViewState {
    /// `title`
    pub title: String,
    /// `URL`
    pub url: String,
    /// `isLoading`
    pub loading: bool,
    /// `estimatedProgress`, 0 to 1: 0.1 when a load starts, 0.5 when the document's head is parsed, 1 when done.
    pub progress: f32,
    /// `canGoBack`
    pub can_go_back: bool,
    /// `canGoForward`
    pub can_go_forward: bool,
    /// `WebFindResult.matchFound` count: matches of the last `find`, `None` before any find.
    pub find_matches: Option<usize>,
    /// `pageZoom` (1.0 is 100%)
    pub page_zoom: f32,
    /// Native page status text, such as the link currently under the pointer.
    pub status_text: String,
}

/// Where `find` searches from (`WebFindConfiguration.backwards`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindDirection {
    /// a new query: counts matches and selects the first
    New,
    Forward,
    Backward,
}

/// What changed (`WebNavigationDelegate` and `WebUIDelegate` callbacks, plus key-value observation of the state).
#[derive(Clone, Debug, PartialEq)]
pub enum WebViewEvent {
    /// A 32 by 32 straight RGBA favicon decoded by Servo.
    FaviconChanged { id: WebViewId, rgba: Vec<u8> },
    /// Text copied or cut through Servo's native clipboard delegate.
    ClipboardChanged(String),
    /// `didStartProvisionalNavigation`
    LoadStarted(WebViewId),
    /// `didCommitNavigation` (the document's head is parsed)
    LoadCommitted(WebViewId),
    /// `didFinishNavigation`
    LoadFinished(WebViewId),
    TitleChanged(WebViewId),
    UrlChanged(WebViewId),
    StatusTextChanged(WebViewId),
    /// `canGoBack` or `canGoForward` changed
    HistoryChanged(WebViewId),
    /// `createWebViewWithConfiguration`: the page opened a window (`window.open`, `target=_blank`). The new view is
    /// already in the pool, hidden; show it (a new tab) or `close` it to refuse.
    CreateWebView { id: WebViewId, opener: WebViewId, url: String },
    /// `webViewDidClose` (`window.close()`)
    Closed(WebViewId),
    /// `webViewWebContentProcessDidTerminate`
    ContentProcessTerminated(WebViewId, String),
    /// The engine process exited; every view is gone.
    EngineExited,
    /// Web Inspector console: `console.*`, uncaught errors (`WebUIDelegate` has no equivalent; Browser's Web Inspector
    /// reads the same stream). `level` is log, info, debug, warn or error; `source` is a URL or empty; `line` 0 if unknown.
    Console { id: WebViewId, level: String, text: String, source: String, line: u32 },
    /// `evaluateJavaScript:completionHandler:`: the result of `WebProcessPool::eval`, as text.
    EvalResult(WebViewId, String),
    /// `runJavaScriptAlertPanel`, `...ConfirmPanel`, `...TextInputPanel`: a script dialog. Present it as a sheet over
    /// the window and answer with `dialog_reply` (an unanswered dialog blocks the page's script).
    Dialog { id: WebViewId, token: u32, kind: DialogKind, message: String, default: String },
    /// Native file input request. Answer through `file_picker_reply`; paths remain privileged host data.
    FilePicker { id: WebViewId, token: u32, multiple: bool, filters: Vec<String> },
    /// Native color input request. Answer through `color_picker_reply`.
    ColorPicker { id: WebViewId, token: u32, color: (u8, u8, u8) },
    /// A document asks to confirm leaving. Answer through `unload_reply`.
    UnloadRequested { id: WebViewId, token: u32 },
    /// A context menu (`WebUIDelegate` `contextMenuConfiguration`) or a `<select>` popup. `rect` is the element's
    /// area in device pixels inside the page frame. Answer with `menu_reply`.
    Menu { id: WebViewId, token: u32, kind: MenuKind, rect: (i32, i32, i32, i32), items: Vec<MenuEntry> },
    /// The page withdrew dialog or menu `token` (navigation, script): close it without answering.
    ControlHidden(WebViewId, u32),
    /// `navigationAction(_:didBecome:)` / `navigationResponse(_:didBecome:)`: the navigation became a download
    /// (`NavigationResponsePolicy::Download`, see `web_download`). Build a `WebDownload` from `url` and ask the
    /// `WebDownloadDelegate` for a destination.
    DownloadStarted { id: WebViewId, url: String, suggested_filename: String },
    /// The reference script message handler: a page posted a message (`window.bifrost.messageHandlers.<name>.postMessage`).
    /// Route it with `web_bridge::route_script_messages`.
    ScriptMessage { id: WebViewId, message: crate::web_config::ScriptMessage },
    /// `decidePolicyFor navigationAction`: a navigation is about to start and the engine waits for the app's answer
    /// (`WebProcessPool::navigation_reply`). Sent only after `set_navigation_policy_enabled(true)`. The engine
    /// reports neither the frame nor the cause, so a request from a frame inside the page looks like any other.
    NavigationRequested { id: WebViewId, token: u32, url: String },
    /// The answer to `request_cookies`: the cookies the engine holds for the page URL, for `web_bridge::reconcile_cookies`.
    Cookies { id: WebViewId, token: u32, cookies: Vec<crate::web_data::Cookie> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Context,
    Select,
}

/// One row of a page menu. `index` is what `menu_reply` sends back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub index: usize,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    /// An `<optgroup>` label: shown, not selectable.
    pub heading: bool,
}

/// Parses the engine's `menu` item records (`INDEX|FLAGS|LABEL`, split by U+0001).
pub fn parse_menu_items(s: &str) -> Vec<MenuEntry> {
    s.split('\u{1}')
        .filter_map(|rec| {
            let mut p = rec.splitn(3, '|');
            let index = p.next()?.parse().ok()?;
            let flags: u8 = p.next()?.parse().ok()?;
            let label = unescape_line(p.next().unwrap_or(""));
            Some(MenuEntry { index, label, enabled: flags & 1 != 0, checked: flags & 2 != 0, separator: flags & 4 != 0, heading: flags & 8 != 0 })
        })
        .collect()
}

/// Longest unterminated engine line kept while waiting for its newline, bytes.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

/// Longest console line the protocol carries, bytes.
pub const CONSOLE_LINE_MAX: usize = 4096;

/// Escapes a console text for the line protocol (`\n`, `\r`, `\\`) and caps it at `CONSOLE_LINE_MAX` bytes.
pub fn escape_line(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let part = match c {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\\' => "\\\\".to_string(),
            c => c.to_string(),
        };
        if out.len() + part.len() > CONSOLE_LINE_MAX {
            break;
        }
        out.push_str(&part);
    }
    out
}

/// The inverse of `escape_line`.
pub fn unescape_line(s: &str) -> String {
    let mut out = String::new();
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

/// Splits the tail of `console ID LEVEL SOURCE LINE TEXT` (after the id).
fn parse_console(tail: &str) -> Option<(String, String, u32, String)> {
    let mut f = tail.splitn(4, ' ');
    let level = f.next()?.to_string();
    let source = f.next()?;
    let source = if source == "-" { String::new() } else { source.to_string() };
    let line = f.next()?.parse().unwrap_or(0);
    Some((level, source, line, unescape_line(f.next().unwrap_or(""))))
}

/// The engine process and its web views (`WebProcessPool`).
pub struct WebProcessPool {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    buf: Vec<u8>,
    shm: Owned,
    shm_len: usize,
    map: Option<Map>,
    views: HashMap<WebViewId, WebViewState>,
    next: WebViewId,
    visible: Option<WebViewId>,
    /// the content rectangle in window pixels and the scale
    frame: (i32, i32, u32, u32),
    scale: f32,
    /// Last complete frame owned by the SDK. Shared pixels may be rewritten after acknowledgment.
    pixels: Vec<u32>,
    pixels_size: (usize, usize),
    dark: Option<bool>,
    alive: bool,
    startup_error: Option<String>,
    /// `dev ...` lines from the engine, for `webview_dev` (devtools control replies); not part of `WebViewEvent`
    dev_lines: Vec<String>,
    /// a `pong N` arrived since the last `take_pong`
    pong: bool,
    /// views whose engine side was closed by `discard`; their `WebViewState` (URL, title) stays here and the
    /// next `show`, `load`, `reload` or `revive` re-creates the engine view under the same id
    discarded: HashSet<WebViewId>,
}

impl WebProcessPool {
    /// Starts the engine (`engine` is the `bifrost-webview` binary); cookies and site data live in `profile`.
    pub fn spawn(engine: &Path, profile: &Path) -> io::Result<Self> {
        Self::spawn_with_renderer(engine, profile, WebRenderer::default())
    }

    /// Starts a new engine with the requested backend. Backend selection is scoped to this child process;
    /// changing it requires starting a new pool and reloading the open pages.
    pub fn spawn_with_renderer(engine: &Path, profile: &Path, renderer: WebRenderer) -> io::Result<Self> {
        std::fs::create_dir_all(profile)?;
        let shm = sys::shm_create("bifrost-webview", 4)?;
        sys::make_inheritable(shm.0);
        let mut child = Command::new(engine)
            // A private group keeps cleanup scoped to this engine and its Servo content processes.
            .process_group(0)
            .env("BIFROST_BROWSER_RENDERER", renderer.as_str())
            .env("BIFROST_WEBVIEW_FD", shm.0.to_string())
            .env("BIFROST_WEBVIEW_PROFILE", profile)
            .env_remove("BIFROST_SOCK")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let pipes = (|| {
            let stdin = child.stdin.take().ok_or_else(|| io::Error::other("engine stdin"))?;
            let stdout = child.stdout.take().ok_or_else(|| io::Error::other("engine stdout"))?;
            sys::set_nonblocking(stdout.as_raw_fd())?;
            Ok::<_, io::Error>((stdin, stdout))
        })();
        let (stdin, stdout) = match pipes {
            Ok(pipes) => pipes,
            Err(error) => {
                terminate_engine_group(&mut child);
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(WebProcessPool {
            child,
            stdin,
            stdout,
            buf: Vec::new(),
            shm,
            shm_len: 4,
            map: None,
            views: HashMap::new(),
            next: 1,
            visible: None,
            frame: (0, 0, 0, 0),
            scale: 1.0,
            pixels: Vec::new(),
            pixels_size: (0, 0),
            dark: None,
            alive: true,
            startup_error: None,
            dev_lines: Vec::new(),
            pong: false,
            discarded: HashSet::new(),
        })
    }

    /// Heartbeat for the app's watchdog: the engine answers `ping N` with `pong N` (see `take_pong`).
    pub fn send_ping(&mut self, n: u64) {
        self.send(&format!("ping {n}"));
    }

    /// True once per `pong` received.
    pub fn take_pong(&mut self) -> bool {
        std::mem::take(&mut self.pong)
    }

    /// Kills a wedged engine; the next `poll` reports `EngineExited` and the app restarts it.
    pub fn kill_engine(&mut self) {
        terminate_engine_group(&mut self.child);
    }

    /// A fatal initialization error reported before the engine exited, such as an unavailable GPU context.
    pub fn startup_error(&self) -> Option<&str> {
        self.startup_error.as_deref()
    }

    fn send(&mut self, line: &str) {
        let clean: String = line.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
        if self.alive && writeln!(self.stdin, "{clean}").is_err() {
            self.alive = false;
        }
    }

    /// Additive hook for `webview_dev`: one raw protocol line to the engine.
    pub(crate) fn send_raw(&mut self, line: &str) {
        self.send(line);
    }

    /// Additive hook for `webview_dev`: the engine's `dev ...` lines received so far.
    pub(crate) fn take_dev_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.dev_lines)
    }

    /// Additive hook for `webview_dev`: the engine process id.
    pub(crate) fn engine_pid(&self) -> u32 {
        self.child.id()
    }

    /// Device pixels per CSS pixel of the content (the `scale` of `set_frame`).
    pub fn device_scale(&self) -> f32 {
        self.scale
    }

    /// `devtools on|off`: whether the engine forwards console messages (on at start).
    pub fn set_devtools(&mut self, on: bool) {
        self.send(if on { "devtools on" } else { "devtools off" });
    }

    /// `evaluateJavaScript:`: runs `js` (one line) in view `id`; the answer arrives as `WebViewEvent::EvalResult`.
    pub fn eval(&mut self, id: WebViewId, js: &str) {
        self.send(&format!("eval {id} {js}"));
    }

    /// The descriptor that becomes readable when the engine has news; the app waits on it (`App::wake_fd`).
    pub fn fd(&self) -> i32 {
        self.stdout.as_raw_fd()
    }

    /// False once the engine has exited (stop waiting on `fd` then; spawn a new pool to recover).
    /// Process id of the engine, for memory accounting (`/proc/PID/smaps_rollup`).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn alive(&self) -> bool {
        self.alive
    }

    /// Creates a web view and starts loading `url` (`initWithFrame:configuration:` + `loadRequest:`). It is hidden
    /// until `show`.
    pub fn create(&mut self, url: &str) -> WebViewId {
        let id = self.next;
        self.next += 1;
        self.views.insert(id, WebViewState { url: url.into(), loading: true, progress: 0.1, ..Default::default() });
        self.send(&format!("new {id} {url}"));
        id
    }

    /// Restores a dormant tab without creating an engine view or starting a network request. Selecting the
    /// tab later revives it through the normal `show` path.
    pub fn create_discarded(&mut self, url: &str, title: &str) -> WebViewId {
        let id = self.next;
        self.next += 1;
        self.views.insert(id, WebViewState { url: url.into(), title: title.into(), ..Default::default() });
        self.discarded.insert(id);
        id
    }

    /// Makes `id` the view drawn in the frame and the one that receives input.
    pub fn show(&mut self, id: WebViewId) {
        if self.views.contains_key(&id) {
            self.revive(id);
            self.visible = Some(id);
            self.pixels_size = (0, 0);
            self.send(&format!("show {id}"));
        }
    }

    /// Browser-style tab discard: closes the engine view of a background tab (freeing its content process memory)
    /// but keeps its id, URL and title here, so the app, the control socket and the inspector keep addressing the
    /// same tab. The visible view is never discarded. `show`, `load`, `reload` and `revive` bring it back.
    pub fn discard(&mut self, id: WebViewId) -> bool {
        if self.visible == Some(id) || self.discarded.contains(&id) || !self.views.contains_key(&id) {
            return false;
        }
        self.discarded.insert(id);
        self.send(&format!("close {id}"));
        if let Some(st) = self.views.get_mut(&id) {
            st.loading = false;
            st.progress = 0.0;
        }
        true
    }

    pub fn is_discarded(&self, id: WebViewId) -> bool {
        self.discarded.contains(&id)
    }

    /// Re-creates the engine view of a discarded tab (hidden, loading its last URL). No-op otherwise.
    pub fn revive(&mut self, id: WebViewId) {
        if !self.discarded.remove(&id) {
            return;
        }
        let Some(st) = self.views.get_mut(&id) else { return };
        st.loading = true;
        st.progress = 0.1;
        let url = if st.url.is_empty() { "about:blank".to_string() } else { st.url.clone() };
        self.send(&format!("new {id} {url}"));
    }

    pub fn visible(&self) -> Option<WebViewId> {
        self.visible
    }

    pub fn close(&mut self, id: WebViewId) {
        self.discarded.remove(&id);
        if self.views.remove(&id).is_none() {
            return;
        }
        if self.visible == Some(id) {
            self.visible = None;
        }
        self.send(&format!("close {id}"));
    }

    pub fn state(&self, id: WebViewId) -> Option<&WebViewState> {
        self.views.get(&id)
    }

    /// `loadRequest:`. `url` must be absolute; turning typed text into a URL is the app's job (`url_for_input`).
    pub fn load(&mut self, id: WebViewId, url: &str) {
        if self.discarded.contains(&id) {
            // the new URL replaces the discarded one; create the view straight on it
            if let Some(st) = self.views.get_mut(&id) {
                st.url = url.into();
            }
            self.revive(id);
            return;
        }
        self.started(id);
        self.send(&format!("load {id} {url}"));
    }
    pub fn go_back(&mut self, id: WebViewId) {
        self.send(&format!("back {id}"));
    }
    pub fn go_forward(&mut self, id: WebViewId) {
        self.send(&format!("forward {id}"));
    }
    /// Remove back/forward entries while keeping the current document.
    pub fn clear_session_history(&mut self, id: WebViewId) {
        self.send(&format!("clear-history {id}"));
    }
    /// Clear engine-owned cookies, local/session storage and the HTTP cache.
    pub fn clear_website_data(&mut self) {
        self.send("clear-site-data");
    }
    /// Answers a `Dialog`: `ok` is OK (true) or Cancel; `text` is a prompt's value.
    pub fn dialog_reply(&mut self, id: WebViewId, token: u32, ok: bool, text: &str) {
        self.send(&format!("dialog-reply {id} {token} {} {}", if ok { "ok" } else { "cancel" }, escape_line(text)));
    }

    /// Answer a native file input. `None` cancels. Invalid, relative, non-UTF-8 or oversized
    /// path lists cancel rather than silently uploading a different path. At most 128 paths,
    /// 4096 UTF-8 bytes per path and 64 KiB encoded JSON are accepted.
    pub fn file_picker_reply(&mut self, id: WebViewId, token: u32, paths: Option<&[PathBuf]>) {
        use bifrost_core::json::Json;
        if token == 0 || !self.views.contains_key(&id) { return; }
        let selection = paths.filter(|paths| paths.len() <= MAX_PICKER_PATHS).and_then(|paths| {
            paths.iter().map(|path| {
                let text = path.to_str()?;
                (path.is_absolute() && text.len() <= MAX_PICKER_PATH_BYTES && !text.contains('\0'))
                    .then(|| Json::Str(text.to_owned()))
            }).collect::<Option<Vec<_>>>()
        });
        let json = selection.map(Json::Arr).unwrap_or(Json::Null).encode();
        let json = if json.len() <= MAX_PICKER_JSON { json.as_str() } else { "null" };
        self.send(&format!("file-reply {id} {token} {json}"));
    }

    /// Answer a native color input with RGB channels, or cancel with `None`.
    pub fn color_picker_reply(&mut self, id: WebViewId, token: u32, color: Option<(u8, u8, u8)>) {
        if token == 0 || !self.views.contains_key(&id) { return; }
        let response = match color {
            Some((r, g, b)) => format!("ok {r} {g} {b}"),
            None => "cancel".into(),
        };
        self.send(&format!("color-reply {id} {token} {response}"));
    }

    /// Answer a document's request to leave its current page.
    pub fn unload_reply(&mut self, id: WebViewId, token: u32, allow: bool) {
        if token == 0 || !self.views.contains_key(&id) { return; }
        self.send(&format!("unload-reply {id} {token} {}", if allow { "allow" } else { "cancel" }));
    }

    /// Answers a `Menu` with the chosen entry indexes (one for a context menu or single select; none dismisses).
    pub fn menu_reply(&mut self, id: WebViewId, token: u32, picks: &[usize]) {
        let list: String = picks.iter().map(|i| format!(" {i}")).collect();
        self.send(&format!("menu-reply {id} {token}{list}"));
    }

    pub fn reload(&mut self, id: WebViewId) {
        if self.discarded.contains(&id) {
            self.revive(id);
            return;
        }
        self.started(id);
        self.send(&format!("reload {id}"));
    }
    /// Reload From Origin (Browser's Develop menu): the engine reloads bypassing the HTTP cache.
    pub fn reload_ignoring_cache(&mut self, id: WebViewId) {
        self.started(id);
        self.send(&format!("reload-hard {id}"));
    }
    /// `WebView.customUserAgent`: `None` restores the default. Applies to loads after the next reload.
    pub fn set_user_agent(&mut self, ua: Option<&str>) {
        self.send(&format!("ua {}", ua.unwrap_or("")));
    }
    /// Responsive Design Mode: renders the page at `css_w x css_h` points with `dpr`, centred inside the
    /// `(x, y, w, h)` area (window pixels). Call instead of `set_frame` while active; returns the content rect.
    pub fn set_viewport_override(&mut self, area: (i32, i32, u32, u32), css_w: u32, css_h: u32, dpr: f32) -> (i32, i32, u32, u32) {
        let (pw, ph) = (((css_w as f32 * dpr) as u32).max(1), ((css_h as f32 * dpr) as u32).max(1));
        let (pw, ph) = (pw.min(area.2.max(1)), ph.min(area.3.max(1)));
        let r = (area.0 + ((area.2 - pw) / 2) as i32, area.1 + ((area.3 - ph) / 2) as i32, pw, ph);
        if self.frame != r || self.scale != dpr {
            let need = pw as usize * ph as usize * 4;
            if need > self.shm_len && sys::shm_resize(&self.shm, need).is_ok() {
                self.shm_len = need;
                self.map = None;
            }
            self.frame = r;
            self.scale = dpr;
            self.send(&format!("emulate {pw} {ph} {dpr}"));
        }
        r
    }
    /// The engine reports `Started` late or not at all for some loads; a request shows progress at once.
    fn started(&mut self, id: WebViewId) {
        if let Some(st) = self.views.get_mut(&id) {
            st.status_text.clear();
            st.loading = true;
            st.progress = 0.1;
        }
    }
    /// `stopLoading`. Servo 0.7.0 cannot cancel a load yet, so this does nothing until the engine can.
    pub fn stop_loading(&mut self, id: WebViewId) {
        self.send(&format!("stop {id}"));
    }

    /// Places the content in the window: `x, y, w, h` in window pixels, `scale` pixels per point.
    pub fn set_frame(&mut self, x: i32, y: i32, w: u32, h: u32, scale: f32) {
        let (w, h) = (w.max(1), h.max(1));
        if self.frame == (x, y, w, h) && self.scale == scale {
            return;
        }
        let need = w as usize * h as usize * 4;
        if need > self.shm_len && sys::shm_resize(&self.shm, need).is_ok() {
            self.shm_len = need;
            self.map = None;
        }
        self.frame = (x, y, w, h);
        self.scale = scale;
        self.send(&format!("size {w} {h} {scale}"));
    }

    /// Follows the system appearance (`prefers-color-scheme` in pages).
    pub fn set_dark(&mut self, dark: bool) {
        if self.dark != Some(dark) {
            self.dark = Some(dark);
            self.send(if dark { "theme dark" } else { "theme light" });
        }
    }

    /// The window became key or stopped being key.
    pub fn set_focused(&mut self, on: bool) {
        self.send(if on { "focus 1" } else { "focus 0" });
    }

    /// The window is hidden or shown again: the engine stops or restarts painting (views are kept).
    pub fn set_suspended(&mut self, on: bool) {
        self.send(if on { "suspend" } else { "resume" });
    }

    /// True when the window point `(x, y)` (pixels) is in the content frame.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (fx, fy, w, h) = self.frame;
        x >= fx as f32 && y >= fy as f32 && x < (fx + w as i32) as f32 && y < (fy + h as i32) as f32
    }

    /// A touch (or the desktop pointer, which the shell sends as a finger) at window pixels. The page scrolls under
    /// a drag and clicks on a tap, as on a touch screen.
    pub fn touch(&mut self, kind: TouchKind, finger: u32, x: f32, y: f32) {
        let k = match kind {
            TouchKind::Down => "d",
            TouchKind::Move => "m",
            TouchKind::Up => "u",
            TouchKind::Cancel => "c",
        };
        let (fx, fy) = (x - self.frame.0 as f32, y - self.frame.1 as f32);
        self.send(&format!("touch {k} {finger} {fx} {fy}"));
    }

    /// A primary mouse press, drag, release or hover, preserving native text selection and pointer events.
    pub fn mouse(&mut self, kind: TouchKind, x: f32, y: f32) {
        let kind = match kind {
            TouchKind::Down => "d",
            TouchKind::Move => "m",
            TouchKind::Up => "u",
            TouchKind::Cancel => "c",
        };
        self.send(&format!("mouse {kind} {} {}", x - self.frame.0 as f32, y - self.frame.1 as f32));
    }
    /// Refresh the text available to the native engine clipboard delegate before a page key event.
    pub fn set_clipboard(&mut self, text: &str) {
        self.send(&format!("clipboard {}", crate::webview_dev::escape_full(text)));
    }
    /// Clear the native page hover target when the pointer enters chrome or leaves the window.
    pub fn mouse_leave(&mut self) {
        if let Some(st) = self.visible.and_then(|id| self.views.get_mut(&id)) {
            st.status_text.clear();
        }
        self.send("mouseleave");
    }

    /// Secondary click (right-click, or a long press) at window pixel `(x, y)`: the page may answer with a `Menu` event.
    pub fn secondary_click(&mut self, x: f32, y: f32) {
        let (fx, fy) = (x - self.frame.0 as f32, y - self.frame.1 as f32);
        self.send(&format!("rclick {fx} {fy}"));
    }

    /// Scrolls the content under window pixel `(x, y)` by `dx, dy` pixels.
    pub fn scroll(&mut self, dx: f32, dy: f32, x: f32, y: f32) {
        let (fx, fy) = (x - self.frame.0 as f32, y - self.frame.1 as f32);
        self.send(&format!("wheel {dx} {dy} {fx} {fy}"));
    }

    /// A typed key for the page (the focused field, or the page's key handlers).
    pub fn key(&mut self, k: &KeyInput) {
        if let Some((key, mods)) = k.as_chord() {
            if let Some(c) = evdev_char(key) {
                self.send(&format!("chord {mods} {c}"));
            }
            return;
        }
        match k {
            KeyInput::Text(s) => self.send(&format!("text {s}")),
            KeyInput::Code(code) => {
                let name = match code {
                    8 => "Backspace",
                    13 => "Enter",
                    9 => "Tab",
                    1 | 27 => "Escape",
                    37 => "ArrowLeft",
                    38 => "ArrowUp",
                    39 => "ArrowRight",
                    40 => "ArrowDown",
                    46 => "Delete",
                    33 => "PageUp",
                    34 => "PageDown",
                    35 => "End",
                    36 => "Home",
                    _ => return,
                };
                self.send(&format!("key {name}"));
            }
        }
    }

    /// Reads what the engine sent. Returns the events and whether a new frame arrived (the app redraws then).
    pub fn poll(&mut self) -> (Vec<WebViewEvent>, bool) {
        let mut events = Vec::new();
        let mut frame = false;
        let mut chunk = [0u8; 8192];
        loop {
            match self.stdout.read(&mut chunk) {
                Ok(0) => {
                    if self.alive {
                        self.alive = false;
                        events.push(WebViewEvent::EngineExited);
                    }
                    break;
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    if self.alive {
                        self.alive = false;
                        events.push(WebViewEvent::EngineExited);
                    }
                    break;
                }
            }
        }
        if !self.alive {
            let _ = self.child.try_wait(); // reap the exited engine
        }
        // take the buffer so lines are parsed in place (no per-line Vec/String), then keep the partial tail
        let mut buf = std::mem::take(&mut self.buf);
        let mut start = 0;
        while let Some(off) = buf[start..].iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&buf[start..start + off]);
            frame |= self.handle(&line, &mut events);
            start += off + 1;
        }
        buf.drain(..start);
        // a line that never ends is garbage (or a flood): drop it rather than grow without bound
        if buf.len() > MAX_LINE {
            buf.clear();
        }
        self.buf = buf;
        (events, frame)
    }

    fn handle(&mut self, line: &str, events: &mut Vec<WebViewEvent>) -> bool {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        if verb == "fatal" {
            self.startup_error = Some(unescape_line(rest));
            return false;
        }
        if verb == "clipboard" {
            events.push(WebViewEvent::ClipboardChanged(unescape_line(rest)));
            return false;
        }
        if verb == "pong" {
            self.pong = true;
            return false;
        }
        if verb == "dev" {
            if self.dev_lines.len() < 4096 {
                self.dev_lines.push(rest.to_string());
            }
            return false;
        }
        if verb == "frame" {
            let mut n = rest.split(' ').map(|v| v.parse::<usize>().unwrap_or(0));
            let (w, h) = (n.next().unwrap_or(0), n.next().unwrap_or(0));
            let from = rest.split(' ').nth(2).and_then(|v| v.parse::<WebViewId>().ok());
            let len = w.saturating_mul(h).saturating_mul(4);
            if len > 0 && frame_is_current(self.visible, from) && len <= self.shm_len {
                if self.map.as_ref().is_none_or(|m| m.len() < len) {
                    self.map = Map::new(&self.shm, self.shm_len).ok();
                }
                if let Some(map) = &self.map {
                    let count = len / 4;
                    // The producer is waiting for ack. Finish this snapshot before allowing reuse;
                    // chrome redraws must read this owned frame, never the producer's buffer.
                    self.pixels.resize(count, 0);
                    self.pixels.copy_from_slice(&map.pixels()[..count]);
                    self.pixels_size = (w, h);
                }
            }
            // Stale/invalid frames also release the producer without replacing the visible snapshot.
            self.send("ack");
            return true;
        }
        let (id_text, tail) = rest.split_once(' ').unwrap_or((rest, ""));
        let Ok(id) = id_text.parse::<WebViewId>() else { return false };
        if matches!(verb, "file-picker" | "color-picker" | "unload" | "status")
            && (id_text.is_empty() || !id_text.bytes().all(|b| b.is_ascii_digit()))
        {
            return false;
        }
        if verb == "created" {
            let (opener, url) = tail.split_once(' ').unwrap_or((tail, ""));
            let opener = opener.parse().unwrap_or(0);
            self.views.insert(id, WebViewState { url: url.into(), ..Default::default() });
            events.push(WebViewEvent::CreateWebView { id, opener, url: url.into() });
            return false;
        }
        let Some(st) = self.views.get_mut(&id) else { return false };
        match verb {
            "file-picker" => {
                if let Some((token, request)) = tail.split_once(' ') {
                    if let (Some(token), Some((multiple, filters))) = (control_token(token), picker_request(request)) {
                        events.push(WebViewEvent::FilePicker { id, token, multiple, filters });
                    }
                }
            }
            "color-picker" => {
                let mut fields = tail.split(' ');
                if let (Some(token), Some(r), Some(g), Some(b), None) =
                    (fields.next(), fields.next(), fields.next(), fields.next(), fields.next())
                {
                    if let (Some(token), Some(r), Some(g), Some(b)) =
                        (control_token(token), color_channel(r), color_channel(g), color_channel(b))
                    {
                        events.push(WebViewEvent::ColorPicker { id, token, color: (r, g, b) });
                    }
                }
            }
            "unload" => {
                if let Some(token) = control_token(tail) {
                    // load/reload optimistically showed progress. The old document is still current
                    // until the user permits leaving; Stay may produce no further load notification.
                    st.loading = false;
                    st.progress = 1.0;
                    events.push(WebViewEvent::UnloadRequested { id, token });
                }
            }
            "status" => {
                // Escaping can expand a 4096-byte native status into twice as many wire bytes.
                if tail.len() <= MAX_STATUS_BYTES * 2 {
                    let text = unescape_line(tail);
                    if text.len() <= MAX_STATUS_BYTES && st.status_text != text {
                        st.status_text = text;
                        events.push(WebViewEvent::StatusTextChanged(id));
                    }
                }
            }
            "favicon" => {
                if tail.len() == 32 * 32 * 8 {
                    if let Some(rgba) = bifrost_core::plist::hex_decode(tail) {
                        events.push(WebViewEvent::FaviconChanged { id, rgba });
                    }
                }
            }
            "title" => {
                st.title = tail.into();
                events.push(WebViewEvent::TitleChanged(id));
            }
            "url" => {
                st.status_text.clear();
                st.url = tail.into();
                events.push(WebViewEvent::UrlChanged(id));
            }
            "load" => match tail {
                "started" => {
                    st.status_text.clear();
                    st.loading = true;
                    st.progress = 0.1;
                    events.push(WebViewEvent::LoadStarted(id));
                }
                "head" => {
                    st.progress = 0.5;
                    events.push(WebViewEvent::LoadCommitted(id));
                }
                _ => {
                    st.loading = false;
                    st.progress = 1.0;
                    events.push(WebViewEvent::LoadFinished(id));
                }
            },
            "history" => {
                let mut b = tail.split(' ');
                st.can_go_back = b.next() == Some("1");
                st.can_go_forward = b.next() == Some("1");
                events.push(WebViewEvent::HistoryChanged(id));
            }
            "found" => st.find_matches = tail.parse().ok(),
            "closed" => events.push(WebViewEvent::Closed(id)),
            "console" => {
                if let Some((level, source, line, text)) = parse_console(tail) {
                    events.push(WebViewEvent::Console { id, level, text, source, line });
                }
            }
            "dialog" => {
                let mut p = tail.splitn(3, ' ');
                let token = p.next().and_then(|t| t.parse().ok()).unwrap_or(0);
                let kind = match p.next() {
                    Some("confirm") => DialogKind::Confirm,
                    Some("prompt") => DialogKind::Prompt,
                    _ => DialogKind::Alert,
                };
                let (default, message) = p.next().unwrap_or("").split_once('\u{1}').unwrap_or(("", ""));
                events.push(WebViewEvent::Dialog { id, token, kind, message: unescape_line(message), default: unescape_line(default) });
            }
            "menu" => {
                let mut p = tail.splitn(8, ' ');
                let token = p.next().and_then(|t| t.parse().ok()).unwrap_or(0);
                let kind = if p.next() == Some("select") { MenuKind::Select } else { MenuKind::Context };
                let mut n = || p.next().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
                let rect = (n(), n(), n(), n());
                let items = parse_menu_items(p.next().unwrap_or(""));
                events.push(WebViewEvent::Menu { id, token, kind, rect, items });
            }
            "control-hide" => events.push(WebViewEvent::ControlHidden(id, tail.parse().unwrap_or(0))),
            "scriptmsg" => {
                if let Some(message) = crate::web_bridge::parse_script_message(tail) {
                    events.push(WebViewEvent::ScriptMessage { id, message });
                }
            }
            "navreq" => {
                if let Some((token, url)) = crate::web_bridge::parse_navigation_request(tail) {
                    events.push(WebViewEvent::NavigationRequested { id, token, url });
                }
            }
            "cookies" => {
                if let Some((token, cookies)) = crate::web_bridge::parse_cookies(tail) {
                    events.push(WebViewEvent::Cookies { id, token, cookies });
                }
            }
            "download" => events.push(WebViewEvent::DownloadStarted { id, url: tail.into(), suggested_filename: crate::web_bridge::suggested_filename(tail) }),
            "evalresult" => events.push(WebViewEvent::EvalResult(id, unescape_line(tail))),
            "crashed" => {
                st.loading = false;
                events.push(WebViewEvent::ContentProcessTerminated(id, tail.into()));
            }
            _ => {}
        }
        false
    }

    /// Draws the visible view's last frame into its frame rectangle (clipped to the surface). Before the first
    /// frame, nothing is drawn; the app's background shows.
    pub fn draw(&mut self, s: &mut Surface) {
        let (fx, fy, fw, fh) = self.frame;
        let (w, h) = self.pixels_size;
        if w == 0 || self.visible.is_none() || self.pixels.len() < w * h {
            return;
        }
        let pixels = &self.pixels;
        let cols = w.min(fw as usize);
        let rows = h.min(fh as usize);
        // Fast path: full-width contiguous copy avoiding row-by-row slice iterations
        if fx == 0 && cols == w && s.stride == w && s.y0 == 0 && fy >= 0 {
            let start_y = fy as usize;
            let max_rows = rows.min(s.h.saturating_sub(start_y));
            let total = max_rows * w;
            let dst_start = start_y * w;
            let dst_end = dst_start + total;
            if dst_end <= s.px.len() && total <= pixels.len() {
                s.px[dst_start..dst_end].copy_from_slice(&pixels[..total]);
            } else {
                for row in 0..max_rows {
                    let y = start_y + row;
                    let at = y * s.stride;
                    if at + cols <= s.px.len() && (row + 1) * w <= pixels.len() {
                        s.px[at..at + cols].copy_from_slice(&pixels[row * w..row * w + cols]);
                    }
                }
            }
        } else {
            for row in 0..rows {
                let y = fy as usize + row;
                if y < s.y0 || y >= s.h {
                    continue;
                }
                let x0 = (fx.max(0) as usize).min(s.w);
                let n = cols.min(s.w - x0);
                let at = (y - s.y0) * s.stride + x0;
                if at + n > s.px.len() {
                    break;
                }
                s.px[at..at + n].copy_from_slice(&pixels[row * w..row * w + n]);
            }
        }
    }

    /// The pixel size of the content frame.
    pub fn frame(&self) -> (i32, i32, u32, u32) {
        self.frame
    }

    /// The dimensions of the last rendered frame from the engine.
    pub fn pixels_size(&self) -> (usize, usize) {
        self.pixels_size
    }

    /// True when a valid frame with pixels has been received and is ready for display.
    pub fn has_frame(&self) -> bool {
        self.pixels_size.0 > 0 && self.pixels_size.1 > 0 && !self.pixels.is_empty() && self.visible.is_some()
    }
}

impl WebProcessPool {
    /// `pageZoom`, clamped to 0.3 through 5.0 (Browser's 30% to 500%).
    pub fn set_zoom(&mut self, id: WebViewId, factor: f32) {
        let f = factor.clamp(ZOOM_MIN, ZOOM_MAX);
        if let Some(st) = self.views.get_mut(&id) {
            st.page_zoom = f;
        }
        self.send(&format!("zoom {id} {f}"));
    }

    /// `find(_:configuration:)`. Case-insensitive, wraps around. The count arrives in `state(id).find_matches`.
    /// Servo 0.7.0 has no find API; the engine counts and selects through injected script.
    pub fn find(&mut self, id: WebViewId, query: &str, dir: FindDirection) {
        let d = match dir {
            FindDirection::New => 'n',
            FindDirection::Forward => 'f',
            FindDirection::Backward => 'b',
        };
        if let Some(st) = self.views.get_mut(&id) {
            st.find_matches = None;
        }
        self.send(&format!("find {id} {d} {query}"));
    }

    /// Clears the find highlight.
    pub fn find_clear(&mut self, id: WebViewId) {
        if let Some(st) = self.views.get_mut(&id) {
            st.find_matches = None;
        }
        self.send(&format!("find {id} c"));
    }
}

/// Page zoom range (`pageZoom`), 30% to 500%.
pub const ZOOM_MIN: f32 = 0.3;
pub const ZOOM_MAX: f32 = 5.0;

impl Drop for WebProcessPool {
    fn drop(&mut self) {
        self.send("quit");
        self.kill_engine();
        let _ = self.child.wait();
    }
}

fn terminate_engine_group(child: &mut Child) {
    extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
    // Each child creates its own process group before exec. A negative PID addresses only that group;
    // killing it also cleans up Servo content processes when the engine is wedged or has already exited.
    if let Ok(pid) = i32::try_from(child.id()) {
        if pid > 0 { unsafe { kill(-pid, 9); } }
    }
    let _ = child.kill();
}

/// The character of a letter, digit or punctuation key (evdev code), for Ctrl/Cmd shortcuts in the page.
fn evdev_char(key: u32) -> Option<char> {
    const ROWS: [(u32, &str); 4] = [(2, "1234567890-="), (16, "qwertyuiop[]"), (30, "asdfghjkl;'`"), (44, "zxcvbnm,./")];
    ROWS.iter().find_map(|(start, chars)| key.checked_sub(*start).and_then(|i| chars.chars().nth(i as usize)))
}

/// Turns what was typed in an address field into a URL (Browser's smart search field): a URL with a scheme is
/// kept, a host-like word (`example.com`, `localhost:8080`) gets `https://` (`http://` for localhost), anything else is a web search.
pub fn url_for_input(text: &str) -> String {
    let t = text.trim();
    if t.contains("://") || t.starts_with("about:") || t.starts_with("data:") || t.starts_with("file:") {
        return t.into();
    }
    let host = t.split('/').next().unwrap_or("");
    let bare = host.split(':').next().unwrap_or("");
    if !t.contains(' ') && (bare == "localhost" || bare == "127.0.0.1" || bare.ends_with(".local") || host.starts_with("localhost")) {
        return format!("http://{t}");
    }
    if !t.contains(' ') && host.contains('.') {
        return format!("https://{t}");
    }
    let q: String = t
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://duckduckgo.com/?q={q}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn frames_of_hidden_views_are_dropped() {
        use super::frame_is_current as ok;
        assert!(ok(Some(2), Some(2)) && ok(Some(2), None) && ok(None, None));
        assert!(!ok(Some(2), Some(1)) && !ok(None, Some(1)));
    }

    #[test]
    fn parsers_survive_garbage() {
        // deterministic xorshift fuzz over the pure parsers
        let mut x: u64 = 0x9E3779B97F4A7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let alphabet: Vec<char> = "abc \\|\u{1}\n\r0123456789-é\u{0}\u{10ffff}".chars().collect();
        for _ in 0..2000 {
            let len = (next() % 64) as usize;
            let s: String = (0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect();
            let _ = parse_menu_items(&s);
            let _ = parse_console(&s);
            let _ = unescape_line(&s);
            assert!(!escape_line(&s).contains('\n'));
        }
        assert!(parse_menu_items("18446744073709551616|1|x\u{1}-1|1|y\u{1}2|999|z").len() <= 1);
        assert!(escape_line(&"\u{10ffff}".repeat(5000)).len() <= CONSOLE_LINE_MAX);
    }

    #[test]
    fn handle_survives_malformed_lines() {
        let Ok(mut pool) = WebProcessPool::spawn(Path::new("/bin/cat"), Path::new("/tmp/bifrost-webview-test")) else { return };
        let mut ev = Vec::new();
        for l in ["", " ", "frame", "frame x y", "frame 18446744073709551615 18446744073709551615 1", "dialog", "dialog 1 2", "menu 1 select 99999999999 x", "console 1", "console 1 a", "unknown 1 x", "frame 99999999 99999999", "created", "created x", "title", "title 7", "load 1", "crashed -1 x", "closed zz", "\u{0}\u{ff} 1 2", "history 1", "scriptmsg 1", "scriptmsg 1 1", "navreq 1 x", "cookies 1 x", "cookies 1", "download 1"] {
            let _ = pool.handle(l, &mut ev);
        }
        assert!(ev.iter().all(|e| !matches!(e, WebViewEvent::EngineExited)));
    }

    use super::*;

    #[test]
    fn native_favicon_and_clipboard_events_validate_the_wire_payload() {
        let profile = std::env::temp_dir().join(format!("bifrost-native-events-{}", std::process::id()));
        let mut pool = WebProcessPool::spawn(Path::new("/bin/cat"), &profile).unwrap();
        pool.views.insert(7, WebViewState::default());
        let mut events = Vec::new();
        let encoded = "01ab02ff".repeat(32 * 32);
        pool.handle(&format!("favicon 7 {encoded}"), &mut events);
        assert_eq!(events, vec![WebViewEvent::FaviconChanged { id: 7, rgba: [1, 171, 2, 255].repeat(32 * 32) }]);
        events.clear();
        for line in [format!("favicon 8 {encoded}"), "favicon 7 00".into(), format!("favicon 7 {}", "z".repeat(encoded.len()))] {
            pool.handle(&line, &mut events);
        }
        assert!(events.is_empty(), "unknown views and malformed icon data must be ignored");
        pool.handle("clipboard Halló\\nworld\\\\path", &mut events);
        pool.handle("clipboard ", &mut events);
        assert_eq!(events, vec![WebViewEvent::ClipboardChanged("Halló\nworld\\path".into()), WebViewEvent::ClipboardChanged(String::new())]);
        drop(pool);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn zoom_clamps_and_find_resets_count() {
        assert_eq!(1.0f32.clamp(ZOOM_MIN, ZOOM_MAX), 1.0);
        assert!(ZOOM_MIN < 1.0 && ZOOM_MAX > 1.0);
        assert_eq!(WebViewState::default().find_matches, None);
    }

    #[test]
    fn address_input_becomes_a_url_or_a_search() {
        assert_eq!(url_for_input("example.com"), "https://example.com");
        assert_eq!(url_for_input("http://a.b/c"), "http://a.b/c");
        assert_eq!(url_for_input("rust lang"), "https://duckduckgo.com/?q=rust+lang");
        assert_eq!(url_for_input("localhost:8080/x"), "http://localhost:8080/x");
        assert_eq!(url_for_input("127.0.0.1:3000"), "http://127.0.0.1:3000");
        assert_eq!(url_for_input("nas.local/ui"), "http://nas.local/ui");
    }

    #[test]
    fn console_lines_round_trip_and_cap() {
        let t = "a\nb\\c\r";
        assert_eq!(unescape_line(&escape_line(t)), t);
        assert!(!escape_line(t).contains('\n'));
        assert!(escape_line(&"x".repeat(10_000)).len() <= CONSOLE_LINE_MAX);
        assert_eq!(parse_console("warn - 0 hi\\nthere"), Some(("warn".into(), String::new(), 0, "hi\nthere".into())));
        assert_eq!(parse_console("error http://a/b.js 12 boom"), Some(("error".into(), "http://a/b.js".into(), 12, "boom".into())));
    }

    #[test]
    fn address_input_edge_cases() {
        assert_eq!(url_for_input("  example.com  "), "https://example.com", "whitespace is trimmed");
        assert_eq!(url_for_input("about:blank"), "about:blank");
        assert_eq!(url_for_input("localhost"), "http://localhost");
        assert_eq!(url_for_input("a b.com"), "https://duckduckgo.com/?q=a+b.com", "a space makes it a search");
        assert_eq!(url_for_input("c++ & rust"), "https://duckduckgo.com/?q=c%2B%2B+%26+rust");
    }

    #[test]
    fn shortcut_key_rows_cover_the_keyboard() {
        assert_eq!(evdev_char(11), Some('0'));
        assert_eq!(evdev_char(16), Some('q'));
        assert_eq!(evdev_char(44), Some('z'));
        assert_eq!(evdev_char(53), Some('/'));
        assert_eq!(evdev_char(0), None);
    }

    #[test]
    fn shortcut_keys_map_to_characters() {
        assert_eq!(evdev_char(46), Some('c'));
        assert_eq!(evdev_char(30), Some('a'));
        assert_eq!(evdev_char(2), Some('1'));
        assert_eq!(evdev_char(200), None);
    }

    #[test]
    fn initial_frame_state_is_empty() {
        let Ok(pool) = WebProcessPool::spawn(Path::new("/bin/cat"), Path::new("/tmp/bifrost-webview-test-state")) else { return };
        assert_eq!(pool.pixels_size(), (0, 0));
        assert!(!pool.has_frame());
    }
}

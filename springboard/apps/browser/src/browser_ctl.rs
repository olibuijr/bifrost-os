//! Runs the parsed `browser.*` control requests (control.rs) against the Browser: tabs, navigation, console, DOM,
//! screenshots, network, input, viewport. Calls that need an answer from the page or the engine return a pending
//! marker (`RpcError::pending`) and are completed from `ctl_tick` / `ctl_on_event` when the engine answers
//! (sdk `control::complete`); a call that runs past its `timeout_ms` is answered here (navigate, wait) or by the
//! SDK with error -32000.
use std::collections::HashSet;
use std::time::{Duration, Instant};

use bifrost_app::control::{self as rpc, RpcError, INTERNAL_ERROR, INVALID_PARAMS, TIMEOUT};
use bifrost_app::webview::{url_for_input, FindDirection, WebViewEvent, WebViewId};
use bifrost_app::webview_dev::{DevEvent, EvalValue, ShotMode};
use bifrost_core::json::Json;

use crate::control::{self, FindDir, Input, Req, Wait, WaitFor};
use crate::{devmenu, network, pagetools, resample, session, Browser, ENGINE, START};

/// Poll interval of conditions that need a script (`browser.wait`).
const POLL: Duration = Duration::from_millis(100);
/// How long the page must stay unchanged for `browser.wait for=idle`.
const IDLE_QUIET: Duration = Duration::from_millis(500);
/// A navigation that never reports a start is taken as done after this, once the view is not loading.
const NAV_GRACE: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct CtlState {
    pend: Vec<Pend>,
    crashed: HashSet<WebViewId>,
    relayout: bool,
    pub(crate) ua: Option<String>,
    /// content blockers and the page message log (webbridge.rs)
    pub(crate) bridge: crate::webbridge::Bridge,
}

impl CtlState {
    /// True once after a call changed the layout (new tab, viewport, devtools): the app re-places the web content.
    pub fn take_relayout(&mut self) -> bool {
        std::mem::take(&mut self.relayout)
    }
}

struct Pend {
    /// the control call to answer
    token: u64,
    view: WebViewId,
    deadline: Instant,
    started: Instant,
    /// the engine request in flight (0 when none)
    eval_tok: u64,
    /// Script held back until the revived (discarded) tab finishes loading
    deferred: Option<String>,
    step: Step,
}

enum Step {
    Memory,
    EnginePreferences,
    Eval,
    Dom,
    Source { url: String },
    /// draining the page's captured console entries before the log is read
    Console { level: Option<String>, limit: usize, since_seq: Option<u64>, clear: bool },
    Inspect,
    Net { since: Option<u64>, limit: usize, clear: bool },
    /// waiting for the viewport (or element) geometry before the screenshot is taken
    ShotInfo { full: bool, selector: Option<String>, rect: Option<(f32, f32, f32, f32)>, max_pixels: u64, scale: f32, pdf: Option<bifrost_app::web_print::PrintLayout> },
    Shot { path: String, dpr: f64, scroll: (f64, f64), scale: f32, pdf: Option<bifrost_app::web_print::PrintLayout> },
    Nav { wait: Wait, saw: bool },
    Wait { what: WaitFor, base_seq: u64, inflight: bool, next_poll: Instant, quiet: Option<(String, Instant)> },
    Find,
}

impl Step {
    fn engine_wide(&self) -> bool {
        matches!(self, Self::Memory | Self::EnginePreferences)
    }
}

fn internal(msg: impl Into<String>) -> RpcError {
    RpcError::new(INTERNAL_ERROR, msg)
}

fn done(token: u64, r: Result<Json, RpcError>) {
    rpc::complete(token, r);
}

fn ms(d: Duration) -> Json {
    control::num(d.as_millis() as f64)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}...", s.chars().take(max).collect::<String>()) }
}

impl Browser {
    // ---- helpers ----

    fn engine_check(&self) -> Result<(), RpcError> {
        match &self.pool {
            Some(p) if p.alive() => Ok(()),
            Some(_) => Err(control::engine_down(self.error.as_deref().unwrap_or("the engine process exited"))),
            None => Err(control::engine_down(self.error.as_deref().unwrap_or("the engine has not started"))),
        }
    }

    fn ctl_tab(&self, id: Option<u32>) -> Result<WebViewId, RpcError> {
        match id {
            Some(i) if self.tabs.iter().any(|t| t.view == i) => Ok(i),
            Some(i) => Err(control::no_tab(i)),
            None => self.view().ok_or_else(|| RpcError::new(control::NO_TAB, "no tab is open")),
        }
    }

    fn tab_index(&self, id: u32) -> Result<usize, RpcError> {
        self.tabs.iter().position(|t| t.view == id).ok_or_else(|| control::no_tab(id))
    }

    fn page_zoom(&self, view: WebViewId) -> f64 {
        let z = self.pool.as_ref().and_then(|p| p.state(view)).map_or(1.0, |s| s.page_zoom);
        if z == 0.0 { 1.0 } else { z as f64 }
    }

    /// Context menu Inspect Element: opens the Elements panel and selects the element under the last click.
    /// Window pixels become CSS pixels through the content frame, the device scale and the page zoom.
    pub(crate) fn inspect_element(&mut self) {
        let (x, y) = self.pageui.last_point;
        let Some(view) = self.view() else { return };
        let zoom = self.page_zoom(view) as f32;
        let Some(p) = self.pool.as_mut() else { return };
        self.inspector.set(Some(true), Some(control::Panel::Elements), Some(&mut *p));
        let (fx, fy, _, _) = p.frame();
        let s = (p.device_scale() * zoom).max(0.1);
        let (cx, cy) = ((x - fx as f32) / s, (y - fy as f32) / s);
        if cx >= 0.0 && cy >= 0.0 {
            self.inspector.inspect_point(p, view, cx, cy);
        }
    }

    /// Registers an async call (optionally sending `js` to the engine first) and returns its pending marker.
    fn ctl_pend(&mut self, view: WebViewId, timeout_ms: u64, js: Option<String>, step: Step) -> Result<RpcError, RpcError> {
        // a discarded tab comes back (same id) when the control socket or inspector needs its page
        let mut revived = false;
        if let Some(p) = self.pool.as_mut() {
            revived = p.is_discarded(view);
            p.revive(view);
        }
        self.engine_check()?;
        let token = rpc::new_token();
        let mut eval_tok = 0;
        let mut deferred = None;
        if let (Some(js), Some(p)) = (js, self.pool.as_mut()) {
            eval_tok = rpc::new_token();
            if revived {
                // a revived page has not loaded yet: run the script on LoadFinished (ctl_on_event)
                deferred = Some(js);
            } else {
                p.dev_eval(view, eval_tok, &js);
            }
        }
        let now = Instant::now();
        self.ctl.pend.push(Pend { token, view, deadline: now + Duration::from_millis(timeout_ms), started: now, eval_tok, deferred, step });
        Ok(RpcError::pending(token, timeout_ms + 1500))
    }

    /// Engine diagnostics have their own pending path: inspecting resource use must not revive discarded tabs.
    fn ctl_engine_pend(&mut self, timeout_ms: u64, step: Step) -> Result<RpcError, RpcError> {
        self.engine_check()?;
        let token = rpc::new_token();
        let eval_tok = rpc::new_token();
        if let Some(pool) = self.pool.as_mut() {
            match step {
                Step::Memory => pool.dev_memory(eval_tok),
                Step::EnginePreferences => pool.dev_preferences(eval_tok),
                _ => return Err(internal("not an engine diagnostic request")),
            }
        }
        let now = Instant::now();
        self.ctl.pend.push(Pend { token, view: 0, deadline: now + Duration::from_millis(timeout_ms), started: now, eval_tok, deferred: None, step });
        Ok(RpcError::pending(token, timeout_ms + 1500))
    }

    fn view_json(&self, i: usize) -> Json {
        let Some(t) = self.tabs.get(i) else { return Json::Null };
        let st = self.pool.as_ref().and_then(|p| p.state(t.view));
        let url = st.map(|s| s.url.clone()).unwrap_or_default();
        control::obj(vec![
            ("id", control::num(t.view as f64)),
            ("index", control::num(i as f64)),
            ("url", control::text(&url)),
            ("title", control::text(st.map_or("", |s| s.title.as_str()))),
            ("loading", Json::Bool(st.is_some_and(|s| s.loading))),
            ("can_back", Json::Bool(st.is_some_and(|s| s.can_go_back))),
            ("can_forward", Json::Bool(st.is_some_and(|s| s.can_go_forward))),
            ("current", Json::Bool(i == self.current)),
            ("crashed", Json::Bool(self.ctl.crashed.contains(&t.view))),
            ("zoom", control::num(self.page_zoom(t.view))),
            ("secure", Json::Bool(url.starts_with("https://"))),
        ])
    }

    /// CSS size and ratio of the page viewport.
    fn viewport_now(&self) -> (f64, f64, f64) {
        if let Some(r) = &self.dev.responsive {
            return (r.w as f64, r.h as f64, r.dpr as f64);
        }
        match &self.pool {
            Some(p) => {
                let (_, _, w, h) = p.frame();
                let s = if p.device_scale() > 0.0 { p.device_scale() as f64 } else { 1.0 };
                (w as f64 / s, h as f64 / s, s)
            }
            None => (0.0, 0.0, 1.0),
        }
    }

    // ---- entry points ----

    pub(crate) fn ctl_methods(&self) -> Vec<&'static str> {
        control::METHODS.to_vec()
    }

    /// Answers one `browser.*` call (or `console`). Err with `RpcError::pending` means "answer later".
    pub(crate) fn ctl_rpc(&mut self, method: &str, params: &Json) -> Result<Json, RpcError> {
        let req = control::parse(method, params)?;
        match req {
            Req::Memory { timeout_ms } => Err(self.ctl_engine_pend(timeout_ms, Step::Memory)?),
            Req::EnginePreferences { timeout_ms } => Err(self.ctl_engine_pend(timeout_ms, Step::EnginePreferences)?),
            Req::State => {
                let (w, h, dpr) = self.viewport_now();
                let cur = self.view();
                Ok(control::obj(vec![
                    ("version", control::text(env!("CARGO_PKG_VERSION"))),
                    ("engine_version", control::text(ENGINE)),
                    ("engine_alive", Json::Bool(self.pool.as_ref().is_some_and(|p| p.alive()))),
                    ("engine_pid", self.pool.as_ref().filter(|p| p.alive()).map_or(Json::Null, |p| control::num(p.dev_pid() as f64))),
                    ("restarts", control::num(self.storm.restarts_in_window() as f64)),
                    ("tabs", control::num(self.tabs.len() as f64)),
                    ("current", cur.map_or(Json::Null, |v| control::num(v as f64))),
                    ("devtools_open", Json::Bool(self.inspector.is_open())),
                    ("viewport", control::obj(vec![("w", control::num(w.round())), ("h", control::num(h.round())), ("dpr", control::num(dpr))])),
                    ("zoom", control::num(cur.map_or(1.0, |v| self.page_zoom(v)))),
                    ("user_agent", control::text(self.ctl.ua.as_deref().or(self.dev.user_agent()).unwrap_or("default"))),
                ]))
            }
            Req::Downloads { cancel, clear } => {
                let cancelled = cancel.map(|i| self.downloads.cancel(i));
                if clear {
                    self.downloads.clear_finished();
                }
                let mut o = vec![("downloads", self.downloads.json())];
                if let Some(c) = cancelled {
                    o.push(("cancelled", Json::Bool(c)));
                }
                Ok(control::obj(o))
            }
            Req::Bookmarks { op, url, title, query, limit, path } => {
                let now = crate::store::now();
                let mut imported = None;
                let cur = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| (s.url.clone(), s.title.clone()));
                let done = match op.as_str() {
                    "add" => {
                        let (u, t) = match (url, cur) {
                            (Some(u), c) => (u.clone(), title.or_else(|| c.filter(|c| c.0 == u).map(|c| c.1)).unwrap_or_default()),
                            (None, Some((u, t))) => (u, title.unwrap_or(t)),
                            (None, None) => return Err(control::invalid("no url and no current page")),
                        };
                        Some(self.store.add_bookmark(&u, &t, now))
                    }
                    "remove" => Some(self.store.remove_bookmark(&url.unwrap_or_default())),
                    "rename" => Some(self.store.rename_bookmark(&url.unwrap_or_default(), &title.unwrap_or_default())),
                    "import" => {
                        let file = path.unwrap_or_default();
                        let html = std::fs::read_to_string(&file).map_err(|e| control::invalid(format!("cannot read {file}: {e}")))?;
                        imported = Some(crate::library::import_html(&mut self.store, &html, now));
                        Some(true)
                    }
                    _ => None,
                };
                let mut o = vec![("bookmarks", entries_json(&self.store.bookmarks, query.as_deref(), limit))];
                if let Some(d) = done {
                    o.push(("ok", Json::Bool(d)));
                }
                if let Some(n) = imported {
                    o.push(("imported", Json::Num(n as f64)));
                }
                Ok(control::obj(o))
            }
            Req::History { query, limit, remove, clear, clear_since } => {
                let removed = remove.map(|u| self.store.remove_history(&u));
                if clear {
                    self.clear_browsing_history();
                }
                let cleared = clear_since.map(|since| self.store.clear_history_during(since, u64::MAX));
                let mut sorted = self.store.history.clone();
                sorted.sort_by(|a, b| b.last.cmp(&a.last));
                let mut o = vec![("history", entries_json(&sorted, query.as_deref(), limit))];
                if let Some(r) = removed {
                    o.push(("removed", Json::Bool(r)));
                }
                if let Some(n) = cleared {
                    o.push(("cleared", Json::Num(n as f64)));
                }
                Ok(control::obj(o))
            }
            Req::Favicon { id } => {
                let view = self.ctl_tab(id)?;
                let page = self.pool.as_ref().and_then(|p| p.state(view)).map(|s| s.url.clone()).unwrap_or_default();
                let url = self.favicons.url_of(view, &page);
                Ok(control::obj(vec![
                    ("id", control::num(view as f64)),
                    ("host", crate::favicon::host_key(&page).map_or(Json::Null, |h| control::text(&h))),
                    ("url", url.as_deref().map_or(Json::Null, control::text)),
                    ("cached", Json::Bool(url.is_some())),
                ]))
            }
            Req::Tabs => Ok(Json::Arr((0..self.tabs.len()).map(|i| self.view_json(i)).collect())),
            Req::TabNew { url, background } => {
                self.engine_check()?;
                let url = url.map(|u| url_for_input(&u)).unwrap_or_else(|| START.to_string());
                let prev = self.current;
                let had = self.tabs.len();
                self.new_tab(&url);
                let id = self.tabs.last().filter(|_| self.tabs.len() > had).map(|t| t.view).ok_or_else(|| internal("the tab could not be created"))?;
                if background && had > 0 {
                    self.select(prev);
                }
                self.ctl.relayout = true;
                Ok(control::obj(vec![("id", control::num(id as f64))]))
            }
            Req::TabSelect { id } => {
                let i = self.tab_index(id)?;
                self.select(i);
                self.ctl.relayout = true;
                Ok(control::obj(vec![("ok", Json::Bool(true))]))
            }
            Req::TabClose { id } => {
                let i = self.tab_index(id)?;
                self.close_tab(i);
                self.ctl.relayout = true;
                Ok(control::obj(vec![("ok", Json::Bool(true))]))
            }
            Req::TabReopen => {
                self.engine_check()?;
                let had = self.tabs.len();
                self.reopen_closed();
                if self.tabs.len() == had {
                    return Err(control::invalid("there is no closed tab to reopen"));
                }
                self.ctl.relayout = true;
                // the reopened tab is selected, at the place it had in the strip
                let id = self.view().unwrap_or(0);
                Ok(control::obj(vec![("id", control::num(id as f64))]))
            }
            Req::Navigate { id, url, wait, timeout_ms } => {
                self.engine_check()?;
                let view = self.ctl_tab(id)?;
                let url = url_for_input(&url);
                if let Some(p) = self.pool.as_mut() {
                    p.load(view, &url);
                }
                if wait == Wait::None {
                    return Ok(self.nav_reply(view, "started"));
                }
                Err(self.ctl_pend(view, timeout_ms, None, Step::Nav { wait, saw: false })?)
            }
            Req::Back { id } | Req::Forward { id } | Req::Reload { id, .. } => {
                self.engine_check()?;
                let view = self.ctl_tab(id)?;
                let st = self.pool.as_ref().and_then(|p| p.state(view)).cloned().unwrap_or_default();
                let Some(p) = self.pool.as_mut() else { return Err(control::engine_down("the engine has not started")) };
                let ok = match method {
                    "browser.back" => {
                        p.go_back(view);
                        st.can_go_back
                    }
                    "browser.forward" => {
                        p.go_forward(view);
                        st.can_go_forward
                    }
                    _ => {
                        if params.get("hard") == Some(&Json::Bool(true)) {
                            p.reload_ignoring_cache(view);
                        } else {
                            p.reload(view);
                        }
                        true
                    }
                };
                Ok(control::obj(vec![("ok", Json::Bool(ok))]))
            }
            Req::Eval { id, js, timeout_ms } => {
                let view = self.ctl_tab(id)?;
                self.inspector.record(view, "input", js.clone(), String::new(), 0);
                Err(self.ctl_pend(view, timeout_ms, Some(js), Step::Eval)?)
            }
            Req::Console { id, level, limit, since_seq, clear } => {
                let view = self.ctl_tab(id)?;
                // drain the page's captured entries first so every line has its file and line
                let step = Step::Console { level: level.clone(), limit, since_seq, clear };
                match self.ctl_pend(view, 5_000, Some(pagetools::drain_console_js()), step) {
                    Ok(pending) => Err(pending),
                    Err(_) => Ok(self.inspector.console_json(view, level.as_deref(), limit, since_seq, clear)),
                }
            }
            Req::Dom { id, selector, mode, limit, max_bytes, timeout_ms } => {
                let view = self.ctl_tab(id)?;
                let js = control::js_dom(&selector, mode, limit, max_bytes);
                Err(self.ctl_pend(view, timeout_ms, Some(js), Step::Dom)?)
            }
            Req::Source { id, timeout_ms } => {
                let view = self.ctl_tab(id)?;
                let url = self.pool.as_ref().and_then(|p| p.state(view)).map(|s| s.url.clone()).unwrap_or_default();
                Err(self.ctl_pend(view, timeout_ms, Some(control::JS_SOURCE.to_string()), Step::Source { url })?)
            }
            Req::Screenshot { id, full_page, selector, rect, max_pixels, timeout_ms, scale, pdf } => {
                let view = self.ctl_tab(id)?;
                let js = match &selector {
                    Some(s) => control::js_element_rect(s),
                    None => control::JS_VIEW_INFO.to_string(),
                };
                Err(self.ctl_pend(view, timeout_ms, Some(js), Step::ShotInfo { full: full_page, selector, rect, max_pixels, scale, pdf })?)
            }
            Req::Inspect { id, x, y, timeout_ms } => {
                let view = self.ctl_tab(id)?;
                Err(self.ctl_pend(view, timeout_ms, Some(pagetools::js_inspect_point(x, y)), Step::Inspect)?)
            }
            Req::Network { id, since_seq, limit, clear } => {
                let view = self.ctl_tab(id)?;
                Err(self.ctl_pend(view, 5_000, Some(network::drain_js()), Step::Net { since: since_seq, limit, clear })?)
            }
            Req::Wait { id, what, timeout_ms } => {
                let view = self.ctl_tab(id)?;
                if what == WaitFor::Load && !self.pool.as_ref().and_then(|p| p.state(view)).is_some_and(|s| s.loading) {
                    return Ok(control::obj(vec![("ok", Json::Bool(true)), ("waited_ms", control::num(0.0))]));
                }
                let base_seq = self.inspector.last_seq(view);
                let step = Step::Wait { what, base_seq, inflight: false, next_poll: Instant::now(), quiet: None };
                Err(self.ctl_pend(view, timeout_ms, None, step)?)
            }
            Req::Viewport { size, dpr, reset } => {
                if reset {
                    self.dev.responsive = None;
                } else if size.is_some() || dpr.is_some() {
                    let (cw, ch, cd) = self.viewport_now();
                    let (w, h) = size.unwrap_or((cw.round().max(1.0) as u32, ch.round().max(1.0) as u32));
                    let d = dpr.map_or(cd.round().max(1.0) as u32, |d| d.round().max(1.0) as u32);
                    self.dev.responsive = Some(devmenu::Responsive { w, h, dpr: d });
                }
                self.ctl.relayout = true;
                let (w, h, d) = self.viewport_now();
                Ok(control::obj(vec![("width", control::num(w.round())), ("height", control::num(h.round())), ("dpr", control::num(d))]))
            }
            Req::Zoom { factor } => {
                let view = self.ctl_tab(None)?;
                if let Some(f) = factor {
                    self.engine_check()?;
                    let url = self.pool.as_ref().and_then(|p| p.state(view)).map(|s| s.url.clone()).unwrap_or_default();
                    if let Some(p) = self.pool.as_mut() {
                        p.set_zoom(view, f);
                    }
                    let o = session::origin(&url);
                    if !o.is_empty() {
                        if (f - 1.0).abs() < 0.001 {
                            self.session.zoom.remove(&o);
                        } else {
                            self.session.zoom.insert(o, f);
                        }
                    }
                }
                Ok(control::obj(vec![("factor", control::num(self.page_zoom(view)))]))
            }
            Req::Find { id, query, dir } => {
                self.engine_check()?;
                let view = self.ctl_tab(id)?;
                let Some(p) = self.pool.as_mut() else { return Err(control::engine_down("the engine has not started")) };
                match dir {
                    FindDir::Clear => {
                        p.find_clear(view);
                        Ok(control::obj(vec![("count", control::num(0.0))]))
                    }
                    d => {
                        let dir = match d {
                            FindDir::Back => FindDirection::Backward,
                            FindDir::Forward => FindDirection::Forward,
                            _ => FindDirection::New,
                        };
                        p.find(view, &query, dir);
                        Err(self.ctl_pend(view, 3_000, None, Step::Find)?)
                    }
                }
            }
            Req::ContentRules { op, identifier, rules, url, resource, top_url } => {
                let profile = self.profile();
                let pool = self.pool.as_mut();
                self.ctl
                    .bridge
                    .content_rules(pool, &profile, &op, identifier.as_deref(), rules.as_deref(), url.as_deref(), resource.as_deref(), top_url.as_deref())
                    .map_err(control::invalid)
            }
            Req::Messages { since_seq, clear } => Ok(self.ctl.bridge.messages_json(since_seq, clear)),
            Req::UserAgent { ua } => {
                match ua {
                    None => Ok(control::obj(vec![("ua", control::text(self.ctl.ua.as_deref().or(self.dev.user_agent()).unwrap_or("default")))])),
                    Some(ua) => {
                        self.ctl.ua = Some(ua.clone());
                        if let Some(p) = self.pool.as_mut() {
                            p.set_user_agent(Some(&ua).filter(|u| !u.is_empty()).map(|u| u.as_str()));
                        }
                        Ok(control::obj(vec![("ua", control::text(&ua)), ("restart_required", Json::Bool(false))]))
                    }
                }
            }
            Req::Input { id, input } => {
                self.engine_check()?;
                let view = self.ctl_tab(id)?;
                if self.view() != Some(view) {
                    return Err(control::invalid("page input goes to the current tab: select the tab first (browser.tab.select)"));
                }
                let (vw, vh, _) = self.viewport_now();
                let Some(p) = self.pool.as_mut() else { return Err(control::engine_down("the engine has not started")) };
                match input {
                    Input::Click { x, y } => {
                        p.dev_pointer("d", x, y);
                        p.dev_pointer("u", x, y);
                    }
                    Input::Move { x, y } => p.dev_pointer("m", x, y),
                    Input::Text(t) => p.dev_text(&t),
                    Input::Key { key, mods } => {
                        let mut chars = key.chars();
                        match (chars.next(), chars.next()) {
                            (Some(c), None) if mods == 0 => p.dev_text(&c.to_string()),
                            (Some(c), None) => p.dev_chord(mods, c.to_ascii_lowercase()),
                            _ => p.dev_key(&key),
                        }
                    }
                    Input::Scroll { dx, dy, x, y } => {
                        let (x, y) = if x < 0.0 || y < 0.0 { ((vw / 2.0) as f32, (vh / 2.0) as f32) } else { (x, y) };
                        p.dev_wheel(dx, dy, x, y);
                    }
                }
                Ok(control::obj(vec![("ok", Json::Bool(true))]))
            }
            Req::Devtools { open, panel } => {
                self.inspector.set(open, panel, self.pool.as_mut());
                self.ctl.relayout = true;
                Ok(control::obj(vec![("open", Json::Bool(self.inspector.is_open())), ("panel", control::text(self.inspector.panel_kind().name()))]))
            }
        }
    }

    fn nav_reply(&self, view: WebViewId, status: &str) -> Json {
        let st = self.pool.as_ref().and_then(|p| p.state(view));
        control::obj(vec![
            ("id", control::num(view as f64)),
            ("url", control::text(st.map_or("", |s| s.url.as_str()))),
            ("title", control::text(st.map_or("", |s| s.title.as_str()))),
            ("status", control::text(status)),
        ])
    }

    // ---- engine answers ----

    /// Engine events that matter to pending calls (load progress, tab and engine death).
    pub(crate) fn ctl_on_event(&mut self, ev: &WebViewEvent) {
        match ev {
            WebViewEvent::ContentProcessTerminated(v, _) => {
                self.ctl.crashed.insert(*v);
            }
            WebViewEvent::LoadStarted(v) | WebViewEvent::LoadCommitted(v) => {
                self.ctl.crashed.remove(v);
            }
            WebViewEvent::EngineExited => {
                for p in std::mem::take(&mut self.ctl.pend) {
                    done(p.token, Err(control::engine_down("the engine process exited")));
                }
                self.ctl.crashed.clear();
                return;
            }
            _ => {}
        }
        let mut keep = Vec::new();
        for mut p in std::mem::take(&mut self.ctl.pend) {
            if p.step.engine_wide() {
                keep.push(p);
                continue;
            }
            let mine = match ev {
                WebViewEvent::LoadStarted(v) | WebViewEvent::LoadCommitted(v) | WebViewEvent::LoadFinished(v) | WebViewEvent::Closed(v) | WebViewEvent::ContentProcessTerminated(v, _) => *v == p.view,
                _ => false,
            };
            if !mine {
                keep.push(p);
                continue;
            }
            if let (WebViewEvent::LoadFinished(_), Some(js)) = (ev, p.deferred.take()) {
                if let Some(pool) = self.pool.as_mut() {
                    pool.dev_eval(p.view, p.eval_tok, &js);
                }
            }
            if matches!(ev, WebViewEvent::Closed(_)) {
                done(p.token, Err(control::no_tab(p.view)));
                continue;
            }
            if let (WebViewEvent::ContentProcessTerminated(_, reason), Step::Nav { .. }) = (ev, &p.step) {
                done(p.token, Err(RpcError::new(INTERNAL_ERROR, "the page's content process crashed").with_reason(reason.clone())));
                continue;
            }
            if let Step::Nav { wait, saw } = &mut p.step {
                match ev {
                    WebViewEvent::LoadStarted(_) => *saw = true,
                    WebViewEvent::LoadCommitted(_) => {
                        *saw = true;
                        if *wait == Wait::Commit {
                            done(p.token, Ok(self.nav_reply(p.view, "committed")));
                            continue;
                        }
                    }
                    WebViewEvent::LoadFinished(_) if *saw => {
                        done(p.token, Ok(self.nav_reply(p.view, "loaded")));
                        continue;
                    }
                    _ => {}
                }
            }
            keep.push(p);
        }
        self.ctl.pend = keep;
    }

    /// Reads the engine's token-correlated answers, completes what they finish, and runs the polls and deadlines.
    /// Returns true when the panel needs a redraw.
    pub(crate) fn ctl_tick(&mut self) -> bool {
        let mut changed = false;
        let events = self.pool.as_mut().map(|p| p.dev_events()).unwrap_or_default();
        for ev in events {
            if self.pool.as_mut().is_some_and(|p| self.reader.on_dev(p, &ev)) {
                changed = true;
                continue;
            }
            if self.inspector.on_dev(&ev) {
                changed = true;
                continue;
            }
            self.ctl_on_dev(ev);
        }
        let view = self.view();
        if let Some(p) = self.pool.as_mut() {
            changed |= self.inspector.poll(p, view);
        }
        self.ctl_polls();
        changed
    }

    fn ctl_on_dev(&mut self, ev: DevEvent) {
        let token = match &ev {
            DevEvent::Eval { token, .. } | DevEvent::Shot { token, .. } | DevEvent::Memory { token, .. } | DevEvent::Preferences { token, .. } => *token,
        };
        let Some(i) = self.ctl.pend.iter().position(|p| p.eval_tok == token) else { return };
        let mut p = self.ctl.pend.remove(i);
        p.eval_tok = 0;
        let kept = match ev {
            DevEvent::Eval { result, .. } => self.on_eval(p, result),
            DevEvent::Shot { result, .. } => self.on_shot(p, result),
            DevEvent::Memory { result, .. } if matches!(p.step, Step::Memory) => {
                done(p.token, result.map_err(internal));
                None
            }
            DevEvent::Preferences { result, .. } if matches!(p.step, Step::EnginePreferences) => {
                done(p.token, result.map_err(internal));
                None
            }
            DevEvent::Memory { .. } | DevEvent::Preferences { .. } => Some(p),
        };
        if let Some(p) = kept {
            self.ctl.pend.push(p);
        }
    }

    /// An eval answer for a pending call. `Some` keeps the call pending.
    fn on_eval(&mut self, mut p: Pend, res: Result<EvalValue, String>) -> Option<Pend> {
        let token = p.token;
        match &mut p.step {
            Step::Eval => {
                let reply = match res {
                    Ok(v) => {
                        self.inspector.record(p.view, "result", shorten(&v.json, 4000), String::new(), 0);
                        control::eval_ok(&v.ty, &v.json)
                    }
                    Err(e) => {
                        self.inspector.record(p.view, "error", e.clone(), String::new(), 0);
                        control::eval_err(&e)
                    }
                };
                done(token, Ok(reply));
                None
            }
            Step::Dom => {
                let r = res.map_err(internal).and_then(|v| control::inner_string(&v.ty, &v.json).ok_or_else(|| internal("unexpected reply from the page"))).and_then(|raw| control::dom_reply(&raw));
                done(token, r);
                None
            }
            Step::Source { url } => {
                let r = res.map_err(internal).and_then(|v| control::inner_string(&v.ty, &v.json).ok_or_else(|| internal("the page has no document"))).map(|html| control::obj(vec![("url", control::text(url)), ("html", Json::Str(html))]));
                done(token, r);
                None
            }
            Step::Console { level, limit, since_seq, clear } => {
                // a failed drain (page mid-load) still answers from what the log has
                if let Ok(v) = &res {
                    if let Some(raw) = control::inner_string(&v.ty, &v.json) {
                        self.inspector.ingest_caps(p.view, &raw);
                    }
                }
                done(token, Ok(self.inspector.console_json(p.view, level.as_deref(), *limit, *since_seq, *clear)));
                None
            }
            Step::Inspect => {
                let found = res.map_err(internal).and_then(|v| match v.ty.as_str() {
                    "null" => Ok(None),
                    _ => control::inner_string(&v.ty, &v.json).and_then(|s| bifrost_core::json::parse(&s).ok()).map(Some).ok_or_else(|| internal("unexpected reply from the page")),
                });
                match found {
                    Ok(Some(j)) => {
                        // select it in the Elements panel (the highlight stays in the page until deselected)
                        if let (Some(path), Some(pool)) = (j.get("path").and_then(Json::as_str), self.pool.as_mut()) {
                            self.inspector.select(pool, p.view, Some(path.to_string()));
                        }
                        done(token, Ok(j));
                    }
                    Ok(None) => done(token, Err(control::invalid("no element at that point (outside the viewport?)"))),
                    Err(e) => done(token, Err(e)),
                }
                None
            }
            Step::Net { since, limit, clear } => {
                if let Ok(v) = &res {
                    if let Some(raw) = control::inner_string(&v.ty, &v.json) {
                        self.inspector.net.ingest(p.view, &raw, control::now_ms());
                    }
                }
                let reply = self.inspector.net.query(p.view, *since, *limit, *clear);
                done(token, Ok(reply));
                None
            }
            Step::ShotInfo { full, selector, rect, max_pixels, scale, pdf } => {
                let info = match res.map_err(internal).and_then(|v| match v.ty.as_str() {
                    "null" => Err(control::invalid("no element matches the selector")),
                    _ => control::inner_string(&v.ty, &v.json).and_then(|s| bifrost_core::json::parse(&s).ok()).ok_or_else(|| internal("unexpected reply from the page")),
                }) {
                    Ok(j) => j,
                    Err(e) => {
                        done(token, Err(e));
                        return None;
                    }
                };
                if let Some(msg) = info.get("error").and_then(Json::as_str) {
                    done(token, Err(RpcError::new(INVALID_PARAMS, format!("selector: {msg}"))));
                    return None;
                }
                let n = |k: &str| info.get(k).and_then(Json::number).unwrap_or(0.0);
                let mode = if selector.is_some() {
                    if n("w") < 1.0 || n("h") < 1.0 {
                        done(token, Err(control::invalid("the element has no visible area")));
                        return None;
                    }
                    ShotMode::Rect { x: n("x") as f32, y: n("y") as f32, w: n("w") as f32, h: n("h") as f32 }
                } else if let Some((x, y, w, h)) = rect {
                    ShotMode::Rect { x: *x, y: *y, w: *w, h: *h }
                } else if *full {
                    ShotMode::Full
                } else {
                    ShotMode::View
                };
                let dpr = if n("dpr") > 0.0 { n("dpr") } else { 1.0 };
                let path = std::env::temp_dir().join(format!("bifrost-browser-shot-{}-{}.png", std::process::id(), token)).to_string_lossy().into_owned();
                let tok = rpc::new_token();
                let (view, max_px) = (p.view, *max_pixels);
                match self.pool.as_mut() {
                    Some(pool) if pool.alive() => pool.dev_screenshot(view, tok, &mode, max_px, &path),
                    _ => {
                        done(token, Err(control::engine_down("the engine process exited")));
                        return None;
                    }
                }
                p.eval_tok = tok;
                p.step = Step::Shot { path, dpr, scroll: (n("sx"), n("sy")), scale: *scale, pdf: *pdf };
                Some(p)
            }
            Step::Wait { what, inflight, next_poll, quiet, .. } => {
                *inflight = false;
                *next_poll = Instant::now() + POLL;
                let waited = ms(p.started.elapsed());
                let ok = |waited: Json| control::obj(vec![("ok", Json::Bool(true)), ("waited_ms", waited)]);
                match (what.clone(), res) {
                    (WaitFor::Idle, Ok(v)) => {
                        let sample = control::inner_string(&v.ty, &v.json).unwrap_or_default();
                        let loading = self.pool.as_ref().and_then(|pl| pl.state(p.view)).is_some_and(|s| s.loading);
                        let complete = sample.starts_with("complete|");
                        match quiet {
                            Some((prev, since)) if *prev == sample && !loading && complete => {
                                if since.elapsed() >= IDLE_QUIET {
                                    done(token, Ok(ok(waited)));
                                    return None;
                                }
                            }
                            _ => *quiet = Some((sample, Instant::now())),
                        }
                    }
                    (WaitFor::Selector(_) | WaitFor::Js(_) | WaitFor::Text(_), Ok(v)) => {
                        if v.json == "true" {
                            done(token, Ok(ok(waited)));
                            return None;
                        }
                    }
                    _ => {}
                }
                Some(p)
            }
            Step::Nav { .. } | Step::Shot { .. } | Step::Find | Step::Memory | Step::EnginePreferences => Some(p),
        }
    }

    fn on_shot(&mut self, p: Pend, res: Result<(u32, u32), String>) -> Option<Pend> {
        if !matches!(p.step, Step::Shot { .. }) {
            return Some(p);
        }
        let Step::Shot { path, dpr, scroll, scale, pdf } = &p.step else { return None };
        let title = self.pool.as_ref().and_then(|pl| pl.state(p.view)).map(|s| s.title.clone()).unwrap_or_default();
        let r = match res {
            Ok(_) => match std::fs::read(path) {
                // a PDF prints the capture as pages (bifrost_app::web_print); the capture is not resampled first
                Ok(bytes) if pdf.is_some() => {
                    let layout = pdf.unwrap_or_else(bifrost_app::web_print::PrintLayout::letter);
                    match resample::png_to_pdf(&bytes, &title, &layout) {
                        Ok((doc, pages)) => control::pdf_reply(&doc, pages, control::png_size(&bytes).unwrap_or((0, 0)), *dpr),
                        Err(e) => Err(internal(e)),
                    }
                }
                // `scale` resamples the capture here: the engine always writes device pixels
                Ok(bytes) => match resample::scale_png(&bytes, *scale) {
                    Ok(scaled) => control::screenshot_reply(&scaled, *dpr, *scroll),
                    Err(e) => Err(internal(e)),
                },
                Err(e) => Err(internal(format!("cannot read the screenshot: {e}"))),
            },
            Err(e) => Err(internal(format!("screenshot failed: {e}"))),
        };
        let _ = std::fs::remove_file(path);
        done(p.token, r);
        None
    }

    /// Deadlines and polling conditions of pending calls.
    fn ctl_polls(&mut self) {
        if self.ctl.pend.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut keep = Vec::new();
        for mut p in std::mem::take(&mut self.ctl.pend) {
            let state = self.pool.as_ref().and_then(|pl| pl.state(p.view)).cloned();
            let waited = ms(p.started.elapsed());
            if now >= p.deadline {
                match &p.step {
                    Step::Nav { .. } => done(p.token, Ok(self.nav_reply(p.view, "timeout"))),
                    Step::Wait { .. } => done(p.token, Ok(control::obj(vec![("ok", Json::Bool(false)), ("waited_ms", waited), ("timeout", Json::Bool(true))]))),
                    Step::Find => done(p.token, Ok(control::obj(vec![("count", control::num(0.0))]))),
                    Step::Console { level, limit, since_seq, clear } => done(p.token, Ok(self.inspector.console_json(p.view, level.as_deref(), *limit, *since_seq, *clear))),
                    _ => {
                        if let Step::Shot { path, .. } = &p.step {
                            let _ = std::fs::remove_file(path);
                        }
                        if matches!(p.step, Step::Shot { .. } | Step::ShotInfo { .. }) {
                            // a full-page capture may have left the view enlarged: always restore the size
                            if let Some(pool) = self.pool.as_mut() {
                                pool.dev_screenshot_abort(p.view);
                            }
                        }
                        done(p.token, Err(RpcError::new(TIMEOUT, "timeout")));
                    }
                }
                continue;
            }
            if !p.step.engine_wide() && state.is_none() && !self.tabs.iter().any(|t| t.view == p.view) {
                done(p.token, Err(control::no_tab(p.view)));
                continue;
            }
            match &mut p.step {
                Step::Nav { saw, wait } => {
                    // a load that never reported its start: done once the view is quiet
                    if !*saw && p.started.elapsed() >= NAV_GRACE && !state.as_ref().is_some_and(|s| s.loading) {
                        let status = if *wait == Wait::Commit { "committed" } else { "loaded" };
                        done(p.token, Ok(self.nav_reply(p.view, status)));
                        continue;
                    }
                }
                Step::Find => {
                    if let Some(c) = state.as_ref().and_then(|s| s.find_matches) {
                        done(p.token, Ok(control::obj(vec![("count", control::num(c as f64))])));
                        continue;
                    }
                }
                Step::Wait { what, base_seq, inflight, next_poll, .. } => match what.clone() {
                    WaitFor::Load => {
                        if !state.as_ref().is_some_and(|s| s.loading) {
                            done(p.token, Ok(control::obj(vec![("ok", Json::Bool(true)), ("waited_ms", waited)])));
                            continue;
                        }
                    }
                    WaitFor::Console(needle) => {
                        let hit = self.inspector.console_since(p.view, *base_seq).iter().any(|r| needle.as_deref().is_none_or(|n| r.text.contains(n)));
                        if hit {
                            done(p.token, Ok(control::obj(vec![("ok", Json::Bool(true)), ("waited_ms", waited)])));
                            continue;
                        }
                    }
                    other => {
                        if !*inflight && now >= *next_poll {
                            if let (Some(js), Some(pool)) = (control::js_wait_poll(&other), self.pool.as_mut()) {
                                let tok = rpc::new_token();
                                pool.dev_eval(p.view, tok, &js);
                                p.eval_tok = tok;
                                *inflight = true;
                            }
                        }
                    }
                },
                _ => {}
            }
            keep.push(p);
        }
        // calls registered while polling (none today) stay in front
        keep.append(&mut self.ctl.pend);
        self.ctl.pend = keep;
    }
}

/// Entries matching `query` (words in title or URL), at most `limit`, as `{url, title, visits, last}`.
fn entries_json(entries: &[crate::store::Entry], query: Option<&str>, limit: usize) -> Json {
    Json::Arr(
        entries
            .iter()
            .filter(|e| crate::library::matches(e, query.unwrap_or("")))
            .take(limit)
            .map(|e| control::obj(vec![("url", control::text(&e.url)), ("title", control::text(&e.title)), ("visits", control::num(e.visits as f64)), ("last", control::num(e.last as f64))]))
            .collect(),
    )
}

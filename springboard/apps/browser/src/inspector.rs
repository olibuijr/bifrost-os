//! Web Inspector: a docked bottom panel with Console, Network and Elements tabs (Browser's Web Inspector).
//!
//! Console keeps up to `MAX_ENTRIES` messages per tab, each with a sequence number and a timestamp, filters them by
//! level and text, copies one line (tap) or all visible lines (Copy), and evaluates expressions in the page with
//! history (Up/Down) and multi-line input (a trailing backslash or an open bracket continues the statement).
//! Network lists the page's requests (network.rs). Elements shows a read-only DOM tree (elements.rs); the filter
//! field takes a CSS selector. The panel's top edge resizes it; its height and tab are remembered.
//! Shortcuts: Cmd/Ctrl+Option+I toggles the panel, +J shows the Console, +N the Network, +C the Elements.
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::ui::system::SYSTEM_ORANGE;
use bifrost_app::kit::ui::{Weight, CARD_BG, DESTRUCTIVE, LABEL, LABEL_2, MAC_FOOTNOTE_PT, SEGMENT_ON, SEPARATOR_C, SPACE_XS, TINT};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META};
use bifrost_app::webview::{WebProcessPool, WebViewEvent, WebViewId};
use bifrost_app::webview_dev::DevEvent;
use bifrost_app::{Ctx, KeyInput, TouchKind};
use bifrost_core::json::{self, Json};

use crate::control::{self, Cell, Panel, Rec, Row};
use crate::pagetools::{self, Cap, NodeInfo};
use crate::{devtools, elements, network};

/// Segmented tabs and chips: the small-control radius (the prefs tile token).
const SEG_RADIUS_PT: f32 = m::ICON_RADIUS;

/// Entries kept per tab; the oldest are dropped first.
pub const MAX_ENTRIES: usize = 5000;
/// Most console input lines kept for Up and Down recall.
pub const MAX_INPUT_HISTORY: usize = 500;
/// evdev keys I, J, C and N (Cmd+Option+key).
const KEY_I: u32 = 23;
const KEY_J: u32 = 36;
const KEY_C: u32 = 46;
const KEY_N: u32 = 49;
const PANEL_PT: f32 = 220.0;
const PANEL_MIN_PT: f32 = 120.0;
const HEADER_PT: f32 = 28.0;
const PROMPT_PT: f32 = 28.0;
const ROW_PT: f32 = 17.0;
const TAB_W_PT: f32 = 76.0;
const CHIP_W_PT: f32 = 68.0;
const BTN_W_PT: f32 = 52.0;
const SEARCH_W_PT: f32 = 170.0;
/// Grab zone of the resize edge, points on each side of the panel's top.
const GRAB_PT: f32 = 4.0;
/// A touch that moved less than this is a tap, points.
const TAP_SLOP_PT: f32 = 5.0;
/// Network polling interval while its panel is open.
const NET_POLL: Duration = Duration::from_millis(500);
/// Console capture polling interval while the panel is open.
const CON_POLL: Duration = Duration::from_millis(500);
/// Captured entries waiting for the engine's matching console line, per tab.
const MAX_PENDING_CAPS: usize = 200;
/// Attribute rows shown for the selected node.
const MAX_ATTR_ROWS: usize = 12;
const PANELS: [Panel; 5] = [Panel::Console, Panel::Network, Panel::Elements, Panel::Sources, Panel::Storage];
/// Two taps on one row within this are a double tap (edit).
const DOUBLE_TAP: Duration = Duration::from_millis(450);
const CONSOLE_FILTERS: [&str; 4] = ["All", "Errors", "Warnings", "Logs"];
const PREF_HEIGHT: &str = "browser.inspector.height";
const PREF_PANEL: &str = "browser.inspector.panel";

/// The node selected in the Elements panel (highlighted in the page).
struct Sel {
    view: WebViewId,
    path: String,
    info: Option<NodeInfo>,
    error: Option<String>,
    /// computed style of the node (property, value)
    styles: Vec<(String, String)>,
}

/// Kinds of requests answered through `ext_req`.
#[derive(Clone, Copy, PartialEq)]
enum Ext {
    Source,
    Storage,
    Edit,
}

/// What the edit bar is changing.
#[derive(Clone)]
enum EditTarget {
    Attr { view: WebViewId, path: String, name: String },
    Text { view: WebViewId, path: String },
    Entry { view: WebViewId, store: devtools::Store, key: String },
}

#[derive(Default)]
struct TabLog {
    recs: VecDeque<Rec>,
    last_seq: u64,
    /// text bytes held in `recs` (see `MAX_BYTES`)
    bytes: usize,
}

/// Console text kept per tab, bytes; the oldest lines go first (the entry cap alone allowed 5000 x 4 KiB).
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

fn rec_bytes(r: &Rec) -> usize {
    r.text.len() + r.source.len() + r.level.len() + 48
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    None,
    Search,
    Prompt,
    Edit,
}

#[derive(Clone, Copy)]
enum Drag {
    Resize,
    List { start_y: f32, last_y: f32, moved: f32 },
}

pub struct Inspector {
    open: bool,
    panel: Panel,
    logs: HashMap<WebViewId, TabLog>,
    /// the network entries of every tab (also read by `browser.network`)
    pub net: network::NetLog,
    trees: HashMap<WebViewId, elements::Tree>,
    tree_error: Option<String>,
    toggled: HashSet<String>,
    filter: usize,
    net_filter: usize,
    search: TextField,
    prompt: TextField,
    focus: Focus,
    history: Vec<String>,
    hist_at: usize,
    /// earlier lines of a statement that is not finished yet
    multi: Vec<String>,
    relayout: bool,
    to_log: Vec<String>,
    height_pt: f32,
    /// rows scrolled away from the tail (Console, Network) or from the top (Elements)
    scroll: f32,
    drag: Option<Drag>,
    prefs_dirty: bool,
    net_req: Option<(u64, WebViewId)>,
    net_polled: Option<Instant>,
    tree_req: Option<(u64, WebViewId)>,
    con_req: Option<(u64, WebViewId)>,
    con_polled: Option<Instant>,
    /// page-captured console entries whose engine line has not arrived yet
    caps: HashMap<WebViewId, VecDeque<Cap>>,
    sel: Option<Sel>,
    /// the reply to the highlight script of the selection
    sel_req: Option<u64>,
    /// the reply to an Inspect Element point lookup
    pt_req: Option<(u64, WebViewId)>,
    /// a node found by point lookup, selected on the next `poll` (which has the pool)
    want_select: Option<(WebViewId, String)>,
    /// scroll the tree to the selected row when it next arrives
    reveal: bool,
    sources: HashMap<WebViewId, devtools::Source>,
    storage: HashMap<WebViewId, Vec<devtools::Entry>>,
    ext_req: Vec<(u64, WebViewId, Ext)>,
    styles_req: Option<u64>,
    edit: Option<EditTarget>,
    last_tap: Option<(Instant, String)>,
    store_sel: Option<String>,
    ext_error: Option<String>,
    /// a tab whose page an edit changed; re-read on the next `poll`
    refresh: Option<WebViewId>,
}

fn level_matches(filter: usize, level: &str) -> bool {
    match filter {
        1 => level == "error",
        2 => level == "warn",
        3 => !matches!(level, "error" | "warn"),
        _ => true,
    }
}

/// Appends a console line to the recall history, dropping the oldest past [`MAX_INPUT_HISTORY`].
fn remember_input(history: &mut Vec<String>, src: String) {
    history.push(src);
    if history.len() > MAX_INPUT_HISTORY {
        let excess = history.len() - MAX_INPUT_HISTORY;
        history.drain(..excess);
    }
}

/// True when `src` is not a finished statement: a trailing backslash, an open bracket or an open template string.
pub fn needs_more(src: &str) -> bool {
    let t = src.trim_end();
    if t.ends_with('\\') {
        return true;
    }
    let (mut depth, mut quote, mut esc) = (0i32, None::<char>, false);
    for c in t.chars() {
        if let Some(q) = quote {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth > 0 || quote == Some('`')
}

/// The statement as one line for the engine's `eval` verb, which carries no newlines.
pub fn one_line(src: &str) -> String {
    if src.contains('\n') { format!("(0,eval)({})", control::js_str(src)) } else { src.to_string() }
}

/// `HH:MM:SS.mmm` (UTC) of a Unix time in milliseconds.
pub fn clock(ts_ms: u64) -> String {
    let s = ts_ms / 1000;
    format!("{:02}:{:02}:{:02}.{:03}", s / 3600 % 24, s / 60 % 60, s % 60, ts_ms % 1000)
}

fn window<T>(all: Vec<T>, n: usize, scroll: usize, from_tail: bool) -> Vec<T> {
    let len = all.len();
    let start = if from_tail { len.saturating_sub(n + scroll.min(len.saturating_sub(n))) } else { scroll.min(len.saturating_sub(n)) };
    all.into_iter().skip(start).take(n).collect()
}

impl Inspector {
    pub fn new() -> Self {
        Inspector {
            open: false,
            panel: Panel::Console,
            logs: HashMap::new(),
            net: network::NetLog::default(),
            trees: HashMap::new(),
            tree_error: None,
            toggled: HashSet::new(),
            filter: 0,
            net_filter: 0,
            search: TextField::new(),
            prompt: TextField::new(),
            focus: Focus::None,
            history: Vec::new(),
            hist_at: 0,
            multi: Vec::new(),
            relayout: false,
            to_log: Vec::new(),
            height_pt: PANEL_PT,
            scroll: 0.0,
            drag: None,
            prefs_dirty: false,
            net_req: None,
            net_polled: None,
            tree_req: None,
            con_req: None,
            con_polled: None,
            caps: HashMap::new(),
            sel: None,
            sel_req: None,
            pt_req: None,
            want_select: None,
            reveal: false,
            sources: HashMap::new(),
            storage: HashMap::new(),
            ext_req: Vec::new(),
            styles_req: None,
            edit: None,
            last_tap: None,
            store_sel: None,
            ext_error: None,
            refresh: None,
        }
    }

    /// Restores the remembered height and tab.
    pub fn load(&mut self, ctx: &Ctx) {
        let h = ctx.defaults().double(PREF_HEIGHT) as f32;
        if h >= PANEL_MIN_PT {
            self.height_pt = h;
        }
        if let Some(p) = ctx.defaults().string(PREF_PANEL).and_then(|s| Panel::parse(&s)) {
            self.panel = p;
        }
    }

    /// Writes the height and tab when they changed (call from `tick`).
    pub fn flush_prefs(&mut self, ctx: &Ctx) {
        if std::mem::take(&mut self.prefs_dirty) {
            let mut d = ctx.defaults_mut();
            d.set_double(PREF_HEIGHT, self.height_pt as f64);
            d.set_string(PREF_PANEL, self.panel.name());
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn panel_kind(&self) -> Panel {
        self.panel
    }

    /// Height taken from the bottom of the content, window pixels.
    pub fn reserved_px(&self, ui: &Ui) -> u32 {
        if self.open { ui.pt(self.height_pt) as u32 } else { 0 }
    }

    /// True once after the panel opened, closed or was resized: the app re-places the web content.
    pub fn take_relayout(&mut self) -> bool {
        std::mem::take(&mut self.relayout)
    }

    /// Warnings and errors to forward to the unified log (`bifrost pc log`).
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.to_log)
    }

    /// Opens or closes the panel and/or switches its tab (`browser.devtools`).
    pub fn set(&mut self, open: Option<bool>, panel: Option<Panel>, mut pool: Option<&mut WebProcessPool>) {
        let was = (self.open, self.panel);
        if let Some(p) = panel {
            self.panel = p;
            if open.is_none() {
                self.open = true;
            }
        }
        if let Some(o) = open {
            self.open = o;
        }
        if was.1 != self.panel {
            self.scroll = 0.0;
            self.prefs_dirty = true;
        }
        if was.0 != self.open {
            self.relayout = true;
        }
        self.edit = None;
        self.ext_error = None;
        self.focus = if self.open && self.panel == Panel::Console { Focus::Prompt } else { Focus::None };
        if !self.open || self.panel != Panel::Elements {
            // the page highlight belongs to the Elements panel
            if let Some(sel) = self.sel.take() {
                if let Some(p) = pool.as_deref_mut() {
                    p.dev_eval(sel.view, bifrost_app::control::new_token(), &pagetools::js_clear_highlight());
                }
            }
        }
        if let Some(p) = pool {
            p.set_devtools(true);
        }
    }

    pub fn toggle(&mut self, pool: Option<&mut WebProcessPool>) {
        self.set(Some(!self.open), None, pool);
    }

    pub fn forget(&mut self, view: WebViewId) {
        self.logs.remove(&view);
        self.net.forget(view);
        self.trees.remove(&view);
        self.sources.remove(&view);
        self.storage.remove(&view);
        self.caps.remove(&view);
        if self.sel.as_ref().is_some_and(|s| s.view == view) {
            self.sel = None;
        }
    }

    pub fn blur(&mut self) {
        self.focus = Focus::None;
    }

    /// Adds a console line to tab `view` with the next sequence number.
    pub fn record(&mut self, view: WebViewId, level: &str, text: String, source: String, line: u32) {
        let t = self.logs.entry(view).or_default();
        t.last_seq += 1;
        let rec = Rec { seq: t.last_seq, ts_ms: control::now_ms(), level: level.to_string(), text, source, line };
        t.bytes += rec_bytes(&rec);
        t.recs.push_back(rec);
        while t.recs.len() > MAX_ENTRIES || (t.bytes > MAX_BYTES && t.recs.len() > 1) {
            if let Some(old) = t.recs.pop_front() {
                t.bytes = t.bytes.saturating_sub(rec_bytes(&old));
            }
        }
    }

    /// Highest console sequence number of the tab (0 when it has none).
    pub fn last_seq(&self, view: WebViewId) -> u64 {
        self.logs.get(&view).map_or(0, |t| t.last_seq)
    }

    /// Console entries of a tab at or after `seq`, for `browser.wait for=console`.
    pub fn console_since(&self, view: WebViewId, seq: u64) -> Vec<&Rec> {
        self.logs.get(&view).map(|t| t.recs.iter().filter(|r| r.seq > seq).collect()).unwrap_or_default()
    }

    /// `browser.console`.
    pub fn console_json(&mut self, view: WebViewId, level: Option<&str>, limit: usize, since_seq: Option<u64>, clear: bool) -> Json {
        let out = match self.logs.get(&view) {
            Some(t) => control::console_query(&t.recs, t.last_seq, level, limit, since_seq),
            None => control::console_query(&VecDeque::new(), 0, level, limit, since_seq),
        };
        if clear {
            if let Some(t) = self.logs.get_mut(&view) {
                t.recs.clear();
                t.bytes = 0;
            }
        }
        out
    }

    /// Takes the engine events that belong to the Inspector. Returns true when the panel needs a redraw.
    pub fn on_event(&mut self, ev: &WebViewEvent) -> bool {
        match ev {
            WebViewEvent::Console { id, level, text, source, line } => {
                if matches!(level.as_str(), "warn" | "error") {
                    self.to_log.push(format!("console.{level}: {text}"));
                }
                // the page hook is the source of truth: when it already captured this message, its entry (text,
                // file and line) is recorded and the engine's copy is dropped; engine-only messages stay
                let (mut text, mut source, mut line) = (text.clone(), source.clone(), *line);
                if let Some(q) = self.caps.get_mut(id) {
                    if let Some(i) = q.iter().position(|c| !c.own && c.line > 0 && c.level == *level && pagetools::same_text(&text, &c.text)) {
                        if let Some(c) = q.remove(i) {
                            (text, source, line) = (c.text, c.source, c.line);
                        }
                    }
                }
                self.record(*id, level, text, source, line);
                self.open
            }
            // scroll probe of a tab about to be discarded (tabdiscard.rs): not console output
            WebViewEvent::EvalResult(_, text) if text.trim().trim_matches('"').starts_with(crate::tabdiscard::SCROLL_PREFIX) => false,
            WebViewEvent::EvalResult(_, text) if text.trim().trim_matches('"').starts_with(crate::downloads::COOKIE_PREFIX) => false,
            WebViewEvent::EvalResult(_, text) if text.trim().trim_matches('"').starts_with(crate::favicon::FAV_PREFIX) => false,
            WebViewEvent::EvalResult(id, text) => {
                self.record(*id, "result", text.clone(), String::new(), 0);
                self.open
            }
            WebViewEvent::LoadStarted(id) => {
                // a new document: the old tree no longer describes the page, and the overlay went with it
                self.trees.remove(id);
                self.sources.remove(id);
                self.storage.remove(id);
                self.caps.remove(id);
                if self.sel.as_ref().is_some_and(|s| s.view == *id) {
                    self.sel = None;
                }
                false
            }
            _ => false,
        }
    }

    /// Takes the token-correlated answers that belong to the panel (network drain, element tree).
    pub fn on_dev(&mut self, ev: &DevEvent) -> bool {
        let DevEvent::Eval { token, result } = ev else { return false };
        if let Some((t, view)) = self.con_req {
            if t == *token {
                self.con_req = None;
                if let Ok(v) = result {
                    if let Some(raw) = control::inner_string(&v.ty, &v.json) {
                        self.ingest_caps(view, &raw);
                    }
                }
                return true;
            }
        }
        if self.sel_req == Some(*token) {
            self.sel_req = None;
            if let Some(sel) = self.sel.as_mut() {
                match result.as_ref().map_err(|e| e.clone()).and_then(|v| control::inner_string(&v.ty, &v.json).ok_or_else(|| "unexpected reply".to_string())).and_then(|raw| pagetools::parse_info(&raw)) {
                    Ok(info) => (sel.info, sel.error) = (info, None),
                    Err(e) => (sel.info, sel.error) = (None, Some(e)),
                }
            }
            return true;
        }
        if self.styles_req == Some(*token) {
            self.styles_req = None;
            if let Some(sel) = self.sel.as_mut() {
                sel.styles = result.as_ref().ok().and_then(|v| control::inner_string(&v.ty, &v.json)).and_then(|raw| devtools::parse_styles(&raw).ok()).unwrap_or_default();
            }
            return true;
        }
        if let Some(i) = self.ext_req.iter().position(|r| r.0 == *token) {
            let (_, view, kind) = self.ext_req.remove(i);
            let raw = result.as_ref().ok().and_then(|v| control::inner_string(&v.ty, &v.json));
            let why = result.as_ref().err().cloned().unwrap_or_else(|| "unexpected reply".to_string());
            match kind {
                Ext::Source => match raw {
                    Some(r) => {
                        self.ext_error = None;
                        self.sources.insert(view, devtools::tokenize(&r));
                    }
                    None => {
                        self.ext_error = Some(why);
                        self.sources.insert(view, devtools::Source::default());
                    }
                },
                Ext::Storage => match raw.map(|r| devtools::parse_storage(&r)) {
                    Some(Ok(e)) => {
                        self.ext_error = None;
                        self.storage.insert(view, e);
                    }
                    _ => {
                        self.ext_error = Some(why);
                        self.storage.insert(view, Vec::new());
                    }
                },
                Ext::Edit => {
                    self.ext_error = match raw.as_deref() {
                        Some("ok") => None,
                        Some(e) => Some(e.to_string()),
                        None => Some(why),
                    };
                    self.refresh = Some(view);
                }
            }
            return true;
        }
        if let Some((t, view)) = self.pt_req {
            if t == *token {
                self.pt_req = None;
                let path = result.as_ref().ok().and_then(|v| control::inner_string(&v.ty, &v.json)).and_then(|raw| json::parse(&raw).ok()).and_then(|j| j.get("path").and_then(Json::as_str).map(str::to_string));
                if let Some(path) = path {
                    self.set(Some(true), Some(Panel::Elements), None);
                    self.want_select = Some((view, path));
                }
                return true;
            }
        }
        if let Some((t, view)) = self.net_req {
            if t == *token {
                self.net_req = None;
                if let Ok(v) = result {
                    if let Some(raw) = control::inner_string(&v.ty, &v.json) {
                        self.net.ingest(view, &raw, control::now_ms());
                    }
                }
                return true;
            }
        }
        if let Some((t, view)) = self.tree_req {
            if t == *token {
                self.tree_req = None;
                match result {
                    Ok(v) => match control::inner_string(&v.ty, &v.json).map(|raw| elements::parse(&raw)) {
                        Some(Ok(tree)) => {
                            self.tree_error = None;
                            self.trees.insert(view, tree);
                            self.reveal_selected(view);
                        }
                        Some(Err(e)) => self.tree_error = Some(e),
                        None => self.tree_error = Some("unexpected reply".into()),
                    },
                    Err(e) => self.tree_error = Some(e.clone()),
                }
                return true;
            }
        }
        false
    }

    /// Drains the page's captured console entries (source and line) and merges them into the log.
    pub fn request_console(&mut self, pool: &mut WebProcessPool, view: WebViewId) {
        if self.con_req.is_some() {
            return;
        }
        let token = bifrost_app::control::new_token();
        self.con_req = Some((token, view));
        self.con_polled = Some(Instant::now());
        pool.dev_eval(view, token, &pagetools::drain_console_js());
    }

    /// Merges the page's captured entries (`pagetools::drain_console_js` reply) into tab `view`'s log: the engine's
    /// line with the same level and text gets its file and line, an entry the engine never reports is added, and one
    /// whose engine line has not arrived yet waits for it.
    pub fn ingest_caps(&mut self, view: WebViewId, raw: &str) {
        for cap in pagetools::parse_caps(raw) {
            if cap.own {
                if cap.level == "error" {
                    self.to_log.push(format!("console.error: {}", cap.text));
                }
                self.record(view, &cap.level, cap.text, cap.source, cap.line);
                continue;
            }
            if cap.line == 0 {
                continue;
            }
            let hit = self.logs.get_mut(&view).and_then(|t| t.recs.iter_mut().rev().find(|r| r.line == 0 && r.source.is_empty() && r.level == cap.level && pagetools::same_text(&r.text, &cap.text)));
            match hit {
                // the engine's copy arrived first: the hook's entry replaces it in place
                Some(r) => {
                    r.text = cap.text;
                    r.source = cap.source;
                    r.line = cap.line;
                }
                None => {
                    let q = self.caps.entry(view).or_default();
                    if q.len() >= MAX_PENDING_CAPS {
                        q.pop_front();
                    }
                    q.push_back(cap);
                }
            }
        }
    }

    /// Selects the node at `path` of tab `view` (None deselects): draws the highlight in the page, unfolds the tree
    /// down to it and asks for its box and attributes.
    pub fn select(&mut self, pool: &mut WebProcessPool, view: WebViewId, path: Option<String>) {
        let token = bifrost_app::control::new_token();
        match path {
            Some(p) => {
                for k in pagetools::ancestor_keys(&p) {
                    elements::unfold(&mut self.toggled, &k);
                }
                pool.dev_eval(view, token, &pagetools::js_highlight(&p));
                let st = bifrost_app::control::new_token();
                pool.dev_eval(view, st, &devtools::js_styles_of(&p));
                self.styles_req = Some(st);
                self.sel = Some(Sel { view, path: p, info: None, error: None, styles: Vec::new() });
                self.sel_req = Some(token);
            }
            None => {
                self.sel = None;
                self.sel_req = None;
                self.styles_req = None;
                pool.dev_eval(view, token, &pagetools::js_clear_highlight());
            }
        }
    }

    /// Path of the selected node of `view`.
    pub fn selected(&self, view: WebViewId) -> Option<&str> {
        self.sel.as_ref().filter(|s| s.view == view).map(|s| s.path.as_str())
    }

    /// Inspect Element: finds the element at page point `x, y` (CSS px) and shows it in the Elements panel.
    pub fn inspect_point(&mut self, pool: &mut WebProcessPool, view: WebViewId, x: f32, y: f32) {
        let token = bifrost_app::control::new_token();
        self.pt_req = Some((token, view));
        pool.dev_eval(view, token, &pagetools::js_inspect_point(x, y));
    }

    /// Scrolls the Elements list so the selected row is in view (after a point lookup).
    fn reveal_selected(&mut self, view: WebViewId) {
        if !std::mem::take(&mut self.reveal) {
            return;
        }
        let (Some(sel), Some(tree)) = (self.sel.as_ref().filter(|s| s.view == view), self.trees.get(&view)) else { return };
        let rows = elements::rows(tree, &self.toggled, None);
        if let Some(i) = rows.iter().position(|r| r.select.as_deref() == Some(sel.path.as_str())) {
            let info = sel.info.as_ref().map_or(3, |n| 1 + n.attrs.len().min(MAX_ATTR_ROWS)) + if sel.styles.is_empty() { 0 } else { sel.styles.len() + 1 };
            self.scroll = (i + info).saturating_sub(2) as f32;
        }
    }

    /// The box and attribute rows of the selected node, shown above the tree.
    fn info_rows(&self, view: WebViewId, width: f32) -> Vec<Row> {
        let Some(sel) = self.sel.as_ref().filter(|s| s.view == view) else { return Vec::new() };
        let cell = |text: String, color: u32| Cell { x: 10.0, w: width - 20.0, text, color };
        if let Some(e) = &sel.error {
            return vec![Row { cells: vec![cell(e.clone(), DESTRUCTIVE)], copy: e.clone(), toggle: None, select: None }];
        }
        let Some(i) = &sel.info else { return Vec::new() };
        let head = format!("{}  {}", pagetools::selector_of(i), pagetools::box_text(i));
        let mut rows = vec![Row { cells: vec![cell(head, TINT)], copy: pagetools::selector_of(i), toggle: None, select: None }];
        for (k, v) in i.attrs.iter().take(MAX_ATTR_ROWS) {
            rows.push(Row { cells: vec![cell(format!("   {k} = \"{v}\""), LABEL_2.now())], copy: v.clone(), toggle: Some(devtools::attr_id(k)), select: None });
        }
        if i.attrs.len() > MAX_ATTR_ROWS {
            rows.push(Row { cells: vec![cell(format!("   ... {} more attributes", i.attrs.len() - MAX_ATTR_ROWS), LABEL_2.now())], copy: String::new(), toggle: None, select: None });
        }
        rows.extend(devtools::style_rows(&sel.styles, width));
        rows
    }

    /// Asks the page for its document source (Sources tab).
    pub fn request_source(&mut self, pool: &mut WebProcessPool, view: WebViewId) {
        self.request_ext(pool, view, Ext::Source, control::JS_SOURCE);
    }

    /// Asks the page for its storage (Storage tab).
    pub fn request_storage(&mut self, pool: &mut WebProcessPool, view: WebViewId) {
        self.request_ext(pool, view, Ext::Storage, devtools::JS_STORAGE);
    }

    fn request_ext(&mut self, pool: &mut WebProcessPool, view: WebViewId, kind: Ext, js: &str) {
        if self.ext_req.iter().any(|r| r.1 == view && r.2 == kind) {
            return;
        }
        let token = bifrost_app::control::new_token();
        self.ext_req.push((token, view, kind));
        pool.dev_eval(view, token, js);
    }

    /// Opens the edit bar on `target` with `value` in it.
    fn begin_edit(&mut self, target: EditTarget, value: &str) {
        self.edit = Some(target);
        self.prompt.clear();
        self.prompt.push_str(value);
        self.focus = Focus::Edit;
    }

    /// Writes the edit bar's text to the page.
    fn commit_edit(&mut self, pool: Option<&mut WebProcessPool>) {
        let Some(target) = self.edit.take() else { return };
        let val = self.prompt.to_string();
        self.prompt.clear();
        self.focus = Focus::None;
        let Some(pool) = pool else { return };
        let (view, js) = match &target {
            EditTarget::Attr { view, path, name } => (*view, devtools::js_set_attr(path, name, &val)),
            EditTarget::Text { view, path } => (*view, devtools::js_set_text(path, &val)),
            EditTarget::Entry { view, store, key } => (*view, devtools::js_storage_edit(*store, key, Some(&val))),
        };
        let token = bifrost_app::control::new_token();
        self.ext_req.push((token, view, Ext::Edit));
        pool.dev_eval(view, token, &js);
    }

    /// Removes the selected Storage entry.
    fn delete_entry(&mut self, pool: &mut WebProcessPool, view: WebViewId) {
        let Some((store, key)) = self.store_sel.take().and_then(|id| devtools::parse_entry_id(&id)) else { return };
        let token = bifrost_app::control::new_token();
        self.ext_req.push((token, view, Ext::Edit));
        pool.dev_eval(view, token, &devtools::js_storage_edit(store, &key, None));
    }

    /// Asks the page for its element tree (filtered by the selector in the search field).
    pub fn request_tree(&mut self, pool: &mut WebProcessPool, view: WebViewId) {
        if self.tree_req.is_some() {
            return;
        }
        let token = bifrost_app::control::new_token();
        self.tree_req = Some((token, view));
        let filter = self.search.to_string();
        pool.dev_eval(view, token, &control::js_tree(filter.trim(), elements::MAX_NODES));
    }

    /// Drains the page's network entries into the log.
    pub fn request_net(&mut self, pool: &mut WebProcessPool, view: WebViewId) {
        if self.net_req.is_some() {
            return;
        }
        let token = bifrost_app::control::new_token();
        self.net_req = Some((token, view));
        self.net_polled = Some(Instant::now());
        pool.dev_eval(view, token, &network::drain_js());
    }

    /// Periodic work while the panel is open: network polling, the first tree fetch. True when a redraw is due.
    pub fn poll(&mut self, pool: &mut WebProcessPool, view: Option<WebViewId>) -> bool {
        let Some(view) = view.filter(|_| self.open) else { return false };
        if let Some((v, path)) = self.want_select.take() {
            // Inspect Element: re-read the tree so the node is in it, unfold down to it and highlight it
            self.trees.remove(&v);
            self.tree_error = None;
            self.reveal = true;
            self.select(pool, v, Some(path));
            self.request_tree(pool, v);
        }
        if self.con_req.is_none() && self.con_polled.is_none_or(|t| t.elapsed() >= CON_POLL) {
            self.request_console(pool, view);
        }
        if let Some(v) = self.refresh.take() {
            if v == view {
                match self.panel {
                    Panel::Elements => {
                        self.tree_error = None;
                        self.trees.remove(&v);
                        self.request_tree(pool, v);
                        if let Some(path) = self.sel.as_ref().filter(|s| s.view == v).map(|s| s.path.clone()) {
                            self.select(pool, v, Some(path));
                        }
                    }
                    Panel::Storage => {
                        self.storage.remove(&v);
                    }
                    _ => {}
                }
            }
        }
        match self.panel {
            Panel::Sources if !self.sources.contains_key(&view) => self.request_source(pool, view),
            Panel::Storage if !self.storage.contains_key(&view) => self.request_storage(pool, view),
            Panel::Network if self.net_polled.is_none_or(|t| t.elapsed() >= NET_POLL) => self.request_net(pool, view),
            Panel::Elements if !self.trees.contains_key(&view) && self.tree_req.is_none() && self.tree_error.is_none() => self.request_tree(pool, view),
            _ => {}
        }
        false
    }

    // ---- rows ----

    fn console_line(e: &Rec) -> String {
        let body = if e.source.is_empty() { e.text.clone() } else { format!("{}  ({}:{})", e.text, e.source, e.line) };
        if e.level == "input" { format!("> {body}") } else { body }
    }

    fn console_copy_line(e: &Rec) -> String {
        format!("[{}] {} {} {}", e.seq, clock(e.ts_ms), e.level, Self::console_line(e))
    }

    fn visible_console(&self, view: WebViewId) -> Vec<&Rec> {
        let needle = self.search.to_string().to_lowercase();
        self.logs
            .get(&view)
            .map(|t| t.recs.iter().filter(|e| level_matches(self.filter, &e.level) && (needle.is_empty() || e.text.to_lowercase().contains(&needle))).collect())
            .unwrap_or_default()
    }

    fn rows(&self, view: WebViewId, width: f32) -> Vec<Row> {
        match self.panel {
            Panel::Console => self
                .visible_console(view)
                .into_iter()
                .map(|e| {
                    let color = match e.level.as_str() {
                        "error" => DESTRUCTIVE,
                        "warn" => SYSTEM_ORANGE.now(),
                        "info" | "debug" | "result" => LABEL_2.now(),
                        "input" => TINT,
                        _ => LABEL.now(),
                    };
                    Row {
                        cells: vec![Cell { x: 10.0, w: width - 20.0, text: format!("{}  {}", clock(e.ts_ms), Self::console_line(e)), color }],
                        copy: Self::console_line(e),
                        toggle: None,
                        select: None,
                    }
                })
                .collect(),
            Panel::Network => self.net.entries(view).map(|q| network::rows(q, self.net_filter, &self.search.to_string(), width)).unwrap_or_default(),
            Panel::Elements => match (&self.tree_error, self.trees.get(&view)) {
                (Some(e), _) => vec![Row { cells: vec![Cell { x: 10.0, w: width - 20.0, text: e.clone(), color: DESTRUCTIVE }], copy: e.clone(), toggle: None, select: None }],
                (None, Some(t)) => {
                    let mut rows = self.info_rows(view, width);
                    rows.extend(elements::rows(t, &self.toggled, self.selected(view)));
                    rows
                }
                (None, None) => vec![Row { cells: vec![Cell { x: 10.0, w: width - 20.0, text: "Loading the page structure...".into(), color: LABEL_2.now() }], copy: String::new(), toggle: None, select: None }],
            },
            Panel::Sources => {
                let mut rows = self.error_row(width);
                match self.sources.get(&view) {
                    Some(s) => rows.extend(devtools::source_rows(s, &self.search.to_string())),
                    None => rows.push(self.note_row("Loading the page source...", width)),
                }
                rows
            }
            Panel::Storage => {
                let mut rows = self.error_row(width);
                match self.storage.get(&view) {
                    Some(e) => rows.extend(devtools::storage_rows(e, &self.search.to_string(), self.store_sel.as_deref(), width)),
                    None => rows.push(self.note_row("Loading the page storage...", width)),
                }
                rows
            }
        }
    }

    fn note_row(&self, text: &str, width: f32) -> Row {
        Row { cells: vec![Cell { x: 10.0, w: width - 20.0, text: text.into(), color: LABEL_2.now() }], copy: String::new(), toggle: None, select: None }
    }

    /// The last edit or fetch error as a row (empty when there is none).
    fn error_row(&self, width: f32) -> Vec<Row> {
        self.ext_error.iter().map(|e| Row { cells: vec![Cell { x: 10.0, w: width - 20.0, text: e.clone(), color: DESTRUCTIVE }], copy: e.clone(), toggle: None, select: None }).collect()
    }

    /// Elements, Sources and Storage lists start at the top; Console and Network pin the tail.
    fn from_top(&self) -> bool {
        !matches!(self.panel, Panel::Console | Panel::Network)
    }

    /// The bottom bar: the Console prompt, or the edit field while a value is edited.
    fn has_bar(&self) -> bool {
        self.panel == Panel::Console || self.edit.is_some()
    }

    // ---- layout (points, window space) ----

    fn height(&self, ui: &Ui, h: u32) -> f32 {
        let max = (h as f32 / ui.pt(1.0) * 0.7).max(PANEL_MIN_PT);
        self.height_pt.clamp(PANEL_MIN_PT, max)
    }
    fn panel_rect(&self, ui: &Ui, w: u32, h: u32) -> Rect {
        let ph = self.height(ui, h);
        Rect { x: 0.0, y: h as f32 / ui.pt(1.0) - ph, w: w as f32 / ui.pt(1.0), h: ph }
    }
    fn tab_rect(p: Rect, i: usize) -> Rect {
        Rect { x: p.x + 8.0 + i as f32 * TAB_W_PT, y: p.y + 4.0, w: TAB_W_PT - 4.0, h: HEADER_PT - 8.0 }
    }
    fn chips_x(p: Rect) -> f32 {
        p.x + 8.0 + PANELS.len() as f32 * TAB_W_PT + 12.0
    }
    fn chip(p: Rect, i: usize) -> Rect {
        Rect { x: Self::chips_x(p) + i as f32 * CHIP_W_PT, y: p.y + 4.0, w: CHIP_W_PT - 4.0, h: HEADER_PT - 8.0 }
    }
    fn chips_shown(&self, p: Rect) -> bool {
        matches!(self.panel, Panel::Console | Panel::Network) && p.w > Self::chips_x(p) + 4.0 * CHIP_W_PT + SEARCH_W_PT + 2.0 * BTN_W_PT + 40.0
    }
    /// The rightmost button: Clear (Console, Network) or Refresh (Elements).
    fn clear_rect(p: Rect) -> Rect {
        Rect { x: p.x + p.w - BTN_W_PT - 8.0, y: p.y + 4.0, w: BTN_W_PT, h: HEADER_PT - 8.0 }
    }
    fn copy_rect(p: Rect) -> Rect {
        Rect { x: p.x + p.w - 2.0 * BTN_W_PT - 12.0, y: p.y + 4.0, w: BTN_W_PT, h: HEADER_PT - 8.0 }
    }
    fn search_rect(&self, p: Rect) -> Rect {
        let buttons = if self.panel == Panel::Console { 2.0 } else { 1.0 };
        Rect { x: p.x + p.w - buttons * BTN_W_PT - SEARCH_W_PT - 16.0 - (buttons - 1.0) * 4.0, y: p.y + 4.0, w: SEARCH_W_PT, h: HEADER_PT - 8.0 }
    }
    fn prompt_rect(p: Rect) -> Rect {
        Rect { x: p.x + 8.0, y: p.y + p.h - PROMPT_PT + 4.0, w: p.w - 16.0, h: PROMPT_PT - 8.0 }
    }
    fn list_rect(&self, p: Rect) -> Rect {
        let below = if self.has_bar() { PROMPT_PT } else { 0.0 };
        // the Network column header takes the first row
        let head = if self.panel == Panel::Network { ROW_PT } else { 0.0 };
        Rect { x: p.x, y: p.y + HEADER_PT + head, w: p.w, h: p.h - HEADER_PT - below - head }
    }
    fn handle_rect(p: Rect) -> Rect {
        Rect { x: p.x, y: p.y - GRAB_PT, w: p.w, h: 2.0 * GRAB_PT }
    }

    fn visible_rows(&self, rows: Vec<Row>, list: Rect) -> Vec<Row> {
        let n = (list.h / ROW_PT) as usize;
        window(rows, n, self.scroll as usize, !self.from_top())
    }

    // ---- input ----

    /// A touch in window pixels. True when the panel consumed it.
    pub fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, view: Option<WebViewId>, pool: Option<&mut WebProcessPool>) -> bool {
        if !self.open {
            return false;
        }
        let ui = &ctx.ui;
        let p = self.panel_rect(ui, ctx.cfg.w, ctx.cfg.h);
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        match kind {
            TouchKind::Down => {
                if Self::handle_rect(p).contains(px, py) {
                    self.drag = Some(Drag::Resize);
                    return true;
                }
                if !p.contains(px, py) {
                    return false;
                }
                self.header_press(ctx, p, px, py, view, pool);
                if self.list_rect(p).contains(px, py) {
                    self.drag = Some(Drag::List { start_y: py, last_y: py, moved: 0.0 });
                }
                true
            }
            TouchKind::Move => match self.drag {
                Some(Drag::Resize) => {
                    let total = ctx.cfg.h as f32 / ui.pt(1.0);
                    self.height_pt = (total - py).clamp(PANEL_MIN_PT, (total * 0.7).max(PANEL_MIN_PT));
                    self.relayout = true;
                    true
                }
                Some(Drag::List { start_y, last_y, moved }) => {
                    let dy = py - last_y;
                    let rows = dy / ROW_PT;
                    self.scroll = if self.from_top() { self.scroll - rows } else { self.scroll + rows }.max(0.0);
                    self.drag = Some(Drag::List { start_y, last_y: py, moved: moved + dy.abs() });
                    true
                }
                None => p.contains(px, py),
            },
            TouchKind::Up | TouchKind::Cancel => {
                let was = self.drag.take();
                match was {
                    Some(Drag::Resize) => {
                        self.prefs_dirty = true;
                        true
                    }
                    Some(Drag::List { start_y, moved, .. }) => {
                        if kind == TouchKind::Up && moved < TAP_SLOP_PT {
                            self.tap_row(ctx, p, start_y, view, pool);
                        }
                        true
                    }
                    None => p.contains(px, py),
                }
            }
        }
    }

    fn header_press(&mut self, ctx: &Ctx, p: Rect, px: f32, py: f32, view: Option<WebViewId>, mut pool: Option<&mut WebProcessPool>) {
        if let Some(i) = (0..PANELS.len()).find(|i| Self::tab_rect(p, *i).contains(px, py)) {
            self.set(Some(true), Some(PANELS[i]), pool.as_deref_mut());
            self.search.clear();
            if let (Some(pool), Some(v)) = (pool, view) {
                match self.panel {
                    Panel::Elements => {
                        self.tree_error = None;
                        self.request_tree(pool, v);
                    }
                    Panel::Network => self.request_net(pool, v),
                    Panel::Sources => self.request_source(pool, v),
                    Panel::Storage => self.request_storage(pool, v),
                    Panel::Console => {}
                }
            }
            return;
        }
        if self.chips_shown(p) {
            if let Some(i) = (0..4).find(|i| Self::chip(p, *i).contains(px, py)) {
                if self.panel == Panel::Console {
                    self.filter = i;
                } else {
                    self.net_filter = i;
                }
                self.scroll = 0.0;
                return;
            }
        }
        if Self::clear_rect(p).contains(px, py) {
            match (self.panel, view) {
                (Panel::Console, Some(v)) => {
                    if let Some(t) = self.logs.get_mut(&v) {
                        t.recs.clear();
                        t.bytes = 0;
                    }
                }
                (Panel::Network, Some(v)) => self.net.clear(v),
                (Panel::Elements, Some(v)) => {
                    self.tree_error = None;
                    self.trees.remove(&v);
                    if let Some(pool) = pool {
                        self.request_tree(pool, v);
                    }
                }
                (Panel::Sources, Some(v)) => {
                    self.sources.remove(&v);
                    self.ext_error = None;
                }
                (Panel::Storage, Some(v)) => {
                    self.storage.remove(&v);
                    self.ext_error = None;
                }
                _ => {}
            }
        } else if self.panel == Panel::Console && Self::copy_rect(p).contains(px, py) {
            if let Some(v) = view {
                let all: Vec<String> = self.visible_console(v).into_iter().map(Self::console_copy_line).collect();
                ctx.set_pasteboard_string(&all.join("\n"));
            }
        } else if self.search_rect(p).contains(px, py) {
            self.focus = Focus::Search;
        } else if self.panel == Panel::Console && Self::prompt_rect(p).contains(px, py) {
            self.focus = Focus::Prompt;
        } else if self.edit.is_some() && Self::prompt_rect(p).contains(px, py) {
            self.focus = Focus::Edit;
        }
    }

    fn tap_row(&mut self, ctx: &Ctx, p: Rect, y: f32, view: Option<WebViewId>, pool: Option<&mut WebProcessPool>) {
        let Some(v) = view else { return };
        let list = self.list_rect(p);
        let row = ((y - list.y) / ROW_PT) as usize;
        let rows = self.rows(v, list.w);
        let shown = self.visible_rows(rows, list);
        let Some(r) = shown.get(row) else { return };
        // double tap on an attribute value, a leaf's text or a storage entry edits it
        let id = r.toggle.clone().filter(|k| k.starts_with('@')).or_else(|| r.select.clone());
        let double = id.as_ref().is_some_and(|i| self.last_tap.as_ref().is_some_and(|(t, k)| k == i && t.elapsed() < DOUBLE_TAP));
        self.last_tap = id.clone().map(|i| (Instant::now(), i));
        if self.panel == Panel::Storage {
            if let Some(id) = id {
                if double {
                    if let (Some((store, key)), Some(e)) = (devtools::parse_entry_id(&id), self.storage.get(&v).and_then(|es| es.iter().find(|e| devtools::entry_id(e) == id))) {
                        let value = e.value.clone();
                        self.begin_edit(EditTarget::Entry { view: v, store, key }, &value);
                    }
                } else {
                    ctx.set_pasteboard_string(&r.copy);
                }
                self.store_sel = Some(id);
            }
            return;
        }
        if double && self.panel == Panel::Elements {
            let id = id.unwrap_or_default();
            if let Some(name) = id.strip_prefix("@attr:") {
                let sel = self.sel.as_ref().filter(|s| s.view == v);
                if let (Some(path), Some(val)) = (sel.map(|s| s.path.clone()), sel.and_then(|s| s.info.as_ref()).and_then(|i| i.attrs.iter().find(|(k, _)| k == name)).map(|(_, v)| v.clone())) {
                    self.begin_edit(EditTarget::Attr { view: v, path, name: name.to_string() }, &val);
                    return;
                }
            } else if let Some(n) = self.trees.get(&v).and_then(|t| devtools::find_node(t, &id)).filter(|n| n.children.is_empty() && !n.text.is_empty()) {
                let text = n.text.clone();
                self.begin_edit(EditTarget::Text { view: v, path: id }, &text);
                return;
            }
        }
        if r.toggle.as_deref().is_some_and(|k| k.starts_with('@')) {
            ctx.set_pasteboard_string(&r.copy);
            return;
        }
        if let Some(key) = &r.toggle {
            if !self.toggled.remove(key) {
                self.toggled.insert(key.clone());
            }
        }
        if let (Some(path), Some(pool)) = (&r.select, pool) {
            // a tap selects the element and highlights it in the page; a second tap on it deselects
            let again = self.selected(v) == Some(path.as_str());
            self.select(pool, v, if again { None } else { Some(path.clone()) });
        } else if r.toggle.is_some() {
        } else if !r.copy.is_empty() {
            ctx.set_pasteboard_string(&r.copy);
        }
    }

    /// Runs the prompt's statement, or keeps collecting lines of an unfinished one.
    fn submit(&mut self, pool: Option<&mut WebProcessPool>, view: Option<WebViewId>) {
        let line = self.prompt.to_string();
        self.prompt.clear();
        let mut lines = std::mem::take(&mut self.multi);
        lines.push(line.clone());
        let src = lines.join("\n");
        if src.trim().is_empty() {
            return;
        }
        if needs_more(&src) {
            let mut kept = lines;
            if let Some(last) = kept.last_mut() {
                if last.trim_end().ends_with('\\') {
                    *last = last.trim_end().trim_end_matches('\\').to_string();
                }
            }
            self.multi = kept;
            return;
        }
        if let (Some(p), Some(v)) = (pool, view) {
            remember_input(&mut self.history, src.clone());
            self.hist_at = self.history.len();
            self.record(v, "input", src.clone(), String::new(), 0);
            p.eval(v, &one_line(&src));
        }
    }

    /// A key. `Some(redraw)` when the Inspector took it.
    pub fn key(&mut self, k: &KeyInput, mut pool: Option<&mut WebProcessPool>, view: Option<WebViewId>) -> Option<bool> {
        if let Some((key, mods)) = k.as_chord() {
            if mods & CHORD_ALT != 0 && mods & (CHORD_CTRL | CHORD_META) != 0 {
                let target = match key {
                    KEY_I => None,
                    KEY_J => Some(Panel::Console),
                    KEY_C => Some(Panel::Elements),
                    KEY_N => Some(Panel::Network),
                    _ => return None,
                };
                match target {
                    None => self.toggle(pool.as_deref_mut()),
                    Some(panel) => self.set(Some(true), Some(panel), pool.as_deref_mut()),
                }
                if let (true, Some(p), Some(v)) = (self.open, pool, view) {
                    match self.panel {
                        Panel::Elements if !self.trees.contains_key(&v) => self.request_tree(p, v),
                        Panel::Network => self.request_net(p, v),
                        _ => {}
                    }
                }
                return Some(true);
            }
            return None;
        }
        if self.open && self.panel == Panel::Storage && self.focus == Focus::None && self.store_sel.is_some() && matches!(k, KeyInput::Code(8) | KeyInput::Code(46)) {
            if let (Some(p), Some(v)) = (pool, view) {
                self.delete_entry(p, v);
            }
            return Some(true);
        }
        if !self.open || self.focus == Focus::None {
            return None;
        }
        match k {
            KeyInput::Code(1) | KeyInput::Code(27) => {
                if self.focus == Focus::Edit {
                    self.edit = None;
                    self.prompt.clear();
                }
                self.focus = Focus::None;
                self.multi.clear();
                return Some(true);
            }
            KeyInput::Code(38) if self.focus == Focus::Prompt => {
                if self.hist_at > 0 {
                    self.hist_at -= 1;
                    self.prompt.clear();
                    self.prompt.push_str(&self.history[self.hist_at]);
                }
                return Some(true);
            }
            KeyInput::Code(40) if self.focus == Focus::Prompt => {
                self.hist_at = (self.hist_at + 1).min(self.history.len());
                self.prompt.clear();
                if let Some(h) = self.history.get(self.hist_at) {
                    self.prompt.push_str(h);
                }
                return Some(true);
            }
            KeyInput::Code(13) => {
                match self.focus {
                    Focus::Prompt => self.submit(pool, view),
                    Focus::Edit => self.commit_edit(pool),
                    Focus::Search if self.panel == Panel::Elements => {
                        if let (Some(p), Some(v)) = (pool, view) {
                            self.tree_error = None;
                            self.trees.remove(&v);
                            self.request_tree(p, v);
                        }
                    }
                    _ => {}
                }
                return Some(true);
            }
            _ => {}
        }
        let field = if self.focus == Focus::Search { &mut self.search } else { &mut self.prompt };
        match k {
            KeyInput::Text(s) => field.push_str(s),
            KeyInput::Code(8) => field.pop(),
            KeyInput::Code(46) => field.delete(),
            KeyInput::Code(37) => field.move_caret(false),
            KeyInput::Code(39) => field.move_caret(true),
            _ => return None,
        }
        Some(true)
    }

    pub fn draw(&self, ctx: &Ctx, s: &mut Surface, view: Option<WebViewId>) {
        if !self.open {
            return;
        }
        let ui = &ctx.ui;
        let p = self.panel_rect(ui, ctx.cfg.w, ctx.cfg.h);
        let px = |v: f32| ui.pt(v) as usize;
        s.fill_rect(px(p.x), px(p.y), px(p.w), px(p.h), CARD_BG.now());
        s.fill_rect(px(p.x), px(p.y), px(p.w), 1, SEPARATOR_C.now());
        s.fill_rect(px(p.x), px(p.y + HEADER_PT), px(p.w), 1, SEPARATOR_C.now());
        if self.has_bar() {
            s.fill_rect(px(p.x), px(p.y + p.h - PROMPT_PT), px(p.w), 1, SEPARATOR_C.now());
        }
        let text = |s: &mut Surface, t: &str, bx: f32, by: f32, room: f32, weight: Weight, color: u32| {
            let img = ui.text(&ui.fit(t, MAC_FOOTNOTE_PT, ui.pt(room.max(8.0))), MAC_FOOTNOTE_PT, weight, color);
            s.blit(&img, ui.pt(bx) as i32, ui.pt(by) as i32 - img.h as i32 / 2, 255, 255);
        };
        for (i, panel) in PANELS.iter().enumerate() {
            let c = Self::tab_rect(p, i);
            let on = *panel == self.panel;
            if on {
                s.fill_rrect(ui.pt(c.x), ui.pt(c.y), ui.pt(c.w), ui.pt(c.h), ui.pt(SEG_RADIUS_PT), SEGMENT_ON.now());
            }
            let name = match panel {
                Panel::Console => "Console",
                Panel::Network => "Network",
                Panel::Elements => "Elements",
                Panel::Sources => "Sources",
                Panel::Storage => "Storage",
            };
            text(s, name, c.x + 8.0, c.y + c.h / 2.0, c.w - 12.0, if on { Weight::Semibold } else { Weight::Regular }, if on { LABEL.now() } else { LABEL_2.now() });
        }
        if self.chips_shown(p) {
            let (names, current): (Vec<&str>, usize) = if self.panel == Panel::Console { (CONSOLE_FILTERS.to_vec(), self.filter) } else { (network::FILTERS.to_vec(), self.net_filter) };
            for (i, name) in names.iter().enumerate() {
                let c = Self::chip(p, i);
                let on = i == current;
                if on {
                    s.fill_rrect(ui.pt(c.x), ui.pt(c.y), ui.pt(c.w), ui.pt(c.h), ui.pt(SEG_RADIUS_PT), SEGMENT_ON.now());
                }
                text(s, name, c.x + 8.0, c.y + c.h / 2.0, c.w - 12.0, if on { Weight::Semibold } else { Weight::Regular }, if on { LABEL.now() } else { LABEL_2.now() });
            }
        }
        let placeholder = match self.panel {
            Panel::Elements => "CSS selector",
            Panel::Sources => "Find in file",
            _ => "Filter",
        };
        bifrost_app::widgets::search_field(s, ui, &self.search, self.search_rect(p), placeholder, self.focus == Focus::Search);
        let cl = Self::clear_rect(p);
        text(s, if matches!(self.panel, Panel::Console | Panel::Network) { "Clear" } else { "Refresh" }, cl.x + 8.0, cl.y + cl.h / 2.0, cl.w - 8.0, Weight::Regular, TINT);
        if self.panel == Panel::Console {
            let cp = Self::copy_rect(p);
            text(s, "Copy", cp.x + 8.0, cp.y + cp.h / 2.0, cp.w - 8.0, Weight::Regular, TINT);
        }

        let list = self.list_rect(p);
        if self.panel == Panel::Network {
            let hy = list.y - ROW_PT;
            draw_cells(s, ui, &network::header(list.w), list, hy + ROW_PT / 2.0, hy, &text);
            s.fill_rect(px(list.x), px(list.y).saturating_sub(1), px(list.w), 1, SEPARATOR_C.now());
        }
        if let Some(v) = view {
            for (i, row) in self.visible_rows(self.rows(v, list.w), list).into_iter().enumerate() {
                let top = list.y + i as f32 * ROW_PT;
                draw_cells(s, ui, &row, list, top + ROW_PT / 2.0, top, &text);
            }
        }
        if self.edit.is_some() {
            bifrost_app::widgets::search_field(s, ui, &self.prompt, Self::prompt_rect(p), "New value, Return saves, Esc cancels", self.focus == Focus::Edit);
        } else if self.panel == Panel::Console {
            let ph = if self.multi.is_empty() { "Evaluate JavaScript".to_string() } else { format!("{} lines so far, finish the statement", self.multi.len()) };
            bifrost_app::widgets::search_field(s, ui, &self.prompt, Self::prompt_rect(p), &ph, self.focus == Focus::Prompt);
        }
    }
}

type TextFn<'a> = dyn Fn(&mut Surface, &str, f32, f32, f32, Weight, u32) + 'a;

/// Draws one row's cells. Conventions (see `elements.rs`): a negative `w` right-aligns the cell in `|w|`; text starting
/// with `elements::MARK` is a disclosure triangle plus indent guides, with `elements::RUNS` a syntax-coloured run list.
fn draw_cells(s: &mut Surface, ui: &Ui, row: &Row, list: Rect, cy: f32, top: f32, text: &TextFn) {
    let px = |v: f32| ui.pt(v) as usize;
    for c in &row.cells {
        if let Some(rest) = c.text.strip_prefix(elements::MARK) {
            let depth = ((c.x - 10.0) / elements::INDENT_PT).round().max(0.0) as usize;
            for d in 0..depth {
                s.fill_rect(px(list.x + 10.0 + d as f32 * elements::INDENT_PT + 5.0), px(top), 1, px(ROW_PT), SEPARATOR_C.now());
            }
            let sz = (ui.pt(SPACE_XS) as i32).max(2);
            let (x0, y0) = (px(list.x + c.x + 2.0) as i32, px(cy) as i32);
            match rest {
                // closed: points right
                "c" => (0..sz).for_each(|i| {
                    let hh = sz - i;
                    s.fill_rect((x0 + i) as usize, (y0 - hh) as usize, 1, (2 * hh + 1) as usize, LABEL_2.now());
                }),
                // open: points down
                "o" => (0..sz).for_each(|j| {
                    let hw = sz - j;
                    s.fill_rect((x0 + sz - hw) as usize, (y0 - sz / 2 + j) as usize, (2 * hw + 1) as usize, 1, LABEL_2.now());
                }),
                _ => {}
            }
        } else if let Some(runs) = c.text.strip_prefix(elements::RUNS) {
            let mut x = ui.pt(list.x + c.x);
            let end = ui.pt(list.x + list.w - 6.0);
            for seg in runs.split(elements::SEP) {
                let mut ch = seg.chars();
                let Some(k) = ch.next() else { continue };
                let t = ch.as_str();
                if x >= end || t.is_empty() {
                    continue;
                }
                let color = if c.color == TINT { TINT } else { elements::run_color(k) };
                let t = ui.fit(t, MAC_FOOTNOTE_PT, (end - x).max(8.0));
                let img = ui.text(&t, MAC_FOOTNOTE_PT, Weight::Regular, color);
                s.blit(&img, x as i32, ui.pt(cy) as i32 - img.h as i32 / 2, 255, 255);
                // advance by the text's width; the image carries side padding, which spread "<", tag and "=" apart
                x += ui.measure(&t, MAC_FOOTNOTE_PT, Weight::Regular);
            }
        } else if c.w < 0.0 {
            let room = (-c.w).min(list.w - c.x - 6.0).max(8.0);
            let img = ui.text(&ui.fit(&c.text, MAC_FOOTNOTE_PT, ui.pt(room)), MAC_FOOTNOTE_PT, Weight::Regular, c.color);
            s.blit(&img, ui.pt(list.x + c.x - c.w) as i32 - img.w as i32, ui.pt(cy) as i32 - img.h as i32 / 2, 255, 255);
        } else {
            text(s, &c.text, list.x + c.x, cy, c.w.min(list.w - c.x - 6.0), Weight::Regular, c.color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_history_keeps_only_the_newest_lines() {
        let mut history = Vec::new();
        for i in 0..(MAX_INPUT_HISTORY + 25) {
            remember_input(&mut history, format!("x{i}"));
        }
        assert_eq!(history.len(), MAX_INPUT_HISTORY);
        assert_eq!(history[0], "x25");
        assert_eq!(history.last().map(String::as_str), Some(format!("x{}", MAX_INPUT_HISTORY + 24).as_str()));
    }

    fn console(i: &mut Inspector, id: u32, level: &str, text: &str) {
        i.on_event(&WebViewEvent::Console { id, level: level.into(), text: text.into(), source: String::new(), line: 0 });
    }

    #[test]
    fn buffer_drops_oldest_and_filters_by_level() {
        let mut i = Inspector::new();
        for n in 0..MAX_ENTRIES + 5 {
            console(&mut i, 1, if n % 2 == 0 { "warn" } else { "log" }, &format!("m{n}"));
        }
        assert_eq!(i.logs[&1].recs.len(), MAX_ENTRIES);
        assert_eq!(i.logs[&1].recs[0].text, "m5");
        i.filter = 2;
        assert!(i.visible_console(1).iter().all(|e| e.level == "warn"));
        i.search.push_str("m6");
        assert!(i.visible_console(1).iter().all(|e| e.text.contains("m6")));
    }

    #[test]
    fn entries_get_increasing_seq_and_since_seq_pages() {
        let mut i = Inspector::new();
        for n in 0..5 {
            console(&mut i, 3, "log", &format!("a{n}"));
        }
        let first = i.console_json(3, None, 200, None, false);
        let entries = first.get("entries").and_then(Json::as_array).unwrap();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].get("seq").and_then(Json::as_u64), Some(1));
        assert_eq!(first.get("next_seq").and_then(Json::as_u64), Some(5));
        console(&mut i, 3, "error", "boom");
        let more = i.console_json(3, None, 200, Some(5), true);
        assert_eq!(more.get("entries").and_then(Json::as_array).map(|e| e.len()), Some(1));
        assert_eq!(more.get("next_seq").and_then(Json::as_u64), Some(6));
        // cleared, but the counter goes on
        console(&mut i, 3, "log", "after");
        let after = i.console_json(3, None, 200, None, false);
        assert_eq!(after.get("entries").and_then(Json::as_array).map(|e| e.len()), Some(1));
        assert_eq!(after.get("next_seq").and_then(Json::as_u64), Some(7));
    }

    #[test]
    fn multi_line_input_continues_until_balanced() {
        assert!(needs_more("function f() {"));
        assert!(needs_more("let a = 1 + \\"));
        assert!(needs_more("`abc"));
        assert!(!needs_more("foo('(')"));
        assert!(!needs_more("1 + 1"));
        assert_eq!(one_line("a\nb"), "(0,eval)(\"a\\nb\")");
        assert_eq!(one_line("a"), "a");
    }

    #[test]
    fn window_pins_the_tail_or_the_top() {
        let all: Vec<u32> = (0..10).collect();
        assert_eq!(window(all.clone(), 3, 0, true), vec![7, 8, 9]);
        assert_eq!(window(all.clone(), 3, 2, true), vec![5, 6, 7]);
        assert_eq!(window(all.clone(), 3, 99, true), vec![0, 1, 2]);
        assert_eq!(window(all, 3, 4, false), vec![4, 5, 6]);
    }

    #[test]
    fn clock_formats_milliseconds() {
        assert_eq!(clock(3_661_123), "01:01:01.123");
    }

    #[test]
    fn page_hook_entry_replaces_the_engine_copy_either_way() {
        // hook first: the engine's later copy takes the hook's text, file and line
        let mut i = Inspector::new();
        i.caps.entry(7).or_default().push_back(pagetools::Cap { level: "error".into(), text: "boom: full".into(), source: "a.js".into(), line: 3, own: false });
        console(&mut i, 7, "error", "boom");
        let r = &i.logs[&7].recs;
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].text.as_str(), r[0].source.as_str(), r[0].line), ("boom: full", "a.js", 3));
        assert!(i.caps[&7].is_empty());
        // an engine-only message stays as it is
        console(&mut i, 7, "log", "engine only");
        assert_eq!(i.logs[&7].recs.len(), 2);
        assert_eq!(i.logs[&7].recs[1].line, 0);
    }

    #[test]
    fn page_hook_entry_replaces_the_engine_copy() {
        // hook first: the engine's later copy takes the hook's text, file and line
        let mut i = Inspector::new();
        i.caps.entry(7).or_default().push_back(pagetools::Cap { level: "error".into(), text: "boom: full".into(), source: "a.js".into(), line: 3, own: false });
        console(&mut i, 7, "error", "boom");
        let r = &i.logs[&7].recs;
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].text.as_str(), r[0].source.as_str(), r[0].line), ("boom: full", "a.js", 3));
        assert!(i.caps[&7].is_empty());
        // an engine-only message stays as it is
        console(&mut i, 7, "log", "engine only");
        assert_eq!(i.logs[&7].recs.len(), 2);
        assert_eq!(i.logs[&7].recs[1].line, 0);
    }
}

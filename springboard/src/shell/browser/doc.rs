//! One page's document state machine: the `Dom`, its style sheets, the
//! optional `JsPage`, and every subresource of the page.
//!
//! Flow: `new` -> `feed` (raw bytes, held once) -> `finish_body` (decode,
//! parse, discover subresources, start scripts) -> the shell loops
//! `take_fetches` / `complete` / `run_timers` / `drain` and, when
//! `needs_restyle` / `needs_layout` say so, `restyle` and lay out.
//!
//! Scripting runs after the whole body is parsed (the parser is not
//! incremental), in the order a browser would run them: parser-blocking
//! scripts in document order, then `defer` and module scripts in order, then
//! `DOMContentLoaded`; `async` scripts run when they arrive; `load` fires when
//! every blocking subresource and script is done.

use super::dom::{parse_html, Dom, NodeId};
use super::js::{EventInit, JsPage, NetRequest, NetResponse};
use super::nav;
use super::resources::*;
use super::style::{parse_stylesheet, ua_stylesheet, FontSrc, MediaEnv, Origin, StyleEngine, StyleTree, Stylesheet};
use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

thread_local! {
    /// The parsed user-agent sheet, parsed once per thread and cloned (cheap, rules are `Arc`ed) per engine build.
    static UA_SHEET: OnceCell<Stylesheet> = const { OnceCell::new() };
}

fn ua_sheet() -> Stylesheet {
    UA_SHEET.with(|c| c.get_or_init(ua_stylesheet).clone())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Receiving the body.
    Loading,
    /// Parsed; running scripts, waiting for blocking resources.
    Scripting,
    /// `DOMContentLoaded` fired.
    Interactive,
    /// `load` fired.
    Complete,
}

struct SheetNode {
    url: String,
    media: Option<String>,
    depth: u8,
    pending: bool,
    own: Option<Stylesheet>,
    children: Vec<usize>,
}

enum Pending {
    Sheet(usize),
    Script(usize),
    Module(usize),
    Image(String),
    Font(String),
    Fetch(u32),
}

struct Inflight {
    what: Pending,
    url: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Sync,
    Defer,
    Async,
}

struct Script {
    mode: Mode,
    url: String,
    code: Option<String>,
    /// Root module record for `type=module`.
    module: Option<usize>,
    ready: bool,
    done: bool,
}

struct ModuleRec {
    url: String,
    depth: u8,
    code: Option<String>,
    /// Source arrived (or failed); false while the request is in flight.
    loaded: bool,
    executed: bool,
    deps: Vec<(String, usize)>,
}

/// Counters for the shell and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DocStats {
    pub requests: usize,
    pub bytes: usize,
    pub scripts_run: usize,
    pub sheets: usize,
}

pub struct Document {
    url: String,
    base: String,
    env: MediaEnv,
    limits: Limits,
    budget: Budget,
    dom: Rc<RefCell<Dom>>,
    js: Option<JsPage>,
    content_type: Option<String>,
    raw: Vec<u8>,
    phase: Phase,
    nodes: Vec<SheetNode>,
    order: Vec<usize>,
    sheets_version: u64,
    seen: HashSet<NodeId>,
    queue: Vec<ResourceRequest>,
    inflight: HashMap<ResourceId, Inflight>,
    next_id: ResourceId,
    requested_images: HashSet<String>,
    scripts: Vec<Script>,
    modules: Vec<ModuleRec>,
    module_by_url: HashMap<String, usize>,
    import_map: ImportMap,
    inline_modules: usize,
    scripts_run: usize,
    pub images: HashMap<String, (u32, u32)>,
    font_urls: Vec<String>,
    fonts: HashMap<String, Vec<u8>>,
    cookies: CookieJar,
    engine: Option<StyleEngine>,
    styled: (u64, u64),
    laid: Option<(u64, u64)>,
    refresh: Option<(f64, String)>,
    fired: Vec<&'static str>,
    errors: Vec<String>,
}

fn same(a: &Option<String>, b: &str) -> bool {
    a.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(b))
}

fn wrap_media<'a>(css: &'a str, media: Option<&str>) -> Cow<'a, str> {
    match media.map(str::trim) {
        Some(m) if !m.is_empty() && !m.eq_ignore_ascii_case("all") => Cow::Owned(format!("@media {m}{{{css}}}")),
        _ => Cow::Borrowed(css),
    }
}

fn js_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', " ").replace('\r', " ")
}

impl Document {
    pub fn new(url: &str, env: MediaEnv) -> Document {
        Document::with_limits(url, env, Limits::default())
    }

    pub fn with_limits(url: &str, env: MediaEnv, limits: Limits) -> Document {
        Document {
            url: url.to_owned(),
            base: url.to_owned(),
            env,
            limits,
            budget: Budget::default(),
            dom: Rc::new(RefCell::new(Dom::new())),
            js: None,
            content_type: None,
            raw: Vec::new(),
            phase: Phase::Loading,
            nodes: Vec::new(),
            order: Vec::new(),
            sheets_version: 0,
            seen: HashSet::new(),
            queue: Vec::new(),
            inflight: HashMap::new(),
            next_id: 0,
            requested_images: HashSet::new(),
            scripts: Vec::new(),
            modules: Vec::new(),
            module_by_url: HashMap::new(),
            import_map: ImportMap::default(),
            inline_modules: 0,
            scripts_run: 0,
            images: HashMap::new(),
            font_urls: Vec::new(),
            fonts: HashMap::new(),
            cookies: CookieJar::new(),
            engine: None,
            styled: (u64::MAX, u64::MAX),
            laid: None,
            refresh: None,
            fired: Vec::new(),
            errors: Vec::new(),
        }
    }

    // ---- accessors -------------------------------------------------------

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The document base URL (`<base href>` applied once the body is parsed).
    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Shared handle to the DOM (the JS page holds the same one).
    pub fn dom(&self) -> Rc<RefCell<Dom>> {
        self.dom.clone()
    }

    pub fn js(&self) -> Option<&JsPage> {
        self.js.as_ref()
    }

    pub fn js_mut(&mut self) -> Option<&mut JsPage> {
        self.js.as_mut()
    }

    /// Console output of the page so far.
    pub fn console(&self) -> Vec<String> {
        self.js.as_ref().map(JsPage::console_log).unwrap_or_default()
    }

    /// Script errors (and blocked-resource notes), oldest first.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Lifecycle events fired so far, in order (`DOMContentLoaded`, `load`).
    pub fn fired(&self) -> &[&'static str] {
        &self.fired
    }

    /// `(seconds, url)` of a `<meta http-equiv=refresh>`.
    pub fn refresh(&self) -> Option<(f64, String)> {
        self.refresh.clone()
    }

    pub fn stats(&self) -> DocStats {
        DocStats {
            requests: self.budget.requests,
            bytes: self.budget.bytes,
            scripts_run: self.scripts_run,
            sheets: self.nodes.len(),
        }
    }

    /// True while a style sheet is still loading: first paint should wait.
    pub fn render_blocked(&self) -> bool {
        self.nodes.iter().any(|n| n.pending)
    }

    /// Requests in flight (handed out, not yet completed) plus queued ones.
    pub fn pending_count(&self) -> usize {
        self.inflight.len()
    }

    // ---- loading the body -----------------------------------------------

    /// Content-Type of the main response (charset source).
    pub fn set_content_type(&mut self, ct: &str) {
        self.content_type = Some(ct.to_owned());
    }

    /// Main response headers: Content-Type and Set-Cookie.
    pub fn set_response_headers(&mut self, headers: &[(String, String)]) {
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("content-type") {
                self.content_type = Some(v.clone());
            } else if k.eq_ignore_ascii_case("set-cookie") {
                self.cookies.set_from_header(&self.url, v);
            }
        }
    }

    /// Appends body bytes. The bytes are kept raw (once) until `finish_body`
    /// decodes and parses them; anything past `max_html_bytes` is dropped.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.phase != Phase::Loading {
            return;
        }
        let room = self.limits.max_html_bytes.saturating_sub(self.raw.len());
        self.raw.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    /// The body is complete: decode (consuming the raw buffer), parse, find
    /// subresources and start running scripts.
    pub fn finish_body(&mut self) {
        if self.phase != Phase::Loading {
            return;
        }
        let raw = std::mem::take(&mut self.raw);
        let (cs, declared) = detect_charset(&raw, self.content_type.as_deref());
        let text = decode(raw, cs, declared);
        let parsed = parse_html(&text);
        drop(text);
        *self.dom.borrow_mut() = parsed;
        self.phase = Phase::Scripting;
        self.discover(true);
        self.advance();
    }

    // ---- subresource queue ----------------------------------------------

    /// Requests made since the last call, most urgent first (css, scripts,
    /// script fetches, images, fonts); order within a priority is discovery order.
    pub fn take_fetches(&mut self) -> Vec<ResourceRequest> {
        let mut q = std::mem::take(&mut self.queue);
        q.sort_by_key(|r| r.priority);
        q
    }

    fn request(&mut self, url: String, kind: ResourceKind, what: Pending) -> bool {
        if kind == ResourceKind::Image && !self.requested_images.insert(url.clone()) {
            return false;
        }
        if !self.budget.admit(kind, &self.limits) {
            self.errors.push(format!("request cap reached, skipped {url}"));
            return false;
        }
        self.next_id += 1;
        let id = self.next_id;
        let mut headers = Vec::new();
        if let Some(c) = self.cookies.header_for(&url) {
            headers.push(("Cookie".to_owned(), c));
        }
        self.queue.push(ResourceRequest {
            id,
            url: url.clone(),
            kind,
            priority: kind.priority(),
            render_blocking: kind == ResourceKind::Stylesheet,
            method: "GET".into(),
            headers,
            body: None,
        });
        self.inflight.insert(id, Inflight { what, url });
        true
    }

    /// Answers request `id`. Raw bytes are dropped as soon as they are decoded or measured.
    pub fn complete(&mut self, id: ResourceId, resp: ResourceResponse) {
        let Some(Inflight { what, url }) = self.inflight.remove(&id) else { return };
        let final_url = if resp.url.is_empty() { url.clone() } else { resp.url.clone() };
        let mut set_cookies = Vec::new();
        for (k, v) in &resp.headers {
            if k.eq_ignore_ascii_case("set-cookie") {
                self.cookies.set_from_header(&final_url, v);
                set_cookies.push(v.clone());
            }
        }
        let is_fetch = matches!(what, Pending::Fetch(_));
        let mut failed = resp.error.is_some() || (resp.status >= 400 && !is_fetch);
        if !failed && !self.budget.add_bytes(resp.body.len(), &self.limits) {
            self.errors.push(format!("byte cap reached, dropped {url}"));
            failed = true;
        }
        match what {
            Pending::Sheet(idx) => {
                if failed || resp.body.len() > self.limits.max_css_bytes {
                    self.nodes[idx].pending = false;
                    self.sheets_version += 1;
                } else {
                    let text = decode_text(&resp.body, resp.content_type());
                    drop(resp);
                    self.load_sheet_text(idx, &text);
                }
            }
            Pending::Script(i) => {
                if failed || resp.body.len() > self.limits.max_script_bytes {
                    self.scripts[i].done = true;
                } else {
                    self.scripts[i].code = Some(decode_text(&resp.body, resp.content_type()));
                    self.scripts[i].ready = true;
                }
            }
            Pending::Module(m) => {
                self.modules[m].loaded = true;
                if !failed && resp.body.len() <= self.limits.max_script_bytes {
                    self.modules[m].code = Some(decode_text(&resp.body, resp.content_type()));
                    self.link_module(m);
                }
            }
            Pending::Image(src) => {
                if !failed {
                    if let Some(size) = image_size(&resp.body) {
                        if final_url != src {
                            self.images.insert(final_url, size);
                        }
                        self.images.insert(src, size);
                    }
                }
            }
            Pending::Font(src) => {
                if !failed {
                    self.fonts.insert(src, resp.body);
                }
            }
            Pending::Fetch(js_id) => {
                let origin = origin_of(&self.url);
                let net = if let Some(e) = resp.error.clone() {
                    NetResponse::failed(&e)
                } else if failed {
                    NetResponse::failed("blocked: response over the page byte cap")
                } else if !cors_allows(&origin, &url, &resp.headers) {
                    NetResponse::failed("blocked by CORS")
                } else {
                    NetResponse {
                        status: resp.status,
                        status_text: if resp.status < 400 { "OK".into() } else { String::new() },
                        headers: resp.headers.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("set-cookie")).cloned().collect(),
                        body: resp.body,
                        url: resp.url,
                        error: None,
                    }
                };
                if let Some(js) = self.js.as_mut() {
                    for c in &set_cookies {
                        if !c.to_ascii_lowercase().contains("httponly") {
                            let _ = js.run_script(&format!("document.cookie='{}'", js_quote(c)), "cookie");
                        }
                    }
                    js.complete_request(js_id, net);
                }
            }
        }
        self.after_js();
        self.advance();
    }

    // ---- style sheets ----------------------------------------------------

    fn new_sheet(&mut self, url: String, media: Option<String>, depth: u8, pending: bool) -> usize {
        self.nodes.push(SheetNode { url, media, depth, pending, own: None, children: Vec::new() });
        self.nodes.len() - 1
    }

    /// Parses `text` into sheet `idx`, requests its `@import`s (depth-limited) and notes its fonts.
    fn load_sheet_text(&mut self, idx: usize, text: &str) {
        let (url, media, depth) = {
            let n = &self.nodes[idx];
            (n.url.clone(), n.media.clone(), n.depth)
        };
        let imports = extract_imports(text);
        let sheet = parse_stylesheet(&wrap_media(text, media.as_deref()), Origin::Author);
        for face in &sheet.font_faces {
            for src in &face.sources {
                if let FontSrc::Url { url: u, .. } = src {
                    if let Some(abs) = nav::resolve(&url, u) {
                        if !self.font_urls.contains(&abs) {
                            self.font_urls.push(abs);
                        }
                    }
                }
            }
        }
        for (spec, m) in imports {
            if depth >= self.limits.max_import_depth {
                break;
            }
            let Some(abs) = nav::resolve(&url, &spec) else { continue };
            let child = self.new_sheet(abs.clone(), m.or_else(|| media.clone()), depth + 1, true);
            self.nodes[idx].children.push(child);
            if !self.request(abs, ResourceKind::Stylesheet, Pending::Sheet(child)) {
                self.nodes[child].pending = false;
            }
        }
        let n = &mut self.nodes[idx];
        n.own = Some(sheet);
        n.pending = false;
        self.sheets_version += 1;
    }

    fn flatten(&self, idx: usize, out: &mut Vec<Stylesheet>) {
        let n = &self.nodes[idx];
        for &c in &n.children {
            self.flatten(c, out);
        }
        if let Some(s) = &n.own {
            out.push(s.clone());
        }
    }

    /// `@font-face` URLs found so far (absolute). Fonts are never fetched unless asked.
    pub fn font_urls(&self) -> &[String] {
        &self.font_urls
    }

    /// Queues a font download for a family the layout actually needs.
    pub fn request_font(&mut self, url: &str) -> bool {
        if !self.font_urls.iter().any(|u| u == url) || self.fonts.contains_key(url) {
            return false;
        }
        self.request(url.to_owned(), ResourceKind::Font, Pending::Font(url.to_owned()))
    }

    /// Hands over a downloaded font file (it is not kept here afterwards).
    pub fn take_font(&mut self, url: &str) -> Option<Vec<u8>> {
        self.fonts.remove(url)
    }

    /// Changes the media environment (rotation, dark mode); dirties style.
    pub fn set_env(&mut self, env: MediaEnv) {
        if env != self.env {
            self.env = env;
            self.sheets_version += 1;
        }
    }

    // ---- dirty flags and styling -----------------------------------------

    fn dom_gen(&self) -> u64 {
        self.dom.borrow().generation()
    }

    pub fn needs_restyle(&self) -> bool {
        self.engine.is_none() || self.styled != (self.dom_gen(), self.sheets_version)
    }

    pub fn needs_layout(&self) -> bool {
        self.laid != Some((self.dom_gen(), self.sheets_version))
    }

    /// Rebuilds the engine from the UA sheet plus the author sheets in document
    /// order (imports before their importer) when anything changed.
    pub fn restyle(&mut self) -> bool {
        if !self.needs_restyle() {
            return false;
        }
        let mut sheets = vec![ua_sheet()];
        for &i in &self.order {
            self.flatten(i, &mut sheets);
        }
        self.engine = Some(StyleEngine::new(sheets, self.env));
        self.styled = (self.dom_gen(), self.sheets_version);
        true
    }

    /// The shell calls this after it laid the page out for the current state.
    pub fn mark_laid_out(&mut self) {
        self.laid = Some((self.dom_gen(), self.sheets_version));
    }

    pub fn style_engine(&self) -> Option<&StyleEngine> {
        self.engine.as_ref()
    }

    /// Computes the whole style tree with the current engine.
    pub fn compute_styles(&self) -> Option<StyleTree> {
        self.engine.as_ref().map(|e| e.compute(&self.dom.borrow()))
    }

    // ---- cookies -----------------------------------------------------------

    /// What `document.cookie` reads for this page.
    pub fn document_cookie(&self) -> String {
        self.cookies.document_cookie(&self.url)
    }

    pub fn set_cookie(&mut self, header: &str) {
        self.cookies.set_from_header(&self.url, header);
    }

    pub fn cookies(&self) -> &CookieJar {
        &self.cookies
    }

    /// Takes cookies the page script set into the jar (so requests carry them).
    fn sync_cookies_from_js(&mut self) {
        let Some(js) = self.js.as_mut() else { return };
        if let Ok(s) = js.eval_string("document.cookie") {
            for part in s.split("; ").filter(|p| p.contains('=')) {
                self.cookies.set_from_header(&self.url, part);
            }
        }
    }

    // ---- discovery ---------------------------------------------------------

    /// Walks the DOM for subresources not seen before. The first pass (after
    /// parsing) classifies scripts as sync/defer/async by their attributes;
    /// later passes (after script mutations) treat new scripts as async.
    fn discover(&mut self, initial: bool) {
        let dom_rc = self.dom.clone();
        let dom = dom_rc.borrow();
        let nodes: Vec<NodeId> = dom
            .descendants(dom.root())
            .filter(|n| matches!(dom.tag(*n), Some("base" | "meta" | "link" | "style" | "img" | "script")))
            .filter(|n| !self.seen.contains(n))
            .collect();
        if initial {
            if let Some(href) = nodes.iter().filter(|n| dom.tag(**n) == Some("base")).find_map(|n| dom.attr(*n, "href")) {
                if let Some(b) = nav::resolve(&self.url, href) {
                    self.base = b;
                }
            }
        }
        for n in nodes {
            self.seen.insert(n);
            let tag = dom.tag(n).unwrap_or("");
            match tag {
                "meta" => {
                    if initial && self.refresh.is_none() && same(&dom.attr(n, "http-equiv").map(str::to_owned), "refresh") {
                        if let Some(c) = dom.attr(n, "content") {
                            self.refresh = self.parse_refresh(c);
                        }
                    }
                }
                "link" => self.discover_link(&dom, n),
                "style" => {
                    let media = dom.attr(n, "media").map(str::to_owned);
                    let css = dom.text_content(n);
                    let idx = self.new_sheet(self.base.clone(), media, 0, false);
                    self.order.push(idx);
                    self.load_sheet_text(idx, &css);
                }
                "img" => {
                    if let Some(src) = self.image_source(&dom, n) {
                        if let Some(abs) = nav::resolve(&self.base, &src) {
                            self.request(abs.clone(), ResourceKind::Image, Pending::Image(abs));
                        }
                    }
                }
                "script" => self.discover_script(&dom, n, initial),
                _ => {}
            }
        }
    }

    fn parse_refresh(&self, content: &str) -> Option<(f64, String)> {
        let (secs, rest) = match content.split_once(|c: char| c == ';' || c == ',') {
            Some((s, r)) => (s, Some(r)),
            None => (content, None),
        };
        let secs: f64 = secs.trim().parse().ok()?;
        let target = match rest {
            Some(r) => {
                let r = r.trim();
                let lower = r.to_ascii_lowercase();
                let u = if lower.starts_with("url") { r[3..].trim_start().strip_prefix('=').unwrap_or(&r[3..]).trim() } else { r };
                let u = u.trim_matches(|c: char| c == '"' || c == '\'');
                nav::resolve(&self.base, u).unwrap_or_else(|| self.url.clone())
            }
            None => self.url.clone(),
        };
        Some((secs.max(0.0), target))
    }

    fn discover_link(&mut self, dom: &Dom, n: NodeId) {
        let is_sheet = dom
            .attr(n, "rel")
            .is_some_and(|r| r.to_ascii_lowercase().split_ascii_whitespace().any(|x| x == "stylesheet"));
        if !is_sheet {
            return; // preload, prefetch, preconnect, icon, ... are ignored
        }
        let Some(abs) = dom.attr(n, "href").and_then(|h| nav::resolve(&self.base, h)) else { return };
        let media = dom.attr(n, "media").map(str::to_owned);
        let idx = self.new_sheet(abs.clone(), media, 0, true);
        self.order.push(idx);
        if !self.request(abs, ResourceKind::Stylesheet, Pending::Sheet(idx)) {
            self.nodes[idx].pending = false;
        }
    }

    fn image_source(&self, dom: &Dom, n: NodeId) -> Option<String> {
        if let Some(p) = dom.parent(n).filter(|p| dom.tag(*p) == Some("picture")) {
            for s in dom.children(p) {
                if s == n {
                    break;
                }
                if dom.tag(s) != Some("source") {
                    continue;
                }
                if dom.attr(s, "media").is_some_and(|m| !media_matches(m, &self.env)) {
                    continue;
                }
                if dom.attr(s, "type").is_some_and(|t| !supported_image_type(t)) {
                    continue;
                }
                if let Some(u) = dom.attr(s, "srcset").and_then(pick_srcset) {
                    return Some(u);
                }
            }
        }
        dom.attr(n, "srcset").and_then(pick_srcset).or_else(|| dom.attr(n, "src").map(str::to_owned))
    }

    fn discover_script(&mut self, dom: &Dom, n: NodeId, initial: bool) {
        if !self.limits.scripting || dom.attr(n, "nomodule").is_some() {
            return;
        }
        let kind = dom.attr(n, "type").map(|t| t.trim().to_ascii_lowercase()).unwrap_or_default();
        let module = match kind.as_str() {
            "" | "text/javascript" | "application/javascript" | "text/ecmascript" | "application/ecmascript" | "text/jscript" => false,
            "module" => true,
            "importmap" => {
                self.import_map.merge(ImportMap::parse(&dom.text_content(n)));
                return;
            }
            _ => return,
        };
        if self.scripts.len() >= self.limits.max_scripts {
            self.errors.push("script cap reached".into());
            return;
        }
        let src = dom.attr(n, "src").map(str::to_owned);
        let has_async = dom.attr(n, "async").is_some();
        let has_defer = dom.attr(n, "defer").is_some();
        let abs = match &src {
            Some(s) => match nav::resolve(&self.base, s) {
                Some(a) => Some(a),
                None => return,
            },
            None => None,
        };
        let mode = if !initial || has_async && abs.is_some() || has_async && module {
            Mode::Async
        } else if module || (abs.is_some() && has_defer) {
            Mode::Defer
        } else {
            Mode::Sync
        };
        let idx = self.scripts.len();
        if module {
            let (m, url) = match &abs {
                Some(a) => (self.module_for(a, 0), a.clone()),
                None => {
                    self.inline_modules += 1;
                    let url = format!("{}#module{}", self.base, self.inline_modules);
                    let m = self.new_module(url.clone(), 0);
                    self.modules[m].loaded = true;
                    self.modules[m].code = Some(dom.text_content(n));
                    self.link_module(m);
                    (m, url)
                }
            };
            self.scripts.push(Script { mode, url, code: None, module: Some(m), ready: false, done: false });
            return;
        }
        match abs {
            Some(a) => {
                self.scripts.push(Script { mode, url: a.clone(), code: None, module: None, ready: false, done: false });
                if !self.request(a, ResourceKind::Script, Pending::Script(idx)) {
                    self.scripts[idx].done = true;
                }
            }
            None => {
                let code = dom.text_content(n);
                self.scripts.push(Script { mode, url: format!("{}#inline{idx}", self.base), code: Some(code), module: None, ready: true, done: false });
            }
        }
    }

    // ---- modules -------------------------------------------------------------

    fn new_module(&mut self, url: String, depth: u8) -> usize {
        self.modules.push(ModuleRec { url: url.clone(), depth, code: None, loaded: false, executed: false, deps: Vec::new() });
        let idx = self.modules.len() - 1;
        self.module_by_url.insert(url, idx);
        idx
    }

    /// The record for `url`, creating it and requesting the source if new.
    fn module_for(&mut self, url: &str, depth: u8) -> usize {
        if let Some(&m) = self.module_by_url.get(url) {
            return m;
        }
        let m = self.new_module(url.to_owned(), depth);
        if !self.request(url.to_owned(), ResourceKind::Module, Pending::Module(m)) {
            self.modules[m].loaded = true;
        }
        m
    }

    /// Resolves and requests the static imports of module `m`.
    fn link_module(&mut self, m: usize) {
        let (url, depth) = (self.modules[m].url.clone(), self.modules[m].depth);
        let Some(specs) = self.modules[m].code.as_deref().map(module_deps) else { return };
        for spec in specs {
            let Some(abs) = self.import_map.resolve(&spec, &url) else { continue };
            if depth >= self.limits.max_module_depth && !self.module_by_url.contains_key(&abs) {
                self.errors.push(format!("module import depth limit, skipped {abs}"));
                continue;
            }
            let d = self.module_for(&abs, depth + 1);
            self.modules[m].deps.push((spec, d));
        }
    }

    fn graph_ready(&self, m: usize, seen: &mut HashSet<usize>) -> bool {
        if !seen.insert(m) {
            return true;
        }
        let r = &self.modules[m];
        r.loaded && r.deps.iter().all(|(_, d)| self.graph_ready(*d, seen))
    }

    fn exec_order(&self, m: usize, seen: &mut HashSet<usize>, out: &mut Vec<usize>) {
        if !seen.insert(m) {
            return;
        }
        for (_, d) in &self.modules[m].deps {
            self.exec_order(*d, seen, out);
        }
        if !self.modules[m].executed {
            out.push(m);
        }
    }

    // ---- the scripting phase ---------------------------------------------------

    fn ensure_js(&mut self) {
        if self.js.is_some() || !self.limits.scripting {
            return;
        }
        let mut js = JsPage::new(self.dom.clone(), &self.base);
        let cookie = self.cookies.document_cookie(&self.url);
        for part in cookie.split("; ").filter(|p| p.contains('=')) {
            let _ = js.run_script(&format!("document.cookie='{}'", js_quote(part)), "cookie");
        }
        self.js = Some(js);
    }

    fn script_ready(&self, i: usize) -> bool {
        let s = &self.scripts[i];
        match s.module {
            Some(m) => self.graph_ready(m, &mut HashSet::new()),
            None => s.ready,
        }
    }

    fn run_script_at(&mut self, i: usize) {
        self.scripts[i].done = true;
        if self.scripts_run >= self.limits.max_scripts {
            self.errors.push("script run cap reached".into());
            return;
        }
        self.scripts_run += 1;
        self.ensure_js();
        if let Some(m) = self.scripts[i].module {
            let mut order = Vec::new();
            self.exec_order(m, &mut HashSet::new(), &mut order);
            for idx in order {
                self.run_module(idx);
            }
        } else if let Some(code) = self.scripts[i].code.take() {
            let url = self.scripts[i].url.clone();
            if code.len() <= self.limits.max_script_bytes {
                if let Some(js) = self.js.as_mut() {
                    if let Err(e) = js.run_script(&code, &url) {
                        self.errors.push(e);
                    }
                }
            }
        }
        self.after_js();
    }

    fn run_module(&mut self, idx: usize) {
        self.modules[idx].executed = true;
        let Some(code) = self.modules[idx].code.take() else { return };
        let url = self.modules[idx].url.clone();
        let deps = self.modules[idx].deps.clone();
        let rewritten = rewrite_module(&code, idx, &|spec| deps.iter().find(|(s, _)| s == spec).map(|d| d.1));
        drop(code);
        if let Some(js) = self.js.as_mut() {
            if let Err(e) = js.run_script(&rewritten, &url) {
                self.errors.push(e);
            }
        }
    }

    fn fire(&mut self, name: &'static str) {
        self.fired.push(name);
        if let Some(js) = self.js.as_mut() {
            js.dispatch_event(NodeId(0), name, EventInit::bubbling());
        }
        self.after_js();
    }

    /// Collects what script did: new subresources and fetch/XHR requests.
    fn after_js(&mut self) {
        let Some(js) = self.js.as_mut() else { return };
        let dirty = js.dirty();
        let reqs = js.take_requests();
        if dirty {
            self.discover(false);
        }
        if !reqs.is_empty() {
            self.sync_cookies_from_js();
        }
        for r in reqs {
            self.queue_js_request(r);
        }
    }

    fn queue_js_request(&mut self, r: NetRequest) {
        if !self.budget.admit(ResourceKind::Fetch, &self.limits) {
            if let Some(js) = self.js.as_mut() {
                js.complete_request(r.id, NetResponse::failed("blocked: request cap reached"));
            }
            return;
        }
        self.next_id += 1;
        let id = self.next_id;
        let mut headers = r.headers;
        if origin_of(&r.url) == origin_of(&self.url) {
            if let Some(c) = self.cookies.header_for(&r.url) {
                headers.push(("Cookie".to_owned(), c));
            }
        }
        self.queue.push(ResourceRequest {
            id,
            url: r.url.clone(),
            kind: ResourceKind::Fetch,
            priority: ResourceKind::Fetch.priority(),
            render_blocking: false,
            method: r.method,
            headers,
            body: r.body,
        });
        self.inflight.insert(id, Inflight { what: Pending::Fetch(r.id), url: r.url });
    }

    /// Runs whatever is runnable: blocking scripts in order, then deferred ones,
    /// async ones as they are ready, then the lifecycle events.
    fn advance(&mut self) {
        if self.phase == Phase::Loading {
            return;
        }
        loop {
            let mut progressed = false;
            // parser-blocking scripts, in document order
            while let Some(i) = self.scripts.iter().position(|s| s.mode == Mode::Sync && !s.done) {
                if !self.script_ready(i) {
                    break;
                }
                self.run_script_at(i);
                progressed = true;
            }
            let sync_done = !self.scripts.iter().any(|s| s.mode == Mode::Sync && !s.done);
            if sync_done {
                while let Some(i) = self.scripts.iter().position(|s| s.mode == Mode::Defer && !s.done) {
                    if !self.script_ready(i) {
                        break;
                    }
                    self.run_script_at(i);
                    progressed = true;
                }
            }
            let ready_async: Vec<usize> = (0..self.scripts.len())
                .filter(|&i| self.scripts[i].mode == Mode::Async && !self.scripts[i].done && self.script_ready(i))
                .collect();
            for i in ready_async {
                self.run_script_at(i);
                progressed = true;
            }
            let blocking_done = sync_done && !self.scripts.iter().any(|s| s.mode == Mode::Defer && !s.done);
            if blocking_done && self.phase == Phase::Scripting {
                self.phase = Phase::Interactive;
                self.fire("DOMContentLoaded");
                progressed = true;
            }
            if self.phase == Phase::Interactive && self.load_ready() {
                self.phase = Phase::Complete;
                self.fire("load");
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
    }

    /// `load` waits for sheets, scripts, modules and images (not fonts, not script fetches).
    fn load_ready(&self) -> bool {
        self.scripts.iter().all(|s| s.done)
            && !self.inflight.values().any(|f| matches!(f.what, Pending::Sheet(_) | Pending::Script(_) | Pending::Module(_) | Pending::Image(_)))
    }

    /// Fires timers due at `now_ms`; true when the DOM changed.
    pub fn run_timers(&mut self, now_ms: f64) -> bool {
        let changed = self.js.as_mut().is_some_and(|js| js.run_timers(now_ms));
        self.after_js();
        self.advance();
        changed
    }

    /// Runs pending microtasks and collects their effects.
    pub fn drain(&mut self) {
        if let Some(js) = self.js.as_mut() {
            js.drain_microtasks();
        }
        self.after_js();
        self.advance();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> MediaEnv {
        MediaEnv { width: 1200.0, height: 1920.0, dark: false }
    }

    /// In-memory resource provider.
    #[derive(Default)]
    struct Fake {
        map: HashMap<String, ResourceResponse>,
    }

    impl Fake {
        fn add(mut self, url: &str, ct: &str, body: &[u8]) -> Fake {
            self.map.insert(url.to_owned(), ResourceResponse::ok(ct, body));
            self
        }

        fn respond(&self, req: &ResourceRequest) -> ResourceResponse {
            self.map.get(&req.url).cloned().unwrap_or_else(|| ResourceResponse::status(404))
        }
    }

    /// Serves queued requests until none are left; returns the urls in request order.
    fn settle(d: &mut Document, f: &Fake) -> Vec<String> {
        let mut order = Vec::new();
        loop {
            let reqs = d.take_fetches();
            if reqs.is_empty() {
                return order;
            }
            for r in reqs {
                order.push(r.url.clone());
                d.complete(r.id, f.respond(&r));
            }
        }
    }

    fn open(html: &str, f: &Fake) -> Document {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.set_content_type("text/html; charset=utf-8");
        d.feed(html.as_bytes());
        d.finish_body();
        settle(&mut d, f);
        d
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    fn request_for(reqs: &[ResourceRequest], url: &str) -> ResourceRequest {
        reqs.iter().find(|r| r.url == url).unwrap_or_else(|| panic!("no request for {url}: {reqs:?}")).clone()
    }

    #[test]
    fn blocking_scripts_run_in_document_order() {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.feed(b"<script src=\"/a.js\"></script><script>console.log('inline')</script><script src=\"/b.js\"></script>");
        d.finish_body();
        let reqs = d.take_fetches();
        assert_eq!(reqs.len(), 2);
        // b arrives first but must wait for a and the inline script after it
        d.complete(request_for(&reqs, "http://a.test/b.js").id, ResourceResponse::ok("text/javascript", b"console.log('b')"));
        assert!(d.console().is_empty());
        assert_eq!(d.phase(), Phase::Scripting);
        d.complete(request_for(&reqs, "http://a.test/a.js").id, ResourceResponse::ok("text/javascript", b"console.log('a')"));
        assert_eq!(d.console(), vec!["a", "inline", "b"]);
        assert_eq!(d.phase(), Phase::Complete);
    }

    #[test]
    fn failed_blocking_script_does_not_block_the_rest() {
        let f = Fake::default();
        let d = open("<script src=\"/gone.js\"></script><script>console.log('after')</script>", &f);
        assert_eq!(d.console(), vec!["after"]);
        assert_eq!(d.phase(), Phase::Complete);
    }

    #[test]
    fn defer_runs_after_parse_in_order_before_dom_content_loaded() {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.feed(
            b"<script defer src=\"/d1.js\"></script><script defer src=\"/d2.js\"></script>\
              <script>console.log('sync');document.addEventListener('DOMContentLoaded',()=>console.log('dcl'))</script>",
        );
        d.finish_body();
        let reqs = d.take_fetches();
        d.complete(request_for(&reqs, "http://a.test/d2.js").id, ResourceResponse::ok("text/javascript", b"console.log('d2')"));
        assert_eq!(d.console(), vec!["sync"]);
        assert!(d.fired().is_empty());
        d.complete(request_for(&reqs, "http://a.test/d1.js").id, ResourceResponse::ok("text/javascript", b"console.log('d1')"));
        assert_eq!(d.console(), vec!["sync", "d1", "d2", "dcl"]);
        assert_eq!(d.fired(), &["DOMContentLoaded", "load"]);
    }

    #[test]
    fn async_script_runs_when_ready_and_does_not_hold_dom_content_loaded() {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.feed(b"<script async src=\"/x.js\"></script><script>console.log('sync')</script>");
        d.finish_body();
        assert_eq!(d.console(), vec!["sync"]);
        assert_eq!(d.fired(), &["DOMContentLoaded"]);
        assert_eq!(d.phase(), Phase::Interactive);
        let reqs = d.take_fetches();
        d.complete(reqs[0].id, ResourceResponse::ok("text/javascript", b"console.log('x')"));
        assert_eq!(d.console(), vec!["sync", "x"]);
        assert_eq!(d.fired(), &["DOMContentLoaded", "load"]);
    }

    #[test]
    fn load_waits_for_images_and_stylesheets() {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.feed(b"<link rel=stylesheet href=\"s.css\"><img src=\"i.png\"><p>x</p>");
        d.finish_body();
        assert_eq!(d.fired(), &["DOMContentLoaded"]);
        assert!(d.render_blocked());
        let reqs = d.take_fetches();
        let img = request_for(&reqs, "http://a.test/p/i.png").id;
        let css = request_for(&reqs, "http://a.test/p/s.css").id;
        d.complete(css, ResourceResponse::ok("text/css", b"p{color:red}"));
        assert!(!d.render_blocked());
        assert_eq!(d.fired(), &["DOMContentLoaded"]);
        d.complete(img, ResourceResponse::ok("image/png", &png(30, 20)));
        assert_eq!(d.fired(), &["DOMContentLoaded", "load"]);
        assert_eq!(d.images.get("http://a.test/p/i.png"), Some(&(30, 20)));
    }

    #[test]
    fn requests_are_prioritised_css_scripts_images() {
        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<img src=\"i.png\"><script src=\"s.js\"></script><link rel=stylesheet href=\"c.css\"><link rel=preload href=\"x.woff2\" as=font>");
        d.finish_body();
        let kinds: Vec<ResourceKind> = d.take_fetches().iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec![ResourceKind::Stylesheet, ResourceKind::Script, ResourceKind::Image]);
    }

    #[test]
    fn import_chain_is_depth_limited() {
        let f = Fake::default()
            .add("http://a.test/a.css", "text/css", b"@import \"b.css\"; a{color:red}")
            .add("http://a.test/b.css", "text/css", b"@import \"c.css\"; b{color:red}")
            .add("http://a.test/c.css", "text/css", b"@import \"d.css\"; c{color:red}")
            .add("http://a.test/d.css", "text/css", b"d{color:red}");
        let limits = Limits { max_import_depth: 2, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<link rel=stylesheet href=\"a.css\">");
        d.finish_body();
        let order = settle(&mut d, &f);
        assert_eq!(order, vec!["http://a.test/a.css", "http://a.test/b.css", "http://a.test/c.css"]);
        assert!(!d.render_blocked());
    }

    #[test]
    fn imported_sheets_are_counted_and_styles_compute() {
        let f = Fake::default()
            .add("http://a.test/a.css", "text/css", b"@import \"b.css\"; p{color:red}")
            .add("http://a.test/b.css", "text/css", b"p{color:blue}");
        let mut d = open("<link rel=stylesheet href=\"/a.css\"><p id=x>t</p>", &f);
        assert_eq!(d.stats().sheets, 2);
        d.restyle();
        let tree = d.compute_styles().expect("styles");
        let id = d.dom().borrow().get_element_by_id("x").expect("p");
        assert!(tree.get(id).is_some());
    }

    #[test]
    fn media_attribute_gates_a_sheet() {
        let f = Fake::default().add("http://a.test/p/p.css", "text/css", b"body{color:red} p{margin:0} div{margin:0}");
        let mut with = open("<link rel=stylesheet href=\"p.css\" media=\"print\">", &f);
        let mut without = open("<p>x</p>", &f);
        with.restyle();
        without.restyle();
        assert_eq!(with.style_engine().map(|e| e.rule_count()), without.style_engine().map(|e| e.rule_count()));
        let mut screen = open("<link rel=stylesheet href=\"p.css\" media=\"screen and (min-width: 600px)\">", &f);
        screen.restyle();
        assert!(screen.style_engine().map(|e| e.rule_count()) > without.style_engine().map(|e| e.rule_count()));
    }

    #[test]
    fn font_faces_are_collected_not_fetched() {
        let f = Fake::default().add("http://a.test/f.css", "text/css", b"@font-face{font-family:X;src:url(fonts/x.woff2) format('woff2')}");
        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<link rel=stylesheet href=\"f.css\">");
        d.finish_body();
        let order = settle(&mut d, &f);
        assert_eq!(order, vec!["http://a.test/f.css"]);
        assert_eq!(d.font_urls(), &["http://a.test/fonts/x.woff2".to_owned()]);
        assert!(d.request_font("http://a.test/fonts/x.woff2"));
        let reqs = d.take_fetches();
        assert_eq!(reqs[0].kind, ResourceKind::Font);
        d.complete(reqs[0].id, ResourceResponse::ok("font/woff2", b"wOF2"));
        assert_eq!(d.take_font("http://a.test/fonts/x.woff2"), Some(b"wOF2".to_vec()));
        assert_eq!(d.take_font("http://a.test/fonts/x.woff2"), None);
        assert!(!d.request_font("http://a.test/not-declared.woff2"));
    }

    #[test]
    fn charset_from_header_meta_and_bom() {
        let mut d = Document::new("http://a.test/", env());
        d.set_content_type("text/html; charset=iso-8859-1");
        d.feed(b"<p id=a>caf\xE9</p>");
        d.finish_body();
        assert_eq!(d.dom().borrow().text_content(d.dom().borrow().get_element_by_id("a").unwrap()), "caf\u{e9}");

        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<meta charset=\"windows-1252\"><p id=a>na\xEFve</p>");
        d.finish_body();
        assert_eq!(d.dom().borrow().text_content(d.dom().borrow().get_element_by_id("a").unwrap()), "na\u{ef}ve");

        let mut bytes = vec![0xFF, 0xFE];
        for u in "<p id=a>h\u{e9}</p>".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let mut d = Document::new("http://a.test/", env());
        d.feed(&bytes);
        d.finish_body();
        assert_eq!(d.dom().borrow().text_content(d.dom().borrow().get_element_by_id("a").unwrap()), "h\u{e9}");
    }

    #[test]
    fn feeding_in_chunks_matches_one_shot() {
        let html = "<p id=a>caf\u{e9} \u{1F600}</p>".as_bytes();
        let mut d = Document::new("http://a.test/", env());
        for chunk in html.chunks(3) {
            d.feed(chunk);
        }
        d.finish_body();
        assert_eq!(d.dom().borrow().text_content(d.dom().borrow().get_element_by_id("a").unwrap()), "caf\u{e9} \u{1F600}");
    }

    #[test]
    fn body_is_capped() {
        let limits = Limits { max_html_bytes: 20, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<p id=a>1234567890</p><p id=b>x</p>");
        d.finish_body();
        assert!(d.dom().borrow().get_element_by_id("b").is_none());
    }

    #[test]
    fn dom_content_loaded_then_load_with_listeners() {
        let f = Fake::default();
        let d = open(
            "<script>document.addEventListener('DOMContentLoaded',()=>console.log('dcl'));\
             window.addEventListener('load',()=>console.log('load'));console.log('run')</script>",
            &f,
        );
        assert_eq!(d.fired(), &["DOMContentLoaded", "load"]);
        let log = d.console();
        assert_eq!(log[0], "run");
        assert_eq!(log[1], "dcl");
    }

    #[test]
    fn js_fetch_is_bridged_and_answered() {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.feed(b"<script>fetch('/api/x').then(r=>r.text()).then(t=>console.log('got '+t))</script>");
        d.finish_body();
        let reqs = d.take_fetches();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].kind, ResourceKind::Fetch);
        assert_eq!(reqs[0].url, "http://a.test/api/x");
        d.complete(reqs[0].id, ResourceResponse::ok("text/plain", b"hello"));
        assert_eq!(d.console(), vec!["got hello"]);
    }

    #[test]
    fn js_fetch_cross_origin_needs_cors_header() {
        let mut d = Document::new("http://a.test/", env());
        d.feed(
            b"<script>fetch('http://b.test/x').then(()=>console.log('ok1'),()=>console.log('blocked1'));\
              fetch('http://c.test/x').then(()=>console.log('ok2'),()=>console.log('blocked2'));</script>",
        );
        d.finish_body();
        let reqs = d.take_fetches();
        d.complete(request_for(&reqs, "http://b.test/x").id, ResourceResponse::ok("text/plain", b"x"));
        d.complete(
            request_for(&reqs, "http://c.test/x").id,
            ResourceResponse::ok("text/plain", b"x").with_header("Access-Control-Allow-Origin", "*"),
        );
        let log = d.console();
        assert!(log.contains(&"blocked1".to_owned()), "{log:?}");
        assert!(log.contains(&"ok2".to_owned()), "{log:?}");
    }

    #[test]
    fn xhr_from_script_goes_through_the_same_queue() {
        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<script>var x=new XMLHttpRequest();x.open('GET','/data');x.onload=()=>console.log('xhr '+x.responseText);x.send()</script>");
        d.finish_body();
        let reqs = d.take_fetches();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].url, "http://a.test/data");
        d.complete(reqs[0].id, ResourceResponse::ok("text/plain", b"ok"));
        assert_eq!(d.console(), vec!["xhr ok"]);
    }

    #[test]
    fn script_added_image_is_discovered() {
        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<script>var i=document.createElement('img');i.setAttribute('src','/late.png');document.body.appendChild(i)</script>");
        d.finish_body();
        let reqs = d.take_fetches();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].url, "http://a.test/late.png");
        assert_eq!(reqs[0].kind, ResourceKind::Image);
    }

    #[test]
    fn timers_run_and_their_requests_are_queued() {
        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<script>setTimeout(()=>{console.log('tick');fetch('/t')},100)</script>");
        d.finish_body();
        assert!(d.take_fetches().is_empty());
        d.run_timers(10.0);
        assert!(d.console().is_empty());
        d.run_timers(500.0);
        assert_eq!(d.console(), vec!["tick"]);
        assert_eq!(d.take_fetches().len(), 1);
    }

    #[test]
    fn dirty_flags_follow_dom_and_sheets() {
        let f = Fake::default().add("http://a.test/p/s.css", "text/css", b"p{color:red}");
        let mut d = open("<link rel=stylesheet href=\"s.css\"><p id=x>hi</p>", &f);
        assert!(d.needs_restyle());
        assert!(d.needs_layout());
        assert!(d.restyle());
        assert!(!d.needs_restyle());
        assert!(!d.restyle());
        assert!(d.needs_layout());
        d.mark_laid_out();
        assert!(!d.needs_layout());
        let p = d.dom().borrow().get_element_by_id("x").unwrap();
        d.dom().borrow_mut().set_attr(p, "class", "y");
        assert!(d.needs_restyle());
        assert!(d.needs_layout());
        d.restyle();
        assert!(!d.needs_restyle());
        assert!(d.needs_layout());
        d.mark_laid_out();
        d.set_env(MediaEnv { width: 800.0, ..env() });
        assert!(d.needs_restyle());
        assert!(d.needs_layout());
    }

    #[test]
    fn byte_cap_drops_late_responses() {
        let limits = Limits { max_total_bytes: 30, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<img src=\"1.png\"><img src=\"2.png\">");
        d.finish_body();
        for r in d.take_fetches() {
            d.complete(r.id, ResourceResponse::ok("image/png", &png(5, 5)));
        }
        assert_eq!(d.images.len(), 1);
        assert_eq!(d.stats().bytes, 24);
        assert!(d.errors().iter().any(|e| e.contains("byte cap")));
    }

    #[test]
    fn request_count_cap() {
        let limits = Limits { max_resources: 2, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<img src=\"1.png\"><img src=\"2.png\"><img src=\"3.png\">");
        d.finish_body();
        assert_eq!(d.take_fetches().len(), 2);
    }

    #[test]
    fn script_cap_limits_execution() {
        let limits = Limits { max_scripts: 1, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<script>console.log(1)</script><script>console.log(2)</script>");
        d.finish_body();
        assert_eq!(d.console(), vec!["1"]);
    }

    #[test]
    fn scripting_can_be_turned_off() {
        let limits = Limits { scripting: false, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<script src=\"/a.js\"></script><script>console.log(1)</script>");
        d.finish_body();
        assert!(d.js().is_none());
        assert!(d.take_fetches().is_empty());
        assert_eq!(d.fired(), &["DOMContentLoaded", "load"]);
    }

    #[test]
    fn base_href_and_picture_and_srcset() {
        let mut d = Document::new("http://a.test/p/index.html", env());
        d.feed(
            b"<base href=\"http://cdn.test/x/\"><img src=\"i.png\">\
              <picture><source media=\"(min-width: 5000px)\" srcset=\"big.png\"><source srcset=\"small.png 400w, mid.png 900w\"><img src=\"fb.png\"></picture>\
              <img src=\"fallback.png\" srcset=\"s1.png 1x, s2.png 2x\">",
        );
        d.finish_body();
        assert_eq!(d.base_url(), "http://cdn.test/x/");
        let urls: Vec<String> = d.take_fetches().into_iter().map(|r| r.url).collect();
        assert_eq!(urls, vec!["http://cdn.test/x/i.png", "http://cdn.test/x/mid.png", "http://cdn.test/x/s2.png"]);
    }

    #[test]
    fn meta_refresh_is_parsed() {
        let mut d = Document::new("http://a.test/p/", env());
        d.feed(b"<meta http-equiv=\"refresh\" content=\"5; url=/next\">");
        d.finish_body();
        assert_eq!(d.refresh(), Some((5.0, "http://a.test/next".to_owned())));
        let mut d = Document::new("http://a.test/p/", env());
        d.feed(b"<meta http-equiv=\"refresh\" content=\"30\">");
        d.finish_body();
        assert_eq!(d.refresh(), Some((30.0, "http://a.test/p/".to_owned())));
    }

    #[test]
    fn cookies_flow_both_ways() {
        let mut d = Document::new("http://a.test/", env());
        d.set_response_headers(&[("Set-Cookie".into(), "sid=abc; Path=/".into()), ("Set-Cookie".into(), "h=1; HttpOnly".into())]);
        d.feed(b"<script>console.log('c:'+document.cookie);document.cookie='js=1';fetch('/api')</script><img src=\"/i.png\">");
        d.finish_body();
        assert_eq!(d.console(), vec!["c:sid=abc"]);
        let reqs = d.take_fetches();
        let img = request_for(&reqs, "http://a.test/i.png");
        assert!(img.headers.iter().any(|(k, v)| k == "Cookie" && v.contains("sid=abc") && v.contains("h=1")));
        let api = request_for(&reqs, "http://a.test/api");
        assert!(api.headers.iter().any(|(k, v)| k == "Cookie" && v.contains("js=1")), "{:?}", api.headers);
        assert_eq!(d.document_cookie(), "sid=abc; js=1");
        // a Set-Cookie on a response lands in the jar
        d.complete(api.id, ResourceResponse::ok("text/plain", b"").with_header("Set-Cookie", "later=2; Path=/"));
        assert!(d.cookies().header_for("http://a.test/").unwrap().contains("later=2"));
    }

    #[test]
    fn module_graph_with_import_map() {
        let f = Fake::default()
            .add("http://a.test/vendor/lib.js", "text/javascript", b"import {k} from './k.js'; export const x = 5 + k; console.log('lib');")
            .add("http://a.test/vendor/k.js", "text/javascript", b"export const k = 1; console.log('k');");
        let d = open(
            "<script type=\"importmap\">{\"imports\":{\"lib\":\"/vendor/lib.js\"}}</script>\
             <script type=\"module\">import {x} from 'lib'; console.log('main ' + x)</script>\
             <script>console.log('classic')</script>",
            &f,
        );
        // classic first (modules are deferred), then dependencies before importers
        assert_eq!(d.console(), vec!["classic", "k", "lib", "main 6"], "{:?}", d.errors());
        assert_eq!(d.fired(), &["DOMContentLoaded", "load"]);
    }

    #[test]
    fn module_default_export_and_alias() {
        let f = Fake::default().add("http://a.test/m.js", "text/javascript", b"export default 41; export const a = 1;");
        let d = open("<script type=\"module\">import v, {a as b} from '/m.js'; console.log(v + b)</script>", &f);
        assert_eq!(d.console(), vec!["42"], "{:?}", d.errors());
    }

    #[test]
    fn module_import_depth_is_limited() {
        let f = Fake::default()
            .add("http://a.test/1.js", "text/javascript", b"import './2.js';")
            .add("http://a.test/2.js", "text/javascript", b"import './3.js';")
            .add("http://a.test/3.js", "text/javascript", b"console.log('3')");
        let limits = Limits { max_module_depth: 1, ..Limits::default() };
        let mut d = Document::with_limits("http://a.test/", env(), limits);
        d.feed(b"<script type=\"module\" src=\"/1.js\"></script>");
        d.finish_body();
        let order = settle(&mut d, &f);
        assert_eq!(order, vec!["http://a.test/1.js", "http://a.test/2.js"]);
        assert_eq!(d.phase(), Phase::Complete);
    }

    #[test]
    fn json_and_other_script_types_are_ignored() {
        let mut d = Document::new("http://a.test/", env());
        d.feed(b"<script type=\"application/json\">{\"a\":1}</script><script type=\"text/template\">x</script>");
        d.finish_body();
        assert!(d.js().is_none());
        assert_eq!(d.stats().scripts_run, 0);
    }

    #[test]
    fn script_errors_are_collected_and_later_scripts_still_run() {
        let f = Fake::default();
        let d = open("<script>throw new Error('boom')</script><script>console.log('next')</script>", &f);
        assert_eq!(d.console(), vec!["next"]);
        assert_eq!(d.errors().len(), 1);
        assert!(d.errors()[0].contains("boom"), "{:?}", d.errors());
    }
}

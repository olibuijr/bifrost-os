//! Downloads (Browser): a main-frame navigation whose response is not displayable (Content-Disposition attachment,
//! or a MIME type the browser cannot show) is saved to `~/Downloads` and listed in a popover from a toolbar button.
//!
//! Servo 0.7.0 has no download path: `WebViewDelegate::load_web_resource` can only replace a response with one the
//! embedder builds, it never hands over the real body, and `request_navigation` does not carry response headers
//! (read in servo-0.7.0 `components/servo/webview_delegate.rs`). So this is app-side: when a view starts loading an
//! http(s) URL, `curl -I` runs on a thread; if the headers say "download", the view is stopped and a second curl
//! writes `NAME.download`, renamed when it finishes. The probe and fetch use a privileged native cookie snapshot
//! (read after each load, HttpOnly included, restricted to the exact origin), the previous page as Referer and the browser's
//! User-Agent. A failed or 405/501 HEAD is retried as `GET Range: bytes=0-0`. URLs with no query and an HTML-like or
//! missing extension are not probed (ordinary navigations).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use bifrost_app::dispatch::DispatchQueue;

use crate::inbox::{self, Slot};

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Golden Gate 10_15_7) WebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Browser/605.1.15";
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static WAKER: AtomicBool = AtomicBool::new(false);

/// Request context for the probe and the fetch.
#[derive(Clone, Default)]
struct Ctx {
    cookie: String,
    referer: String,
    ua: String,
}

impl Ctx {
    fn args(&self) -> Vec<String> {
        let mut a = Vec::new();
        // curl strips explicit Cookie headers on cross-origin redirects; raw -b strings have no domain metadata.
        if !self.cookie.is_empty() {
            a.extend(["-H".to_string(), format!("Cookie: {}", self.cookie)]);
        }
        for (flag, v) in [("-e", &self.referer), ("-A", &self.ua)] {
            if !v.is_empty() {
                a.push(flag.to_string());
                a.push(v.clone());
            }
        }
        a
    }
}

/// An ordinary navigation: no query and an HTML-like or missing extension. Attachments usually carry a query or a file extension.
fn skip_probe(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |x| x.1);
    let Some((_, path)) = rest.split_once('/') else { return true };
    if path.contains('?') {
        return false;
    }
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        None => true,
        Some((_, e)) => matches!(e.to_ascii_lowercase().as_str(), "html" | "htm" | "xhtml" | "php" | "asp" | "aspx" | "jsp" | "shtml"),
    }
}

use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::Rect;
use bifrost_app::glyphs;
use bifrost_app::kit::ui::{Weight, CAPSULE_SHADOW, CARD_BG, DESTRUCTIVE, LABEL, LABEL_2, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, ROW_PRESSED, SEPARATOR_C, accent_now};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::webview::WebViewId;

/// Row glyph buttons and the "Clear" text follow the user's accent.
fn button_color() -> u32 {
    accent_now()
}
use bifrost_core::json::Json;

use crate::control::{num, obj, text};

pub const POPOVER_W_PT: f32 = 320.0;
const ROW_PT: f32 = 52.0;
const MAX_ROWS: usize = 6;
const PAD_PT: f32 = 12.0;
const BTN_PT: f32 = 24.0;
const ICON_PT: f32 = 28.0;
const ARROW_PT: f32 = 7.0;
const FOOT_PT: f32 = 28.0;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Active,
    Done,
    Failed(String),
    Cancelled,
}

pub struct Item {
    pub id: u32,
    pub url: String,
    pub name: String,
    pub path: PathBuf,
    part: PathBuf,
    pub total: Option<u64>,
    pub received: u64,
    pub state: State,
    child: Option<Child>,
}

/// What a click in the popover asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Cancel(u32),
    Reveal(u32),
    Open(u32),
    Clear,
    Inside,
}

struct Probe {
    url: String,
    /// `Some(file name)` when the response should be downloaded.
    download: Option<String>,
    total: Option<u64>,
}

#[derive(Default)]
pub struct Downloads {
    pub items: Vec<Item>,
    pub open: bool,
    next: u32,
    last: HashMap<WebViewId, String>,
    probes: Vec<(WebViewId, Slot<Probe>)>,
    cookies: HashMap<WebViewId, String>,
    pub ua: String,
    ctx: HashMap<String, Ctx>,
}

pub fn dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join("Downloads")
}

/// Drops the fragment, which never changes the response.
fn base_url(u: &str) -> &str {
    u.split('#').next().unwrap_or(u)
}

/// Browser shows text, images, PDF, audio/video and structured text; everything else is saved.
pub fn displayable(mime: &str) -> bool {
    let m = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    m.is_empty()
        || m.starts_with("text/")
        || m.starts_with("image/")
        || m.starts_with("audio/")
        || m.starts_with("video/")
        || m.ends_with("+xml")
        || m.ends_with("+json")
        || matches!(m.as_str(), "application/xml" | "application/json" | "application/pdf" | "application/javascript" | "application/x-javascript" | "application/ecmascript")
}

/// Parses the last response block of `curl -I -L -D -` output: (status, headers with lowercase names).
fn last_block(raw: &str) -> (u32, Vec<(String, String)>) {
    let all = raw.replace("\r\n", "\n");
    let block = all.split("\n\n").filter(|b| b.trim_start().starts_with("HTTP/")).last().unwrap_or("");
    let mut lines = block.trim().lines();
    let status = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let hs = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    (status, hs)
}

fn header<'a>(hs: &'a [(String, String)], k: &str) -> Option<&'a str> {
    hs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

/// `filename*=UTF-8''x%20y` or `filename="x"` from a Content-Disposition value.
pub fn disposition_name(cd: &str) -> Option<String> {
    let lower = cd.to_ascii_lowercase();
    if let Some(i) = lower.find("filename*=") {
        let v = cd[i + 10..].split(';').next()?.trim().trim_matches('"');
        let v = v.split_once("''").map_or(v, |(_, r)| r);
        let b = v.as_bytes();
        let mut out = Vec::new();
        let mut k = 0;
        while k < b.len() {
            if b[k] == b'%' && k + 2 < b.len() {
                if let Some(x) = v.get(k + 1..k + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(x);
                    k += 3;
                    continue;
                }
            }
            out.push(b[k]);
            k += 1;
        }
        return Some(String::from_utf8_lossy(&out).into_owned());
    }
    let i = lower.find("filename=")?;
    Some(cd[i + 9..].split(';').next()?.trim().trim_matches('"').to_string())
}

/// A safe single path component: no separators, no leading dots.
pub fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let s: String = s.trim().trim_start_matches('.').chars().take(200).collect();
    if s.is_empty() {
        "download".into()
    } else {
        s
    }
}

fn url_name(url: &str) -> String {
    let path = base_url(url).split('?').next().unwrap_or("");
    sanitize(path.rsplit('/').next().unwrap_or(""))
}

/// `name.ext`, then `name 2.ext`, `name 3.ext` ... (Browser), skipping names taken by a file or a partial download.
pub fn unique_in(dir: &Path, name: &str) -> String {
    let taken = |n: &str| dir.join(n).exists() || dir.join(format!("{n}.download")).exists();
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..10_000).map(|n| format!("{stem} {n}{ext}")).find(|c| !taken(c)).unwrap_or_else(|| format!("{stem} {}{ext}", std::process::id()))
}

fn probe_once(url: &str, cx: &Ctx, ranged: bool) -> (u32, Vec<(String, String)>, String) {
    let mut c = Command::new("curl");
    c.args(["-s", "-L", "-m", "15", "--max-redirs", "10", "-D", "-", "-o", "/dev/null", "-w", "\n@@%{url_effective}"]);
    if ranged {
        c.args(["-r", "0-0"]);
    } else {
        c.arg("-I");
    }
    let out = c.args(cx.args()).arg(url).stdin(Stdio::null()).output();
    let Ok(out) = out else { return (0, Vec::new(), url.to_string()) };
    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let (head, effective) = raw.rsplit_once("\n@@").unwrap_or((&raw, url));
    let (status, hs) = last_block(head);
    (status, hs, effective.to_string())
}

fn probe(url: &str, cx: &Ctx) -> Probe {
    let none = Probe { url: url.to_string(), download: None, total: None };
    let (mut status, mut hs, mut effective) = probe_once(url, cx, false);
    let mut ranged = false;
    if status == 0 || status == 405 || status == 501 {
        (status, hs, effective) = probe_once(url, cx, true);
        ranged = true;
    }
    // a server that still errors is left to the page; Servo shows the error itself
    if !(200..300).contains(&status) {
        return none;
    }
    let effective = effective.as_str();
    let cd = header(&hs, "content-disposition");
    let attachment = cd.is_some_and(|c| c.trim_start().to_ascii_lowercase().starts_with("attachment"));
    if !attachment && displayable(header(&hs, "content-type").unwrap_or("")) {
        return none;
    }
    let name = cd.and_then(disposition_name).map(|n| sanitize(&n)).unwrap_or_else(|| url_name(effective.trim()));
    let total = if ranged {
        header(&hs, "content-range").and_then(|v| v.rsplit('/').next()).and_then(|v| v.trim().parse().ok())
    } else {
        header(&hs, "content-length").and_then(|v| v.parse().ok())
    };
    Probe { url: url.to_string(), download: Some(name), total }
}

/// Redraws while a download runs: one wake every half second on the main queue, until nothing is active (or after an
/// hour). A download that starts as this stops finds the flag still set, so it restarts the chain.
fn progress_tick(n: u32) {
    if n >= 7200 || ACTIVE.load(Ordering::SeqCst) == 0 {
        WAKER.store(false, Ordering::SeqCst);
        if ACTIVE.load(Ordering::SeqCst) > 0 && !WAKER.swap(true, Ordering::SeqCst) {
            progress_tick(0);
        }
        return;
    }
    inbox::wake();
    DispatchQueue::main().async_after(Duration::from_millis(500), move || progress_tick(n + 1));
}

fn size_text(n: u64) -> String {
    let f = n as f64;
    if n >= 1 << 30 {
        format!("{:.1} GB", f / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MB", f / (1u64 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.0} KB", f / 1024.0)
    } else {
        format!("{n} bytes")
    }
}

impl Item {
    /// A finished or running item without a child process, for the tests of the system glue (`system.rs`).
    #[cfg(test)]
    pub fn sample(path: &str, state: State, total: Option<u64>, received: u64) -> Item {
        Item { id: 0, url: String::new(), name: String::new(), path: PathBuf::from(path), part: PathBuf::new(), total, received, state, child: None }
    }

    pub fn status(&self) -> String {
        match &self.state {
            State::Active => match self.total {
                Some(t) => format!("{} of {}", size_text(self.received), size_text(t)),
                None => size_text(self.received),
            },
            State::Done => size_text(self.received),
            State::Failed(e) => format!("Failed: {e}"),
            State::Cancelled => "Cancelled".into(),
        }
    }

    pub fn fraction(&self) -> f32 {
        match (self.total, &self.state) {
            (_, State::Done) => 1.0,
            (Some(t), _) if t > 0 => (self.received as f32 / t as f32).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

impl Downloads {
    /// A view started loading `url`: probe it once (fragment-only changes do not count).
    pub fn consider(&mut self, view: WebViewId, url: &str) {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let base = base_url(url).to_string();
        if self.last.get(&view) == Some(&base) {
            return;
        }
        let referer = self.last.insert(view, base.clone()).unwrap_or_default();
        if skip_probe(&base) {
            return;
        }
        // the cookies were read from the page being left (the referer): send them only to that site
        let cookie = if same_site(&base, &referer) { self.cookies.get(&view).cloned().unwrap_or_default() } else { String::new() };
        let cx = Ctx { cookie, referer, ua: self.ua.clone() };
        self.ctx.insert(base.clone(), cx.clone());
        let slot = inbox::spawn(move || probe(&base, &cx));
        self.probes.push((view, slot));
    }

    /// A privileged native cookie snapshot; it is never presented as a JavaScript eval result.
    pub fn on_cookies(&mut self, view: WebViewId, cookies: &[bifrost_app::web_data::Cookie]) {
        let header = cookies.iter().map(|cookie| format!("{}={}", cookie.name, cookie.value)).collect::<Vec<_>>().join("; ");
        self.cookies.insert(view, header);
    }

    pub fn forget(&mut self, view: WebViewId) {
        self.last.remove(&view);
        self.cookies.remove(&view);
    }

    /// Starts the download of `url` as `name`; false when `~/Downloads` or curl is unavailable.
    pub fn start(&mut self, url: &str, name: &str, total: Option<u64>) -> bool {
        let d = dir();
        if std::fs::create_dir_all(&d).is_err() {
            return false;
        }
        let name = unique_in(&d, &sanitize(name));
        let (path, part) = (d.join(&name), d.join(format!("{name}.download")));
        let child = Command::new("curl")
            .args(["-sS", "-f", "-L", "--max-redirs", "10"])
            .args(self.ctx.get(url).map(|c| c.args()).unwrap_or_default())
            .arg("-o")
            .arg(&part)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(child) = child else { return false };
        self.next += 1;
        self.items.insert(0, Item { id: self.next, url: url.into(), name, path, part, total, received: 0, state: State::Active, child: Some(child) });
        self.open = true;
        // wake the idle loop while a download runs so progress redraws; one thread, gone when none is active
        self.sync_active();
        if !WAKER.swap(true, Ordering::SeqCst) {
            progress_tick(0);
        }
        true
    }

    /// Collects probe results and progress. Returns (changed, views whose load must be stopped).
    pub fn tick(&mut self) -> (bool, Vec<WebViewId>) {
        let mut changed = false;
        let mut stop = Vec::new();
        let mut ready = Vec::new();
        self.probes.retain(|(v, slot)| match inbox::take(slot) {
            Some(p) => {
                ready.push((*v, p));
                false
            }
            None => true,
        });
        for (v, p) in ready {
            if let Some(name) = p.download {
                if self.start(&p.url, &name, p.total) {
                    stop.push(v);
                    changed = true;
                }
            }
        }
        for it in &mut self.items {
            if it.state != State::Active {
                continue;
            }
            let size = std::fs::metadata(&it.part).map(|m| m.len()).unwrap_or(0);
            if size != it.received {
                it.received = size;
                changed = true;
            }
            let Some(c) = it.child.as_mut() else { continue };
            match c.try_wait() {
                Ok(Some(status)) => {
                    it.child = None;
                    changed = true;
                    if status.success() && std::fs::rename(&it.part, &it.path).is_ok() {
                        it.received = std::fs::metadata(&it.path).map(|m| m.len()).unwrap_or(it.received);
                        it.state = State::Done;
                    } else {
                        let _ = std::fs::remove_file(&it.part);
                        it.state = State::Failed(format!("curl exit {}", status.code().unwrap_or(-1)));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    it.child = None;
                    it.state = State::Failed(e.to_string());
                    changed = true;
                }
            }
        }
        self.sync_active();
        (changed, stop)
    }

    fn sync_active(&self) {
        ACTIVE.store(self.items.iter().filter(|i| i.state == State::Active).count(), Ordering::SeqCst);
    }

    pub fn cancel(&mut self, id: u32) -> bool {
        let Some(it) = self.items.iter_mut().find(|i| i.id == id && i.state == State::Active) else { return false };
        if let Some(mut c) = it.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&it.part);
        it.state = State::Cancelled;
        self.sync_active();
        true
    }

    /// Removes finished entries from the list (never the files).
    pub fn clear_finished(&mut self) {
        self.items.retain(|i| i.state == State::Active);
        if self.items.is_empty() {
            self.open = false;
        }
    }

    fn spawn_open(arg: &Path) {
        let _ = Command::new("xdg-open").arg(arg).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }

    /// Opens the finished file with the desktop's handler.
    pub fn open_file(&self, id: u32) {
        if let Some(it) = self.items.iter().find(|i| i.id == id && i.state == State::Done) {
            Self::spawn_open(&it.path);
        }
    }

    /// Shows the containing folder (the portable form of Reveal in Files).
    pub fn reveal(&self, id: u32) {
        if let Some(it) = self.items.iter().find(|i| i.id == id) {
            Self::spawn_open(it.path.parent().unwrap_or(&dir()));
        }
    }

    pub fn json(&self) -> Json {
        Json::Arr(
            self.items
                .iter()
                .map(|i| {
                    let st = match &i.state {
                        State::Active => "active".to_string(),
                        State::Done => "done".into(),
                        State::Cancelled => "cancelled".into(),
                        State::Failed(e) => format!("failed: {e}"),
                    };
                    obj(vec![
                        ("id", num(i.id as f64)),
                        ("name", text(&i.name)),
                        ("url", text(&i.url)),
                        ("path", text(&i.path.to_string_lossy())),
                        ("state", text(&st)),
                        ("received", num(i.received as f64)),
                        ("total", i.total.map_or(Json::Null, |t| num(t as f64))),
                    ])
                })
                .collect(),
        )
    }

    // ---- popover ----

    fn rect(&self, wpt: f32) -> Rect {
        let rows = self.items.len().min(MAX_ROWS) as f32;
        Rect { x: wpt - POPOVER_W_PT - 8.0, y: UNIFIED_BAR_PT + ARROW_PT, w: POPOVER_W_PT, h: rows * ROW_PT + FOOT_PT }
    }

    fn row_rect(&self, wpt: f32, i: usize) -> Rect {
        let r = self.rect(wpt);
        Rect { x: r.x, y: r.y + i as f32 * ROW_PT, w: r.w, h: ROW_PT }
    }

    fn button_rect(&self, wpt: f32, i: usize) -> Rect {
        let r = self.row_rect(wpt, i);
        Rect { x: r.x + r.w - PAD_PT - BTN_PT, y: r.y + (ROW_PT - BTN_PT) / 2.0, w: BTN_PT, h: BTN_PT }
    }

    fn clear_rect(&self, wpt: f32) -> Rect {
        let r = self.rect(wpt);
        Rect { x: r.x, y: r.y + r.h - FOOT_PT, w: r.w, h: FOOT_PT }
    }

    /// Hit test in points; `None` outside the popover (the caller closes it).
    pub fn hit(&self, wpt: f32, px: f32, py: f32) -> Option<Action> {
        if !self.open || self.items.is_empty() || !self.rect(wpt).contains(px, py) {
            return None;
        }
        if self.clear_rect(wpt).contains(px, py) {
            return Some(Action::Clear);
        }
        for (i, it) in self.items.iter().take(MAX_ROWS).enumerate() {
            if self.button_rect(wpt, i).contains(px, py) {
                return Some(if it.state == State::Active { Action::Cancel(it.id) } else { Action::Reveal(it.id) });
            }
            if self.row_rect(wpt, i).contains(px, py) {
                return Some(if it.state == State::Done { Action::Open(it.id) } else { Action::Inside });
            }
        }
        Some(Action::Inside)
    }

    /// Draws the popover under the toolbar button (`anchor_x`, points) with a shadow and an arrow (Browser).
    pub fn draw(&self, s: &mut Surface, ui: &Ui, wpt: f32, anchor_x: Option<f32>) {
        if !self.open || self.items.is_empty() {
            return;
        }
        let r = self.rect(wpt);
        let rad = ui.pt(m::CARD_RADIUS);
        for (grow, off) in [(10.0, 4.0), (5.0, 2.0)] {
            s.fill_rrect(ui.pt(r.x - grow), ui.pt(r.y - grow + off), ui.pt(r.w + 2.0 * grow), ui.pt(r.h + 2.0 * grow), rad + ui.pt(grow), CAPSULE_SHADOW.now());
        }
        s.fill_rrect(ui.pt(r.x) - 1.0, ui.pt(r.y) - 1.0, ui.pt(r.w) + 2.0, ui.pt(r.h) + 2.0, rad, SEPARATOR_C.now());
        s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), rad, CARD_BG.now());
        // arrow pointing at the toolbar button: stacked rows, widest at the popover edge
        let ax = anchor_x.unwrap_or(r.x + r.w - 40.0).clamp(r.x + 20.0, r.x + r.w - 20.0);
        for k in 0..ARROW_PT as usize {
            let half = ARROW_PT - k as f32;
            let y = ui.pt(r.y) - ui.pt(ARROW_PT) + ui.pt(k as f32 + 1.0) - 1.0;
            s.fill_rect(ui.pt(ax - half) as usize, y.max(0.0) as usize, ui.pt(half * 2.0) as usize, ui.pt(1.0).ceil() as usize, CARD_BG.now());
        }
        let shown = self.items.len().min(MAX_ROWS);
        for (i, it) in self.items.iter().take(MAX_ROWS).enumerate() {
            let row = self.row_rect(wpt, i);
            let icon = Rect { x: row.x + PAD_PT, y: row.y + (ROW_PT - ICON_PT) / 2.0, w: ICON_PT, h: ICON_PT };
            glyphs::glyph(s, ui, "doc", icon, LABEL_2.now());
            let tx = row.x + PAD_PT + ICON_PT + 10.0;
            let room = ui.pt(row.x + row.w - PAD_PT - BTN_PT - 8.0 - tx);
            let title = ui.text(&ui.fit(&it.name, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, Weight::Regular, LABEL.now());
            s.blit(&title, ui.pt(tx) as i32, ui.pt(row.y + 9.0) as i32, 255, 255);
            let color = if matches!(it.state, State::Failed(_)) { DESTRUCTIVE } else { LABEL_2.now() };
            let status = ui.text(&ui.fit(&it.status(), MAC_FOOTNOTE_PT, room), MAC_FOOTNOTE_PT, Weight::Regular, color);
            s.blit(&status, ui.pt(tx) as i32, ui.pt(row.y + 27.0) as i32, 255, 255);
            if it.state == State::Active {
                let (x, y, w) = (ui.pt(tx), ui.pt(row.y + 43.0), room);
                let h = ui.pt(3.0).max(1.0);
                s.fill_rrect(x, y, w, h, h / 2.0, ROW_PRESSED.now());
                s.fill_rrect(x, y, (w * it.fraction()).max(h), h, h / 2.0, accent_now());
            }
            let glyph = if it.state == State::Active { "xmark" } else { "magnifyingglass" };
            glyphs::glyph(s, ui, glyph, self.button_rect(wpt, i), button_color());
            if i + 1 < shown {
                s.fill_rect(ui.pt(tx) as usize, ui.pt(row.y + row.h) as usize, ui.pt(row.x + row.w - PAD_PT - tx) as usize, 1, SEPARATOR_C.now());
            }
        }
        let c = self.clear_rect(wpt);
        s.fill_rect(ui.pt(c.x) as usize, ui.pt(c.y) as usize, ui.pt(c.w) as usize, 1, SEPARATOR_C.now());
        let img = ui.text("Clear", MAC_SUBHEAD_PT, Weight::Regular, button_color());
        s.blit(&img, ui.pt(c.x + c.w / 2.0) as i32 - img.w as i32 / 2, ui.pt(c.y + c.h / 2.0) as i32 - img.h as i32 / 2, 255, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::appearance::{accent, set_accent, Accent};
    use bifrost_app::kit::ui::TINT;

    /// The accent is process-wide: this test changes it and restores it.
    static ACCENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn row_buttons_follow_the_chosen_accent_not_system_blue() {
        let _g = ACCENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep = accent();
        set_accent(Accent::Green);
        let (c, green) = (button_color(), accent_now());
        set_accent(keep);
        assert_eq!(c, green);
        assert_ne!(c, TINT);
    }

    #[test]
    fn displayable_types() {
        assert!(displayable("text/html; charset=utf-8"));
        assert!(displayable("image/png"));
        assert!(displayable("application/pdf"));
        assert!(displayable("application/ld+json"));
        assert!(!displayable("application/zip"));
        assert!(!displayable("application/octet-stream"));
    }

    #[test]
    fn disposition_names() {
        assert_eq!(disposition_name("attachment; filename=\"a b.zip\"").as_deref(), Some("a b.zip"));
        assert_eq!(disposition_name("attachment; filename*=UTF-8''a%20b.zip").as_deref(), Some("a b.zip"));
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize(""), "download");
    }

    #[test]
    fn last_header_block_wins() {
        let raw = "HTTP/1.1 302 Found\r\nLocation: /x\r\n\r\nHTTP/2 200\r\nContent-Type: application/zip\r\n\r\n";
        let (st, hs) = last_block(raw);
        assert_eq!(st, 200);
        assert_eq!(header(&hs, "content-type"), Some("application/zip"));
    }

    #[test]
    fn probe_is_skipped_for_plain_pages_only() {
        assert!(skip_probe("https://example.com"));
        assert!(skip_probe("https://example.com/"));
        assert!(skip_probe("https://example.com/docs/page.HTML"));
        assert!(skip_probe("https://example.com/docs/intro"));
        assert!(!skip_probe("https://example.com/a.zip"));
        assert!(!skip_probe("https://example.com/page.html?download=1"));
        assert!(!skip_probe("https://example.com/get?id=3"));
    }

    #[test]
    fn hosts_and_same_site_cookie_rule() {
        assert!(same_site("https://example.com/a", "https://example.com/b"));
        assert!(!same_site("https://cdn.example.com/f.zip", "https://example.com/"));
        assert!(!same_site("https://example.com/f.zip", "https://www.example.com/"));
        assert!(!same_site("http://example.com/f.zip", "https://example.com/"));
        assert!(!same_site("https://example.com:8443/f.zip", "https://example.com/"));
        assert!(!same_site("https://evilexample.com/f.zip", "https://example.com/"));
        assert!(!same_site("https://example.com/f.zip", ""));
        assert!(!same_site("", "https://example.com/"));
    }



    #[test]
    fn unique_names_count_up() {
        let d = std::env::temp_dir().join(format!("bf-dl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        std::fs::write(d.join("a.zip"), b"x").unwrap();
        assert_eq!(unique_in(&d, "a.zip"), "a 2.zip");
        std::fs::write(d.join("a 2.zip.download"), b"x").unwrap();
        assert_eq!(unique_in(&d, "a.zip"), "a 3.zip");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// The fallback cookie snapshot may only be sent to the same origin; it has no cookie-domain metadata.
fn same_site(a: &str, b: &str) -> bool {
    let (a, b) = (crate::session::origin(a), crate::session::origin(b));
    !a.is_empty() && a == b
}

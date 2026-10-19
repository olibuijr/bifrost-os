//! Session restore, reopen-closed-tab and per-site page zoom (Browser: "Reopen All Windows from Last Session",
//! Undo Close Tab, `WebView.pageZoom`). The file is `~/.local/share/bifrost-browser/session.txt`, one record per
//! line (`v N`, `sel N`, `tab URL<TAB>TITLE`, `zoom ORIGIN FACTOR`), written to a temp file, fsynced and renamed into
//! place; the previous file is kept as `session.txt.bak`, an unreadable one is moved to `session.txt.corrupt`.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use bifrost_app::webview::{ZOOM_MAX, ZOOM_MIN};

use crate::{home, Browser, START};

#[path = "recover.rs"]
pub mod recover;

/// Zoom step (Browser: 25%).
pub const ZOOM_STEP: f32 = 0.25;
/// Closed tabs kept for Reopen Closed Tab (the last 20).
pub const CLOSED_MAX: usize = 20;
/// Seconds a change waits before it is written.
const DEBOUNCE_S: f64 = 1.0;

/// A tab that was closed, with what Reopen Closed Tab needs to bring it back.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedTab {
    /// The page it showed (`START` for a tab that never loaded one).
    pub url: String,
    pub title: String,
    /// The tab's Back list up to and including the page it showed (`Discarder::entries`); empty when it kept none. Forward
    /// entries are not kept: the engine cannot replay them in a new view.
    pub history: Vec<String>,
    /// Its place in the tab strip when it was closed.
    pub at: usize,
}

/// The tabs closed in this run, newest last. Every way of closing a tab (the X button, Cmd+W, the Close Tab menu items, a
/// page closing itself, the control socket) pushes here; Cmd+Shift+T and the History menu pop from it.
#[derive(Default)]
pub struct ClosedTabs {
    stack: Vec<ClosedTab>,
}

impl ClosedTabs {
    /// Pushes a closed tab; the oldest goes when more than `CLOSED_MAX` are kept.
    pub fn push(&mut self, tab: ClosedTab) {
        self.stack.push(tab);
        if self.stack.len() > CLOSED_MAX {
            self.stack.remove(0);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// The closed tabs, the one closed last first.
    pub fn newest(&self) -> impl Iterator<Item = &ClosedTab> {
        self.stack.iter().rev()
    }

    /// Removes and returns the `n`-th newest closed tab (0 = the one closed last).
    pub fn take(&mut self, n: usize) -> Option<ClosedTab> {
        let i = self.stack.len().checked_sub(n + 1)?;
        Some(self.stack.remove(i))
    }
}

/// The page and title a closing tab is remembered by: the engine's state when it has a URL, else the last entry of the tab's
/// Back list, else `START` (a tab that never loaded anything).
fn closed_page(state: Option<(String, String)>, history: &[String]) -> (String, String) {
    let (url, title) = state.unwrap_or_default();
    if url.is_empty() {
        (history.last().cloned().unwrap_or_else(|| START.to_string()), title)
    } else {
        (url, title)
    }
}

#[derive(Default)]
pub struct Session {
    pub zoom: HashMap<String, f32>,
    pub closed: ClosedTabs,
    last: Vec<(String, String)>,
    last_sel: usize,
    last_zoom: HashMap<String, f32>,
    changed_at: Option<f64>,
    saved_once: bool,
    /// True when tabs, URLs, titles or selection changed since the last session snapshot.
    pub dirty: bool,
}

fn path() -> PathBuf {
    home().join(".local/share/bifrost-browser/session.txt")
}

/// `scheme://host[:port]` of a URL, or empty for pages without one.
pub fn origin(url: &str) -> String {
    url::Url::parse(url).ok().map(|url| url.origin()).filter(|origin| origin.is_tuple())
        .map(|origin| origin.ascii_serialization()).unwrap_or_default()
}

/// Format version written as the first line (`v N`). A file with a newer version is ignored (and backed up on the next
/// write) rather than half-understood; a file without one is the version-0 format and still loads.
pub const FORMAT: u32 = 1;
/// Tabs read from a session file; a damaged file cannot open thousands of tabs.
pub const MAX_TABS: usize = 200;
/// Longest accepted URL or title in a record.
const MAX_FIELD: usize = 8192;

/// What a session file holds.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub tabs: Vec<(String, String)>,
    pub sel: usize,
    pub zoom: HashMap<String, f32>,
    /// False for text that is not a session file (binary, newer version).
    pub valid: bool,
}

/// Parses session text; never panics, skips damaged lines.
pub fn parse(text: &str) -> Parsed {
    let mut p = Parsed { valid: true, ..Parsed::default() };
    if text.contains('\0') {
        p.valid = false;
        return p;
    }
    for line in text.lines() {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        match verb {
            "v" => {
                if rest.trim().parse::<u32>().map_or(true, |v| v > FORMAT) {
                    return Parsed { valid: false, ..Parsed::default() };
                }
            }
            "sel" => p.sel = rest.trim().parse().unwrap_or(0),
            "tab" if p.tabs.len() < MAX_TABS => {
                let (u, t) = rest.split_once('\t').unwrap_or((rest, ""));
                if u.len() <= MAX_FIELD {
                    let t: String = t.chars().take(MAX_FIELD).collect();
                    p.tabs.push((u.to_string(), t));
                }
            }
            "zoom" => {
                if let Some((o, f)) = rest.rsplit_once(' ') {
                    if let Some(f) = f.parse::<f32>().ok().filter(|f| f.is_finite()) {
                        p.zoom.insert(o.to_string(), f.clamp(ZOOM_MIN, ZOOM_MAX));
                    }
                }
            }
            _ => {}
        }
    }
    p
}

/// The session text for the given state (sorted zoom entries so the file is stable).
pub fn render(tabs: &[(String, String)], sel: usize, zoom: &HashMap<String, f32>) -> String {
    let mut out = format!("v {FORMAT}\nsel {sel}\n");
    for (u, t) in tabs.iter().take(MAX_TABS) {
        out.push_str(&format!("tab {}\t{}\n", u.replace(['\n', '\r', '\t'], " "), t.replace(['\n', '\r', '\t'], " ")));
    }
    let mut z: Vec<_> = zoom.iter().filter(|(_, f)| f.is_finite()).collect();
    z.sort_by(|a, b| a.0.cmp(b.0));
    for (o, f) in z {
        out.push_str(&format!("zoom {} {f}\n", o.replace(['\n', '\r'], " ")));
    }
    out
}

/// Reads the saved tabs, selected index and zoom table. A file that is not text or not a session file is moved aside
/// to `session.txt.corrupt` and the `.bak` copy of the last good write is tried instead.
pub fn load() -> (Vec<(String, String)>, usize, HashMap<String, f32>) {
    let main = path();
    for (i, candidate) in [main.clone(), main.with_extension("txt.bak")].into_iter().enumerate() {
        let Ok(bytes) = fs::read(&candidate) else { continue };
        let parsed = String::from_utf8(bytes).map(|t| parse(&t)).unwrap_or_default();
        if parsed.valid {
            return (parsed.tabs, parsed.sel, parsed.zoom);
        }
        if i == 0 {
            let _ = fs::rename(&candidate, main.with_extension("txt.corrupt"));
        }
    }
    (Vec::new(), 0, HashMap::new())
}

/// Atomic write: a unique temp file in the same directory, fsync, previous file kept as `.bak`, rename into place.
fn write(tabs: &[(String, String)], sel: usize, zoom: &HashMap<String, f32>) {
    use std::io::Write;
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    let ok = fs::File::create(&tmp).and_then(|mut f| f.write_all(render(tabs, sel, zoom).as_bytes()).and_then(|_| f.sync_all())).is_ok();
    if !ok {
        let _ = fs::remove_file(&tmp);
        return;
    }
    if p.exists() {
        let _ = fs::copy(&p, p.with_extension("txt.bak"));
    }
    if fs::rename(&tmp, &p).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

impl Browser {
    /// The open tabs as (url, title).
    fn snapshot(&self) -> Vec<(String, String)> {
        self.tabs
            .iter()
            .map(|t| self.pool.as_ref().and_then(|p| p.state(t.view)).map(|s| (s.url.clone(), s.title.clone())).unwrap_or_default())
            .collect()
    }

    /// Opens the saved tabs and selects the saved one; false when there was nothing to restore.
    pub fn restore_session(&mut self) -> bool {
        let (tabs, sel, zoom) = load();
        self.session.zoom = zoom;
        let tabs: Vec<_> = tabs.into_iter().filter(|(u, _)| !u.is_empty() && u != START).collect();
        if tabs.is_empty() {
            return false;
        }
        for (u, _) in &tabs {
            if let Some(p) = &mut self.pool {
                let view = p.create(u);
                self.tabs.push(crate::Tab { view });
            }
        }
        self.select(sel.min(self.tabs.len().saturating_sub(1)));
        for i in 0..self.tabs.len() {
            self.apply_zoom(i);
        }
        true
    }

    /// Writes the session when it changed, at most once per `DEBOUNCE_S`. Called from `tick`.
    pub fn save_session(&mut self, t: f64) {
        if self.pool.is_none() || self.private {
            return;
        }
        if !self.session.dirty && self.session.changed_at.is_none() && self.session.saved_once && self.current == self.session.last_sel {
            return;
        }
        let snap = self.snapshot();
        let (sel, nz) = (self.current, self.session.zoom.clone());
        let s = &mut self.session;
        if snap == s.last && sel == s.last_sel && s.last_zoom == nz && s.saved_once {
            s.changed_at = None;
            s.dirty = false;
            return;
        }
        let since = *s.changed_at.get_or_insert(t);
        if t - since < DEBOUNCE_S {
            return;
        }
        if snap.iter().any(|(u, _)| !u.is_empty() && u != START) || s.saved_once {
            write(&snap, sel, &s.zoom);
        }
        s.last = snap;
        s.last_sel = sel;
        s.last_zoom = nz;
        s.saved_once = true;
        s.changed_at = None;
        s.dirty = false;
    }

    /// Pushes tab `i` on the closed stack (call before it is removed): its page, title, Back list and place in the strip. A tab
    /// that never loaded a page is kept too (as `START`), so every closed tab can be reopened.
    pub fn remember_closed(&mut self, i: usize) {
        let Some(view) = self.tabs.get(i).map(|t| t.view) else { return };
        let state = self.pool.as_ref().and_then(|p| p.state(view)).map(|st| (st.url.clone(), st.title.clone()));
        let (list, index) = self.discard.entries(view).map(|(list, at)| (list.to_vec(), at)).unwrap_or_default();
        let history: Vec<String> = list.into_iter().take(index + 1).collect();
        let (url, title) = closed_page(state, &history);
        self.session.closed.push(ClosedTab { url, title, history, at: i });
    }

    /// Cmd+Shift+T: brings back the tab closed last.
    pub fn reopen_closed(&mut self) {
        self.reopen_nth_closed(0);
    }

    /// Reopens the `n`-th newest closed tab (0 = the one closed last) at the place it had in the strip, selected, and gives
    /// it the Back list it had. False when there is no such tab or no engine to open it in.
    pub fn reopen_nth_closed(&mut self, n: usize) -> bool {
        if self.pool.is_none() {
            return false;
        }
        let Some(tab) = self.session.closed.take(n) else { return false };
        let Some(view) = self.new_tab_at(&tab.url, tab.at) else { return false };
        self.discard.seed(view, tab.history);
        true
    }

    /// Applies the remembered zoom of tab `i`'s site.
    pub fn apply_zoom(&mut self, i: usize) {
        let Some(view) = self.tabs.get(i).map(|t| t.view) else { return };
        let Some(p) = &mut self.pool else { return };
        let Some(st) = p.state(view) else { return };
        let now = if st.page_zoom == 0.0 { 1.0 } else { st.page_zoom };
        let f = self.session.zoom.get(&origin(&st.url)).copied().unwrap_or(1.0);
        if (now - f).abs() > 0.001 {
            p.set_zoom(view, f);
        }
    }

    /// Cmd+Plus / Cmd+Minus / Cmd+0: `dir` is 1, -1 or 0 (reset). Remembered for the site.
    pub fn zoom_by(&mut self, dir: i32) {
        let Some(view) = self.view() else { return };
        let Some(p) = &mut self.pool else { return };
        let Some(st) = p.state(view) else { return };
        let now = if st.page_zoom == 0.0 { 1.0 } else { st.page_zoom };
        let o = origin(&st.url);
        let f = if dir == 0 { 1.0 } else { ((now + dir as f32 * ZOOM_STEP) * 100.0).round() / 100.0 }.clamp(ZOOM_MIN, ZOOM_MAX);
        p.set_zoom(view, f);
        if !o.is_empty() {
            if (f - 1.0).abs() < 0.001 {
                self.session.zoom.remove(&o);
            } else {
                self.session.zoom.insert(o, f);
            }
            self.session.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_stable_order() {
        let mut z = HashMap::new();
        z.insert("https://b".to_string(), 1.5);
        z.insert("https://a".to_string(), 0.5);
        let tabs = vec![("https://x/\n".to_string(), "t\tt".to_string())];
        let text = render(&tabs, 0, &z);
        assert_eq!(text, render(&tabs, 0, &z));
        let p = parse(&text);
        assert!(p.valid);
        assert_eq!(p.tabs, vec![("https://x/ ".to_string(), "t t".to_string())]);
        assert_eq!(p.zoom.len(), 2);
    }

    #[test]
    fn version_zero_loads_and_future_is_rejected() {
        assert_eq!(parse("sel 1\ntab https://a\tA\n").tabs.len(), 1);
        assert!(!parse("v 99\ntab https://a\n").valid);
        assert!(!parse("v x\n").valid);
    }

    #[test]
    fn corrupt_input_never_panics() {
        for t in ["", "tab", "tab \t", "zoom", "zoom a nan", "zoom a inf", "zoom  ", "sel -1", "sel 99999999999999999999", "\u{1F600}", "v", "tab x\ttab y\n"] {
            let p = parse(t);
            assert!(p.zoom.values().all(|f| f.is_finite() && (ZOOM_MIN..=ZOOM_MAX).contains(f)));
        }
        assert!(!parse("a\0b").valid);
        let many = "tab https://a\n".repeat(MAX_TABS + 50);
        assert_eq!(parse(&many).tabs.len(), MAX_TABS);
        assert_eq!(parse("zoom o 9.0").zoom["o"], ZOOM_MAX);
    }

    fn closed(url: &str, at: usize) -> ClosedTab {
        ClosedTab { url: url.to_string(), title: String::new(), history: Vec::new(), at }
    }

    #[test]
    fn closed_tabs_come_back_newest_first_with_their_place() {
        let mut c = ClosedTabs::default();
        assert!(c.is_empty());
        assert_eq!(c.take(0), None, "an empty stack has nothing to reopen");
        c.push(closed("https://a", 0));
        c.push(closed("https://b", 3));
        c.push(closed("https://c", 1));
        assert!(!c.is_empty());
        assert_eq!(c.newest().map(|t| t.url.as_str()).collect::<Vec<_>>(), vec!["https://c", "https://b", "https://a"]);
        // Cmd+Shift+T pops the last one closed, at the index it had
        assert_eq!(c.take(0), Some(closed("https://c", 1)));
        // the History menu can pick an older one; the rest keep their order
        assert_eq!(c.take(1), Some(closed("https://a", 0)));
        assert_eq!(c.newest().map(|t| t.url.as_str()).collect::<Vec<_>>(), vec!["https://b"]);
        assert_eq!(c.take(1), None, "there is no second tab left");
        assert_eq!(c.take(0), Some(closed("https://b", 3)));
        assert!(c.is_empty());
    }

    #[test]
    fn the_closed_stack_keeps_the_newest_twenty() {
        let mut c = ClosedTabs::default();
        for i in 0..CLOSED_MAX + 5 {
            c.push(closed(&format!("https://site/{i}"), i));
        }
        assert_eq!(c.newest().count(), CLOSED_MAX);
        assert_eq!(c.take(0).map(|t| t.at), Some(CLOSED_MAX + 4), "the newest is kept");
        let oldest = c.newest().last().map(|t| t.at);
        assert_eq!(oldest, Some(5), "the five oldest were dropped");
    }

    #[test]
    fn a_tab_without_a_page_is_remembered_as_the_start_page() {
        let history = vec!["https://a".to_string(), "https://b".to_string()];
        // the engine knows the page: its URL and title win
        assert_eq!(closed_page(Some(("https://b".into(), "B".into())), &history), ("https://b".to_string(), "B".to_string()));
        // no URL yet: the last entry of the Back list
        assert_eq!(closed_page(Some((String::new(), "B".into())), &history), ("https://b".to_string(), "B".to_string()));
        assert_eq!(closed_page(None, &history), ("https://b".to_string(), String::new()));
        // nothing at all: a blank tab is still a closed tab
        assert_eq!(closed_page(None, &[]), (START.to_string(), String::new()));
        assert_eq!(closed_page(Some((String::new(), String::new())), &[]), (START.to_string(), String::new()));
    }

    #[test]
    fn origin_strips_path() {
        assert_eq!(origin("https://a.b:8080/x?y#z"), "https://a.b:8080");
        assert_eq!(origin("about:blank"), "");
        assert_eq!(origin("https://user:password@EXAMPLE.com:443/path"), "https://example.com");
        assert_eq!(origin("http://[::1]:8080/path"), "http://[::1]:8080");
    }
}

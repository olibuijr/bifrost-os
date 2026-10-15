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
const CLOSED_MAX: usize = 10;
/// Seconds a change waits before it is written.
const DEBOUNCE_S: f64 = 1.0;

#[derive(Default)]
pub struct Session {
    pub zoom: HashMap<String, f32>,
    pub closed: Vec<(String, String)>,
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
    let Some((scheme, rest)) = url.split_once("://") else { return String::new() };
    format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or(""))
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

    /// Pushes tab `i` on the closed stack (call before it is removed).
    pub fn remember_closed(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        let Some(st) = self.pool.as_ref().and_then(|p| p.state(tab.view)) else { return };
        if st.url.is_empty() || st.url == START {
            return;
        }
        self.session.closed.push((st.url.clone(), st.title.clone()));
        if self.session.closed.len() > CLOSED_MAX {
            self.session.closed.remove(0);
        }
    }

    /// Cmd+Shift+T.
    pub fn reopen_closed(&mut self) {
        if let Some((url, _)) = self.session.closed.pop() {
            self.new_tab(&url);
        }
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

    #[test]
    fn origin_strips_path() {
        assert_eq!(origin("https://a.b:8080/x?y#z"), "https://a.b:8080");
        assert_eq!(origin("about:blank"), "");
    }
}

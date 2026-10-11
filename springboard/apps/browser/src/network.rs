//! Network log of the Web Inspector and of `browser.network`.
//!
//! Servo 0.7.0 gives an embedder no request hooks, so the log is built in the page: a collector script wraps
//! `fetch` and `XMLHttpRequest`, listens for failed resource loads (`error` on `img`, `script`, `link`) and the drain
//! adds `performance.getEntriesByType('resource')` for everything else. The engine runs the collector at the start of
//! every document when its `devtools` module is wired; otherwise the app injects it when the panel or the method is
//! first used (requests made before that are then only seen through Resource Timing).
//!
//! Limits (documented in the README): status codes of images, scripts and stylesheets are unknown (Resource Timing
//! has no status; shown as "-"), `from_cache` is a guess from `transferSize == 0`, sizes are `content-length` or
//! Resource Timing's transfer size, and requests of iframes and workers are not seen.
use std::collections::{HashMap, VecDeque};

use bifrost_app::kit::ui::system::{SYSTEM_GREEN, SYSTEM_ORANGE};
use bifrost_app::kit::ui::{DESTRUCTIVE, LABEL, LABEL_2};
use bifrost_core::json::{self, Json};

use crate::control::{num, obj, text, Cell, Row};

/// Entries kept per tab; the oldest are dropped first.
pub const MAX_ENTRIES: usize = 2000;

/// Installs the collector in the page (idempotent). Evaluates to 1 on the first call, 0 after.
pub const COLLECTOR_JS: &str = "(function(){if(window.__bn)return 0;var bn=window.__bn={q:[],rt:0,t0:performance.now()};\
function push(e){if(bn.q.length>=2000)bn.q.shift();bn.q.push(e)}\
var f=window.fetch;if(f){window.fetch=function(i,o){var t0=performance.now(),ts=Date.now(),m=(o&&o.method)||(i&&i.method)||'GET',u=(i&&i.url)||String(i);\
return f.apply(this,arguments).then(function(r){var cl=0,dur=performance.now()-t0;try{cl=+r.headers.get('content-length')||0}catch(e){}\
var ev={method:String(m).toUpperCase(),url:u,status:r.status,type:'fetch',ts:ts,dur:dur,size:cl};\
if(cl||!r.clone){push(ev)}else{try{r.clone().arrayBuffer().then(function(b){ev.size=b.byteLength;push(ev)},function(){push(ev)})}catch(e){push(ev)}}return r},\
function(e){push({method:String(m).toUpperCase(),url:u,status:0,type:'fetch',ts:ts,dur:performance.now()-t0,size:0,error:String(e)});throw e})}}\
var X=window.XMLHttpRequest;if(X&&X.prototype){var op=X.prototype.open,sd=X.prototype.send;\
X.prototype.open=function(m,u){this.__bm=m;this.__bu=String(u);return op.apply(this,arguments)};\
X.prototype.send=function(){var x=this,t0=performance.now(),ts=Date.now();\
function d(ev){var cl=0;try{cl=+x.getResponseHeader('content-length')||0}catch(e){}\
if(!cl){try{var rr=x.response;cl=rr==null?0:(rr.byteLength||rr.size||(typeof rr==='string'?rr.length:0))||0}catch(e){}}\
push({method:String(x.__bm||'GET').toUpperCase(),url:x.__bu||'',status:x.status|0,type:'xhr',ts:ts,dur:performance.now()-t0,size:cl,error:ev==='load'?undefined:ev})}\
x.addEventListener('load',function(){d('load')});x.addEventListener('error',function(){d('network error')});\
x.addEventListener('abort',function(){d('aborted')});x.addEventListener('timeout',function(){d('timeout')});\
return sd.apply(this,arguments)}}\
window.addEventListener('error',function(e){var t=e.target;if(t&&t!==window&&(t.src||t.href)){push({method:'GET',url:String(t.src||t.href),status:0,type:String(t.tagName||'').toLowerCase(),ts:Date.now(),dur:0,size:0,error:'failed to load'})}},true);\
return 1})()";

/// Installs the collector if needed, then returns (and removes) the page's new entries as a JSON string.
pub fn drain_js() -> String {
    format!(
        "{COLLECTOR_JS};(function(){{var bn=window.__bn,out=bn.q.splice(0);var rt=performance.getEntriesByType?performance.getEntriesByType('resource'):[];\
         for(;bn.rt<rt.length;bn.rt++){{var e=rt[bn.rt];\
         if((e.initiatorType==='fetch'||e.initiatorType==='xmlhttprequest')&&e.startTime>=bn.t0)continue;\
         out.push({{method:'GET',url:e.name,status:0,type:e.initiatorType||'other',ts:Date.now()-(performance.now()-e.startTime),dur:e.duration||(e.responseEnd>e.startTime?e.responseEnd-e.startTime:0),\
         size:e.transferSize||e.encodedBodySize||e.decodedBodySize||0,cache:e.transferSize===0&&e.decodedBodySize>0}})}}\
         return JSON.stringify(out)}})()"
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetRec {
    pub seq: u64,
    pub ts_ms: u64,
    pub method: String,
    pub url: String,
    /// 0 when unknown (Resource Timing) or when the request failed
    pub status: u32,
    pub kind: String,
    pub from_cache: bool,
    pub duration_ms: f64,
    pub size: u64,
    pub error: Option<String>,
}

impl NetRec {
    pub fn failed(&self) -> bool {
        self.error.is_some() || self.status >= 400
    }
    pub fn is_xhr(&self) -> bool {
        matches!(self.kind.as_str(), "fetch" | "xhr" | "xmlhttprequest")
    }
}

pub fn net_json(r: &NetRec) -> Json {
    let mut f = vec![
        ("seq", num(r.seq as f64)),
        ("ts_ms", num(r.ts_ms as f64)),
        ("method", text(&r.method)),
        ("url", text(&r.url)),
        ("status", num(r.status as f64)),
        ("type", text(&r.kind)),
        ("from_cache", Json::Bool(r.from_cache)),
        ("duration_ms", num((r.duration_ms * 10.0).round() / 10.0)),
        ("size", num(r.size as f64)),
    ];
    if let Some(e) = &r.error {
        f.push(("error", text(e)));
    }
    obj(f)
}

#[derive(Default)]
struct TabNet {
    recs: VecDeque<NetRec>,
    last_seq: u64,
    bytes: usize,
}

/// Network log text kept per tab, bytes (URLs and errors); oldest entries go first.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

fn rec_bytes(r: &NetRec) -> usize {
    r.url.len() + r.method.len() + r.kind.len() + r.error.as_ref().map_or(0, |e| e.len()) + 64
}

/// The network entries of every tab.
#[derive(Default)]
pub struct NetLog {
    tabs: HashMap<u32, TabNet>,
}

fn raw_to_rec(v: &Json, seq: u64, now_ms: u64) -> Option<NetRec> {
    let url = v.get("url")?.as_str()?.to_string();
    let n = |k: &str| v.get(k).and_then(Json::number).filter(|x| x.is_finite()).unwrap_or(0.0);
    Some(NetRec {
        seq,
        ts_ms: if n("ts") > 0.0 { n("ts") as u64 } else { now_ms },
        method: v.get("method").and_then(Json::as_str).unwrap_or("GET").to_string(),
        url,
        status: n("status").max(0.0) as u32,
        kind: v.get("type").and_then(Json::as_str).unwrap_or("other").to_string(),
        from_cache: matches!(v.get("cache"), Some(Json::Bool(true))),
        duration_ms: n("dur").max(0.0),
        size: n("size").max(0.0) as u64,
        error: v.get("error").and_then(Json::as_str).map(str::to_string),
    })
}

impl NetLog {
    /// Adds the entries of a `drain_js` result (a JSON array) to tab `view`. Returns how many were added.
    pub fn ingest(&mut self, view: u32, drained: &str, now_ms: u64) -> usize {
        let Ok(Json::Arr(items)) = json::parse(drained) else { return 0 };
        let tab = self.tabs.entry(view).or_default();
        let mut added = 0;
        for it in &items {
            let seq = tab.last_seq + 1;
            if let Some(rec) = raw_to_rec(it, seq, now_ms) {
                tab.last_seq = seq;
                tab.bytes += rec_bytes(&rec);
                tab.recs.push_back(rec);
                while tab.recs.len() > MAX_ENTRIES || (tab.bytes > MAX_BYTES && tab.recs.len() > 1) {
                    if let Some(old) = tab.recs.pop_front() {
                        tab.bytes = tab.bytes.saturating_sub(rec_bytes(&old));
                    }
                }
                added += 1;
            }
        }
        added
    }

    pub fn entries(&self, view: u32) -> Option<&VecDeque<NetRec>> {
        self.tabs.get(&view).map(|t| &t.recs)
    }

    pub fn last_seq(&self, view: u32) -> u64 {
        self.tabs.get(&view).map_or(0, |t| t.last_seq)
    }

    pub fn clear(&mut self, view: u32) {
        if let Some(t) = self.tabs.get_mut(&view) {
            t.recs.clear();
            t.bytes = 0;
        }
    }

    pub fn forget(&mut self, view: u32) {
        self.tabs.remove(&view);
    }

    /// `browser.network`: entries newer than `since_seq`, the newest `limit`, oldest first; `next_seq` is the highest
    /// sequence number so far. `clear` empties the tab's log afterwards.
    pub fn query(&mut self, view: u32, since_seq: Option<u64>, limit: usize, clear: bool) -> Json {
        let last = self.last_seq(view);
        let mut picked: Vec<&NetRec> = self
            .tabs
            .get(&view)
            .map(|t| t.recs.iter().rev().filter(|r| since_seq.is_none_or(|s| r.seq > s)).take(limit).collect())
            .unwrap_or_default();
        picked.reverse();
        let out = obj(vec![("entries", Json::Arr(picked.into_iter().map(net_json).collect())), ("next_seq", num(last as f64))]);
        if clear {
            self.clear(view);
        }
        out
    }
}

/// Panel filter chips.
pub const FILTERS: [&str; 4] = ["All", "Errors", "Fetch/XHR", "Other"];

pub fn filter_matches(filter: usize, r: &NetRec) -> bool {
    match filter {
        1 => r.failed(),
        2 => r.is_xhr(),
        3 => !r.is_xhr(),
        _ => true,
    }
}

/// `1.2 kB`, `340 B`, `-` for unknown.
pub fn human_size(bytes: u64) -> String {
    match bytes {
        0 => "-".into(),
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} kB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1024.0 / 1024.0),
    }
}

/// `820 ms`, `1.4 s`, `-` for unknown (Servo 0.7.0 leaves some timings empty).
pub fn human_time(ms: f64) -> String {
    if ms <= 0.0 {
        "-".into()
    } else if ms < 1000.0 { format!("{} ms", ms.round() as u64) } else { format!("{:.1} s", ms / 1000.0) }
}

/// The path and query of a URL (the "Name" column); the whole text when it has no host.
pub fn short_name(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => match rest.find('/') {
            Some(i) if i + 1 < rest.len() => rest[i..].to_string(),
            _ => rest.trim_end_matches('/').to_string(),
        },
        None => url.chars().take(80).collect(),
    }
}

/// Row colour by outcome: failed red, 4xx/5xx red, redirects secondary, unknown status secondary, cached orange.
pub fn row_color(r: &NetRec) -> u32 {
    if r.failed() {
        DESTRUCTIVE
    } else if r.status >= 300 || r.status == 0 {
        LABEL_2.now()
    } else if r.from_cache {
        SYSTEM_ORANGE.now()
    } else if r.status >= 200 {
        LABEL.now()
    } else {
        SYSTEM_GREEN.now()
    }
}

/// Column layout for a list `width` points wide: (title, x, w, right-aligned). A negative `Cell.w` means right-aligned.
pub fn columns(width: f32) -> [(&'static str, f32, f32, bool); 6] {
    let (method, status, kind, size, time) = (52.0, 48.0, 72.0, 70.0, 70.0);
    let name = (width - 10.0 - method - status - kind - size - time - 10.0).max(60.0);
    let mut x = 10.0;
    let mut col = |t: &'static str, w: f32, r: bool| {
        let c = (t, x, w, r);
        x += w;
        c
    };
    [col("Name", name, false), col("Method", method, false), col("Status", status, false), col("Type", kind, false), col("Size", size, true), col("Time", time, true)]
}

/// The column header row (drawn above the list, not scrolled).
pub fn header(width: f32) -> Row {
    let cells = columns(width).iter().map(|&(t, x, w, r)| Cell { x, w: if r { -(w - 8.0) } else { w - 8.0 }, text: t.into(), color: LABEL_2.now() }).collect();
    Row { cells, copy: String::new(), toggle: None, select: None }
}

/// Panel rows for the visible entries of one tab, columns laid out for a list `width` points wide.
pub fn rows(entries: &VecDeque<NetRec>, filter: usize, needle: &str, width: f32) -> Vec<Row> {
    let needle = needle.to_lowercase();
    let cols = columns(width);
    entries
        .iter()
        .filter(|r| filter_matches(filter, r) && (needle.is_empty() || r.url.to_lowercase().contains(&needle)))
        .map(|r| {
            let c = row_color(r);
            let status = match (&r.error, r.status) {
                (Some(_), _) => "failed".to_string(),
                (None, 0) => "-".to_string(),
                (None, s) => s.to_string(),
            };
            let texts = [short_name(&r.url), r.method.clone(), status, r.kind.clone(), human_size(r.size), human_time(r.duration_ms)];
            let colors = [c, c, c, LABEL_2.now(), LABEL_2.now(), LABEL_2.now()];
            let cells = cols
                .iter()
                .zip(texts)
                .zip(colors)
                .map(|((&(_, x, w, right), text), color)| Cell { x, w: if right { -(w - 8.0) } else { w - 8.0 }, text, color })
                .collect();
            Row {
                cells,
                copy: format!("{} {} ({})", r.method, r.url, r.error.clone().unwrap_or_else(|| r.status.to_string())),
                toggle: None,
                select: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn sizes_and_times_read_like_devtools() {
        assert_eq!(human_size(0), "-");
        assert_eq!(human_size(340), "340 B");
        assert_eq!(human_size(1024), "1.0 kB");
        assert_eq!(human_size(1536), "1.5 kB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human_time(0.0), "-");
        assert_eq!(human_time(-5.0), "-");
        assert_eq!(human_time(819.6), "820 ms");
        assert_eq!(human_time(1400.0), "1.4 s");
    }

    #[test]
    fn names_are_path_and_query() {
        assert_eq!(short_name("https://example.com/a/b.js?v=1"), "/a/b.js?v=1");
        assert_eq!(short_name("https://example.com/"), "example.com");
        assert_eq!(short_name("https://example.com"), "example.com");
        assert_eq!(short_name("data-without-host"), "data-without-host");
    }

    #[test]
    fn columns_fit_and_keep_a_minimum_name() {
        let c = columns(600.0);
        let last = c[5];
        assert!((last.1 + last.2 - 590.0).abs() < 0.01);
        assert!(c[0].2 >= 60.0);
        assert_eq!(columns(10.0)[0].2, 60.0);
    }
}


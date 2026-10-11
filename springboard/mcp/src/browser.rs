//! The `browser` MCP tool: Bifrost Browser state, tabs, navigation, eval, console, network, DOM, screenshots,
//! waits, emulation, input and the inspector, over the Browser's app control socket (`browser.*` methods,
//! docs/bifrost-mcp-browser.md). The transport is a closure so tests run against a mock socket.

use crate::sys;
use crate::tools::{b64_decode, f, s, text, Ctx};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const APP: &str = "browser";
const MAX_INLINE: usize = 60_000;
const MAX_HTML: u64 = 200_000;
const MAX_CONSOLE: u64 = 200;

static CURSORS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
static FILES: Mutex<u32> = Mutex::new(0);

pub type Rpc<'a> = dyn FnMut(&str, &Value, Duration) -> Result<Value, String> + 'a;

fn next_file() -> u32 {
    let mut g = FILES.lock().unwrap();
    *g += 1;
    *g
}

fn cursor_get(k: &str) -> Option<u64> {
    CURSORS.lock().ok()?.as_ref()?.get(k).copied()
}
fn cursor_set(k: &str, v: u64) {
    if let Ok(mut g) = CURSORS.lock() {
        g.get_or_insert_with(Default::default).insert(k.to_string(), v);
    }
}
fn cursor_clear(prefix: &str) {
    if let Ok(mut g) = CURSORS.lock() {
        if let Some(m) = g.as_mut() {
            m.retain(|k, _| !k.starts_with(prefix));
        }
    }
}

/// Add a hint to a transport or JSON-RPC error message.
pub fn hint(method: &str, e: &str) -> String {
    let l = e.to_lowercase();
    let h = if l.contains("not running") || l.contains("no such file") || l.contains("refused") {
        "the Browser is not running; this tool launches it on the first call, or use dev restart target=app:is.olibuijr.browser.desktop"
    } else if l.contains("-32002") || l.contains("engine not running") {
        "the web engine process is dead; the Browser restarts it on its own (browser state shows restarts); retry in a moment or reload"
    } else if l.contains("-32001") || l.contains("no such tab") {
        "the tab is gone; call browser action=tabs for the current ids"
    } else if l.contains("-32000") || l.contains("timeout") || l.contains("timed out") || l.contains("no reply") {
        "the page did not answer in time; raise timeout_ms, check browser action=state (engine_alive) and console for a hung script"
    } else if missing(e) {
        "this Browser build lacks that control method; update the Browser (bifrost pc midget update) or use the app tool: app app=browser lists methods"
    } else if l.contains("-32602") || l.contains("invalid") {
        "invalid parameters; see the action's parameters in the tool description"
    } else {
        return format!("browser {method}: {e}");
    };
    format!("browser {method}: {e}\nhint: {h}")
}

fn missing(e: &str) -> bool {
    let l = e.to_lowercase();
    l.contains("-32601") || l.contains("method not found")
}

/// Keep text within the inline budget; longer text goes to a file and the reply carries a head and the path.
pub fn spill(label: &str, body: String) -> String {
    if body.len() <= MAX_INLINE {
        return body;
    }
    let path = format!("/tmp/bifrost-browser-{label}-{}.txt", next_file());
    let _ = std::fs::write(&path, &body);
    let mut cut = 4000.min(body.len());
    while !body.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n... truncated: {} bytes total, full text in {path}", &body[..cut], body.len())
}

fn pretty(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn pass(a: &Value, keys: &[&str]) -> Value {
    let mut p = serde_json::Map::new();
    for k in keys {
        if let Some(v) = a.get(*k) {
            if !v.is_null() {
                p.insert((*k).to_string(), v.clone());
            }
        }
    }
    Value::Object(p)
}

fn timeout_for(a: &Value, default_ms: f64) -> Duration {
    // transport waits longer than the page-side timeout so the page's own error wins
    Duration::from_millis((f(a, "timeout_ms").unwrap_or(default_ms).clamp(100.0, 60_000.0) + 2000.0) as u64)
}

fn tab_key(a: &Value) -> String {
    a.get("id").map(|v| v.to_string()).unwrap_or_else(|| "cur".into())
}

/// Entry from tools.rs dispatch: ensures the Browser runs, then performs the action over the real socket.
pub fn run(ctx: &mut Ctx, a: &Value) -> Result<Value, String> {
    let path = ensure_running()?;
    let mut rpc = |m: &str, p: &Value, t: Duration| sys::app_rpc_full(&path, m, p, t).map_err(|e| e.to_string());
    let mut fallback = |args: &Value| ctx.call("screenshot", args);
    exec(&mut rpc, &mut fallback, a)
}

fn socket() -> Option<std::path::PathBuf> {
    sys::app_sockets().into_iter().find(|(id, _)| id.to_lowercase().contains(APP)).map(|x| x.1)
}

fn ensure_running() -> Result<std::path::PathBuf, String> {
    if let Some(p) = socket() {
        if sys::app_rpc(&p, "app.info", &json!({}), Duration::from_millis(800)).is_ok() {
            return Ok(p);
        }
    }
    let id = sys::app_sockets().into_iter().map(|x| x.0).find(|i| i.to_lowercase().contains(APP)).unwrap_or_else(|| "is.olibuijr.browser.desktop".into());
    sys::shell_text(&format!("open-app {id}")).map_err(|e| format!("could not launch the Browser ({id}): {e}"))?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(15) {
        if let Some(p) = socket() {
            if sys::app_rpc(&p, "app.info", &json!({}), Duration::from_millis(500)).is_ok() {
                return Ok(p);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("Browser launched ({id}) but its control socket did not answer within 15 s; check logs app={id} and crash"))
}

/// The action logic, independent of the transport. `fallback` takes screenshot-tool args (window capture).
pub fn exec(rpc: &mut Rpc, fallback: &mut dyn FnMut(&Value) -> Value, a: &Value) -> Result<Value, String> {
    let action = s(a, "action").ok_or("action is required: state tabs new_tab select_tab close_tab navigate back forward reload eval console network dom screenshot wait viewport zoom find input devtools source inspect downloads bookmarks history")?;
    let mut call = |m: &str, p: Value, t: Duration| -> Result<Value, String> { rpc(m, &p, t).map_err(|e| hint(m, &e)) };
    let quick = Duration::from_secs(5);
    let tab = a.get("id").filter(|v| !v.is_null()).cloned();
    let with_id = |extra: Value| -> Value {
        let mut o = extra.as_object().cloned().unwrap_or_default();
        if let Some(i) = &tab {
            o.insert("id".into(), i.clone());
        }
        Value::Object(o)
    };
    match action {
        "state" => Ok(text(pretty(&call("browser.state", json!({}), quick)?))),
        "tabs" => {
            let v = call("browser.tabs", json!({}), quick)?;
            let mut out = String::new();
            for t in v.as_array().into_iter().flatten() {
                out += &format!(
                    "{}{} [{}] {} {:?}{}{}{}\n",
                    if t["current"] == true { "*" } else { "-" },
                    t["id"],
                    t["index"],
                    t["url"].as_str().unwrap_or(""),
                    t["title"].as_str().unwrap_or(""),
                    if t["loading"] == true { " loading" } else { "" },
                    if t["crashed"] == true { " CRASHED" } else { "" },
                    if t["secure"] == true { " https" } else { "" },
                );
            }
            Ok(text(if out.is_empty() { "no tabs".into() } else { out }))
        }
        "new_tab" => Ok(text(pretty(&call("browser.tab.new", pass(a, &["url", "background"]), quick)?))),
        "select_tab" => Ok(text(pretty(&call("browser.tab.select", with_id(json!({})), quick)?))),
        "close_tab" => Ok(text(pretty(&call("browser.tab.close", with_id(json!({})), quick)?))),
        "favicon" => Ok(text(pretty(&call("browser.favicon", with_id(json!({})), quick)?))),
        "downloads" => Ok(text(pretty(&call("browser.downloads", pass(a, &["cancel", "clear"]), quick)?))),
        "bookmarks" => Ok(text(pretty(&call("browser.bookmarks", pass(a, &["op", "url", "title", "query", "limit"]), quick)?))),
        "history" => Ok(text(pretty(&call("browser.history", pass(a, &["query", "limit", "remove", "clear"]), quick)?))),
        "reopen_tab" => Ok(text(pretty(&call("browser.tab.reopen", json!({}), quick)?))),
        "navigate" => {
            let url = s(a, "url").ok_or("url is required")?;
            let mut p = with_id(json!({"url": url, "wait": s(a, "wait").unwrap_or("load")}));
            if let Some(t) = f(a, "timeout_ms") {
                p["timeout_ms"] = json!(t);
            }
            let v = call("browser.navigate", p, timeout_for(a, 30_000.0))?;
            Ok(text(format!(
                "{} {} {:?} (tab {})",
                v["status"].as_str().unwrap_or("?"),
                v["url"].as_str().unwrap_or(url),
                v["title"].as_str().unwrap_or(""),
                v["id"]
            )))
        }
        "back" | "forward" => Ok(text(pretty(&call(&format!("browser.{action}"), with_id(json!({})), quick)?))),
        "reload" => Ok(text(pretty(&call("browser.reload", with_id(pass(a, &["hard"])), quick)?))),
        "eval" => {
            let js = s(a, "js").ok_or("js is required")?;
            let mut p = with_id(json!({"js": js}));
            if let Some(t) = f(a, "timeout_ms") {
                p["timeout_ms"] = json!(t);
            }
            let v = call("browser.eval", p, timeout_for(a, 10_000.0))?;
            if v["ok"] == false {
                return Ok(json!({"content":[{"type":"text","text":format!("JS error: {}", v["error"].as_str().unwrap_or("unknown"))}],"isError":true}));
            }
            let r = match &v["result"] {
                Value::String(s) => s.clone(),
                r => r.to_string(),
            };
            Ok(text(spill("eval", r)))
        }
        "console" | "network" => {
            let is_net = action == "network";
            let method = if is_net { "browser.network" } else { "browser.console" };
            let session = s(a, "session").unwrap_or("default");
            let ck = format!("{action}|{session}|{}|{}", tab_key(a), s(a, "level").unwrap_or(""));
            if a.get("reset").and_then(Value::as_bool).unwrap_or(false) {
                cursor_clear(&format!("{action}|{session}|"));
            }
            let limit = f(a, "limit").unwrap_or(MAX_CONSOLE as f64).clamp(1.0, MAX_CONSOLE as f64) as u64;
            let follow = Duration::from_millis(f(a, "follow_ms").unwrap_or(0.0).clamp(0.0, 30_000.0) as u64);
            let filter = s(a, "text").map(str::to_lowercase);
            let t0 = Instant::now();
            loop {
                let mut p = with_id(json!({"limit": limit}));
                if !is_net {
                    if let Some(l) = s(a, "level") {
                        p["level"] = json!(l);
                    }
                }
                if let Some(sq) = a.get("since_seq").and_then(Value::as_u64).or_else(|| cursor_get(&ck)) {
                    p["since_seq"] = json!(sq);
                }
                if a.get("clear").and_then(Value::as_bool).unwrap_or(false) {
                    p["clear"] = json!(true);
                }
                let v = call(method, p, quick)?;
                if let Some(n) = v["next_seq"].as_u64() {
                    cursor_set(&ck, n);
                }
                let rows: Vec<&Value> = v["entries"].as_array().into_iter().flatten()
                    .filter(|e| filter.as_ref().is_none_or(|q| e["text"].to_string().to_lowercase().contains(q) || e["url"].to_string().to_lowercase().contains(q)))
                    .collect();
                if rows.is_empty() && t0.elapsed() < follow {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let mut out = String::new();
                for e in &rows {
                    if is_net {
                        out += &format!(
                            "#{} {} {} {} {}{} {}ms {}B{}\n",
                            e["seq"], e["method"].as_str().unwrap_or(""), e["status"], e["url"].as_str().unwrap_or(""), e["type"].as_str().unwrap_or(""),
                            if e["from_cache"] == true { " cached" } else { "" }, e["duration_ms"], e["size"],
                            e.get("error").and_then(Value::as_str).map(|x| format!(" ERROR {x}")).unwrap_or_default()
                        );
                    } else {
                        out += &format!("#{} [{}] {} ({}:{})\n", e["seq"], e["level"].as_str().unwrap_or(""), e["text"].as_str().unwrap_or(""), e["source"].as_str().unwrap_or(""), e["line"]);
                    }
                }
                if out.is_empty() {
                    out = format!("no new {action} entries\n");
                }
                let capped = v["entries"].as_array().map(|x| x.len() as u64 >= limit).unwrap_or(false);
                out += &format!("next_seq {}{}", v["next_seq"], if capped { " (capped; call again for more)" } else { "" });
                return Ok(text(spill(action, out)));
            }
        }
        "dom" => {
            let mode = s(a, "mode").unwrap_or("query");
            let max = f(a, "max_bytes").map(|x| x as u64).unwrap_or(MAX_HTML).min(MAX_HTML);
            let mut p = with_id(json!({"selector": s(a, "selector").unwrap_or("html"), "mode": mode, "max_bytes": max}));
            if let Some(l) = f(a, "limit") {
                p["limit"] = json!(l);
            }
            let v = call("browser.dom", p, Duration::from_secs(15))?;
            if let Some(h) = v.get("html").or_else(|| v.get("text")).and_then(Value::as_str) {
                let note = if v["truncated"] == true { format!("\n[truncated at {max} bytes]") } else { String::new() };
                return Ok(text(spill("dom", format!("{h}{note}"))));
            }
            Ok(text(spill("dom", pretty(&v))))
        }
        "screenshot" => {
            let mut p = with_id(pass(a, &["full_page", "selector", "rect", "scale"]));
            p["format"] = json!("png");
            match call("browser.screenshot", p, Duration::from_secs(30)) {
                Ok(v) => {
                    let data = v["data"].as_str().ok_or("screenshot reply had no data")?;
                    let path = format!("/tmp/bifrost-browser-shot-{}.png", next_file());
                    let wrote = std::fs::write(&path, b64_decode(data)).is_ok();
                    let note = format!(
                        "{}x{} px, dpr {}, scroll ({},{}){}",
                        v["width"], v["height"], v["dpr"], v["scroll"]["x"], v["scroll"]["y"],
                        if wrote { format!(", saved {path}") } else { String::new() }
                    );
                    Ok(json!({"content":[{"type":"image","data":data,"mimeType":"image/png"},{"type":"text","text":note}]}))
                }
                Err(e) if missing(&e) => {
                    let mut r = fallback(&json!({"window": "Browser"}));
                    if let Some(c) = r["content"].as_array_mut() {
                        c.push(json!({"type":"text","text":"fallback: this Browser build has no browser.screenshot, so this is the whole Browser window (chrome included, not cropped to the page); selector/full_page/rect were ignored"}));
                    }
                    Ok(r)
                }
                Err(e) => Err(e),
            }
        }
        "storage" | "styles" => {
            let js = if action == "styles" {
                js_styles(s(a, "selector").ok_or("selector is required")?)
            } else {
                js_storage(s(a, "kind").unwrap_or("all"), s(a, "key"), s(a, "value"), a.get("delete").and_then(Value::as_bool).unwrap_or(false))?
            };
            let v = call("browser.eval", with_id(json!({"js": js})), quick)?;
            if v["ok"] == false {
                return Ok(json!({"content":[{"type":"text","text":format!("JS error: {}", v["error"].as_str().unwrap_or("unknown"))}],"isError":true}));
            }
            Ok(text(match &v["result"] {
                Value::String(s) => s.clone(),
                r => r.to_string(),
            }))
        }
        "inspect" => {
            let p = with_id(pass(a, &["x", "y", "timeout_ms"]));
            Ok(text(pretty(&call("browser.inspect", p, quick)?)))
        }
        "wait" => {
            let mut p = with_id(pass(a, &["selector", "js", "text", "timeout_ms"]));
            p["for"] = json!(s(a, "for").unwrap_or("load"));
            Ok(text(pretty(&call("browser.wait", p, timeout_for(a, 10_000.0))?)))
        }
        "viewport" => Ok(text(pretty(&call("browser.viewport", pass(a, &["width", "height", "dpr", "preset", "reset"]), quick)?))),
        "zoom" => Ok(text(pretty(&call("browser.zoom", pass(a, &["factor"]), quick)?))),
        "find" => {
            let q = s(a, "query").ok_or("query is required")?;
            Ok(text(pretty(&call("browser.find", with_id(json!({"query": q, "dir": s(a, "dir").unwrap_or("n")})), quick)?)))
        }
        "user_agent" => Ok(text(pretty(&call("browser.user_agent", pass(a, &["ua"]), quick)?))),
        "input" => {
            let kind = s(a, "kind").ok_or("kind is required: click move key text scroll")?;
            let mut p = with_id(pass(a, &["x", "y", "button", "key", "mods", "text", "dx", "dy"]));
            p["kind"] = json!(kind);
            // click or move by selector: resolve the element centre through dom first
            if let (Some(sel), true) = (s(a, "selector"), p.get("x").is_none()) {
                let d = call("browser.dom", with_id(json!({"selector": sel, "mode": "query", "limit": 1})), Duration::from_secs(10))?;
                let r = &d["items"][0]["rect"];
                if r.is_null() {
                    return Err(format!("no element matches selector {sel:?}"));
                }
                p["x"] = json!(r["x"].as_f64().unwrap_or(0.0) + r["w"].as_f64().unwrap_or(0.0) / 2.0);
                p["y"] = json!(r["y"].as_f64().unwrap_or(0.0) + r["h"].as_f64().unwrap_or(0.0) / 2.0);
            }
            Ok(text(pretty(&call("browser.input", p, quick)?)))
        }
        "devtools" => Ok(text(pretty(&call("browser.devtools", pass(a, &["open", "panel"]), quick)?))),
        "source" => {
            let v = call("browser.source", with_id(json!({})), Duration::from_secs(15))?;
            Ok(text(spill("source", format!("{}\n{}", v["url"].as_str().unwrap_or(""), v["html"].as_str().unwrap_or("")))))
        }
        other => Err(format!("unknown browser action {other:?}")),
    }
}

/// `styles`: the computed style of the first element matching `selector`, as a JSON string.
fn js_styles(selector: &str) -> String {
    format!(
        "(function(sel){{var e;try{{e=document.querySelector(sel)}}catch(x){{return JSON.stringify({{error:String(x)}})}}\
         if(!e)return JSON.stringify({{error:'no element matches'}});var c=getComputedStyle(e),o={{}};\
         function q(a){{return a.map(function(p){{return c.getPropertyValue(p)}}).join(' ')}}\
         ['display','position','box-sizing','width','height','color','background-color','font-family','font-size','font-weight','line-height','opacity','z-index','overflow','visibility'].forEach(function(p){{o[p]=c.getPropertyValue(p)}});\
         o.margin=q(['margin-top','margin-right','margin-bottom','margin-left']);o.padding=q(['padding-top','padding-right','padding-bottom','padding-left']);\
         o['border-width']=q(['border-top-width','border-right-width','border-bottom-width','border-left-width']);\
         var r=e.getBoundingClientRect();o.rect={{x:r.left,y:r.top,w:r.width,h:r.height}};return JSON.stringify(o)}})({})",
        json!(selector)
    )
}

/// `storage`: lists (`kind` local, session, cookie or all), sets (`key` and `value`) or deletes (`key`, `delete`).
fn js_storage(kind: &str, key: Option<&str>, value: Option<&str>, delete: bool) -> Result<String, String> {
    if !["local", "session", "cookie", "all"].contains(&kind) {
        return Err("kind must be local, session, cookie or all".into());
    }
    let op = match (key, value, delete) {
        (Some(_), _, true) => "del",
        (Some(_), Some(_), false) => "set",
        (Some(_), None, false) => return Err("value is required to set a key; pass delete=true to remove it".into()),
        (None, _, _) => "get",
    };
    if op != "get" && kind == "all" {
        return Err("kind must be local, session or cookie to set or delete".into());
    }
    Ok(format!(
        "(function(kind,op,key,val){{function st(n){{try{{return window[n]}}catch(e){{return null}}}}\
         function ck(){{return document.cookie?document.cookie.split('; ').map(function(s){{var i=s.indexOf('=');return i<0?[s,'']:[s.slice(0,i),s.slice(i+1)]}}):[]}}\
         function lst(n){{var s=st(n),r=[];if(s)for(var i=0;i<s.length;i++){{var k=s.key(i);r.push([k,s.getItem(k)])}}return r}}\
         try{{if(op==='set'){{if(kind==='cookie')document.cookie=key+'='+val;else st(kind+'Storage').setItem(key,val)}}\
         else if(op==='del'){{if(kind==='cookie')document.cookie=key+'=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/';else st(kind+'Storage').removeItem(key)}}}}catch(e){{return JSON.stringify({{error:String(e)}})}}\
         var o={{}};if(kind==='local'||kind==='all')o.local=lst('localStorage');if(kind==='session'||kind==='all')o.session=lst('sessionStorage');if(kind==='cookie'||kind==='all')o.cookie=ck();\
         return JSON.stringify(o)}})({},{},{},{})",
        json!(kind),
        json!(op),
        json!(key.unwrap_or("")),
        json!(value.unwrap_or(""))
    ))
}

/// The tool-list entry; tools.rs `list()` appends it.
pub fn schema() -> Value {
    let o = |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    json!({"name":"browser","description":"Bifrost Browser control and web debugging (launches the Browser if needed). One tool, `action` picks: state (engine, tabs, viewport, zoom, UA); tabs; new_tab {url, background}; select_tab/close_tab/reopen_tab {id}; navigate {url, wait=load|commit|none, timeout_ms} returns status, final url, title; back/forward/reload {hard}; eval {js} runs JS in the page and returns the result or the JS error; console {level, text, limit<=200, follow_ms}: page console messages NEW since your previous console call (cursor per session; reset=true or since_seq to rewind); network {text, follow_ms}: requests (method, url, status, type, cache, ms, size, errors) with the same cursor model (built from fetch/XHR wrappers plus resource timing, so requests made before the page was instrumented may be missing); dom {selector, mode=query|outer_html|text|attrs, limit, max_bytes<=200000}: elements with rects, or markup; screenshot {full_page, selector, rect:{x,y,w,h}, scale 0.1-4 resamples the PNG}: PNG of the PAGE (not the window), returned as an image plus a saved /tmp path; wait {for=load|selector|js|console|idle, selector, js, text, timeout_ms}; viewport {width,height,dpr,preset,reset} device emulation; zoom {factor}; find {query, dir=n|b|c}; user_agent {ua}; input {kind=click|move|key|text|scroll, x,y in page CSS px or selector to use its centre, button, key, mods, text, dx, dy}; devtools {open, panel=console|network|elements|sources|storage}; source (document HTML); storage {kind=local|session|cookie|all, key+value sets, key+delete=true removes} lists or edits the page's localStorage, sessionStorage and document.cookie (HttpOnly cookies are invisible); styles {selector}: computed display, position, box model, colour and font of the first match; inspect {x,y in page CSS px}: the element at that point (path, tag, id, class, selector, rect), selected in the Elements panel with a highlight in the page (devtools open=false panel to clear). downloads {cancel=download id, clear}: files saved to ~/Downloads with state, received and total bytes. Console entries carry source and line; history {query, limit, remove=url, clear}: visited pages newest first with visits and last-visit epoch seconds. downloads {cancel=download id, clear}: files saved to ~/Downloads with state, received and total bytes. Console entries carry source and line.. Debugging a failing request: navigate, console level=error, network text=api, eval to inspect state, screenshot. Long output is written to a /tmp file and the reply names the path. Several steps in one call: use the batch tool.",
        "inputSchema":o(json!({
            "action":{"type":"string","enum":["state","tabs","new_tab","select_tab","close_tab","reopen_tab","navigate","back","forward","reload","eval","console","network","dom","screenshot","wait","viewport","zoom","find","user_agent","input","devtools","source","inspect","downloads","storage","styles","bookmarks","history","favicon"]},
            "id":{"type":"integer","description":"tab id from tabs; default the current tab"},
            "url":{"type":"string"},"wait":{"type":"string","enum":["none","commit","load"]},"background":{"type":"boolean"},"hard":{"type":"boolean"},
            "js":{"type":"string"},"timeout_ms":{"type":"number"},
            "level":{"type":"string","enum":["error","warn","info","log","debug"]},"text":{"type":"string"},"limit":{"type":"integer"},"follow_ms":{"type":"number"},
            "since_seq":{"type":"integer"},"clear":{"type":"boolean"},"reset":{"type":"boolean"},"session":{"type":"string"},
            "selector":{"type":"string"},"mode":{"type":"string","enum":["query","outer_html","text","attrs"]},"max_bytes":{"type":"integer"},
            "full_page":{"type":"boolean"},"rect":{"type":"object"},"scale":{"type":"number"},
            "for":{"type":"string","enum":["load","selector","js","console","idle"]},
            "width":{"type":"number"},"height":{"type":"number"},"dpr":{"type":"number"},"preset":{"type":"string","description":"e.g. mobile, bifrost, desktop"},
            "factor":{"type":"number"},"query":{"type":"string"},"dir":{"type":"string","enum":["n","b","c"]},"ua":{"type":"string"},
            "kind":{"type":"string","enum":["click","move","key","text","scroll","local","session","cookie","all"],"description":"input kind, or for storage which store (default all)"},"value":{"type":"string"},"delete":{"type":"boolean"},"x":{"type":"number"},"y":{"type":"number"},"button":{"type":"string"},
            "key":{"type":"string"},"mods":{"type":"array","items":{"type":"string"}},"dx":{"type":"number"},"dy":{"type":"number"},
            "open":{"type":"boolean"},"panel":{"type":"string","enum":["console","network","elements","sources","storage"]},
            "op":{"type":"string","enum":["list","add","remove","rename"]},"title":{"type":"string"},"remove":{"type":"string","description":"history: URL to delete"}
        }),&["action"])})
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock control socket: records calls, answers from a table keyed by method.
    fn run_mock(a: Value, table: Vec<(&'static str, Value)>) -> (Result<Value, String>, Vec<(String, Value)>) {
        let mut calls = vec![];
        let r = {
            let mut rpc = |m: &str, p: &Value, _t: Duration| -> Result<Value, String> {
                calls.push((m.to_string(), p.clone()));
                table.iter().find(|(k, _)| *k == m).map(|x| x.1.clone()).ok_or_else(|| "-32601 Method not found".to_string())
            };
            let mut fb = |_: &Value| json!({"content":[{"type":"image","data":"AA==","mimeType":"image/png"}]});
            exec(&mut rpc, &mut fb, &a)
        };
        (r, calls)
    }

    fn body(v: &Value) -> String {
        v["content"][0]["text"].as_str().unwrap_or("").to_string()
    }

    #[test]
    fn navigate_reports_status() {
        let (r, c) = run_mock(json!({"action":"navigate","url":"https://x.test"}), vec![("browser.navigate", json!({"id":3,"url":"https://x.test/","title":"X","status":"loaded"}))]);
        assert_eq!(body(&r.unwrap()), "loaded https://x.test/ \"X\" (tab 3)");
        assert_eq!(c[0].1["wait"], "load");
    }

    #[test]
    fn console_cursor_advances_and_filters() {
        let e = json!({"entries":[{"seq":5,"level":"error","text":"boom","source":"a.js","line":2},{"seq":6,"level":"error","text":"other","source":"a.js","line":3}],"next_seq":7});
        let a = json!({"action":"console","session":"t1","text":"boom"});
        let (r, c) = run_mock(a.clone(), vec![("browser.console", e.clone())]);
        let t = body(&r.unwrap());
        assert!(t.contains("boom") && !t.contains("other") && t.contains("next_seq 7"));
        assert!(c[0].1.get("since_seq").is_none());
        let (_, c2) = run_mock(a, vec![("browser.console", e)]);
        assert_eq!(c2[0].1["since_seq"], 7);
    }

    #[test]
    fn storage_and_styles_go_through_eval() {
        let (r, c) = run_mock(json!({"action":"storage","kind":"local","key":"a","value":"b"}), vec![("browser.eval", json!({"ok":true,"result":"{\"local\":[]}"}))]);
        assert_eq!(body(&r.unwrap()), "{\"local\":[]}");
        assert!(c[0].1["js"].as_str().unwrap().contains("\"set\""));
        let (r, _) = run_mock(json!({"action":"storage","kind":"all","key":"a","value":"b"}), vec![]);
        assert!(r.is_err());
        let (_, c) = run_mock(json!({"action":"styles","selector":"h1"}), vec![("browser.eval", json!({"ok":true,"result":"{}"}))]);
        assert!(c[0].1["js"].as_str().unwrap().contains("getComputedStyle"));
    }

    #[test]
    fn eval_js_error_is_error_result() {
        let (r, _) = run_mock(json!({"action":"eval","js":"x"}), vec![("browser.eval", json!({"ok":false,"error":"ReferenceError: x"}))]);
        let r = r.unwrap();
        assert_eq!(r["isError"], true);
        assert!(body(&r).contains("ReferenceError"));
    }

    #[test]
    fn screenshot_returns_image_and_falls_back() {
        let shot = json!({"width":2,"height":1,"dpr":1.0,"scroll":{"x":0,"y":0},"data":"AAAA"});
        let (r, _) = run_mock(json!({"action":"screenshot"}), vec![("browser.screenshot", shot)]);
        assert_eq!(r.unwrap()["content"][0]["type"], "image");
        let (r, _) = run_mock(json!({"action":"screenshot"}), vec![]);
        assert!(r.unwrap()["content"][1]["text"].as_str().unwrap().contains("fallback"));
    }

    #[test]
    fn errors_carry_hints() {
        let (r, _) = run_mock(json!({"action":"source"}), vec![]);
        assert!(r.unwrap_err().contains("hint:"));
        assert!(hint("browser.eval", "-32001 no such tab").contains("tabs"));
        assert!(hint("browser.eval", "-32000 timeout").contains("timeout_ms"));
        assert!(hint("browser.eval", "-32002 engine not running").contains("engine"));
    }

    #[test]
    fn click_by_selector_uses_rect_centre() {
        let dom = json!({"matches":1,"items":[{"rect":{"x":10.0,"y":20.0,"w":100.0,"h":40.0}}]});
        let (r, c) = run_mock(json!({"action":"input","kind":"click","selector":"#go"}), vec![("browser.dom", dom), ("browser.input", json!({"ok":true}))]);
        r.unwrap();
        assert_eq!(c[1].1["x"], 60.0);
        assert_eq!(c[1].1["y"], 40.0);
    }

    #[test]
    fn long_output_spills_to_file() {
        let t = spill("test", "x".repeat(MAX_INLINE + 10));
        assert!(t.contains("full text in /tmp/bifrost-browser-test-"));
    }

    #[test]
    fn schema_lists_every_action() {
        let s = schema();
        assert!(s["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap().len() >= 20);
    }
}

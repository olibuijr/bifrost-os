//! App-centred helpers: the `app` tool (image results, saving, describe), app waits, process families and
//! memory, structured error codes and output caps. Pure helpers are unit-tested at the bottom.

use crate::sys::{self, RpcErr};
use crate::tools::{b64_decode, clip, f, s, text};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Largest text block returned by any tool (bytes); longer ones are cut with a note.
pub const OUT_CAP: usize = 200_000;

// ---------------------------------------------------------------- error codes

/// "[E_CODE] message".
pub fn coded(code: &str, msg: impl std::fmt::Display) -> String {
    format!("[{code}] {msg}")
}

/// Gives a plain error message a stable code prefix (already coded messages pass through).
pub fn classify(e: &str) -> String {
    if e.starts_with("[E_") {
        return e.to_string();
    }
    let l = e.to_lowercase();
    let code = if l.contains("no window matches") {
        "E_NO_WINDOW"
    } else if l.contains("no app control socket") {
        "E_NO_APP"
    } else if l.contains("timed out") || l.contains("timeout") {
        "E_TIMEOUT"
    } else if l.contains("is required") || l.starts_with("unknown") || l.starts_with("bad ") {
        "E_BAD_ARGS"
    } else if l.contains("not running") || l.contains("no reply") {
        "E_APP_DOWN"
    } else {
        "E_FAILED"
    };
    coded(code, e)
}

pub fn rpc_code(e: &RpcErr) -> &'static str {
    match e.code {
        -32601 => "E_NO_METHOD",
        -32602 => "E_BAD_PARAMS",
        -32000 => "E_TIMEOUT",
        -32001 => "E_NO_TAB",
        -32002 => "E_ENGINE_DOWN",
        0 if e.message.starts_with("not running") => "E_APP_DOWN",
        0 if e.message.contains("timed out") || e.message.contains("temporarily unavailable") => "E_TIMEOUT",
        0 => "E_APP_DOWN",
        _ => "E_RPC",
    }
}

// ---------------------------------------------------------------- pure helpers

/// Follows `a.b[2].c` (a leading `$` or `.` is ignored; array elements may also be `a.2`).
pub fn json_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let p = path.trim().trim_start_matches('$').trim_start_matches('.');
    let mut cur = v;
    if p.is_empty() {
        return Some(cur);
    }
    for seg in p.split('.') {
        let (name, idxs) = match seg.find('[') {
            Some(i) => (&seg[..i], &seg[i..]),
            None => (seg, ""),
        };
        if !name.is_empty() {
            cur = match cur {
                Value::Array(a) => a.get(name.parse::<usize>().ok()?)?,
                _ => cur.get(name)?,
            };
        }
        for part in idxs.split('[').filter(|x| !x.is_empty()) {
            cur = cur.get(part.trim_end_matches(']').parse::<usize>().ok()?)?;
        }
    }
    Some(cur)
}

/// `*` wildcard match, case-sensitive, anchored at both ends.
pub fn glob(p: &str, t: &str) -> bool {
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == t;
    }
    if !t.starts_with(parts[0]) {
        return false;
    }
    let mut pos = parts[0].len();
    for mid in &parts[1..parts.len() - 1] {
        match t[pos..].find(mid) {
            Some(i) => pos += i + mid.len(),
            None => return false,
        }
    }
    let last = parts[parts.len() - 1];
    t.len() - pos >= last.len() && t.ends_with(last)
}

/// Strings as themselves, anything else as compact JSON.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => false,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => true,
    }
}

/// Does `v` satisfy the condition in `a`: `equals` (JSON or text equality), `contains` (substring of the text),
/// `matches` (`*` glob on the text); with none of them, `v` is truthy.
pub fn value_matches(v: &Value, a: &Value) -> bool {
    let t = value_text(v);
    if let Some(e) = a.get("equals") {
        return v == e || value_text(e) == t;
    }
    if let Some(c) = s(a, "contains") {
        return t.contains(c);
    }
    if let Some(g) = s(a, "matches") {
        return glob(g, &t);
    }
    truthy(v)
}

/// An `{encoding:"base64", format, data}` payload (also under an `image` key): (base64, mime type).
pub fn find_image(v: &Value) -> Option<(&str, String)> {
    let o = if v.get("image").is_some_and(Value::is_object) { v.get("image")? } else { v };
    let data = o.get("data")?.as_str()?;
    if o.get("encoding").and_then(Value::as_str) != Some("base64") {
        return None;
    }
    let fmt = o.get("format").and_then(Value::as_str).unwrap_or("png");
    Some((data, format!("image/{}", if fmt == "jpg" { "jpeg" } else { fmt })))
}

/// The same object without its `data` field, plus the byte length of the payload.
pub fn strip_data(v: &Value) -> Value {
    let mut c = v.clone();
    if let Some(o) = c.as_object_mut() {
        if let Some(d) = o.remove("data") {
            o.insert("data_b64_chars".into(), json!(d.as_str().map(str::len).unwrap_or(0)));
        }
    }
    c
}

/// Cuts every text block of a tool result longer than `max` bytes, with a note saying how to get the rest.
pub fn cap_content(r: &mut Value, max: usize) {
    for c in r["content"].as_array_mut().into_iter().flatten() {
        if let Some(t) = c["text"].as_str() {
            if t.len() > max {
                c["text"] = json!(format!("{}\n[output truncated at {max} bytes; narrow the query, lower limit, or use `file`/`max_output`]", clip(t, max)));
            }
        }
    }
}

// ---------------------------------------------------------------- processes and memory

/// Parent pid of every process.
pub fn ppids() -> HashMap<u32, u32> {
    let mut m = HashMap::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        if let Some(pp) = stat.rsplit_once(')').and_then(|(_, r)| r.split_whitespace().nth(1)).and_then(|x| x.parse().ok()) {
            m.insert(pid, pp);
        }
    }
    m
}

/// `roots` and all their descendants.
pub fn descendants(roots: &[u32], parents: &HashMap<u32, u32>) -> HashSet<u32> {
    let mut set: HashSet<u32> = roots.iter().copied().collect();
    loop {
        let before = set.len();
        for (pid, pp) in parents {
            if set.contains(pp) {
                set.insert(*pid);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// Resolves a query to a full app id by its control socket name, else returns the query.
pub fn resolve_id(q: &str) -> String {
    let ql = q.to_lowercase();
    let socks = sys::app_sockets();
    socks.iter().find(|(id, _)| id.to_lowercase() == ql).or_else(|| socks.iter().find(|(id, _)| id.to_lowercase().contains(&ql))).map(|(id, _)| id.clone()).unwrap_or_else(|| q.to_string())
}

/// The process family of an app: every process whose command line mentions the id, and their descendants.
pub fn app_family(id: &str) -> Vec<u32> {
    let me = std::process::id();
    let roots: Vec<u32> = sys::processes().into_iter().filter(|p| p.0 != me && p.3.contains(id)).map(|p| p.0).collect();
    let mut v: Vec<u32> = descendants(&roots, &ppids()).into_iter().filter(|p| *p != me).collect();
    v.sort();
    v
}

/// (RSS KiB, PSS KiB) summed over `pids`; PSS is 0 where smaps_rollup is unreadable.
pub fn mem_of(pids: &[u32]) -> (u64, u64) {
    let procs: HashMap<u32, u64> = sys::processes().into_iter().map(|p| (p.0, p.1)).collect();
    let rss = pids.iter().filter_map(|p| procs.get(p)).sum();
    let pss = pids
        .iter()
        .map(|p| {
            sys::read(&format!("/proc/{p}/smaps_rollup")).lines().find_map(|l| l.strip_prefix("Pss:")?.trim().trim_end_matches("kB").trim().parse::<u64>().ok()).unwrap_or(0)
        })
        .sum();
    (rss, pss)
}

/// The cgroup directory of a pid and whether it is private to the app (not a shared `.service`/session scope).
pub fn cgroup_of(pid: u32) -> Option<(PathBuf, bool)> {
    let line = sys::read(&format!("/proc/{pid}/cgroup")).lines().find(|l| l.starts_with("0::"))?.trim_start_matches("0::").to_string();
    let last = line.rsplit('/').next().unwrap_or("").to_string();
    let private = !(last.ends_with(".service") || last.ends_with(".slice") || line == "/");
    Some((Path::new("/sys/fs/cgroup").join(line.trim_start_matches('/')), private))
}

/// One text line about an app: socket, pids, memory, cgroup.
pub fn app_status_line(id: &str, path: &Path) -> String {
    let up = sys::app_rpc(path, "app.info", &json!({}), Duration::from_millis(300)).is_ok();
    if !up {
        return format!("{id}: socket present, not answering");
    }
    let fam = app_family(id);
    let (rss, pss) = mem_of(&fam);
    let mut l = format!("{id}: running pids={} rss={} MiB pss={} MiB", fam.iter().map(u32::to_string).collect::<Vec<_>>().join(","), rss / 1024, pss / 1024);
    if let Some((dir, private)) = fam.first().and_then(|p| cgroup_of(*p)) {
        if private {
            if let Ok(cur) = std::fs::read_to_string(dir.join("memory.current")) {
                l += &format!(" cgroup_mem={} MiB", cur.trim().parse::<u64>().unwrap_or(0) / 1048576);
            }
        } else {
            l += " cgroup=shared";
        }
    }
    l
}

// ---------------------------------------------------------------- the `app` tool

fn find_sock(q: &str) -> Result<(String, PathBuf), String> {
    let socks = sys::app_sockets();
    let ql = q.to_lowercase();
    socks
        .iter()
        .find(|(id, _)| id.to_lowercase() == ql)
        .or_else(|| socks.iter().find(|(id, _)| id.to_lowercase().contains(&ql)))
        .cloned()
        .ok_or_else(|| coded("E_NO_APP", format!("no app control socket matches {q:?}")))
}

fn rpc_err(id: &str, method: &str, e: &RpcErr) -> String {
    coded(rpc_code(e), format!("{id} {method}: {e}"))
}

/// Methods as lines `name  signature/description`, from the entries of `app.methods` (strings or objects).
pub fn describe_methods(v: &Value, only: Option<&str>) -> String {
    let items: Vec<Value> = match v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o.get("methods").and_then(Value::as_array).cloned().unwrap_or_else(|| o.iter().map(|(k, v)| json!({"name": k, "info": v})).collect()),
        _ => vec![],
    };
    let mut out = Vec::new();
    for it in &items {
        let (name, rest) = match it {
            Value::String(s) => (s.clone(), String::new()),
            Value::Object(o) => {
                let n = o.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
                let mut r = o.clone();
                r.remove("name");
                (n, if r.is_empty() { String::new() } else { format!("  {}", Value::Object(r)) })
            }
            v => (v.to_string(), String::new()),
        };
        if only.is_none_or(|q| name.contains(q)) {
            out.push(format!("{name}{rest}"));
        }
    }
    if out.is_empty() {
        return "no methods".into();
    }
    out.join("\n")
}

/// `app`: list sockets, list/describe methods, or call one. Image results become image blocks; `file` saves.
pub fn app_tool(a: &Value) -> Result<Value, String> {
    let socks = sys::app_sockets();
    let Some(q) = s(a, "app") else {
        let mut out = String::new();
        for (id, p) in &socks {
            match sys::app_rpc(p, "app.info", &json!({}), Duration::from_millis(500)) {
                Ok(v) => out += &format!("{id} running {}\n", clip(&v.to_string(), 160)),
                Err(_) => out += &format!("{id} not running\n"),
            }
        }
        return Ok(text(if out.is_empty() { "no app control sockets".into() } else { out }));
    };
    let (id, path) = find_sock(q)?;
    let timeout = Duration::from_secs_f64(f(a, "timeout_s").unwrap_or(10.0).clamp(0.1, 120.0));
    let method = s(a, "method");
    let describe = a.get("describe").and_then(Value::as_bool).unwrap_or(false);
    if method.is_none() || describe {
        // verbose first (signatures); older apps ignore the param
        let v = sys::app_rpc_full(&path, "app.methods", &json!({"verbose": true}), timeout).or_else(|_| sys::app_rpc_full(&path, "app.methods", &json!({}), timeout)).map_err(|e| rpc_err(&id, "app.methods", &e))?;
        return Ok(text(describe_methods(&v, if describe { method } else { None })));
    }
    let method = method.unwrap();
    let params = a.get("params").cloned().unwrap_or(json!({}));
    let v = sys::app_rpc_full(&path, method, &params, timeout).map_err(|e| rpc_err(&id, method, &e))?;
    let file = s(a, "file");
    if let Some((data, mime)) = find_image(&v) {
        let bytes = b64_decode(data);
        let mut note = format!("{id} {method}: {mime} {} bytes {}", bytes.len(), strip_data(&v));
        if let Some(p) = file {
            save(p, &bytes)?;
            note += &format!("\nsaved to {p}");
        }
        return Ok(json!({"content":[{"type":"image","data":data,"mimeType":mime},{"type":"text","text":note}]}));
    }
    let body = match v {
        Value::String(s) => s,
        v => serde_json::to_string(&v).unwrap_or_default(),
    };
    if let Some(p) = file {
        save(p, body.as_bytes())?;
        return Ok(text(format!("{id} {method}: {} bytes saved to {p}\n{}", body.len(), clip(&body, 300))));
    }
    let max = f(a, "max_output").unwrap_or(100_000.0) as usize;
    if body.len() > max {
        return Ok(text(format!("{}\n[result is {} bytes, truncated at {max}; pass file=<path> to save it whole]", clip(&body, max), body.len())));
    }
    Ok(text(body))
}

fn save(path: &str, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(path, bytes).map_err(|e| coded("E_FAILED", format!("write {path}: {e}")))
}

// ---------------------------------------------------------------- wait extensions

pub const WAIT_KEYS: [&str; 5] = ["app_ready", "app_method", "log_line", "process_exit", "app_exit"];

/// `wait` on apps, journal lines and processes. Errors name the last observed value.
pub fn wait_ext(a: &Value) -> Result<Value, String> {
    let timeout = Duration::from_secs_f64(f(a, "timeout_s").unwrap_or(10.0).clamp(0.1, 300.0));
    let t = Instant::now();
    let done = |what: String| Ok(text(format!("{what} after {} ms", t.elapsed().as_millis())));
    let late = |what: &str, last: &str| coded("E_TIMEOUT", format!("timed out after {} ms waiting for {what}; last observed: {last}", t.elapsed().as_millis()));
    let pause = |ms| std::thread::sleep(Duration::from_millis(ms));
    if let Some(q) = s(a, "app_ready") {
        let mut last = String::from("no control socket");
        loop {
            if let Ok((id, p)) = find_sock(q) {
                match sys::app_rpc_full(&p, "app.info", &json!({}), Duration::from_millis(500)) {
                    Ok(_) => return done(format!("app {id} answers")),
                    Err(e) => last = format!("{id}: {e}"),
                }
            }
            if t.elapsed() >= timeout {
                return Err(late(&format!("app {q:?} to answer"), &last));
            }
            pause(100);
        }
    }
    if let Some(q) = s(a, "app_method") {
        let method = s(a, "method").ok_or_else(|| coded("E_BAD_ARGS", "app_method needs method (and optionally params, path, equals|contains|matches)"))?;
        let params = a.get("params").cloned().unwrap_or(json!({}));
        let path = s(a, "path").unwrap_or("");
        let mut last = String::from("no control socket");
        loop {
            if let Ok((id, p)) = find_sock(q) {
                match sys::app_rpc_full(&p, method, &params, Duration::from_secs(2)) {
                    Ok(v) => match json_path(&v, path) {
                        Some(x) if value_matches(x, a) => return done(format!("{id} {method} {path} = {}", clip(&value_text(x), 200))),
                        Some(x) => last = clip(&value_text(x), 300),
                        None => last = format!("path {path:?} not in {}", clip(&v.to_string(), 200)),
                    },
                    Err(e) => last = e.to_string(),
                }
            }
            if t.elapsed() >= timeout {
                return Err(late(&format!("{q} {method} {path} to match"), &last));
            }
            pause(150);
        }
    }
    if let Some(pat) = s(a, "log_line") {
        let start = sys::exec(&crate::dev::journal_cmd(&crate::dev::LogQuery { unit: None, priority: None, grep: None, subsystem: None, category: None, cursor: None, lines: 1 }), false, None, Duration::from_secs(5), None)?;
        let mut cursor = crate::dev::split_cursor(&start.out).1;
        let unit = s(a, "unit");
        let mut seen = 0usize;
        loop {
            let cmd = crate::dev::journal_cmd(&crate::dev::LogQuery { unit, priority: s(a, "priority"), grep: Some(pat), subsystem: s(a, "subsystem"), category: None, cursor: cursor.as_deref(), lines: 5 });
            let r = sys::exec(&cmd, false, None, Duration::from_secs(8), None)?;
            let (body, c) = crate::dev::split_cursor(&r.out);
            if c.is_some() {
                cursor = c;
            }
            if let Some(l) = body.lines().find(|l| !l.trim().is_empty() && !l.starts_with("-- ")) {
                return done(format!("log line matched: {}", clip(l, 300)));
            }
            seen += 1;
            if t.elapsed() >= timeout {
                return Err(late(&format!("a new journal line matching {pat:?}"), &format!("no matching line in {seen} polls")));
            }
            pause(400);
        }
    }
    if let Some(q) = s(a, "process_exit") {
        loop {
            let me = std::process::id();
            let alive: Vec<(u32, String)> = match q.parse::<u32>() {
                Ok(pid) => sys::processes().into_iter().filter(|p| p.0 == pid).map(|p| (p.0, p.3)).collect(),
                Err(_) => sys::processes().into_iter().filter(|p| p.0 != me && p.3.contains(q)).map(|p| (p.0, p.3)).collect(),
            };
            if alive.is_empty() {
                return done(format!("process {q} gone"));
            }
            if t.elapsed() >= timeout {
                return Err(late(&format!("process {q} to exit"), &alive.iter().take(5).map(|(p, c)| format!("{p} {}", clip(c, 80))).collect::<Vec<_>>().join("; ")));
            }
            pause(100);
        }
    }
    if let Some(q) = s(a, "app_exit") {
        let mut last = String::from("running");
        loop {
            match find_sock(q) {
                Err(_) => return done(format!("app {q} socket gone")),
                Ok((id, p)) => match sys::app_rpc_full(&p, "app.info", &json!({}), Duration::from_millis(400)) {
                    Err(e) if e.code == 0 => return done(format!("app {id} no longer answers ({e})")),
                    _ => last = format!("{id} still answers"),
                },
            }
            if t.elapsed() >= timeout {
                return Err(late(&format!("app {q} to exit"), &last));
            }
            pause(100);
        }
    }
    Err(coded("E_BAD_ARGS", "no wait condition"))
}

// ---------------------------------------------------------------- crash / perf by app

/// Crash picture of one app: coredumps naming it, the shell's panic log lines mentioning it, recent error
/// journal lines, and whether its process family and socket are alive.
pub fn app_crash(q: &str, frames: usize) -> Result<Value, String> {
    let id = resolve_id(q);
    let short: String = id.trim_end_matches(".desktop").rsplit('.').next().unwrap_or(&id).to_lowercase();
    let run = |c: &str| sys::exec(c, false, None, Duration::from_secs(10), None).map(|r| r.out).unwrap_or_default();
    let since = crate::dev::boot_since(&sys::read("/proc/uptime"));
    let cores = run(&format!("coredumpctl list --no-pager --no-legend --since {}", crate::tools::shq(&since)));
    let mine: Vec<&str> = cores.lines().filter(|l| l.to_lowercase().contains(&short) || l.contains(&id)).collect();
    let mut out = format!("app {id}\n");
    let fam = app_family(&id);
    out += &format!("processes: {}\n", if fam.is_empty() { "none running".into() } else { fam.iter().map(u32::to_string).collect::<Vec<_>>().join(",") });
    out += &format!("control socket: {}\n", if sys::app_sockets().iter().any(|(i, _)| *i == id) { "present" } else { "absent" });
    out += &if mine.is_empty() { format!("coredumps since {since} matching {short:?}: none\n") } else { format!("coredumps since {since} (TIME PID UID GID SIG COREFILE EXE):\n{}\n", mine.join("\n")) };
    if let Some(pid) = mine.last().and_then(|l| l.split_whitespace().find(|w| w.parse::<u32>().is_ok())) {
        let info = run(&format!("coredumpctl info --no-pager -- {pid}"));
        out += &format!("latest:\n{}\n", crate::dev::trim_info(&info, frames));
    }
    let dir = std::env::var("SPRINGBOARD_DATA").unwrap_or_else(|_| "/var/lib/springboard".into());
    let panics: Vec<String> = sys::read(&format!("{dir}/crash.log")).lines().filter(|l| l.contains(&id) || l.to_lowercase().contains(&short)).rev().take(10).map(String::from).collect();
    out += &if panics.is_empty() { "panic log: no lines for this app\n".to_string() } else { format!("panic log lines:\n{}\n", panics.join("\n")) };
    let last = sys::read(&format!("{dir}/last_exit"));
    if !last.trim().is_empty() {
        out += &format!("shell last_exit: {}\n", last.trim());
    }
    let jr = run(&format!("journalctl -b --no-pager -o short-monotonic -p warning --grep {} -n 15", crate::tools::shq(&id)));
    out += &format!("recent warnings/errors mentioning the id:\n{}", if jr.trim().is_empty() { "none".to_string() } else { jr.trim().to_string() });
    Ok(text(out.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_path_walks_objects_and_arrays() {
        let v = json!({"a":{"b":[1,{"c":"x"}]},"t":[{"n":2}]});
        assert_eq!(json_path(&v, "a.b[1].c"), Some(&json!("x")));
        assert_eq!(json_path(&v, "$.a.b.0"), Some(&json!(1)));
        assert_eq!(json_path(&v, "t[0].n"), Some(&json!(2)));
        assert_eq!(json_path(&v, ""), Some(&v));
        assert_eq!(json_path(&v, "a.z"), None);
        assert_eq!(json_path(&v, "a.b[9]"), None);
    }

    #[test]
    fn glob_matches_with_wildcards() {
        assert!(glob("load*", "loaded"));
        assert!(glob("*ed", "loaded"));
        assert!(glob("a*c*e", "abcde"));
        assert!(glob("exact", "exact"));
        assert!(!glob("exact", "exact!"));
        assert!(!glob("a*z", "abc"));
        assert!(!glob("ab*bc", "abc"));
    }

    #[test]
    fn conditions_pick_the_right_comparison() {
        assert!(value_matches(&json!("loaded"), &json!({"equals":"loaded"})));
        assert!(value_matches(&json!(3), &json!({"equals":"3"})));
        assert!(!value_matches(&json!(3), &json!({"equals":4})));
        assert!(value_matches(&json!({"x":"hello"}), &json!({"contains":"hel"})));
        assert!(value_matches(&json!("a.b"), &json!({"matches":"a.*"})));
        assert!(value_matches(&json!(1), &json!({})));
        assert!(!value_matches(&json!(0), &json!({})));
        assert!(!value_matches(&json!(null), &json!({})));
    }

    #[test]
    fn image_payloads_are_found() {
        let v = json!({"encoding":"base64","format":"jpg","data":"AAAA","width":1});
        assert_eq!(find_image(&v), Some(("AAAA", "image/jpeg".to_string())));
        assert_eq!(find_image(&json!({"image":{"encoding":"base64","data":"QQ=="}})), Some(("QQ==", "image/png".to_string())));
        assert_eq!(find_image(&json!({"data":"x"})), None);
        assert_eq!(strip_data(&v)["data_b64_chars"], 4);
        assert!(strip_data(&v).get("data").is_none());
    }

    #[test]
    fn long_text_is_truncated_with_a_note() {
        let mut r = json!({"content":[{"type":"text","text":"x".repeat(500)},{"type":"image","data":"y"}]});
        cap_content(&mut r, 100);
        let t = r["content"][0]["text"].as_str().unwrap();
        assert!(t.contains("truncated") && t.len() < 400);
        assert_eq!(r["content"][1]["data"], "y");
        let mut small = json!({"content":[{"type":"text","text":"ok"}]});
        cap_content(&mut small, 100);
        assert_eq!(small["content"][0]["text"], "ok");
    }

    #[test]
    fn errors_get_stable_codes() {
        assert!(classify("no window matches \"x\"").starts_with("[E_NO_WINDOW]"));
        assert!(classify("timed out after 5 ms").starts_with("[E_TIMEOUT]"));
        assert!(classify("command is required").starts_with("[E_BAD_ARGS]"));
        assert_eq!(classify("[E_NO_APP] x"), "[E_NO_APP] x");
        assert!(classify("boom").starts_with("[E_FAILED]"));
        let e = RpcErr { code: -32002, message: "engine".into(), data: json!({"reason":"crashed"}) };
        assert_eq!(rpc_code(&e), "E_ENGINE_DOWN");
        assert!(e.to_string().contains("(crashed)"));
    }

    #[test]
    fn process_family_follows_parents() {
        let parents: HashMap<u32, u32> = [(2, 1), (3, 2), (4, 3), (9, 8)].into_iter().collect();
        let fam = descendants(&[2], &parents);
        assert!(fam.contains(&2) && fam.contains(&3) && fam.contains(&4));
        assert!(!fam.contains(&9) && !fam.contains(&1));
    }

    #[test]
    fn methods_are_described() {
        let v = json!([{"name":"console","params":{"level":"string"}},"app.info"]);
        let d = describe_methods(&v, None);
        assert!(d.contains("console") && d.contains("level") && d.contains("app.info"));
        assert_eq!(describe_methods(&v, Some("app.")), "app.info");
        assert_eq!(describe_methods(&json!({"methods":["a"]}), None), "a");
    }
}

//! The `files` MCP tool: drive and inspect the Files app (window path, view, selection, sort, listings, file
//! operations, search, Get Info, tags) over its app control socket (`files.*` methods, docs/bifrost-mcp.md).
//! The transport is a closure so tests run against a mock socket.

use crate::sys;
use crate::tools::{s, text, Ctx};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const APP: &str = "files";
const APP_ID: &str = "is.olibuijr.files.desktop";

pub const ACTIONS: [&str; 15] = [
    "state", "list", "select", "open", "navigate", "set_view", "new_folder", "rename", "move", "copy", "trash", "undo", "search", "get_info", "tags",
];

/// Parameters each action forwards to `files.<action>`.
fn keys(action: &str) -> &'static [&'static str] {
    match action {
        "list" => &["path", "limit", "hidden", "sort"],
        "select" => &["names", "paths", "indices", "all", "add", "none"],
        "open" => &["path", "index"],
        "navigate" => &["path", "back", "forward", "up", "timeout_ms"],
        "set_view" => &["view", "sort", "hidden", "sidebar", "status_bar"],
        "new_folder" => &["name", "path"],
        "rename" => &["path", "name"],
        "move" | "copy" => &["paths", "dest", "timeout_ms"],
        "trash" => &["paths", "timeout_ms"],
        "search" => &["query", "timeout_ms"],
        "get_info" => &["path"],
        "tags" => &["path", "paths", "set", "add", "remove", "toggle"],
        _ => &[],
    }
}

/// Builds the `files.*` call for the arguments: the method name, its params and the transport timeout.
pub fn parse(a: &Value) -> Result<(String, Value, Duration), String> {
    let action = s(a, "action").ok_or_else(|| format!("action is required: {}", ACTIONS.join(" ")))?;
    if !ACTIONS.contains(&action) {
        return Err(format!("unknown action {action:?}; one of: {}", ACTIONS.join(" ")));
    }
    let mut p = serde_json::Map::new();
    for k in keys(action) {
        if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
            p.insert((*k).to_string(), v.clone());
        }
    }
    let p = Value::Object(p);
    match action {
        "rename" if p.get("name").is_none() => return Err("rename needs name (and path, default the selection)".into()),
        "move" | "copy" if p.get("dest").is_none() => return Err(format!("{action} needs dest (a folder); paths default to the selection")),
        "search" if p.get("query").is_none() => return Err("search needs query (empty string clears the search)".into()),
        _ => {}
    }
    // async calls (navigate, move, copy, trash, search) hold the socket open until the app answers
    let wait = a.get("timeout_ms").and_then(|v| v.as_f64()).unwrap_or(10_000.0).clamp(100.0, 60_000.0);
    let t = Duration::from_millis(wait as u64 + 2000);
    Ok((format!("files.{action}"), p, t))
}

/// Add a hint to a transport or JSON-RPC error message.
pub fn hint(method: &str, e: &str) -> String {
    let l = e.to_lowercase();
    let h = if l.contains("not running") || l.contains("no such file") || l.contains("refused") {
        "Files is not running; this tool launches it on the first call"
    } else if l.contains("-32601") || l.contains("method not found") {
        "this Files build lacks that control method; update Files (bifrost pc midget update)"
    } else if l.contains("-32602") || l.contains("invalid") {
        "invalid parameters; see the action's parameters in the tool description"
    } else if l.contains("-32000") || l.contains("timeout") {
        "the operation did not finish in time; raise timeout_ms and check files action=state (busy, notice)"
    } else if l.contains("-32003") || l.contains("busy") {
        "Files is running another job; wait for it (files action=state shows busy) and retry"
    } else {
        return format!("{method}: {e}");
    };
    format!("{method}: {e}\nhint: {h}")
}

pub fn run(_ctx: &mut Ctx, a: &Value) -> Result<Value, String> {
    let path = ensure_running()?;
    let mut rpc = |m: &str, p: &Value, t: Duration| sys::app_rpc_full(&path, m, p, t).map_err(|e| e.to_string());
    exec(&mut rpc, a)
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
    sys::shell_text(&format!("open-app {APP_ID}")).map_err(|e| format!("could not launch Files ({APP_ID}): {e}"))?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(15) {
        if let Some(p) = socket() {
            if sys::app_rpc(&p, "app.info", &json!({}), Duration::from_millis(500)).is_ok() {
                return Ok(p);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("Files launched ({APP_ID}) but its control socket did not answer within 15 s; check logs app={APP_ID} and crash"))
}

pub fn exec(rpc: &mut crate::browser::Rpc, a: &Value) -> Result<Value, String> {
    let (method, params, t) = parse(a)?;
    let v = rpc(&method, &params, t).map_err(|e| hint(&method, &e))?;
    let body = match v {
        Value::String(s) => s,
        v => serde_json::to_string_pretty(&v).unwrap_or_default(),
    };
    Ok(text(crate::browser::spill("files", body)))
}

pub fn schema() -> Value {
    let o = |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    json!({"name":"files","description":"Drive and inspect the Files app (launches it if needed). One tool, `action` picks: state (window path, view icon|list|columns, sort, hidden, sidebar, status bar, selection with paths, row count, busy job, notice, history); list {path, limit, hidden, sort}: entries with name, path, kind, size, mtime, tags (default the open folder's rows); select {names|paths|indices|all|none, add}; open {path|index} opens a folder (navigates) or a file (default app); navigate {path | back | forward | up, timeout_ms} waits for the listing; set_view {view=icon|list|columns, sort=name|date|size|kind, hidden, sidebar, status_bar}; new_folder {name, path}; rename {path, name} (path default the selection); move/copy {paths, dest, timeout_ms} (paths default the selection) wait for the job and report done and failed items; trash {paths, timeout_ms} moves to Recently Deleted; undo reverts the last rename, move, copy or new_folder made through this tool (trash: use Put Back in Recently Deleted); search {query, timeout_ms} searches under the open folder and waits for the hits (empty query clears); get_info {path}: Get Info rows (kind, size, dates, tags); tags {path|paths, set|add|remove|toggle = colour names} reads or changes colour tags. Long output is written to a /tmp file. Several steps in one call: use the batch tool.",
        "inputSchema":o(json!({
            "action":{"type":"string","enum":ACTIONS},
            "path":{"type":"string"},"paths":{"type":"array","items":{"type":"string"}},"dest":{"type":"string"},"name":{"type":"string"},
            "names":{"type":"array","items":{"type":"string"}},"indices":{"type":"array","items":{"type":"integer"}},"index":{"type":"integer"},
            "all":{"type":"boolean"},"none":{"type":"boolean"},"add":{"description":"select: true adds to the selection; tags: colours to add","anyOf":[{"type":"boolean"},{"type":"array","items":{"type":"string"}}]},
            "back":{"type":"boolean"},"forward":{"type":"boolean"},"up":{"type":"boolean"},
            "view":{"type":"string","enum":["icon","list","columns"]},"sort":{"type":"string","enum":["name","date","size","kind"]},
            "hidden":{"type":"boolean"},"sidebar":{"type":"boolean"},"status_bar":{"type":"boolean"},
            "limit":{"type":"integer"},"query":{"type":"string"},"timeout_ms":{"type":"number"},
            "set":{"type":"array","items":{"type":"string"}},"remove":{"type":"array","items":{"type":"string"}},"toggle":{"type":"array","items":{"type":"string"}}
        }),&["action"])})
}

#[cfg(test)]
mod tests {
    use super::*;

    type MockOut = (Result<Value, String>, Vec<(String, Value, Duration)>);

    fn mock(a: Value, reply: Value) -> MockOut {
        let mut calls = vec![];
        let r = {
            let mut rpc = |m: &str, p: &Value, t: Duration| -> Result<Value, String> {
                calls.push((m.to_string(), p.clone(), t));
                Ok(reply.clone())
            };
            exec(&mut rpc, &a)
        };
        (r, calls)
    }

    #[test]
    fn parse_maps_action_to_method_and_params() {
        let (m, p, _) = parse(&json!({"action":"move","paths":["/a/x"],"dest":"/b","junk":1})).unwrap();
        assert_eq!(m, "files.move");
        assert_eq!(p, json!({"paths":["/a/x"],"dest":"/b"}));
    }

    #[test]
    fn parse_rejects_missing_and_unknown() {
        assert!(parse(&json!({})).unwrap_err().contains("action is required"));
        assert!(parse(&json!({"action":"explode"})).unwrap_err().contains("unknown action"));
        assert!(parse(&json!({"action":"rename","path":"/a"})).unwrap_err().contains("name"));
        assert!(parse(&json!({"action":"copy"})).unwrap_err().contains("dest"));
        assert!(parse(&json!({"action":"search"})).unwrap_err().contains("query"));
    }

    #[test]
    fn parse_timeout_extends_transport() {
        let (_, _, t) = parse(&json!({"action":"navigate","path":"/","timeout_ms":5000})).unwrap();
        assert_eq!(t, Duration::from_millis(7000));
        let (_, _, t) = parse(&json!({"action":"state"})).unwrap();
        assert_eq!(t, Duration::from_millis(12_000));
    }

    #[test]
    fn exec_returns_pretty_json() {
        let (r, c) = mock(json!({"action":"state"}), json!({"path":"/home/u","view":"icon"}));
        assert_eq!(c[0].0, "files.state");
        assert!(r.unwrap()["content"][0]["text"].as_str().unwrap().contains("\"view\": \"icon\""));
    }

    #[test]
    fn errors_carry_hints() {
        assert!(hint("files.move", "-32003 busy").contains("another job"));
        assert!(hint("files.x", "-32601 Method not found").contains("lacks"));
    }

    #[test]
    fn schema_lists_every_action() {
        let sc = schema();
        assert_eq!(sc["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap().len(), ACTIONS.len());
    }
}

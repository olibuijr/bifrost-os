//! Control methods for Notes (tasks/app-control.md). Read-only for now: a write
//! would race the open editor's autosave, so writes wait for the main-thread path.

use super::{folders, store};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_core::json::Json;
use std::path::Path;

pub fn methods() -> Vec<&'static str> {
    vec!["notes.list", "notes.get"]
}

/// Answers one call. `dir` is the app's data directory (`Ctx::data_dir`).
pub fn call(dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
    match method {
        "notes.list" => Ok(list(dir, params)),
        "notes.get" => get(dir, params),
        _ => Err(RpcError::new(bifrost_app::control::METHOD_NOT_FOUND, format!("no method {method}"))),
    }
}

/// `folder` absent = every note; "" = the root only; a name = that folder. `query` filters by title or body.
fn list(dir: &Path, params: &Json) -> Json {
    let folder = params.get("folder").and_then(Json::as_str);
    let query = params.get("query").and_then(Json::as_str).unwrap_or("");
    let mut rows: Vec<(Option<String>, store::Note)> = folders::load_all(dir)
        .into_iter()
        .filter(|(f, _)| match folder {
            None => true,
            Some("") => f.is_none(),
            Some(name) => f.as_deref() == Some(name),
        })
        .filter(|(_, n)| query.is_empty() || n.matches(query))
        .collect();
    rows.sort_by(|a, b| b.1.modified.cmp(&a.1.modified));
    Json::Arr(rows.iter().map(|(f, n)| summary(f.as_deref(), n)).collect())
}

fn get(dir: &Path, params: &Json) -> Result<Json, RpcError> {
    let id = params.get("id").and_then(Json::as_str).ok_or_else(|| RpcError::new(INVALID_PARAMS, "id is required"))?;
    let (folder, note) = folders::load_all(dir)
        .into_iter()
        .find(|(_, n)| n.id == id)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("no note {id}")))?;
    let mut fields = match summary(folder.as_deref(), &note) {
        Json::Obj(f) => f,
        _ => Vec::new(),
    };
    fields.push(("text".into(), Json::Str(note.text.clone())));
    Ok(Json::Obj(fields))
}

fn summary(folder: Option<&str>, n: &store::Note) -> Json {
    Json::Obj(vec![
        ("id".into(), Json::Str(n.id.clone())),
        ("title".into(), Json::Str(n.plain_title())),
        ("snippet".into(), Json::Str(springboard::shell::md::plain_line(n.snippet()))),
        ("folder".into(), folder.map_or(Json::Null, |f| Json::Str(f.to_string()))),
        ("modified".into(), Json::Num(n.modified as f64)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("notes-rpc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn obj(pairs: &[(&str, &str)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| ((*k).into(), Json::Str((*v).into()))).collect())
    }

    #[test]
    fn lists_filters_and_gets_notes() {
        let dir = scratch("basic");
        let old = store::Note { id: "100".into(), text: "# Groceries\nmilk".into(), modified: 10 };
        let new = store::Note { id: "200".into(), text: "Trip\npack bags".into(), modified: 20 };
        store::save(&dir, &old).unwrap();
        store::save(&dir, &new).unwrap();

        let all = call(&dir, "notes.list", &Json::Obj(vec![])).unwrap();
        let ids: Vec<&str> = all.as_array().unwrap().iter().map(|n| n.get("id").unwrap().as_str().unwrap()).collect();
        assert_eq!(ids, ["200", "100"], "newest first");

        let hits = call(&dir, "notes.list", &obj(&[("query", "milk")])).unwrap();
        assert_eq!(hits.as_array().unwrap().len(), 1);
        assert_eq!(hits.as_array().unwrap()[0].get("title").unwrap().as_str(), Some("Groceries"));

        let one = call(&dir, "notes.get", &obj(&[("id", "100")])).unwrap();
        assert_eq!(one.get("text").unwrap().as_str(), Some("# Groceries\nmilk"));
        assert!(call(&dir, "notes.get", &obj(&[("id", "999")])).is_err());
        assert_eq!(call(&dir, "notes.get", &Json::Obj(vec![])).unwrap_err().code, INVALID_PARAMS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

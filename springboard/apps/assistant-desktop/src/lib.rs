//! Assistant: a chat client for the Anthropic Messages API, used by the desktop app in `main.rs`.
pub mod api;
pub mod connect;
pub mod model;
pub mod safety;
pub mod session;
pub mod tools;
pub mod view;

use bifrost_core::json::Json;

/// Control socket methods (`bifrost pc control call is.olibuijr.assistant.desktop conversations.list`).
pub fn rpc_methods() -> Vec<&'static str> {
    vec!["conversations.list"]
}

/// `conversations.list`: id, title, updated and message count of every saved conversation, newest first.
pub fn rpc_call(dir: &std::path::Path, method: &str) -> Option<Json> {
    (method == "conversations.list").then(|| {
        Json::Arr(
            model::load(dir)
                .into_iter()
                .map(|c| {
                    Json::Obj(vec![
                        ("id".into(), Json::Str(c.id)),
                        ("title".into(), Json::Str(c.title)),
                        ("updated".into(), Json::Num(c.updated as f64)),
                        ("messages".into(), Json::Num(c.messages.len() as f64)),
                    ])
                })
                .collect(),
        )
    })
}

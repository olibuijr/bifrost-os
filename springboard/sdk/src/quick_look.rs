//! preview panel service client: the typed side of the shell's `quick-look` service (`springboard/src/shell/services/quick_look.rs`)
//! for the floating preview panel (`QLPreviewPanel`).
//!
//! Two ways to show it. [`peek::PreviewPanel`](crate::peek::PreviewPanel) sends `ToShell::Preview`, and the shell draws the
//! panel for the app that sent it. The calls here do the same through the service, and also let the app read the state
//! back and hear what the user did:
//!
//! ```ignore
//! use bifrost_app::quick_look::{self, Event};
//! quick_look::show(ctx, &items, 0);            // answered by App::service_reply
//! // later, in App::service_event(ctx, service, event, body):
//! if let Some(Event::Index(i)) = quick_look::event(event, body) { /* the user stepped to i */ }
//! ```
//!
//! Events arrive only for the app that opened the panel: [`Event::Closed`] when the user closed it (Esc, Space or the
//! close button) and [`Event::Index`] when the user stepped. Shell calls answer with the state (see [`State`]).
use crate::proto::peek::{PreviewItem, MAX_ITEMS};
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "quick-look";

/// The control method an app answers to draw a preview of its own file type. The app declares the type in its manifest
/// (`[[preview]]` with `generator = "preview.render"`, `types` and `timeout_ms`); the shell calls it with
/// `{"file": "<absolute path>", "width": w, "height": h}` and draws the PNG named in the reply. A late, missing or
/// error reply falls back to the built-in preview. Answer with [`picture_reply`].
pub const PREVIEW_METHOD: &str = "preview.render";

/// The reply to [`PREVIEW_METHOD`]: `{"picture": "<absolute path to a PNG>"}`.
pub fn picture_reply(path: &str) -> Json {
    Json::Obj(vec![("picture".into(), Json::Str(path.to_string()))])
}

/// `show` params: `{items: [{url, title}], index}`.
pub fn show_body(items: &[PreviewItem], index: usize) -> Json {
    let list = items
        .iter()
        .take(MAX_ITEMS)
        .map(|it| Json::Obj(vec![("url".into(), Json::Str(it.url.clone())), ("title".into(), Json::Str(it.title.clone()))]))
        .collect();
    Json::Obj(vec![("items".into(), Json::Arr(list)), ("index".into(), Json::Num(index as f64))])
}

/// Shows the panel with `items`, starting at `index`. Returns the request id; the reply is the new [`State`].
pub fn show(ctx: &Ctx, items: &[PreviewItem], index: usize) -> u32 {
    service::call(ctx, SERVICE, "show", &show_body(items, index))
}

/// Hides the panel if this app opened it. Returns the request id.
pub fn close(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "close", &Json::Obj(Vec::new()))
}

/// Asks for the panel state. Returns the request id.
pub fn state(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "state", &Json::Obj(Vec::new()))
}

/// Moves the index by `delta` without wrapping. Returns the request id.
pub fn step(ctx: &Ctx, delta: i32) -> u32 {
    service::call(ctx, SERVICE, "step", &Json::Obj(vec![("delta".into(), Json::Num(delta as f64))]))
}

/// The panel items for local files: each absolute path becomes a `file://` item titled with its file name. Relative
/// paths and paths with control characters are left out. `index` follows the kept items: it moves down past each
/// dropped path before it, and is clamped to the list.
pub fn file_items(paths: &[&str], index: usize) -> (Vec<PreviewItem>, usize) {
    let mut items = Vec::new();
    let mut kept_index = 0;
    for (i, p) in paths.iter().enumerate() {
        if !p.starts_with('/') || p.chars().any(|c| c.is_control()) {
            continue;
        }
        if i < index {
            kept_index += 1;
        }
        let title = p.rsplit('/').next().unwrap_or("").to_string();
        items.push(PreviewItem::new(format!("file://{p}"), title));
    }
    let last = items.len().saturating_sub(1);
    (items, kept_index.min(last))
}

/// Shows the panel for local files, starting at `index` (see [`file_items`]). Returns the request id, or None when no
/// path is usable (the panel is not opened). The reply is the new [`State`].
pub fn preview(ctx: &Ctx, paths: &[&str], index: usize) -> Option<u32> {
    let (items, index) = file_items(paths, index);
    if items.is_empty() {
        return None;
    }
    Some(show(ctx, &items, index))
}

/// The panel state, as the shell answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub visible: bool,
    pub index: usize,
    pub count: usize,
}

/// Reads a state reply (`{visible, index, count}`). None when the reply has another shape.
pub fn parse_state(reply: &Json) -> Option<State> {
    let visible = match reply.get("visible") {
        Some(Json::Bool(b)) => *b,
        _ => return None,
    };
    let index = reply.get("index")?.as_u64()? as usize;
    let count = reply.get("count")?.as_u64()? as usize;
    Some(State { visible, index, count })
}

/// What the user did in the panel, as the shell reports it to the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user closed the panel.
    Closed,
    /// The user stepped to this index.
    Index(usize),
}

/// Reads a `quick-look` service event (`App::service_event`). None for other services' events.
pub fn event(name: &str, body: &Json) -> Option<Event> {
    match name {
        "closed" => Some(Event::Closed),
        "index" => Some(Event::Index(body.get("index")?.as_u64()? as usize)),
        _ => None,
    }
}

/// Whether a `service_event` call is for this service.
pub fn is_quick_look(service: &str) -> bool {
    service == SERVICE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ToShell;

    fn item(url: &str, title: &str) -> PreviewItem {
        PreviewItem { url: url.into(), title: title.into() }
    }

    #[test]
    fn show_body_carries_items_and_index() {
        let body = show_body(&[item("file:///a.txt", "A"), item("file:///b.png", "")], 1);
        let arr = match body.get("items") {
            Some(Json::Arr(a)) => a,
            _ => panic!("items array expected"),
        };
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].get("title").and_then(Json::as_str), Some("A"));
        assert_eq!(body.get("index").and_then(Json::as_u64), Some(1));
    }

    #[test]
    fn show_body_caps_the_list_at_the_wire_maximum() {
        let many: Vec<PreviewItem> = (0..MAX_ITEMS + 5).map(|i| item(&format!("file:///{i}"), "")).collect();
        match show_body(&many, 0).get("items") {
            Some(Json::Arr(a)) => assert_eq!(a.len(), MAX_ITEMS),
            _ => panic!("items array expected"),
        }
    }

    #[test]
    fn calls_go_to_the_quick_look_service() {
        let ctx = crate::runtime::test_ctx();
        let a = show(&ctx, &[item("file:///a", "a")], 0);
        let b = step(&ctx, -1);
        assert_ne!(a, b);
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 2);
        match &sent[1] {
            ToShell::ServiceCall { service, method, body, .. } => {
                assert_eq!(service, "quick-look");
                assert_eq!(method, "step");
                assert_eq!(Json::Obj(vec![("delta".into(), Json::Num(-1.0))]).encode(), *body);
            }
            _ => panic!("service call expected"),
        }
    }

    #[test]
    fn file_paths_become_file_items_titled_by_name() {
        let (items, index) = file_items(&["/home/u/a.txt", "/home/u/pics/b.png"], 1);
        assert_eq!(items, vec![item("file:///home/u/a.txt", "a.txt"), item("file:///home/u/pics/b.png", "b.png")]);
        assert_eq!(index, 1);
    }

    #[test]
    fn relative_and_control_character_paths_are_dropped_and_the_index_follows() {
        let (items, index) = file_items(&["rel.txt", "/x/keep.txt", "/x/bad\nname", "/x/last.txt"], 3);
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["keep.txt", "last.txt"]);
        assert_eq!(index, 1, "the index stays on last.txt, which is now the second kept item");
        let (_, index) = file_items(&["rel", "/x/only"], 0);
        assert_eq!(index, 0);
    }

    #[test]
    fn an_index_past_the_end_is_clamped_to_the_last_item() {
        let (items, index) = file_items(&["/x/a", "/x/b"], 9);
        assert_eq!(items.len(), 2);
        assert_eq!(index, 1);
    }

    #[test]
    fn preview_sends_nothing_when_no_path_is_usable() {
        let ctx = crate::runtime::test_ctx();
        assert_eq!(preview(&ctx, &["relative"], 0), None);
        assert!(ctx.take_sent().is_empty());
        assert!(preview(&ctx, &["/x/a.png"], 0).is_some());
        assert_eq!(ctx.take_sent().len(), 1);
    }

    #[test]
    fn state_reply_parses_and_rejects_other_shapes() {
        let reply = Json::Obj(vec![("visible".into(), Json::Bool(true)), ("index".into(), Json::Num(2.0)), ("count".into(), Json::Num(5.0))]);
        assert_eq!(parse_state(&reply), Some(State { visible: true, index: 2, count: 5 }));
        assert_eq!(parse_state(&Json::Null), None);
    }

    #[test]
    fn a_picture_reply_names_the_png_and_the_method_is_the_declared_one() {
        assert_eq!(picture_reply("/tmp/p.png").get("picture").and_then(Json::as_str), Some("/tmp/p.png"));
        assert_eq!(PREVIEW_METHOD, "preview.render");
    }

    #[test]
    fn events_are_read_for_this_service_only() {
        assert_eq!(event("closed", &Json::Null), Some(Event::Closed));
        let body = Json::Obj(vec![("index".into(), Json::Num(3.0))]);
        assert_eq!(event("index", &body), Some(Event::Index(3)));
        assert_eq!(event("index", &Json::Null), None);
        assert_eq!(event("unknown", &Json::Null), None);
        assert!(is_quick_look("quick-look"));
        assert!(!is_quick_look("system"));
    }
}

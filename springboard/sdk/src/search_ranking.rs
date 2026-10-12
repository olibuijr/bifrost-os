//! Ranked search over the items this app indexed with Core Search, item expiry, and language identification. The
//! shell side is the `search-index` service (`springboard/src/shell/services/search_ranking.rs`), reached through
//! [`crate::service`]. The expiry is the indexed item's expiration date, and the language is its dominant language.
//!
//! ```ignore
//! let id = search_ranking::search(ctx, "milk", 5);
//! // in App::service_reply, for that id:
//! //   let hits = search_ranking::parse_hits(&result?);
//! ```
//! The replies come back like every service reply: `App::service_reply`, or [`crate::service::take_reply`].
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "search-index";

/// One result of [`search`]: an item of this app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// the item's `uniqueIdentifier`
    pub id: String,
    /// the item's `domainIdentifier`
    pub domain: String,
    pub title: String,
    /// the `uttype` identifier of the item's file ("com.adobe.pdf"), from its content URL; `None` when unknown
    pub kind: Option<String>,
}

/// Searches this app's indexed items for `query`: every word must match. A file type word filters by type ("pdf"),
/// and a time phrase filters by when the item was indexed ("last week", "yesterday", "past 3 days"). At most `limit`.
pub fn search(ctx: &Ctx, query: &str, limit: usize) -> u32 {
    service::call(ctx, SERVICE, "search", &Json::Obj(vec![("query".into(), Json::Str(query.into())), ("limit".into(), Json::Num(limit as f64))]))
}

/// Sets the time (Unix seconds) after which the indexed item `id` is dropped; `None` clears the expiry.
pub fn expire(ctx: &Ctx, id: &str, at: Option<u64>) -> u32 {
    let at = at.map_or(Json::Null, |t| Json::Num(t as f64));
    service::call(ctx, SERVICE, "expire", &Json::Obj(vec![("id".into(), Json::Str(id.into())), ("at".into(), at)]))
}

/// Identifies the dominant language of `text`; the reply is its BCP 47 code, or null.
pub fn language(ctx: &Ctx, text: &str) -> u32 {
    service::call(ctx, SERVICE, "language", &Json::Obj(vec![("text".into(), Json::Str(text.into()))]))
}

/// The hits of a [`search`] reply. A reply that is not a list of hits gives none.
pub fn parse_hits(reply: &Json) -> Vec<Hit> {
    let Some(list) = reply.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|h| {
            Some(Hit {
                id: h.get("id")?.as_str()?.to_string(),
                domain: h.get("domain").and_then(Json::as_str).unwrap_or_default().to_string(),
                title: h.get("title").and_then(Json::as_str).unwrap_or_default().to_string(),
                kind: h.get("kind").and_then(Json::as_str).map(str::to_string),
            })
        })
        .collect()
}

/// The BCP 47 code of a [`language`] reply, or `None` when no language was recognised.
pub fn parse_language(reply: &Json) -> Option<String> {
    reply.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ToShell;

    fn test_ctx() -> Ctx {
        use crate::kit::{Font, Ui};
        use crate::proto::Configure;
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    #[test]
    fn calls_go_to_the_search_index_service_with_their_method_and_body() {
        let ctx = test_ctx();
        let a = search(&ctx, "milk", 5);
        let b = expire(&ctx, "n1", Some(1_800_000_000));
        let c = expire(&ctx, "n1", None);
        let d = language(&ctx, "Mjólk");
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 4);
        let body = |i: usize| match &sent[i] {
            ToShell::ServiceCall { service, method, body, .. } => (service.clone(), method.clone(), bifrost_core::json::parse(body).unwrap()),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(body(0).0, "search-index");
        assert_eq!(body(0).1, "search");
        assert_eq!(body(0).2.get("query").and_then(Json::as_str), Some("milk"));
        assert_eq!(body(1).2.get("at").and_then(Json::as_u64), Some(1_800_000_000));
        assert_eq!(body(2).2.get("at"), Some(&Json::Null));
        assert_eq!(body(3).2.get("text").and_then(Json::as_str), Some("Mjólk"));
        assert!(a != b && b != c && c != d);
        if let ToShell::ServiceCall { request, .. } = &sent[0] {
            assert_eq!(*request, a);
        }
    }

    #[test]
    fn hits_and_languages_decode_from_replies() {
        let reply = bifrost_core::json::parse(r#"[{"id":"n1","domain":"notes","title":"Milk"},{"title":"no id"},{"id":"n2"}]"#).unwrap();
        assert_eq!(parse_hits(&reply), vec![Hit { id: "n1".into(), domain: "notes".into(), title: "Milk".into(), kind: None }, Hit { id: "n2".into(), domain: String::new(), title: String::new(), kind: None }]);
        let typed = bifrost_core::json::parse(r#"[{"id":"r1","domain":"files","title":"Q1 report","kind":"com.adobe.pdf"}]"#).unwrap();
        assert_eq!(parse_hits(&typed)[0].kind.as_deref(), Some("com.adobe.pdf"));
        assert!(parse_hits(&Json::Null).is_empty());
        assert_eq!(parse_language(&Json::Str("is".into())), Some("is".to_string()));
        assert_eq!(parse_language(&Json::Null), None);
    }
}

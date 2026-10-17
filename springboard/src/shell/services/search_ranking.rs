//! The `search-index` service: ranked search over the items an app indexed with Core Search, item expiry, and the
//! language of a text. Apps reach it through `bifrost_app::search_ranking`.
//!
//! * `search {query, limit}` answers `[{id, domain, title, kind}]`: the calling app's own items, ranked by [`spotindex`]
//!   (every word must match, language-aware tie-break). `kind` is the `uttype` identifier of the item's file (from its
//!   content URL extension) or null. The query may filter: "pdf" keeps PDFs, "last week" keeps items indexed last week,
//!   and "pdf from last week" keeps both. `limit` is at most [`MAX_LIMIT`].
//! * `expire {id, at}` sets the Unix time (seconds) after which the item is dropped; `at: null` clears it. Answers
//!   true when the app has an item with that identifier.
//! * `language {text}` answers the BCP 47 code of the text's dominant language, or null when none is recognisable.
//! * `index_named {index, protection, items}` indexes items into the named index `index` of the app (its protection
//!   class is `protection`, or null); answers the count sent. `items` are `{id, domain, title, desc, keywords, public,
//!   expires}`.
//! * `delete_named {index, kind, values}` deletes by `kind` "ids", "domains" or "all"; `delete_named_index {index}`
//!   removes the whole index. Answers true.
//! * `search_named {index, query, limit}` answers `[{id, domain}]` for the app's own named index.
//!
//! Expired items are dropped on the shell tick (at most once a second) and before every search, so an expired item
//! never shows in the Search panel.
use super::{Event, Shell, SystemService};
use crate::shell::spotindex;
use bifrost_core::json::Json;
use bifrost_proto::quicksearch::{DELETE_ALL, DELETE_DOMAINS, DELETE_IDS};
use bifrost_ui::language::dominant_language;
use bifrost_ui::searchindex::{IndexDescription, ProtectionClass};

/// The most results one `search` call returns.
pub const MAX_LIMIT: u64 = 50;
/// The default `limit` when the app gives none.
pub const DEFAULT_LIMIT: u64 = 10;

#[derive(Default)]
pub struct SearchRanking {
    /// the shell time (seconds) from which the next expiry sweep runs
    next_sweep: f64,
}

impl SystemService for SearchRanking {
    fn name(&self) -> &'static str {
        "search-index"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "search" => {
                let query = params.get("query").and_then(Json::as_str).ok_or("search needs a query string")?;
                let limit = params.get("limit").and_then(Json::as_u64).unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
                let hits = spotindex::search_app(app_id, query, limit);
                Ok(Json::Arr(
                    hits.into_iter()
                        .map(|h| {
                            let kind = h.kind.map_or(Json::Null, |k| Json::Str(k.into()));
                            Json::Obj(vec![("id".into(), Json::Str(h.unique_id)), ("domain".into(), Json::Str(h.domain)), ("title".into(), Json::Str(h.title)), ("kind".into(), kind)])
                        })
                        .collect(),
                ))
            }
            "expire" => {
                let id = params.get("id").and_then(Json::as_str).ok_or("expire needs an item id")?;
                let at = match params.get("at") {
                    None | Some(Json::Null) => None,
                    Some(v) => Some(v.as_u64().ok_or("at must be a Unix time in seconds")?),
                };
                Ok(Json::Bool(spotindex::set_expiry(app_id, id, at)))
            }
            "language" => {
                let text = params.get("text").and_then(Json::as_str).ok_or("language needs a text string")?;
                Ok(dominant_language(text).map_or(Json::Null, |l| Json::Str(l.code().into())))
            }
            "index_named" => {
                let index = params.get("index").and_then(Json::as_str).ok_or("index_named needs an index name")?;
                let protection = params.get("protection").and_then(Json::as_str).and_then(ProtectionClass::from_raw);
                let list = params.get("items").and_then(Json::as_array).ok_or("index_named needs an items list")?;
                let desc = IndexDescription { name: index.to_string(), protection_class: protection };
                let items = spotindex::parse_named_items(list);
                let count = items.len();
                spotindex::index_named(app_id, &desc, items).map_err(|e| format!("bad index name: {e:?}"))?;
                Ok(Json::Num(count as f64))
            }
            "delete_named" => {
                let index = params.get("index").and_then(Json::as_str).ok_or("delete_named needs an index name")?;
                let kind = match params.get("kind").and_then(Json::as_str) {
                    Some("ids") => DELETE_IDS,
                    Some("domains") => DELETE_DOMAINS,
                    Some("all") => DELETE_ALL,
                    _ => return Err("delete_named needs kind ids, domains or all".into()),
                };
                let values: Vec<&str> = params.get("values").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).collect()).unwrap_or_default();
                spotindex::delete_named(app_id, index, kind, &values);
                Ok(Json::Bool(true))
            }
            "delete_named_index" => {
                let index = params.get("index").and_then(Json::as_str).ok_or("delete_named_index needs an index name")?;
                Ok(Json::Bool(spotindex::delete_named_index(app_id, index)))
            }
            "search_named" => {
                let index = params.get("index").and_then(Json::as_str).ok_or("search_named needs an index name")?;
                let query = params.get("query").and_then(Json::as_str).ok_or("search_named needs a query string")?;
                let limit = params.get("limit").and_then(Json::as_u64).unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
                let hits = spotindex::search_named(app_id, index, query, limit).map_err(|e| format!("query refused: {e:?}"))?;
                Ok(Json::Arr(
                    hits.into_iter().map(|h| Json::Obj(vec![("id".into(), Json::Str(h.unique_identifier)), ("domain".into(), Json::Str(h.domain_identifier))])).collect(),
                ))
            }
            _ => Err(format!("search-index has no method {method}")),
        }
    }

    fn tick(&mut self, _shell: &mut Shell, t: f64) -> Vec<Event> {
        if t >= self.next_sweep {
            self.next_sweep = t + 1.0;
            spotindex::expire_due();
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_is_named_search_index_and_limits_are_sane() {
        assert_eq!(SearchRanking::default().name(), "search-index");
        assert!(DEFAULT_LIMIT <= MAX_LIMIT);
    }
}

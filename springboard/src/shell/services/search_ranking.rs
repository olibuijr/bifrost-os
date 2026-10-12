//! The `search-index` service: ranked search over the items an app indexed with Core Search, item expiry, and the
//! language of a text. Apps reach it through `bifrost_app::search_ranking`.
//!
//! * `search {query, limit}` answers `[{id, domain, title}]`: the calling app's own items, ranked by [`spotindex`]
//!   (every word must match, language-aware tie-break). `limit` is at most [`MAX_LIMIT`].
//! * `expire {id, at}` sets the Unix time (seconds) after which the item is dropped; `at: null` clears it. Answers
//!   true when the app has an item with that identifier.
//! * `language {text}` answers the BCP 47 code of the text's dominant language, or null when none is recognisable.
//!
//! Expired items are dropped on the shell tick (at most once a second) and before every search, so an expired item
//! never shows in the Search panel.
use super::{Event, Shell, SystemService};
use crate::shell::spotindex;
use bifrost_core::json::Json;
use bifrost_ui::language::dominant_language;

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
                        .map(|h| Json::Obj(vec![("id".into(), Json::Str(h.unique_id)), ("domain".into(), Json::Str(h.domain)), ("title".into(), Json::Str(h.title))]))
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

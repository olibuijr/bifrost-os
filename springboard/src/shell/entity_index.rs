//! Entities of apps in Search. An app declares the entity types it offers with `[[entity]] type = "Note"` in its
//! manifest. When the app starts, the shell asks it for the suggested entities of each type (`entity.query` with
//! `"suggested": true` on the app's control socket) and indexes them as Core Search items in the domain
//! `entity.<type>`, through the same `spotindex` store an app's own `CSSearchableIndex` calls use. Search then finds
//! them without the app running, and opening one sends the app the same continue-activity message as any Search item.
//! The app decides what `suggested` returns, so an entity it wants kept out of Search is left out there.

use super::intents;
use super::spotindex;
use bifrost_core::json::{self, Json};
use bifrost_proto::quicksearch::{SearchableItem, DELETE_DOMAINS, MAX_DESC, MAX_TITLE};
use bifrost_proto::ToShell;
use std::time::Duration;

/// How long a refresh waits for the app's control socket to appear (the app is still starting).
const WAIT: Duration = Duration::from_secs(8);

/// The domain of the items of entity type `ty`.
pub fn domain_of(ty: &str) -> String {
    format!("entity.{ty}")
}

/// The `entity.query` request line for the suggested entities of type `ty`.
pub fn suggested_request(ty: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"entity.query\",\"params\":{{\"type\":{},\"suggested\":true}}}}\n", json::quote(ty))
}

/// The Search items of an `entity.query` reply for type `ty`: one per entity with an id and a title, identified as
/// `<type>/<id>` (the same identifier the SDK's `IndexedEntity` uses). Titles and subtitles are cut to the wire caps.
/// An entity the app marks `"hidden": true` (`IndexedEntity::hide_in_spotlight`) is left out.
pub fn items_from_reply(ty: &str, reply: &str) -> Result<Vec<SearchableItem>, String> {
    let v = json::parse(reply.trim()).map_err(|e| format!("malformed reply: {e}"))?;
    if let Some(e) = v.get("error") {
        return Err(e.str_field("message").unwrap_or_else(|| "error".into()));
    }
    let entities = v.get("result").and_then(Json::as_array).ok_or("the reply has no result list")?;
    let domain = domain_of(ty);
    Ok(entities
        .iter()
        .filter(|e| !matches!(e.get("hidden"), Some(Json::Bool(true))))
        .filter_map(|e| {
            let id = e.str_field("id").filter(|id| !id.is_empty())?;
            let title = e.str_field("title").filter(|t| !t.trim().is_empty())?;
            Some(SearchableItem {
                unique_id: format!("{ty}/{id}"),
                domain: domain.clone(),
                title: title.chars().take(MAX_TITLE).collect(),
                description: e.str_field("subtitle").unwrap_or_default().chars().take(MAX_DESC).collect(),
                ..Default::default()
            })
        })
        .collect())
}

/// Asks the app for each declared type and replaces that type's items in the index. Blocking: run it off the UI thread.
/// A type whose request fails keeps the items it had.
pub fn refresh(app_id: &str, types: &[String]) {
    for ty in types {
        let Ok(reply) = intents::exchange(app_id, &suggested_request(ty), WAIT) else { continue };
        let Ok(items) = items_from_reply(ty, &reply) else { continue };
        spotindex::apply(app_id, ToShell::QuickSearchDelete { kind: DELETE_DOMAINS, values: vec![domain_of(ty)] });
        if !items.is_empty() {
            spotindex::apply(app_id, ToShell::QuickSearchIndex(items));
        }
    }
}

/// Starts `refresh` for an app that has just started, on its own thread. Does nothing for an app with no entity types.
pub fn start(app_id: &str, types: &[String]) {
    if types.is_empty() {
        return;
    }
    let (app_id, types) = (app_id.to_string(), types.to_vec());
    let _ = crate::bare::sys::spawn_named("entity-index", move || refresh(&app_id, &types));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_the_type_and_asks_for_suggestions() {
        assert_eq!(suggested_request("Note"), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"entity.query\",\"params\":{\"type\":\"Note\",\"suggested\":true}}\n");
        assert!(suggested_request("a\"b").contains("\"type\":\"a\\\"b\""));
    }

    #[test]
    fn each_entity_becomes_an_item_in_its_type_domain() {
        let reply = r#"{"jsonrpc":"2.0","id":1,"result":[{"id":"n1","type":"Note","title":"Groceries","subtitle":"Milk, eggs"},{"id":"n2","type":"Note","title":"Ideas"}]}"#;
        let items = items_from_reply("Note", reply).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].unique_id, "Note/n1");
        assert_eq!(items[0].domain, "entity.Note");
        assert_eq!(items[0].title, "Groceries");
        assert_eq!(items[0].description, "Milk, eggs");
        assert_eq!(items[1].description, "", "no subtitle is an empty description");
    }

    #[test]
    fn entities_without_an_id_or_title_are_skipped_and_long_text_is_cut() {
        let long = "x".repeat(400);
        let reply = format!(r#"{{"result":[{{"id":"","title":"No id"}},{{"id":"a","title":"  "}},{{"id":"b","title":"{long}"}}]}}"#);
        let items = items_from_reply("Note", &reply).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].unique_id, "Note/b");
        assert_eq!(items[0].title.chars().count(), MAX_TITLE);
    }

    #[test]
    fn an_entity_marked_hidden_is_not_indexed() {
        let reply = r#"{"result":[{"id":"n1","title":"Visible"},{"id":"n2","title":"Private","hidden":true},{"id":"n3","title":"Also visible","hidden":false}]}"#;
        let items = items_from_reply("Note", reply).unwrap();
        let ids: Vec<&str> = items.iter().map(|i| i.unique_id.as_str()).collect();
        assert_eq!(ids, vec!["Note/n1", "Note/n3"]);
    }

    #[test]
    fn an_error_or_a_missing_result_is_refused() {
        assert_eq!(items_from_reply("Note", r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no entities"}}"#), Err("no entities".into()));
        assert!(items_from_reply("Note", r#"{"jsonrpc":"2.0","id":1}"#).is_err());
        assert!(items_from_reply("Note", "nonsense").is_err());
    }
}

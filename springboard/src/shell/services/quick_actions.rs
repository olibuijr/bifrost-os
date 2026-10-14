//! The `quick_actions` service: an app's dynamic Home Screen quick actions. `set` {"items": [...]} replaces the app's list
//! (each item has `type`, `title`, and optionally `subtitle`, `symbol` and `userInfo`, an object of strings), `clear`
//! removes it, and `state` returns the count. The list is kept on the app's host (`AppHost::quick_actions`) and shown
//! with the static items from the manifest by `shell/quick_actions.rs`.
use super::{Event, Shell, SystemService};
use crate::shell::apphost::installed;
use bifrost_core::json::Json;
use bifrost_ui::quick_actions::{clean_all, ShortcutItem};

/// The `quick_actions` service.
pub struct QuickActionsService;

/// One item of the `items` list. `type` and `title` are required; the rest are optional.
fn item_of(item: &Json) -> Result<ShortcutItem, String> {
    let text = |key: &str| -> Result<String, String> {
        match item.get(key) {
            None | Some(Json::Null) => Ok(String::new()),
            Some(Json::Str(s)) => Ok(s.clone()),
            Some(_) => Err(format!("quick_actions: {key} must be a string")),
        }
    };
    let kind = match item.get("type") {
        Some(Json::Str(s)) => s.clone(),
        _ => return Err("quick_actions: each item needs a type".into()),
    };
    let title = match item.get("title") {
        Some(Json::Str(s)) => s.clone(),
        _ => return Err("quick_actions: each item needs a title".into()),
    };
    let user_info = match item.get("userInfo") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Obj(fields)) => fields
            .iter()
            .map(|(k, v)| match v {
                Json::Str(s) => Ok((k.clone(), s.clone())),
                _ => Err("quick_actions: userInfo values must be strings".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("quick_actions: userInfo must be an object".into()),
    };
    Ok(ShortcutItem { kind, title, subtitle: text("subtitle")?, symbol: text("symbol")?, user_info })
}

/// The checked list in `params` (`items`). A missing or null list is empty.
fn items_of(params: &Json) -> Result<Vec<ShortcutItem>, String> {
    let list = match params.get("items") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Arr(v)) => v.iter().map(item_of).collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("quick_actions: items must be a list".into()),
    };
    clean_all(list)
}

/// The slot of the installed app `app_id`. Only an installed app has Home Screen quick actions.
fn slot_of(app_id: &str) -> Result<usize, String> {
    installed().iter().position(|a| a.manifest.id == app_id).ok_or_else(|| "quick_actions: only an installed app has quick actions".to_string())
}

impl SystemService for QuickActionsService {
    fn name(&self) -> &'static str {
        "quick_actions"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let slot = slot_of(app_id)?;
        let Some(host) = shell.ext.get_mut(slot).and_then(Option::as_mut) else {
            return Err("quick_actions: the app is not running".into());
        };
        let reply = match method {
            "set" => {
                host.quick_actions = items_of(params)?;
                Json::Null
            }
            "clear" => {
                host.quick_actions.clear();
                Json::Null
            }
            "state" => Json::Obj(vec![("count".into(), Json::Num(host.quick_actions.len() as f64))]),
            _ => return Err(format!("quick_actions has no method {method}")),
        };
        shell.dirty = true;
        Ok(reply)
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;

    fn json(text: &str) -> Json {
        parse(text).unwrap()
    }

    #[test]
    fn the_service_is_registered_under_quick_actions() {
        assert!(super::super::names(&super::super::all()).contains(&"quick_actions"));
    }

    #[test]
    fn an_item_keeps_its_optional_fields() {
        let item = item_of(&json(r#"{"type":"compose","title":"New Message","subtitle":"Work","symbol":"pencil","userInfo":{"account":"work"}}"#)).unwrap();
        assert_eq!(item.kind, "compose");
        assert_eq!(item.title, "New Message");
        assert_eq!(item.subtitle, "Work");
        assert_eq!(item.symbol, "pencil");
        assert_eq!(item.user_info, vec![("account".to_string(), "work".to_string())]);
        let bare = item_of(&json(r#"{"type":"search","title":"Search"}"#)).unwrap();
        assert!(bare.subtitle.is_empty() && bare.user_info.is_empty());
    }

    #[test]
    fn a_missing_type_or_title_is_refused() {
        assert!(item_of(&json(r#"{"title":"Search"}"#)).is_err());
        assert!(item_of(&json(r#"{"type":"search"}"#)).is_err());
        assert!(item_of(&json(r#"{"type":"search","title":3}"#)).is_err());
    }

    #[test]
    fn user_info_must_hold_only_strings() {
        assert!(item_of(&json(r#"{"type":"a","title":"A","userInfo":{"n":1}}"#)).is_err());
        assert!(item_of(&json(r#"{"type":"a","title":"A","userInfo":"x"}"#)).is_err());
    }

    #[test]
    fn the_list_is_checked_as_a_whole() {
        let ok = items_of(&json(r#"{"items":[{"type":"a","title":"A"},{"type":"b","title":"B"}]}"#)).unwrap();
        assert_eq!(ok.len(), 2);
        assert!(items_of(&json(r#"{"items":[{"type":"a","title":"A"},{"type":"a","title":"Again"}]}"#)).is_err(), "a repeated type");
        assert!(items_of(&json(r#"{"items":"nope"}"#)).is_err());
        assert!(items_of(&json("{}")).unwrap().is_empty(), "no list clears");
        let five: String = (0..5).map(|n| format!(r#"{{"type":"t{n}","title":"T"}}"#)).collect::<Vec<_>>().join(",");
        assert!(items_of(&json(&format!(r#"{{"items":[{five}]}}"#))).is_err(), "more than four");
    }

    #[test]
    fn an_app_that_is_not_installed_has_no_quick_actions() {
        assert!(slot_of("no.such.app.for.quick.actions").is_err());
    }
}

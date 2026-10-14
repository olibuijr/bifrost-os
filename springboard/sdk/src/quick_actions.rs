//! Home Screen quick actions (`UIApplicationShortcutItem`) from the app. Static items are declared in the app's manifest
//! as `[[quick_action]]` tables (see `springboard/src/shell/quick_actions.rs`); this module sets the dynamic ones, which
//! replace the app's previous dynamic list. The shell shows the list on a long press of the app's icon in the Home
//! Screen and the Dock. A choice arrives as `Input::QuickAction` with the item's type and user info.
//!
//! ```ignore
//! quick_actions::set_items(ctx, &[ShortcutItem::new("compose", "New Message")]);
//! quick_actions::clear(ctx);
//! ```
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use bifrost_ui::quick_actions::{clean_all, ShortcutItem};

/// The shell service name.
pub const SERVICE: &str = "quick_actions";

/// A quick action the user chose, as `Input::QuickAction`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickActionPick {
    /// the type the app gave the item
    pub kind: String,
    /// the user info the app gave the item
    pub user_info: Vec<(String, String)>,
}

/// One item as the `set` params carry it: `type`, `title`, and `subtitle`, `symbol` and `userInfo` when set.
fn item_json(item: &ShortcutItem) -> Json {
    let mut fields: Vec<(String, Json)> = vec![("type".into(), Json::Str(item.kind.clone())), ("title".into(), Json::Str(item.title.clone()))];
    if !item.subtitle.is_empty() {
        fields.push(("subtitle".into(), Json::Str(item.subtitle.clone())));
    }
    if !item.symbol.is_empty() {
        fields.push(("symbol".into(), Json::Str(item.symbol.clone())));
    }
    if !item.user_info.is_empty() {
        fields.push(("userInfo".into(), Json::Obj(item.user_info.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect())));
    }
    Json::Obj(fields)
}

/// The params of `set`: `items`, the whole dynamic list. It is checked by `bifrost_ui::quick_actions::clean_all` on the shell.
pub fn set_params(items: &[ShortcutItem]) -> Json {
    Json::Obj(vec![("items".into(), Json::Arr(items.iter().map(item_json).collect()))])
}

/// Replaces the app's dynamic quick actions. Returns an error when the list breaks the limits (at most four items,
/// types non-empty and unique, titles non-empty), before anything is sent.
pub fn set_items(ctx: &Ctx, items: &[ShortcutItem]) -> Result<u32, String> {
    clean_all(items.to_vec())?;
    Ok(service::call(ctx, SERVICE, "set", &set_params(items)))
}

/// Removes the app's dynamic quick actions; the static ones stay.
pub fn clear(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "clear", &Json::Obj(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_params_carry_only_the_fields_that_are_set() {
        let plain = item_json(&ShortcutItem::new("search", "Search"));
        assert_eq!(plain.get("type"), Some(&Json::Str("search".into())));
        assert!(plain.get("subtitle").is_none());
        assert!(plain.get("userInfo").is_none());
        let mut full = ShortcutItem::new("compose", "New Message");
        full.subtitle = "Work".into();
        full.symbol = "square.and.pencil".into();
        full.user_info = vec![("account".into(), "work".into())];
        let json = item_json(&full);
        assert_eq!(json.get("subtitle"), Some(&Json::Str("Work".into())));
        assert_eq!(json.get("symbol"), Some(&Json::Str("square.and.pencil".into())));
        assert_eq!(json.get("userInfo").and_then(|u| u.get("account")), Some(&Json::Str("work".into())));
    }

    #[test]
    fn the_set_params_list_every_item_in_order() {
        let items = vec![ShortcutItem::new("a", "A"), ShortcutItem::new("b", "B")];
        match set_params(&items).get("items") {
            Some(Json::Arr(v)) => assert_eq!(v.len(), 2),
            other => panic!("items should be a list, got {other:?}"),
        }
    }
}

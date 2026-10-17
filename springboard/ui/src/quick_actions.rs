//! Home screen quick actions: the short list an app offers when its icon is long pressed.
//! An item has a type the app reads back, a title, an optional subtitle and symbol, and user info. Static items come
//! from the app's manifest; dynamic items are set at runtime through the SDK. The list shows dynamic items first, then
//! static ones, without repeating a type and with at most [`MAX_ITEMS`] in all.
//!
//! This module is the pure model. The shell reads manifests and the `quick_actions` service into it, and the SDK
//! publishes dynamic items and receives the chosen one.

use std::collections::HashSet;

/// The most quick actions one app shows.
pub const MAX_ITEMS: usize = 4;
/// The longest type, in bytes.
pub const TYPE_MAX: usize = 64;
/// The longest title or subtitle, in characters.
pub const TEXT_MAX: usize = 48;

/// One quick action. `kind` is the app's own identifier for the action; `subtitle` and `symbol` are empty when unset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutItem {
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    /// the name of a system symbol drawn beside the title (empty for none)
    pub symbol: String,
    pub user_info: Vec<(String, String)>,
}

impl ShortcutItem {
    /// An item with a type and a title and nothing else.
    pub fn new(kind: &str, title: &str) -> Self {
        Self { kind: kind.into(), title: title.into(), ..Self::default() }
    }
}

/// The text trimmed and cut to `TEXT_MAX` characters.
fn clamp_text(text: &str) -> String {
    text.trim().chars().take(TEXT_MAX).collect()
}

/// Checks and tidies one item: the type must be non-empty and at most `TYPE_MAX` bytes, the title must not be empty.
/// Text is trimmed and cut to `TEXT_MAX` characters.
pub fn clean(item: ShortcutItem) -> Result<ShortcutItem, String> {
    let kind = item.kind.trim().to_string();
    if kind.is_empty() {
        return Err("a quick action needs a type".into());
    }
    if kind.len() > TYPE_MAX {
        return Err(format!("a quick action type holds at most {TYPE_MAX} bytes"));
    }
    let title = clamp_text(&item.title);
    if title.is_empty() {
        return Err("a quick action needs a title".into());
    }
    Ok(ShortcutItem { kind, title, subtitle: clamp_text(&item.subtitle), symbol: item.symbol.trim().to_string(), user_info: item.user_info })
}

/// Checks a whole dynamic list: at most `MAX_ITEMS` items, each clean, and no type given twice.
pub fn clean_all(items: Vec<ShortcutItem>) -> Result<Vec<ShortcutItem>, String> {
    if items.len() > MAX_ITEMS {
        return Err(format!("an app shows at most {MAX_ITEMS} quick actions"));
    }
    let mut kinds = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let item = clean(item)?;
        if !kinds.insert(item.kind.clone()) {
            return Err(format!("quick action type {} is given twice", item.kind));
        }
        out.push(item);
    }
    Ok(out)
}

/// The list an app shows: `dynamic` items first, then `static_items`, dropping a type already listed and anything past
/// [`MAX_ITEMS`].
pub fn combine(dynamic: &[ShortcutItem], static_items: &[ShortcutItem]) -> Vec<ShortcutItem> {
    let mut seen: HashSet<String> = HashSet::new();
    dynamic.iter().chain(static_items).filter(|item| seen.insert(item.kind.clone())).take(MAX_ITEMS).cloned().collect()
}

/// The suggested actions: the quick actions used most often, most used first. `usage` pairs each action with its use
/// count. An action used never is left out, and a type appears once (its first entry wins among equal counts). At most
/// `limit` items come back, and never more than [`MAX_ITEMS`].
pub fn suggest(usage: &[(ShortcutItem, u32)], limit: usize) -> Vec<ShortcutItem> {
    let mut ranked: Vec<&(ShortcutItem, u32)> = usage.iter().filter(|(_, uses)| *uses > 0).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let mut seen: HashSet<&str> = HashSet::new();
    ranked
        .into_iter()
        .filter(|(item, _)| seen.insert(item.kind.as_str()))
        .take(limit.min(MAX_ITEMS))
        .map(|(item, _)| item.clone())
        .collect()
}

/// A `key=value` pair from a manifest's `user_info` list. The key is trimmed and must not be empty.
pub fn user_info_pair(text: &str) -> Option<(String, String)> {
    let (key, value) = text.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then(|| (key.to_string(), value.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, title: &str) -> ShortcutItem {
        ShortcutItem::new(kind, title)
    }

    #[test]
    fn an_item_needs_a_type_and_a_title() {
        assert!(clean(item("", "New")).is_err());
        assert!(clean(item("   ", "New")).is_err());
        assert!(clean(item("new", "  ")).is_err());
        let ok = clean(item(" new ", " New Note ")).unwrap();
        assert_eq!(ok.kind, "new");
        assert_eq!(ok.title, "New Note");
    }

    #[test]
    fn long_text_is_cut_to_the_limits() {
        let long_type = "t".repeat(TYPE_MAX + 1);
        assert!(clean(item(&long_type, "x")).is_err());
        let title = "x".repeat(TEXT_MAX + 10);
        assert_eq!(clean(item("a", &title)).unwrap().title.chars().count(), TEXT_MAX);
        let mut sub = item("a", "b");
        sub.subtitle = "s".repeat(TEXT_MAX + 5);
        assert_eq!(clean(sub).unwrap().subtitle.chars().count(), TEXT_MAX);
    }

    #[test]
    fn a_dynamic_list_holds_at_most_four_distinct_types() {
        let four = vec![item("a", "A"), item("b", "B"), item("c", "C"), item("d", "D")];
        assert_eq!(clean_all(four.clone()).unwrap().len(), 4);
        let mut five = four;
        five.push(item("e", "E"));
        assert!(clean_all(five).is_err());
        assert!(clean_all(vec![item("a", "A"), item("a", "Again")]).is_err());
        assert!(clean_all(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn dynamic_items_come_before_static_ones_without_repeats() {
        let dynamic = vec![item("compose", "Compose")];
        let fixed = vec![item("search", "Search"), item("compose", "Static compose"), item("open", "Open")];
        let kinds: Vec<String> = combine(&dynamic, &fixed).into_iter().map(|i| i.kind).collect();
        assert_eq!(kinds, ["compose", "search", "open"]);
    }

    #[test]
    fn the_combined_list_is_capped() {
        let fixed: Vec<ShortcutItem> = (0..6).map(|n| item(&format!("k{n}"), "x")).collect();
        let dynamic = vec![item("d", "x")];
        let all = combine(&dynamic, &fixed);
        assert_eq!(all.len(), MAX_ITEMS);
        assert_eq!(all[0].kind, "d");
    }

    #[test]
    fn suggestions_are_the_most_used_actions_without_repeats() {
        let usage = vec![(item("search", "Search"), 2), (item("compose", "Compose"), 5), (item("open", "Open"), 0), (item("compose", "Again"), 9)];
        let kinds: Vec<String> = suggest(&usage, 4).into_iter().map(|i| i.kind).collect();
        assert_eq!(kinds, ["compose", "search"], "an unused action is left out and a type is shown once");
    }

    #[test]
    fn suggestions_respect_the_limit_and_the_maximum() {
        let usage: Vec<(ShortcutItem, u32)> = (0..6).map(|n| (item(&format!("k{n}"), "x"), 10 - n)).collect();
        assert_eq!(suggest(&usage, 2).len(), 2);
        assert_eq!(suggest(&usage, 20).len(), MAX_ITEMS);
        assert!(suggest(&usage, 0).is_empty());
        assert!(suggest(&[], 4).is_empty());
    }

    #[test]
    fn user_info_is_a_key_and_a_value() {
        assert_eq!(user_info_pair("tab = inbox"), Some(("tab".into(), "inbox".into())));
        assert_eq!(user_info_pair("a=b=c"), Some(("a".into(), "b=c".into())), "only the first equals sign splits");
        assert_eq!(user_info_pair("=x"), None);
        assert_eq!(user_info_pair("novalue"), None);
    }
}

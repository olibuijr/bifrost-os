//! Home Screen quick actions in the shell. An app's list is its static items (`[[quick_action]]` tables in the manifest)
//! and its dynamic items (set through the `quick_actions` service), combined by the model in `bifrost_ui::quick_actions`.
//! The Dock's menu shows the list (`desktop_bar.rs`, action `QuickAction`), and a choice goes to the app as
//! `ToApp::QuickAction`. Each pick is counted (`Usage`), and the menu lists the most used actions first.
use super::apphost::{installed, unquote};
use super::Shell;
use bifrost_ui::quick_actions::{clean, combine, user_info_pair, ShortcutItem};

/// The `[[quick_action]]` tables of a manifest. Each has `type`, `title`, and optionally `subtitle`, `symbol` and
/// `user_info = ["key=value", ...]`. A table without a valid type and title is dropped. Other keys and tables are
/// ignored, as the manifest parser ignores what it does not know.
pub fn parse_static(text: &str) -> Vec<ShortcutItem> {
    let mut items: Vec<ShortcutItem> = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            in_table = line == "[[quick_action]]";
            if in_table {
                items.push(ShortcutItem::default());
            }
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some(item) = items.last_mut() else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "type" => item.kind = unquote(value),
            "title" => item.title = unquote(value),
            "subtitle" => item.subtitle = unquote(value),
            "symbol" => item.symbol = unquote(value),
            "user_info" => item.user_info = strings(value).iter().filter_map(|pair| user_info_pair(pair)).collect(),
            _ => {}
        }
    }
    items.into_iter().filter_map(|item| clean(item).ok()).collect()
}

/// The quoted strings of a manifest list, `["a", "b"]`. Empty entries are skipped.
fn strings(list: &str) -> Vec<String> {
    list.trim_start_matches('[').trim_end_matches(']').split(',').map(|s| unquote(s.trim())).filter(|s| !s.is_empty()).collect()
}

/// How often each app's quick actions were picked this session, keyed by the app's slot and the action's type. It only
/// orders the lists: the actions picked most come first, and the rest keep their order (see `rank`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Usage {
    counts: std::collections::BTreeMap<(usize, String), u32>,
}

impl Usage {
    /// Counts one pick of the action of type `kind` in app `slot`.
    pub fn record(&mut self, slot: usize, kind: &str) {
        *self.counts.entry((slot, kind.to_string())).or_insert(0) += 1;
    }

    /// The picks of the action of type `kind` in app `slot`.
    pub fn count(&self, slot: usize, kind: &str) -> u32 {
        self.counts.get(&(slot, kind.to_string())).copied().unwrap_or(0)
    }

    /// `items` ordered by use, most used first. The sort is stable, so an action never picked keeps its place among
    /// the others and nothing is dropped.
    pub fn rank(&self, slot: usize, items: Vec<ShortcutItem>) -> Vec<ShortcutItem> {
        let mut ranked: Vec<(u32, ShortcutItem)> = items.into_iter().map(|item| (self.count(slot, &item.kind), item)).collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        ranked.into_iter().map(|(_, item)| item).collect()
    }
}

impl Shell {
    /// The quick actions of the installed app in `slot`: its dynamic items first, then the manifest's static ones, with
    /// the most used first.
    #[allow(dead_code)]
    pub(super) fn quick_actions_for(&self, slot: usize) -> Vec<ShortcutItem> {
        let dynamic = self.ext.get(slot).and_then(Option::as_ref).map(|host| host.quick_actions.clone()).unwrap_or_default();
        let fixed = installed().get(slot).map(|app| app.manifest.quick_actions.clone()).unwrap_or_default();
        self.quick_action_uses.rank(slot, combine(&dynamic, &fixed))
    }

    /// Counts a pick of the quick action `kind` of app `slot`.
    #[allow(dead_code)]
    pub(super) fn record_quick_action(&mut self, slot: usize, kind: &str) {
        self.quick_action_uses.record(slot, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_items_are_read_from_their_tables() {
        let text = r#"
name = "Mail"
permissions = ["notifications"]
[[quick_action]]
type = "compose"
title = "New Message"
user_info = ["account=work", "no-value"]
[[quick_action]]
type = ""
title = "Dropped: no type"
[[quick_action]]
type = "search"
title = "Search"
subtitle = "Find a message"
symbol = "find"
[[intent]]
id = "not.a.quick.action"
"#;
        let items = parse_static(text);
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["compose", "search"]);
        assert_eq!(items[0].title, "New Message");
        assert_eq!(items[0].user_info, vec![("account".to_string(), "work".to_string())]);
        assert_eq!(items[1].subtitle, "Find a message");
        assert_eq!(items[1].symbol, "find");
    }

    #[test]
    fn a_manifest_without_tables_has_no_static_items() {
        assert!(parse_static("name = \"Notes\"\nsummary = \"Notes\"\n").is_empty());
    }

    #[test]
    fn the_most_picked_actions_come_first_and_the_rest_keep_their_order() {
        let items = vec![ShortcutItem::new("compose", "New"), ShortcutItem::new("search", "Find"), ShortcutItem::new("open", "Open")];
        let mut usage = Usage::default();
        usage.record(3, "open");
        usage.record(3, "open");
        usage.record(3, "search");
        let kinds: Vec<String> = usage.rank(3, items.clone()).into_iter().map(|i| i.kind).collect();
        assert_eq!(kinds, ["open", "search", "compose"]);
        assert_eq!(usage.count(3, "open"), 2);
        assert_eq!(usage.count(4, "open"), 0, "counts are per app slot");
        let untouched: Vec<String> = Usage::default().rank(0, items).into_iter().map(|i| i.kind).collect();
        assert_eq!(untouched, ["compose", "search", "open"], "without picks the order is the manifest's");
    }

    #[test]
    fn quoted_lists_drop_empty_entries() {
        assert_eq!(strings(r#"["a", "", "b"]"#), ["a", "b"]);
    }
}

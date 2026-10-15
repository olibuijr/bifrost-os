//! Home Screen quick actions in the shell. An app's list is its static items (`[[quick_action]]` tables in the manifest)
//! and its dynamic items (set through the `quick_actions` service), combined by the model in `bifrost_ui::quick_actions`.
//! The Dock's menu shows the list (`desktop_bar.rs`, action `QuickAction`), and a choice goes to the app as
//! `ToApp::QuickAction`.
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

impl Shell {
    /// The quick actions of the installed app in `slot`: its dynamic items first, then the manifest's static ones.
    #[allow(dead_code)]
    pub(super) fn quick_actions_for(&self, slot: usize) -> Vec<ShortcutItem> {
        let dynamic = self.ext.get(slot).and_then(Option::as_ref).map(|host| host.quick_actions.clone()).unwrap_or_default();
        let fixed = installed().get(slot).map(|app| app.manifest.quick_actions.clone()).unwrap_or_default();
        combine(&dynamic, &fixed)
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
    fn quoted_lists_drop_empty_entries() {
        assert_eq!(strings(r#"["a", "", "b"]"#), ["a", "b"]);
    }
}

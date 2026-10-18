//! Settings panes from apps. An app declares its panes as `[[settings_pane]]` tables in `manifest.toml`; the shell
//! lists them under the app in Settings. Each table has `id`, `title`, and optionally `symbol`. A table without a
//! valid id and title is dropped, and a repeated id keeps only its first table. The parser follows the other manifest
//! tables (`shell::quick_actions`): it reads the tables it knows and ignores the rest.
//!
//! Settings root lists each app that declares panes. Its page lists the panes. Picking a pane sends
//! `ae.settings.openPane` with `{"id": ...}` over the app's control socket (`sdk/src/settings_panes.rs`); the app
//! must be running and must have installed the handler, otherwise the shell logs the refusal and changes nothing.

use super::apphost::{installed, unquote};
use super::intents;
use super::list::{Group, Item, Spec};
use super::pages::{Action, PageId};
use bifrost_core::json::Json;
use std::time::Duration;

/// The control-socket method that asks an app to open a pane (the SDK's `settings_panes::OPEN_PANE` event).
pub const OPEN_METHOD: &str = "ae.settings.openPane";
/// How long the shell waits for the app's answer.
const OPEN_WAIT: Duration = Duration::from_secs(2);

/// One settings pane an app declares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsPane {
    /// The id the shell uses to open the pane in the app. Non-empty, unique within the app.
    pub id: String,
    /// The title the shell shows in the Settings list.
    pub title: String,
    /// An optional symbol name for the row, empty when none is given.
    pub symbol: String,
}

/// The `[[settings_pane]]` tables of a manifest, in order, without invalid or repeated ids.
pub fn parse_static(text: &str) -> Vec<SettingsPane> {
    let mut panes: Vec<SettingsPane> = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            in_table = line == "[[settings_pane]]";
            if in_table {
                panes.push(SettingsPane::default());
            }
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some(pane) = panes.last_mut() else { continue };
        let value = value.trim();
        match key.trim() {
            "id" => pane.id = unquote(value),
            "title" => pane.title = unquote(value),
            "symbol" => pane.symbol = unquote(value),
            _ => {}
        }
    }
    let mut kept: Vec<SettingsPane> = Vec::new();
    for pane in panes {
        if pane.id.is_empty() || pane.title.trim().is_empty() {
            continue;
        }
        if kept.iter().any(|k| k.id == pane.id) {
            continue;
        }
        kept.push(pane);
    }
    kept
}

/// The JSON-RPC line that asks an app to open `pane_id`.
pub fn open_line(pane_id: &str) -> String {
    let params = Json::Obj(vec![("id".into(), Json::Str(pane_id.into()))]).encode();
    format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"{OPEN_METHOD}\",\"params\":{params}}}\n")
}

/// Asks the installed app in `slot` to open its pane `pane_id`. Errors when the slot holds no app, the app does not
/// declare that pane, its control socket cannot be reached, or it refuses the request.
pub fn open(slot: usize, pane_id: &str) -> Result<(), String> {
    let apps = installed();
    let app = apps.get(slot).ok_or_else(|| "no such app".to_string())?;
    if !app.manifest.settings_panes.iter().any(|p| p.id == pane_id) {
        return Err(format!("{} declares no pane {pane_id}", app.manifest.id));
    }
    let reply = intents::exchange(&app.manifest.id, &open_line(pane_id), OPEN_WAIT)?;
    intents::parse_reply(&reply).map(|_| ())
}

/// The Settings root rows for the apps that declare panes: one row per app, linking to its page.
pub fn root_groups() -> Vec<Group> {
    let rows: Vec<Item> = installed()
        .iter()
        .enumerate()
        .filter(|(_, app)| !app.manifest.settings_panes.is_empty())
        .map(|(slot, app)| Item::new(app.manifest.name.clone()).link(PageId::AppPanes(slot)))
        .collect();
    if rows.is_empty() {
        Vec::new()
    } else {
        vec![Group::new(rows)]
    }
}

/// The rows of one app's page: one per pane, each asking the app to open it.
pub fn pane_items(slot: usize, panes: &[SettingsPane]) -> Vec<Item> {
    panes.iter().map(|p| Item::new(p.title.clone()).action(Action::AppPane(slot, p.id.clone()))).collect()
}

/// The page of the app in `slot`: its name as the title and its panes as rows.
pub fn spec(slot: usize) -> Spec {
    let apps = installed();
    let app = apps.get(slot);
    let title = app.map(|a| a.manifest.name.clone()).unwrap_or_else(|| "Settings".to_string());
    let panes: Vec<SettingsPane> = app.map(|a| a.manifest.settings_panes.clone()).unwrap_or_default();
    let group = Group::new(pane_items(slot, &panes)).footer("The app opens the pane in its own window.");
    Spec { id: PageId::AppPanes(slot), title, large: false, groups: vec![group] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_are_read_from_their_tables() {
        let text = r#"
name = "Mail"
[[settings_pane]]
id = "accounts"
title = "Accounts"
symbol = "person"
[[settings_pane]]
id = ""
title = "Dropped: no id"
[[settings_pane]]
id = "signatures"
title = "Signatures"
[[settings_pane]]
id = "accounts"
title = "Duplicate id"
[[intent]]
id = "not.a.pane"
"#;
        let panes = parse_static(text);
        let ids: Vec<&str> = panes.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["accounts", "signatures"]);
        assert_eq!(panes[0].title, "Accounts");
        assert_eq!(panes[0].symbol, "person");
        assert_eq!(panes[1].symbol, "");
    }

    #[test]
    fn a_manifest_without_tables_has_no_panes() {
        assert!(parse_static("name = \"Notes\"\n").is_empty());
    }

    #[test]
    fn a_pane_without_a_title_is_dropped() {
        assert!(parse_static("[[settings_pane]]\nid = \"x\"\ntitle = \"  \"\n").is_empty());
    }

    #[test]
    fn the_open_line_names_the_shell_event_and_the_pane() {
        let line = open_line("accounts");
        assert!(line.contains("\"method\":\"ae.settings.openPane\""), "{line}");
        assert!(line.contains("\"params\":{\"id\":\"accounts\"}"), "{line}");
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn each_pane_row_asks_for_its_own_pane() {
        let panes = parse_static("[[settings_pane]]\nid = \"accounts\"\ntitle = \"Accounts\"\n[[settings_pane]]\nid = \"sig\"\ntitle = \"Signatures\"\n");
        let rows = pane_items(3, &panes);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Accounts");
        assert_eq!(rows[1].action, Action::AppPane(3, "sig".to_string()));
    }

    #[test]
    fn a_missing_app_has_no_rows_and_no_root_entry() {
        assert!(spec(usize::MAX).groups[0].items.is_empty());
        assert!(root_groups().is_empty());
    }
}

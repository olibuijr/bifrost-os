//! Settings panes from apps. An app declares its panes as `[[settings_pane]]` tables in `manifest.toml`; the shell
//! lists them under the app in Settings. Each table has `id`, `title`, and optionally `symbol`. A table without a
//! valid id and title is dropped, and a repeated id keeps only its first table. The parser follows the other manifest
//! tables (`shell::quick_actions`): it reads the tables it knows and ignores the rest.

use super::apphost::{installed, unquote};
use super::Shell;

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

impl Shell {
    /// The Settings panes the installed app in `slot` declares. Empty when the slot holds no installed app.
    #[allow(dead_code)]
    pub(super) fn settings_panes_for(&self, slot: usize) -> Vec<SettingsPane> {
        installed().get(slot).map(|app| app.manifest.settings_panes.clone()).unwrap_or_default()
    }
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
}

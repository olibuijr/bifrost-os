//! Third-party Control Center controls. An app declares them in `manifest.toml` as `[[control]]` tables:
//!
//! ```toml
//! [[control]]
//! kind = "toggle"          # or "button"
//! title = "Quiet Hours"
//! symbol = "moon"
//! intent = "set-quiet"     # an [[intent]] id of the same app
//! ```
//!
//! The shell registers each control in a [`ControlRegistry`] under `<app id>.<intent>`. Using a button runs the
//! intent. Switching a toggle runs the intent with one string parameter, `on` set to `"true"` or `"false"`. Both go
//! through the app's control socket, the same path as App Intents (`shell::intents`).

use super::apphost::unquote;
use super::intents;
use bifrost_ui::control_widget::{ControlError, ControlKind, ControlRegistry, StaticControlConfiguration, MAX_IDENTIFIER_LEN};
use std::time::Duration;

/// One `[[control]]` table of a manifest, already checked: a known kind, a title, and a safe intent id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestControl {
    pub kind: ControlKind,
    pub title: String,
    /// The SF-style symbol name the panel draws, empty for none.
    pub symbol: String,
    /// The `[[intent]]` id the control runs.
    pub intent: String,
}

#[derive(Default)]
struct Draft {
    kind: String,
    title: String,
    symbol: String,
    intent: String,
}

impl Draft {
    fn checked(self) -> Option<ManifestControl> {
        let kind = match self.kind.as_str() {
            "toggle" => ControlKind::Toggle,
            "button" => ControlKind::Button,
            _ => return None,
        };
        let safe_intent = !self.intent.is_empty() && self.intent.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        (safe_intent && !self.title.trim().is_empty()).then(|| ManifestControl { kind, title: self.title.trim().to_string(), symbol: self.symbol, intent: self.intent })
    }
}

/// Reads the `[[control]]` tables of a manifest. Tables with an unknown kind, a blank title or an unsafe intent id
/// are dropped, so one bad table does not hide the others.
pub fn parse_controls(text: &str) -> Vec<ManifestControl> {
    let mut drafts: Vec<Draft> = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            in_table = line == "[[control]]";
            if in_table {
                drafts.push(Draft::default());
            }
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some(draft) = drafts.last_mut() else { continue };
        let value = unquote(value);
        match key.trim() {
            "kind" => draft.kind = value,
            "title" => draft.title = value,
            "symbol" => draft.symbol = value,
            "intent" => draft.intent = value,
            _ => {}
        }
    }
    drafts.into_iter().filter_map(Draft::checked).collect()
}

impl ManifestControl {
    /// The registry identifier: the app id and the intent id joined by a dot. Always a valid identifier for an app
    /// whose id passed the manifest check.
    pub fn identifier(&self, app_id: &str) -> String {
        format!("{app_id}.{}", self.intent)
    }

    /// What the registry needs to hold the control. A toggle starts off.
    pub fn configuration(&self, app_id: &str) -> StaticControlConfiguration {
        StaticControlConfiguration { identifier: self.identifier(app_id), display_name: self.title.clone(), kind: self.kind, initial_value: false }
    }

    /// The JSON-RPC line that uses the control. A button sends no parameters. A toggle sends its new state as the
    /// string parameter `on`.
    pub fn request_line(&self, on: bool) -> String {
        match self.kind {
            ControlKind::Button => intents::request_line(&self.intent),
            ControlKind::Toggle => intents::request_line_with(&self.intent, &[("on", if on { "true" } else { "false" })]),
        }
    }

    /// Sends the control's request to the app's control socket and returns the app's dialog text, if any.
    pub fn invoke(&self, app_id: &str, on: bool, wait: Duration) -> Result<Option<String>, String> {
        intents::parse_reply(&intents::exchange(app_id, &self.request_line(on), wait)?)
    }
}

/// Registers every control of an app. Stops at the first refusal (a duplicate identifier, for example).
pub fn register(registry: &mut ControlRegistry, app_id: &str, controls: &[ManifestControl]) -> Result<(), ControlError> {
    for control in controls {
        let configuration = control.configuration(app_id);
        if configuration.identifier.len() > MAX_IDENTIFIER_LEN {
            return Err(ControlError::IdentifierTooLong { length: configuration.identifier.len() });
        }
        registry.register(configuration)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "id = \"is.olibuijr.notes\"\nname = \"Notes\"\n\n[[control]]\nkind = \"toggle\"\ntitle = \"Quiet Hours\"\nsymbol = \"moon\"\nintent = \"set-quiet\"\n\n[[control]]\nkind = \"button\"\ntitle = \"New Note\"\nintent = \"new-note\"\n";

    #[test]
    fn parses_toggle_and_button_tables() {
        let controls = parse_controls(MANIFEST);
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0], ManifestControl { kind: ControlKind::Toggle, title: "Quiet Hours".into(), symbol: "moon".into(), intent: "set-quiet".into() });
        assert_eq!(controls[1].kind, ControlKind::Button);
        assert_eq!(controls[1].symbol, "");
    }

    #[test]
    fn bad_tables_are_dropped_and_others_kept() {
        let text = "[[control]]\nkind = \"slider\"\ntitle = \"X\"\nintent = \"a\"\n\n[[control]]\nkind = \"button\"\ntitle = \"  \"\nintent = \"a\"\n\n[[control]]\nkind = \"button\"\ntitle = \"Bad\"\nintent = \"a\\\"b\"\n\n[[control]]\nkind = \"button\"\ntitle = \"Good\"\nintent = \"ok\"\n";
        let controls = parse_controls(text);
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0].title, "Good");
    }

    #[test]
    fn other_tables_do_not_leak_into_controls() {
        let text = "[[intent]]\nid = \"x\"\ntitle = \"X\"\n\n[privacy]\nlocation = \"why\"\n";
        assert!(parse_controls(text).is_empty());
        assert!(parse_controls(&format!("{text}\n[[control]]\nkind = \"button\"\ntitle = \"T\"\nintent = \"x\"\n\n[privacy]\nintent = \"y\"\n"))[0].intent == "x");
    }

    #[test]
    fn identifiers_join_app_and_intent() {
        let controls = parse_controls(MANIFEST);
        assert_eq!(controls[0].identifier("is.olibuijr.notes"), "is.olibuijr.notes.set-quiet");
        let c = controls[0].configuration("is.olibuijr.notes");
        assert_eq!(c.display_name, "Quiet Hours");
        assert!(!c.initial_value);
    }

    #[test]
    fn button_press_maps_to_a_bare_intent_request() {
        let button = &parse_controls(MANIFEST)[1];
        assert_eq!(button.request_line(true), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.new-note\",\"params\":{}}\n");
    }

    #[test]
    fn toggle_sends_its_new_state() {
        let toggle = &parse_controls(MANIFEST)[0];
        assert_eq!(toggle.request_line(true), "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"intent.set-quiet\",\"params\":{\"on\":\"true\"}}\n");
        assert!(toggle.request_line(false).contains("\"on\":\"false\""));
    }

    #[test]
    fn register_fills_the_registry_and_toggles_work() {
        let controls = parse_controls(MANIFEST);
        let mut registry = ControlRegistry::new();
        register(&mut registry, "is.olibuijr.notes", &controls).unwrap();
        assert_eq!(registry.configurations().len(), 2);
        assert_eq!(registry.toggle("is.olibuijr.notes.set-quiet"), Ok(true));
        assert_eq!(registry.value("is.olibuijr.notes.set-quiet"), Ok(true));
        assert!(registry.press("is.olibuijr.notes.new-note").is_ok());
        assert!(matches!(registry.press("is.olibuijr.notes.set-quiet"), Err(ControlError::WrongKind { .. })));
        // a second registration of the same app is a duplicate
        assert!(matches!(register(&mut registry, "is.olibuijr.notes", &controls), Err(ControlError::Duplicate { .. })));
    }

    #[test]
    fn overlong_identifier_is_refused_before_the_registry() {
        let control = ManifestControl { kind: ControlKind::Button, title: "T".into(), symbol: String::new(), intent: "i".repeat(200) };
        let mut registry = ControlRegistry::new();
        assert!(matches!(register(&mut registry, "app", &[control]), Err(ControlError::IdentifierTooLong { .. })));
    }
}

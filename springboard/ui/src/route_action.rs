//! Custom action items for a route picker. Each item names a type that the app's configuration defines, and may
//! override the title that type shows. Items whose type is not in the configuration are not shown.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionItem {
    /// The type identifier. It must match a type in the app's configuration.
    pub kind: String,
    /// A title that replaces the configured title of the type. Empty means no override.
    pub override_title: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ActionItems {
    items: Vec<ActionItem>,
}

impl ActionItems {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an item. Returns false when the type is empty or already has an item. A blank override is no override.
    pub fn add(&mut self, kind: &str, override_title: Option<&str>) -> bool {
        let kind = kind.trim();
        if kind.is_empty() || self.items.iter().any(|i| i.kind == kind) {
            return false;
        }
        let override_title = override_title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        self.items.push(ActionItem { kind: kind.to_string(), override_title });
        true
    }

    /// Removes the item for a type. Returns false when there was none.
    pub fn remove(&mut self, kind: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.kind != kind);
        self.items.len() != before
    }

    /// Items in the order they were added.
    pub fn items(&self) -> &[ActionItem] {
        &self.items
    }

    /// Titles to show, in item order. `configured` lists (type, configured title). Items whose type is not configured
    /// are skipped. An item's override title wins over the configured title.
    pub fn titles(&self, configured: &[(&str, &str)]) -> Vec<String> {
        let mut out = Vec::new();
        for item in &self.items {
            let Some(&(_, default)) = configured.iter().find(|(kind, _)| *kind == item.kind) else {
                continue;
            };
            match &item.override_title {
                Some(title) => out.push(title.clone()),
                None => out.push(default.to_string()),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIGURED: [(&str, &str); 2] = [("send-to-hub", "Send to Hub"), ("open-remote", "Open Remote")];

    #[test]
    fn add_rejects_blank_and_repeated_types() {
        let mut items = ActionItems::new();
        assert!(items.add("send-to-hub", None));
        assert!(!items.add(" send-to-hub ", None));
        assert!(!items.add("  ", None));
        assert_eq!(items.items().len(), 1);
    }

    #[test]
    fn titles_use_the_configured_title_in_item_order() {
        let mut items = ActionItems::new();
        items.add("open-remote", None);
        items.add("send-to-hub", None);
        assert_eq!(items.titles(&CONFIGURED), vec!["Open Remote".to_string(), "Send to Hub".to_string()]);
    }

    #[test]
    fn an_override_title_replaces_the_configured_title() {
        let mut items = ActionItems::new();
        items.add("send-to-hub", Some("  Living Room  "));
        assert_eq!(items.titles(&CONFIGURED), vec!["Living Room".to_string()]);
    }

    #[test]
    fn a_blank_override_falls_back_to_the_configured_title() {
        let mut items = ActionItems::new();
        items.add("send-to-hub", Some("   "));
        assert_eq!(items.items()[0].override_title, None);
        assert_eq!(items.titles(&CONFIGURED), vec!["Send to Hub".to_string()]);
    }

    #[test]
    fn an_unconfigured_type_is_not_shown() {
        let mut items = ActionItems::new();
        items.add("unknown", None);
        items.add("open-remote", None);
        assert_eq!(items.titles(&CONFIGURED), vec!["Open Remote".to_string()]);
    }

    #[test]
    fn remove_drops_only_the_named_item() {
        let mut items = ActionItems::new();
        items.add("send-to-hub", None);
        items.add("open-remote", None);
        assert!(items.remove("send-to-hub"));
        assert!(!items.remove("send-to-hub"));
        assert_eq!(items.items().len(), 1);
        assert_eq!(items.items()[0].kind, "open-remote");
    }
}

//! Today widgets that apps supply, as the shell's model of the widget protocol (`NCWidgetProviding`) and its
//! controller (`NCWidgetController`). Pure model: the two display modes and the largest size each allows, the
//! margin insets a widget accepts or replaces, and the per-widget "has content" flag that decides whether the
//! shell shows the widget at all.

use std::collections::HashMap;

/// The heights a widget can be shown at (`NCWidgetDisplayMode`): compact, or expanded by the More button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    #[default]
    Compact,
    Expanded,
}

impl DisplayMode {
    /// The mode the More button switches to.
    pub fn toggled(self) -> Self {
        match self {
            DisplayMode::Compact => DisplayMode::Expanded,
            DisplayMode::Expanded => DisplayMode::Compact,
        }
    }
}

/// Margins around a widget's content, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl Insets {
    pub const fn uniform(v: f32) -> Self {
        Insets { top: v, left: v, bottom: v, right: v }
    }
}

/// The margins a widget is drawn with: its own replacement when it returns one, otherwise the proposed defaults
/// (`widgetMarginInsets(forProposedMarginInsets:)`: the widget accepts the defaults or returns custom values).
pub fn margin_insets(proposed: Insets, custom: Option<Insets>) -> Insets {
    custom.unwrap_or(proposed)
}

/// The space a widget may take, as the shell reports it in each display mode
/// (`widgetActiveDisplayModeDidChange(_:withMaximumSize:)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Width of the widget's column, the same in both modes.
    pub width: f32,
    pub compact_h: f32,
    /// Never shorter than the compact height: an expanded widget only ever grows.
    pub expanded_h: f32,
}

impl Limits {
    pub fn new(width: f32, compact_h: f32, expanded_h: f32) -> Self {
        Limits { width, compact_h, expanded_h: expanded_h.max(compact_h) }
    }

    /// The largest (width, height) a widget may take in `mode`.
    pub fn max_size(&self, mode: DisplayMode) -> (f32, f32) {
        let h = match mode {
            DisplayMode::Compact => self.compact_h,
            DisplayMode::Expanded => self.expanded_h,
        };
        (self.width, h)
    }
}

/// Which widgets have content to show (`NCWidgetController`). An app reports its flag by bundle identifier and the
/// shell shows only the widgets that have content. A widget that has never reported stays visible, so an app that
/// does not use the flag loses nothing. The framework keeps one shared controller; here the shell owns the instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Controller {
    flags: HashMap<String, bool>,
}

impl Controller {
    /// Records whether the widget of `bundle_id` has content (`setHasContent(_:forWidgetWithBundleIdentifier:)`).
    pub fn set_has_content(&mut self, bundle_id: &str, has_content: bool) {
        self.flags.insert(bundle_id.to_string(), has_content);
    }

    /// True unless the widget reported that it has no content.
    pub fn has_content(&self, bundle_id: &str) -> bool {
        self.flags.get(bundle_id).copied().unwrap_or(true)
    }

    /// Drops the report for a widget whose app was removed; the widget is visible again until it reports.
    pub fn forget(&mut self, bundle_id: &str) {
        self.flags.remove(bundle_id);
    }

    /// The widgets to show, in the order given, with the empty ones left out.
    pub fn visible<'a>(&self, widgets: &'a [String]) -> Vec<&'a str> {
        widgets.iter().filter(|id| self.has_content(id)).map(|id| id.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_more_button_toggles_between_the_two_modes() {
        assert_eq!(DisplayMode::default(), DisplayMode::Compact);
        assert_eq!(DisplayMode::Compact.toggled(), DisplayMode::Expanded);
        assert_eq!(DisplayMode::Compact.toggled().toggled(), DisplayMode::Compact);
    }

    #[test]
    fn a_widget_keeps_the_defaults_unless_it_returns_its_own_margins() {
        let proposed = Insets::uniform(8.0);
        assert_eq!(margin_insets(proposed, None), proposed);
        let custom = Insets { top: 0.0, left: 4.0, bottom: 12.0, right: 4.0 };
        assert_eq!(margin_insets(proposed, Some(custom)), custom);
    }

    #[test]
    fn each_mode_reports_its_own_maximum_height_at_the_column_width() {
        let limits = Limits::new(342.0, 110.0, 340.0);
        assert_eq!(limits.max_size(DisplayMode::Compact), (342.0, 110.0));
        assert_eq!(limits.max_size(DisplayMode::Expanded), (342.0, 340.0));
    }

    #[test]
    fn an_expanded_height_is_never_shorter_than_the_compact_one() {
        let limits = Limits::new(300.0, 120.0, 80.0);
        assert_eq!(limits.expanded_h, 120.0);
        assert_eq!(limits.max_size(DisplayMode::Expanded).1, 120.0);
    }

    #[test]
    fn a_widget_that_never_reported_is_shown() {
        let c = Controller::default();
        assert!(c.has_content("is.example.weather"));
    }

    #[test]
    fn a_widget_that_reports_no_content_is_left_out_in_order() {
        let mut c = Controller::default();
        c.set_has_content("is.example.b", false);
        let ids = vec!["is.example.a".to_string(), "is.example.b".to_string(), "is.example.c".to_string()];
        assert_eq!(c.visible(&ids), vec!["is.example.a", "is.example.c"]);
        c.set_has_content("is.example.b", true);
        assert_eq!(c.visible(&ids), vec!["is.example.a", "is.example.b", "is.example.c"]);
    }

    #[test]
    fn forgetting_a_widget_makes_it_visible_again() {
        let mut c = Controller::default();
        c.set_has_content("is.example.b", false);
        assert!(!c.has_content("is.example.b"));
        c.forget("is.example.b");
        assert!(c.has_content("is.example.b"));
    }
}

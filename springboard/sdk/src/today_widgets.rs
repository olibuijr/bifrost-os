//! The app side of the Today widget list: the widgets an app supplies, in order, with the "has content" flag that
//! decides whether the shell shows each one. This is the SDK type that stands for the widget protocol
//! (`NCWidgetProviding`) that the shell's model in `bifrost_ui::widget` implements. [`WidgetSet::add`] registers a
//! widget by bundle identifier, [`WidgetSet::set_has_content`] reports whether it has anything to show, and
//! [`WidgetSet::visible`] gives the widgets the shell draws, in order, without the empty ones. The sizes come from
//! [`Limits::max_size`]. Gaps: the shell does not yet draw a widget from an app's own view, so this module sets the
//! order and the flags only, and the shell still draws its own widgets.

use bifrost_ui::widget::Controller;

pub use bifrost_ui::widget::{margin_insets, DisplayMode, Insets, Limits};

/// The widgets an app supplies, in the order they are listed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WidgetSet {
    order: Vec<String>,
    controller: Controller,
}

impl WidgetSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a widget at the end of the list. Returns false when the bundle identifier is empty or already listed.
    pub fn add(&mut self, bundle_id: &str) -> bool {
        let bundle_id = bundle_id.trim();
        if bundle_id.is_empty() || self.order.iter().any(|id| id == bundle_id) {
            return false;
        }
        self.order.push(bundle_id.to_string());
        true
    }

    /// Removes a widget and forgets its flag. Returns false when it was not listed.
    pub fn remove(&mut self, bundle_id: &str) -> bool {
        let before = self.order.len();
        self.order.retain(|id| id != bundle_id);
        self.controller.forget(bundle_id);
        self.order.len() != before
    }

    /// Reports whether the widget has content. Returns false, and changes nothing, for a widget that is not listed.
    pub fn set_has_content(&mut self, bundle_id: &str, has_content: bool) -> bool {
        if !self.order.iter().any(|id| id == bundle_id) {
            return false;
        }
        self.controller.set_has_content(bundle_id, has_content);
        true
    }

    /// The listed widgets in order, including the empty ones.
    pub fn order(&self) -> &[String] {
        &self.order
    }

    /// The widgets the shell draws, in order. A widget that has not reported is shown.
    pub fn visible(&self) -> Vec<&str> {
        self.controller.visible(&self.order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_keep_their_order_and_refuse_duplicates_and_blank_ids() {
        let mut set = WidgetSet::new();
        assert!(set.add("com.example.weather"));
        assert!(set.add("com.example.tasks"));
        assert!(!set.add("com.example.weather"));
        assert!(!set.add("   "));
        assert_eq!(set.order(), &["com.example.weather".to_string(), "com.example.tasks".to_string()]);
    }

    #[test]
    fn an_empty_widget_is_left_out_and_a_silent_one_is_shown() {
        let mut set = WidgetSet::new();
        set.add("a.one");
        set.add("a.two");
        set.add("a.three");
        assert!(set.set_has_content("a.two", false));
        assert_eq!(set.visible(), vec!["a.one", "a.three"]);
        assert!(set.set_has_content("a.two", true));
        assert_eq!(set.visible(), vec!["a.one", "a.two", "a.three"]);
    }

    #[test]
    fn a_flag_for_an_unlisted_widget_is_refused() {
        let mut set = WidgetSet::new();
        assert!(!set.set_has_content("a.missing", false));
        assert!(set.visible().is_empty());
    }

    #[test]
    fn removing_a_widget_forgets_its_flag() {
        let mut set = WidgetSet::new();
        set.add("a.one");
        set.set_has_content("a.one", false);
        assert!(set.remove("a.one"));
        assert!(!set.remove("a.one"));
        set.add("a.one");
        assert_eq!(set.visible(), vec!["a.one"]);
    }

    #[test]
    fn the_expanded_mode_is_taller_than_the_compact_one() {
        let limits = Limits::new(320.0, 110.0, 220.0);
        let (_, compact) = limits.max_size(DisplayMode::Compact);
        let (_, expanded) = limits.max_size(DisplayMode::Expanded);
        assert!(expanded > compact);
    }
}

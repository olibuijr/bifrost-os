//! Content-unavailable configuration (`UIContentUnavailableConfiguration`): the image, title, secondary text, buttons and
//! layout axis of an empty-state page, with the empty and search presets. It is a plain value; the SDK view draws it and
//! the app draws the image named by `image`.

/// A button under the text; `action` travels up the responder chain when the button is pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentButton {
    pub title: String,
    pub action: String,
}

/// How the parts are arranged (`UIContentUnavailableConfiguration.axis`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    #[default]
    Vertical,
    Horizontal,
}

/// An empty-state configuration. Presets: [`ContentUnavailable::empty`] and [`ContentUnavailable::search`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentUnavailable {
    /// the name of the symbol the app draws, if any
    pub image: Option<String>,
    pub text: String,
    pub secondary_text: String,
    pub buttons: Vec<ContentButton>,
    pub axis: Axis,
}

impl ContentUnavailable {
    /// The empty preset: a tray symbol and a short title.
    pub fn empty() -> Self {
        ContentUnavailable { image: Some("tray".into()), text: "No Content".into(), ..Default::default() }
    }

    /// The search preset for a search that found nothing.
    pub fn search() -> Self {
        ContentUnavailable {
            image: Some("magnifyingglass".into()),
            text: "No Results".into(),
            secondary_text: "Check the spelling or try a new search.".into(),
            ..Default::default()
        }
    }

    pub fn with_image(mut self, symbol: &str) -> Self {
        self.image = Some(symbol.into());
        self
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.text = text.into();
        self
    }

    pub fn with_secondary_text(mut self, text: &str) -> Self {
        self.secondary_text = text.into();
        self
    }

    pub fn with_button(mut self, title: &str, action: &str) -> Self {
        self.buttons.push(ContentButton { title: title.into(), action: action.into() });
        self
    }

    pub fn with_axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// True when the configuration shows nothing at all.
    pub fn is_blank(&self) -> bool {
        self.image.is_none() && self.text.is_empty() && self.secondary_text.is_empty() && self.buttons.is_empty()
    }

    /// The text a screen reader reads for the page: the title and the secondary text, one sentence after the other.
    pub fn accessibility_label(&self) -> String {
        [self.text.as_str(), self.secondary_text.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(". ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_preset_has_a_tray_and_a_title_but_no_secondary_text() {
        let c = ContentUnavailable::empty();
        assert_eq!(c.image.as_deref(), Some("tray"));
        assert_eq!(c.text, "No Content");
        assert!(c.secondary_text.is_empty());
        assert!(c.buttons.is_empty());
        assert_eq!(c.axis, Axis::Vertical);
    }

    #[test]
    fn search_preset_names_the_magnifier_and_explains_the_result() {
        let c = ContentUnavailable::search();
        assert_eq!(c.image.as_deref(), Some("magnifyingglass"));
        assert_eq!(c.text, "No Results");
        assert_eq!(c.secondary_text, "Check the spelling or try a new search.");
    }

    #[test]
    fn builders_add_parts_and_buttons_in_order() {
        let c = ContentUnavailable::default()
            .with_text("Nothing yet")
            .with_secondary_text("Add a note to begin.")
            .with_button("New Note", "newNote")
            .with_button("Import", "importNotes")
            .with_axis(Axis::Horizontal);
        assert_eq!(c.buttons.len(), 2);
        assert_eq!(c.buttons[1], ContentButton { title: "Import".into(), action: "importNotes".into() });
        assert_eq!(c.axis, Axis::Horizontal);
        assert_eq!(c.image, None);
    }

    #[test]
    fn a_default_configuration_is_blank_and_an_image_alone_is_not() {
        assert!(ContentUnavailable::default().is_blank());
        assert!(!ContentUnavailable::default().with_image("tray").is_blank());
        assert!(!ContentUnavailable::default().with_button("Retry", "retry").is_blank());
    }

    #[test]
    fn accessibility_label_joins_the_non_empty_parts() {
        assert_eq!(ContentUnavailable::search().accessibility_label(), "No Results. Check the spelling or try a new search.");
        assert_eq!(ContentUnavailable::default().with_secondary_text("Only this").accessibility_label(), "Only this");
        assert_eq!(ContentUnavailable::default().accessibility_label(), "");
    }
}

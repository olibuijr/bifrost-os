//! The spoken description of a widget for assistive technology: a label, a value and a hint, and how several
//! widget parts join into one phrase. Pure model: text assembly only. A decorative widget (a background or a
//! repeated picture) is skipped entirely.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidgetAccessibility {
    pub label: String,
    pub value: String,
    pub hint: String,
    /// A decorative widget is left out of the spoken output.
    pub decorative: bool,
}

impl WidgetAccessibility {
    pub fn new(label: &str) -> Self {
        WidgetAccessibility { label: label.to_string(), ..Default::default() }
    }

    pub fn with_value(mut self, value: &str) -> Self {
        self.value = value.to_string();
        self
    }

    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = hint.to_string();
        self
    }

    /// The label, value and hint joined with ", " in that order. Blank parts are left out, and an empty result is
    /// returned for a decorative widget.
    pub fn spoken(&self) -> String {
        if self.decorative {
            return String::new();
        }
        let parts: Vec<&str> = vec![self.label.trim(), self.value.trim(), self.hint.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        parts.join(", ")
    }
}

/// One phrase for a widget made of several parts (a title, then two readouts): the spoken text of each part that
/// has any, joined with ". ".
pub fn combine(parts: &[WidgetAccessibility]) -> String {
    let phrases: Vec<String> = parts.iter().map(|p| p.spoken()).filter(|s| !s.is_empty()).collect();
    phrases.join(". ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_and_value_join_with_a_comma() {
        let w = WidgetAccessibility::new("Weather").with_value("12 °C");
        assert_eq!(w.spoken(), "Weather, 12 °C");
    }

    #[test]
    fn blank_parts_are_left_out_and_text_is_trimmed() {
        let w = WidgetAccessibility::new(" Clock ").with_hint("Double tap to open");
        assert_eq!(w.spoken(), "Clock, Double tap to open");
    }

    #[test]
    fn value_alone_is_spoken_alone() {
        let w = WidgetAccessibility::default().with_value("Off");
        assert_eq!(w.spoken(), "Off");
    }

    #[test]
    fn empty_widget_speaks_nothing() {
        assert_eq!(WidgetAccessibility::new("   ").spoken(), "");
    }

    #[test]
    fn decorative_widget_speaks_nothing() {
        let mut w = WidgetAccessibility::new("Background").with_value("Blue");
        w.decorative = true;
        assert_eq!(w.spoken(), "");
    }

    #[test]
    fn combine_joins_parts_and_skips_empty_ones() {
        let mut bg = WidgetAccessibility::new("Pattern");
        bg.decorative = true;
        let parts = vec![
            WidgetAccessibility::new("Battery").with_value("80%"),
            bg,
            WidgetAccessibility::new("Calendar").with_value("3 events"),
            WidgetAccessibility::default(),
        ];
        assert_eq!(combine(&parts), "Battery, 80%. Calendar, 3 events");
    }
}

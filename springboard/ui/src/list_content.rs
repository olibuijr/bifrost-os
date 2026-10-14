//! Declarative content configuration for list cells (the reference toolkit list content configuration).
//!
//! A cell describes what it shows: a primary text, an optional secondary text, an optional image, the layout margins
//! and the text colours per interaction state. [`ListContentConfiguration::layout`] places those parts inside the cell
//! bounds, and the list cell draws them with the fonts and colours the configuration resolves. An app sets the values
//! and never positions the parts itself.

use crate::background_configuration::{ConfigurationState, Rgba, StateValue};
use crate::geometry::EdgeInsets;
use crate::Rect;

/// The font role of a text part. The toolkit maps each role to a font and a size for the user's text scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Headline,
    Body,
    Subhead,
    Caption,
}

impl TextStyle {
    /// The point size of the role at the default text scale (1.0).
    pub const fn base_point_size(self) -> f32 {
        match self {
            TextStyle::Headline => 17.0,
            TextStyle::Body => 17.0,
            TextStyle::Subhead => 15.0,
            TextStyle::Caption => 12.0,
        }
    }

    /// Whether the role is set in the bold weight.
    pub const fn is_bold(self) -> bool {
        matches!(self, TextStyle::Headline)
    }

    /// The point size at a user text scale (1.0 is the default). Non-positive scales count as the default.
    pub fn point_size(self, text_scale: f32) -> f32 {
        let scale = if text_scale > 0.0 { text_scale } else { 1.0 };
        self.base_point_size() * scale
    }
}

/// The content of a list cell: its text parts, image, margins and colours.
#[derive(Clone, Debug, PartialEq)]
pub struct ListContentConfiguration {
    pub text: Option<String>,
    pub secondary_text: Option<String>,
    /// Identifier of the image the cell shows; the toolkit resolves it to pixels.
    pub image: Option<String>,
    pub text_style: TextStyle,
    pub secondary_style: TextStyle,
    /// Width and height of the image, in points.
    pub image_size: (f32, f32),
    /// Space between the image and the text column, in points.
    pub image_spacing: f32,
    /// Space between the primary and the secondary text, in points.
    pub text_spacing: f32,
    /// Space between the cell bounds and the content.
    pub margins: EdgeInsets,
    /// Colour of the primary text per state. `None` keeps the toolkit's label colour.
    pub text_color: StateValue<Option<Rgba>>,
    /// Colour of the secondary text per state. `None` keeps the toolkit's secondary label colour.
    pub secondary_color: StateValue<Option<Rgba>>,
}

/// Where each part of the content sits, in the coordinate space of the cell bounds. A part with no content is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContentLayout {
    pub image: Option<Rect>,
    pub text: Option<Rect>,
    pub secondary_text: Option<Rect>,
}

impl Default for ListContentConfiguration {
    /// An empty cell with the default margins and spacing: a 16 point side gutter and 11 point top and bottom.
    fn default() -> Self {
        ListContentConfiguration {
            text: None,
            secondary_text: None,
            image: None,
            text_style: TextStyle::Body,
            secondary_style: TextStyle::Subhead,
            image_size: (40.0, 40.0),
            image_spacing: 12.0,
            text_spacing: 2.0,
            margins: EdgeInsets { top: 11.0, left: 16.0, bottom: 11.0, right: 16.0 },
            text_color: StateValue::new(None),
            secondary_color: StateValue::new(None),
        }
    }
}

impl ListContentConfiguration {
    /// A cell with a primary text only.
    pub fn text(text: &str) -> Self {
        ListContentConfiguration { text: Some(text.to_string()), ..Default::default() }
    }

    /// A cell with a primary text and a secondary text below it.
    pub fn subtitle(text: &str, secondary: &str) -> Self {
        ListContentConfiguration {
            text: Some(text.to_string()),
            secondary_text: Some(secondary.to_string()),
            ..Default::default()
        }
    }

    /// True when the cell has no text and no image to show.
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.secondary_text.is_none() && self.image.is_none()
    }

    /// Places the image, the primary text and the secondary text inside `bounds`.
    ///
    /// The image sits at the leading edge, centred vertically. The text column fills the space to its right, and the
    /// two text lines form one block that is centred vertically. A block taller than the content area starts at its top
    /// edge, so the first line stays visible. `text_h` and `secondary_h` are the heights the caller measured for each line
    /// at the current text scale; a part without content ignores its height.
    pub fn layout(&self, bounds: Rect, text_h: f32, secondary_h: f32) -> ContentLayout {
        let area = bounds.inset(self.margins);
        let mut out = ContentLayout::default();
        let (mut column_x, mut column_w) = (area.x, area.w);

        if self.image.is_some() {
            let w = self.image_size.0.max(0.0).min(area.w);
            let h = self.image_size.1.max(0.0).min(area.h);
            out.image = Some(Rect { x: area.x, y: area.y + (area.h - h) / 2.0, w, h });
            let used = (w + self.image_spacing.max(0.0)).min(area.w);
            column_x = area.x + used;
            column_w = (area.w - used).max(0.0);
        }

        let has_text = self.text.is_some();
        let has_secondary = self.secondary_text.is_some();
        let primary_h = if has_text { text_h.max(0.0) } else { 0.0 };
        let second_h = if has_secondary { secondary_h.max(0.0) } else { 0.0 };
        let gap = if has_text && has_secondary { self.text_spacing.max(0.0) } else { 0.0 };
        let block = primary_h + gap + second_h;

        let mut y = area.y + ((area.h - block) / 2.0).max(0.0);
        if has_text {
            out.text = Some(Rect { x: column_x, y, w: column_w, h: primary_h });
            y += primary_h + gap;
        }
        if has_secondary {
            out.secondary_text = Some(Rect { x: column_x, y, w: column_w, h: second_h });
        }
        out
    }

    /// The primary text colour for `state`, or `None` for the toolkit label colour.
    pub fn resolved_text_color(&self, state: ConfigurationState) -> Option<Rgba> {
        self.text_color.resolve(state)
    }

    /// The secondary text colour for `state`, or `None` for the toolkit secondary label colour.
    pub fn resolved_secondary_color(&self, state: ConfigurationState) -> Option<Rgba> {
        self.secondary_color.resolve(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rect = Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 };
    const RED: Rgba = [255, 0, 0, 255];
    const BLUE: Rgba = [0, 0, 255, 255];

    fn no_margins(mut c: ListContentConfiguration) -> ListContentConfiguration {
        c.margins = EdgeInsets::ZERO;
        c
    }

    #[test]
    fn text_styles_scale_and_bold_only_the_headline() {
        assert_eq!(TextStyle::Body.point_size(1.0), 17.0);
        assert_eq!(TextStyle::Caption.point_size(2.0), 24.0);
        assert_eq!(TextStyle::Subhead.point_size(0.0), 15.0);
        assert!(TextStyle::Headline.is_bold());
        assert!(!TextStyle::Body.is_bold());
    }

    #[test]
    fn empty_configuration_has_no_parts() {
        let c = ListContentConfiguration::default();
        assert!(c.is_empty());
        let l = c.layout(BOUNDS, 20.0, 15.0);
        assert_eq!(l, ContentLayout::default());
    }

    #[test]
    fn text_only_is_centred_vertically_in_the_margins() {
        let c = ListContentConfiguration { margins: EdgeInsets::symmetric(10.0, 10.0), ..ListContentConfiguration::text("A") };
        let l = c.layout(BOUNDS, 20.0, 0.0);
        // Content area is 280 x 80 at (10, 10); a 20 point line sits centred at y = 40.
        assert_eq!(l.text, Some(Rect { x: 10.0, y: 40.0, w: 280.0, h: 20.0 }));
        assert_eq!(l.image, None);
        assert_eq!(l.secondary_text, None);
    }

    #[test]
    fn image_and_text_column_share_the_row() {
        let mut c = no_margins(ListContentConfiguration::text("A"));
        c.image = Some("icon".to_string());
        let l = c.layout(BOUNDS, 20.0, 0.0);
        assert_eq!(l.image, Some(Rect { x: 0.0, y: 30.0, w: 40.0, h: 40.0 }));
        // The text column starts after the image and its spacing (40 + 12) and takes the rest.
        assert_eq!(l.text, Some(Rect { x: 52.0, y: 40.0, w: 248.0, h: 20.0 }));
    }

    #[test]
    fn two_lines_form_one_centred_block_with_spacing() {
        let c = no_margins(ListContentConfiguration::subtitle("Title", "Detail"));
        let l = c.layout(BOUNDS, 20.0, 15.0);
        // Block height 20 + 2 + 15 = 37, top at (100 - 37) / 2.
        let top = (100.0 - 37.0) / 2.0;
        assert_eq!(l.text, Some(Rect { x: 0.0, y: top, w: 300.0, h: 20.0 }));
        assert_eq!(l.secondary_text, Some(Rect { x: 0.0, y: top + 22.0, w: 300.0, h: 15.0 }));
    }

    #[test]
    fn oversized_block_starts_at_the_top_edge() {
        let c = no_margins(ListContentConfiguration::subtitle("Title", "Detail"));
        let l = c.layout(BOUNDS, 80.0, 60.0);
        assert_eq!(l.text.unwrap().y, 0.0);
        assert_eq!(l.secondary_text.unwrap().y, 82.0);
    }

    #[test]
    fn secondary_alone_takes_no_spacing() {
        let mut c = no_margins(ListContentConfiguration::default());
        c.secondary_text = Some("Only".to_string());
        let l = c.layout(BOUNDS, 20.0, 15.0);
        assert_eq!(l.text, None);
        assert_eq!(l.secondary_text, Some(Rect { x: 0.0, y: 42.5, w: 300.0, h: 15.0 }));
    }

    #[test]
    fn image_wider_than_the_cell_is_clamped() {
        let mut c = no_margins(ListContentConfiguration::text("A"));
        c.image = Some("big".to_string());
        c.image_size = (500.0, 40.0);
        let l = c.layout(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, 20.0, 0.0);
        assert_eq!(l.image.unwrap().w, 100.0);
        assert_eq!(l.text.unwrap().w, 0.0);
    }

    #[test]
    fn colours_follow_the_interaction_state() {
        let mut c = ListContentConfiguration::default();
        c.text_color = StateValue { normal: None, highlighted: None, selected: Some(BLUE), disabled: None };
        c.secondary_color = StateValue { normal: Some(RED), highlighted: None, selected: None, disabled: None };
        let selected = ConfigurationState { selected: true, ..Default::default() };
        assert_eq!(c.resolved_text_color(selected), Some(BLUE));
        assert_eq!(c.resolved_text_color(ConfigurationState::default()), None);
        assert_eq!(c.resolved_secondary_color(selected), Some(RED));
    }
}

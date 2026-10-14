//! Declarative background configuration for views and list cells (the reference toolkit background configuration).
//!
//! A configuration holds the fill, the stroke, the corner radius and the inset from the view frame, with a value per
//! interaction state. [`BackgroundConfiguration::resolve`] returns the values for one state and the frame the background
//! paints in. The view paints the result; a list cell applies its background configuration the same way.

use crate::geometry::EdgeInsets;
use crate::Rect;

/// A colour as red, green, blue and alpha bytes.
pub type Rgba = [u8; 4];

/// The interaction state a configuration resolves against. A disabled state beats a selected one, and a selected state
/// beats a highlighted one (the pressed state while a touch or pointer holds the cell).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigurationState {
    pub highlighted: bool,
    pub selected: bool,
    pub disabled: bool,
}

/// One value per interaction state. A state without its own value uses `normal`.
#[derive(Clone, Debug, PartialEq)]
pub struct StateValue<T> {
    pub normal: T,
    pub highlighted: Option<T>,
    pub selected: Option<T>,
    pub disabled: Option<T>,
}

impl<T: Clone> StateValue<T> {
    /// A value that is the same in every state.
    pub fn new(normal: T) -> Self {
        StateValue { normal, highlighted: None, selected: None, disabled: None }
    }

    /// The value for `state`: the disabled value, then the selected value, then the highlighted value, then `normal`.
    /// A flag that is set only uses its own value when one is given; otherwise the next flag in the order applies.
    pub fn resolve(&self, state: ConfigurationState) -> T {
        let candidates = [
            (state.disabled, &self.disabled),
            (state.selected, &self.selected),
            (state.highlighted, &self.highlighted),
        ];
        candidates
            .iter()
            .find(|&&(on, value)| on && value.is_some())
            .and_then(|&(_, value)| value.clone())
            .unwrap_or_else(|| self.normal.clone())
    }
}

/// The background of a view or list cell, declared per state.
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundConfiguration {
    pub fill: StateValue<Option<Rgba>>,
    pub stroke: StateValue<Option<Rgba>>,
    /// Width of the stroke in points. Zero paints no stroke whatever the stroke colour is.
    pub stroke_width: f32,
    /// Corner radius in points. Negative values count as zero, and the painted radius is at most half the shorter side.
    pub corner_radius: f32,
    /// Space between the view frame and the painted background, for example the gap between two cells.
    pub edge_insets: EdgeInsets,
}

/// The values a background paints with for one state, ready for a renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBackground {
    /// The painted rectangle: the view frame inset by the configuration's edge insets.
    pub frame: Rect,
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    pub stroke_width: f32,
    pub corner_radius: f32,
}

impl BackgroundConfiguration {
    /// A background that paints nothing in any state.
    pub fn clear() -> Self {
        BackgroundConfiguration {
            fill: StateValue::new(None),
            stroke: StateValue::new(None),
            stroke_width: 0.0,
            corner_radius: 0.0,
            edge_insets: EdgeInsets::ZERO,
        }
    }

    /// A filled background: `normal` at rest, `highlighted` while pressed and `selected` when the cell is selected.
    pub fn filled(normal: Rgba, highlighted: Rgba, selected: Rgba) -> Self {
        BackgroundConfiguration {
            fill: StateValue {
                normal: Some(normal),
                highlighted: Some(highlighted),
                selected: Some(selected),
                disabled: None,
            },
            ..BackgroundConfiguration::clear()
        }
    }

    /// The painted rectangle and the values for `state`, for a view whose frame is `frame`.
    pub fn resolve(&self, frame: Rect, state: ConfigurationState) -> ResolvedBackground {
        let painted = frame.inset(self.edge_insets);
        let stroke_width = self.stroke_width.max(0.0);
        let radius = self.corner_radius.max(0.0).min(painted.w.min(painted.h) / 2.0);
        ResolvedBackground {
            frame: painted,
            fill: self.fill.resolve(state),
            stroke: if stroke_width > 0.0 { self.stroke.resolve(state) } else { None },
            stroke_width,
            corner_radius: radius,
        }
    }
}

impl Default for BackgroundConfiguration {
    fn default() -> Self {
        BackgroundConfiguration::clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = [255, 0, 0, 255];
    const GREEN: Rgba = [0, 255, 0, 255];
    const BLUE: Rgba = [0, 0, 255, 255];
    const GREY: Rgba = [128, 128, 128, 255];

    fn frame(w: f32, h: f32) -> Rect {
        Rect { x: 0.0, y: 0.0, w, h }
    }

    #[test]
    fn state_value_uses_normal_when_no_state_value_is_set() {
        let v = StateValue::new(RED);
        assert_eq!(v.resolve(ConfigurationState::default()), RED);
        let pressed = ConfigurationState { highlighted: true, ..Default::default() };
        assert_eq!(v.resolve(pressed), RED);
    }

    #[test]
    fn disabled_beats_selected_beats_highlighted() {
        let v = StateValue {
            normal: RED,
            highlighted: Some(GREEN),
            selected: Some(BLUE),
            disabled: Some(GREY),
        };
        let all = ConfigurationState { highlighted: true, selected: true, disabled: true };
        assert_eq!(v.resolve(all), GREY);
        let selected_pressed = ConfigurationState { highlighted: true, selected: true, disabled: false };
        assert_eq!(v.resolve(selected_pressed), BLUE);
        let pressed = ConfigurationState { highlighted: true, ..Default::default() };
        assert_eq!(v.resolve(pressed), GREEN);
    }

    #[test]
    fn disabled_without_its_own_value_falls_through_to_selected() {
        let v = StateValue { normal: RED, highlighted: None, selected: Some(BLUE), disabled: None };
        let s = ConfigurationState { highlighted: false, selected: true, disabled: true };
        assert_eq!(v.resolve(s), BLUE);
    }

    #[test]
    fn clear_background_paints_nothing() {
        let r = BackgroundConfiguration::clear().resolve(frame(100.0, 40.0), ConfigurationState::default());
        assert_eq!(r.fill, None);
        assert_eq!(r.stroke, None);
        assert_eq!(r.frame, frame(100.0, 40.0));
    }

    #[test]
    fn filled_background_changes_with_state() {
        let b = BackgroundConfiguration::filled(RED, GREEN, BLUE);
        let rest = b.resolve(frame(10.0, 10.0), ConfigurationState::default());
        assert_eq!(rest.fill, Some(RED));
        let pressed = b.resolve(frame(10.0, 10.0), ConfigurationState { highlighted: true, ..Default::default() });
        assert_eq!(pressed.fill, Some(GREEN));
        let selected = b.resolve(frame(10.0, 10.0), ConfigurationState { selected: true, ..Default::default() });
        assert_eq!(selected.fill, Some(BLUE));
    }

    #[test]
    fn edge_insets_shrink_the_painted_frame() {
        let mut b = BackgroundConfiguration::clear();
        b.edge_insets = EdgeInsets { top: 2.0, left: 4.0, bottom: 2.0, right: 4.0 };
        let r = b.resolve(frame(100.0, 40.0), ConfigurationState::default());
        assert_eq!(r.frame, Rect { x: 4.0, y: 2.0, w: 92.0, h: 36.0 });
    }

    #[test]
    fn corner_radius_fits_the_shape() {
        let mut b = BackgroundConfiguration::clear();
        b.corner_radius = 20.0;
        let big = b.resolve(frame(100.0, 100.0), ConfigurationState::default());
        assert_eq!(big.corner_radius, 20.0);
        b.corner_radius = 20.0;
        let thin = b.resolve(frame(100.0, 4.0), ConfigurationState::default());
        assert_eq!(thin.corner_radius, 2.0);
        b.corner_radius = -3.0;
        assert_eq!(b.resolve(frame(10.0, 10.0), ConfigurationState::default()).corner_radius, 0.0);
    }

    #[test]
    fn stroke_needs_a_positive_width() {
        let mut b = BackgroundConfiguration::clear();
        b.stroke = StateValue::new(Some(GREY));
        b.stroke_width = 0.0;
        assert_eq!(b.resolve(frame(10.0, 10.0), ConfigurationState::default()).stroke, None);
        b.stroke_width = 1.0;
        let r = b.resolve(frame(10.0, 10.0), ConfigurationState::default());
        assert_eq!(r.stroke, Some(GREY));
        assert_eq!(r.stroke_width, 1.0);
    }
}

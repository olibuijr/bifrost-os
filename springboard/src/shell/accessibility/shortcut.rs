//! The Accessibility Shortcut (Option+Cmd+F5). One key switches the chosen accessibility features on, or off again when
//! they are all on already. The choice is the `a11y_shortcut` pref: a comma list of feature keys (`screen_reader`,
//! `zoom`, ...). With nothing chosen the key does nothing. The panel that lets the user pick them is not built yet.
//!
//! The toggle works on the same `AccessibilitySettings` the Settings window edits, so an open Settings page shows the
//! switches moving and the choice is saved like any other setting.

use super::{AccessibilitySettings, ZOOMS};
use crate::shell::prefs::Prefs;
use crate::shell::Shell;

/// The pref that stores the chosen features.
pub const PREF: &str = "a11y_shortcut";

/// A feature the shortcut can switch. Bold Text is left out: its value lives in the display options, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    ScreenReader,
    Zoom,
    ReduceMotion,
    IncreaseContrast,
    ReduceTransparency,
    ButtonShapes,
    DimFlashingLights,
}

impl Feature {
    /// Every feature, in the order the panel lists them.
    pub const ALL: [Feature; 7] = [
        Feature::ScreenReader,
        Feature::Zoom,
        Feature::ReduceMotion,
        Feature::IncreaseContrast,
        Feature::ReduceTransparency,
        Feature::ButtonShapes,
        Feature::DimFlashingLights,
    ];

    /// The key stored in the pref.
    pub fn key(self) -> &'static str {
        match self {
            Feature::ScreenReader => "screen_reader",
            Feature::Zoom => "zoom",
            Feature::ReduceMotion => "reduce_motion",
            Feature::IncreaseContrast => "increase_contrast",
            Feature::ReduceTransparency => "reduce_transparency",
            Feature::ButtonShapes => "button_shapes",
            Feature::DimFlashingLights => "dim_flashing_lights",
        }
    }

    /// The name the Settings page uses for the feature.
    pub fn name(self) -> &'static str {
        match self {
            Feature::ScreenReader => "Screen Reader",
            Feature::Zoom => "Display Zoom",
            Feature::ReduceMotion => "Reduce Motion",
            Feature::IncreaseContrast => "Increase Contrast",
            Feature::ReduceTransparency => "Reduce Transparency",
            Feature::ButtonShapes => "Show Borders",
            Feature::DimFlashingLights => "Dim Flashing Lights",
        }
    }

    /// Whether the feature is on in `a`. Display Zoom is on at any zoom above Standard.
    pub fn is_on(self, a: &AccessibilitySettings) -> bool {
        match self {
            Feature::ScreenReader => a.screen_reader,
            Feature::Zoom => a.zoom != 0,
            Feature::ReduceMotion => a.reduce_motion,
            Feature::IncreaseContrast => a.increase_contrast,
            Feature::ReduceTransparency => a.reduce_transparency,
            Feature::ButtonShapes => a.button_shapes,
            Feature::DimFlashingLights => a.dim_flashing_lights,
        }
    }

    /// Sets the feature in `a`. Display Zoom goes to the largest step, or back to Standard.
    pub fn set(self, a: &mut AccessibilitySettings, on: bool) {
        match self {
            Feature::ScreenReader => a.screen_reader = on,
            Feature::Zoom => a.zoom = if on { ZOOMS.len() - 1 } else { 0 },
            Feature::ReduceMotion => a.reduce_motion = on,
            Feature::IncreaseContrast => a.increase_contrast = on,
            Feature::ReduceTransparency => a.reduce_transparency = on,
            Feature::ButtonShapes => a.button_shapes = on,
            Feature::DimFlashingLights => a.dim_flashing_lights = on,
        }
    }

}

/// The features named in a stored list, in `Feature::ALL` order, each once. Unknown names are ignored.
pub fn parse(list: &str) -> Vec<Feature> {
    let named: Vec<&str> = list.split(',').map(str::trim).collect();
    Feature::ALL.into_iter().filter(|f| named.contains(&f.key())).collect()
}

/// The stored form of `features`: their keys in `Feature::ALL` order, comma separated.
pub fn encode(features: &[Feature]) -> String {
    Feature::ALL.into_iter().filter(|f| features.contains(f)).map(Feature::key).collect::<Vec<_>>().join(",")
}

/// The features the shortcut switches, from the pref.
pub fn chosen(prefs: &Prefs) -> Vec<Feature> {
    parse(prefs.get(PREF).unwrap_or(""))
}

/// Switches `features` in `a`: all off when every one is already on, otherwise all on. Returns whether they are on now.
/// Nothing changes for an empty list.
pub fn toggle(a: &mut AccessibilitySettings, features: &[Feature]) -> bool {
    if features.is_empty() {
        return false;
    }
    let all_on = features.iter().all(|f| f.is_on(a));
    for f in features {
        f.set(a, !all_on);
    }
    !all_on
}

impl Shell {
    /// Option+Cmd+F5: switches the chosen features, updating the open Settings window's copy and saving the result.
    pub(in crate::shell) fn accessibility_shortcut(&mut self) {
        let features = chosen(&self.prefs);
        if features.is_empty() {
            return;
        }
        let mut a = self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs), |st| st.accessibility().clone());
        toggle(&mut a, &features);
        a.save(&mut self.prefs);
        if let Some(st) = &mut self.settings {
            st.set_accessibility(a.clone());
        }
        a.publish();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_known_keys_once_in_panel_order() {
        assert_eq!(parse("zoom,screen_reader,zoom,bold_text,,nope"), vec![Feature::ScreenReader, Feature::Zoom]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn encode_round_trips_through_parse() {
        let f = [Feature::DimFlashingLights, Feature::ScreenReader];
        assert_eq!(encode(&f), "screen_reader,dim_flashing_lights");
        assert_eq!(parse(&encode(&f)), vec![Feature::ScreenReader, Feature::DimFlashingLights]);
    }

    #[test]
    fn the_shortcut_turns_chosen_features_on_then_off() {
        let mut a = AccessibilitySettings::DEFAULT;
        let chosen = [Feature::ReduceMotion, Feature::Zoom];
        assert!(toggle(&mut a, &chosen));
        assert!(a.reduce_motion && a.zoom == ZOOMS.len() - 1);
        assert!(!toggle(&mut a, &chosen));
        assert!(!a.reduce_motion && a.zoom == 0);
    }

    #[test]
    fn a_partly_on_selection_switches_all_on_first() {
        let mut a = AccessibilitySettings { increase_contrast: true, ..AccessibilitySettings::DEFAULT };
        let chosen = [Feature::IncreaseContrast, Feature::ReduceTransparency];
        assert!(toggle(&mut a, &chosen), "not every feature was on, so all go on");
        assert!(a.increase_contrast && a.reduce_transparency);
    }

    #[test]
    fn an_empty_choice_changes_nothing() {
        let mut a = AccessibilitySettings::DEFAULT;
        assert!(!toggle(&mut a, &[]));
        assert_eq!(a, AccessibilitySettings::DEFAULT);
    }

    #[test]
    fn chosen_reads_the_pref() {
        let mut p = Prefs::memory();
        assert!(chosen(&p).is_empty());
        p.set(PREF, "button_shapes,screen_reader").unwrap();
        assert_eq!(chosen(&p), vec![Feature::ScreenReader, Feature::ButtonShapes]);
    }

    #[test]
    fn every_feature_has_a_distinct_key() {
        let mut keys: Vec<&str> = Feature::ALL.iter().map(|f| f.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Feature::ALL.len());
        assert!(Feature::ALL.iter().all(|f| parse(f.key()) == vec![*f]), "each key parses back to its feature");
    }
}

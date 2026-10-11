//! Accessibility settings: Display Zoom, Reduce Motion, Increase Contrast,
//! Reduce Transparency, Button Shapes and Touch Accommodations, persisted in
//! `settings.ini`, plus the central hooks the rest of the shell queries.
//!
//! Larger Text and Bold Text keep living in `Options` (Display & Brightness
//! shares them); `AccessibilitySettings::load` reads the same keys so the
//! hooks cover all eight settings. `publish` mirrors the values into process
//! globals, so any code can ask `accessibility::reduce_motion()` without
//! being handed the settings.

use super::prefs::Prefs;

/// Display Zoom: geometry factor for "Standard" and "Larger".
pub const ZOOMS: [(&str, f32); 2] = [("Standard", 1.0), ("Larger", 1.15)];
/// Touch Accommodations: how long a press must last to count as a hold (ms).
pub const HOLD_MS: [(&str, u32); 5] = [("Off", 0), ("0.5 s", 500), ("1 s", 1000), ("1.5 s", 1500), ("2 s", 2000)];
/// Reduce Motion: an open, close or switch becomes a cross-fade this long.
pub const REDUCED_MS: f64 = 150.0;

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilitySettings {
    pub text_scale: f32,
    pub bold_text: bool,
    /// index into `ZOOMS`
    pub zoom: usize,
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub button_shapes: bool,
    /// index into `HOLD_MS`
    pub hold: usize,
}

impl AccessibilitySettings {
    pub const DEFAULT: Self = AccessibilitySettings { text_scale: 1.0, bold_text: false, zoom: 0, reduce_motion: false, increase_contrast: false, reduce_transparency: false, button_shapes: false, hold: 0 };

    /// Reads every key, keeping the default for a missing or malformed one.
    pub fn load(p: &Prefs) -> Self {
        let d = Self::DEFAULT;
        let flag = |k: &str, default: bool| p.get_bool(k, default);
        let index = |k: &str, len: usize| p.get_parsed::<usize>(k).filter(|&i| i < len);
        AccessibilitySettings {
            text_scale: p.get_parsed::<f32>("text_scale").map_or(d.text_scale, |v| v.clamp(super::ui::TEXT_SCALE_MIN, super::ui::TEXT_SCALE_MAX)),
            bold_text: flag("bold_text", d.bold_text),
            zoom: index("a11y_zoom", ZOOMS.len()).unwrap_or(d.zoom),
            reduce_motion: flag("a11y_reduce_motion", d.reduce_motion),
            increase_contrast: flag("a11y_contrast", d.increase_contrast),
            reduce_transparency: flag("a11y_reduce_transparency", d.reduce_transparency),
            button_shapes: flag("a11y_button_shapes", d.button_shapes),
            hold: index("a11y_hold", HOLD_MS.len()).unwrap_or(d.hold),
        }
    }

    /// Writes the keys this type owns (text size and weight belong to `Options`).
    pub fn save(&self, p: &mut Prefs) {
        let flag = |b: bool| if b { "1" } else { "0" };
        let result = p.set_many(&[
            ("a11y_zoom", &self.zoom.to_string()),
            ("a11y_reduce_motion", flag(self.reduce_motion)),
            ("a11y_contrast", flag(self.increase_contrast)),
            ("a11y_reduce_transparency", flag(self.reduce_transparency)),
            ("a11y_button_shapes", flag(self.button_shapes)),
            ("a11y_hold", &self.hold.to_string()),
        ]);
        if let Err(e) = result {
            eprintln!("springboard: settings.ini: {e}");
        }
    }

    pub fn zoom_factor(&self) -> f32 {
        ZOOMS[self.zoom.min(ZOOMS.len() - 1)].1
    }

    pub fn hold_ms(&self) -> u32 {
        HOLD_MS[self.hold.min(HOLD_MS.len() - 1)].1
    }

    /// Makes these the values the hooks report.
    pub fn publish(&self) {
        bifrost_render::appearance::publish(bifrost_render::appearance::Appearance {
            text_scale: self.text_scale, zoom: self.zoom_factor(), hold_ms: self.hold_ms(),
            bold: self.bold_text, reduce_motion: self.reduce_motion,
            increase_contrast: self.increase_contrast, reduce_transparency: self.reduce_transparency,
            button_shapes: self.button_shapes,
        });
    }
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub use bifrost_render::appearance::{text_scale, bold_text, display_zoom, reduce_motion, increase_contrast, reduce_transparency, button_shapes, touch_hold_ms, contrast, contrast_on, button_shape_radius, anim_ms, long_press_ms};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let mut p = Prefs::memory();
        let a = AccessibilitySettings { zoom: 1, reduce_motion: true, increase_contrast: true, reduce_transparency: true, button_shapes: true, hold: 3, ..Default::default() };
        a.save(&mut p);
        assert_eq!(AccessibilitySettings::load(&p), a);
        assert_eq!(a.hold_ms(), 1500);
        assert!((a.zoom_factor() - 1.15).abs() < 1e-6);
    }

    #[test]
    fn malformed_values_fall_back_to_defaults() {
        let mut p = Prefs::memory();
        p.set("a11y_zoom", "9").unwrap();
        p.set("a11y_hold", "x").unwrap();
        p.set("text_scale", "5.0").unwrap();
        let a = AccessibilitySettings::load(&p);
        assert_eq!((a.zoom, a.hold), (0, 0));
        assert_eq!(a.text_scale, super::super::ui::TEXT_SCALE_MAX);
    }

    #[test]
    fn contrast_lifts_greys_and_leaves_the_rest() {
        use super::super::ui::{SECONDARY, SEPARATOR, WHITE};
        assert_eq!(contrast(SECONDARY), SECONDARY, "off by default");
        assert_ne!(contrast_on(SECONDARY), SECONDARY);
        assert_ne!(contrast_on(SEPARATOR), SEPARATOR);
        assert_eq!(contrast_on(WHITE), WHITE);
        assert!(contrast_on(SECONDARY) & 0xff > SECONDARY & 0xff);
    }

    #[test]
    fn long_press_never_shortens() {
        assert_eq!(long_press_ms(500.0), 500.0_f64.max(touch_hold_ms() as f64));
    }

    #[test]
    fn reduced_motion_caps_the_duration() {
        // only the pure arithmetic: the global stays untouched by tests
        assert_eq!(REDUCED_MS.min(350.0), 150.0);
        assert_eq!(REDUCED_MS.min(100.0), 100.0);
    }
}

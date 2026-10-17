//! Accessibility settings: Display Zoom, Reduce Motion, Increase Contrast,
//! Reduce Transparency, Button Shapes, Dim Flashing Lights and Touch
//! Accommodations, persisted in
//! `settings.ini`, plus the central hooks the rest of the shell queries.
//!
//! Larger Text and Bold Text keep living in `Options` (Display & Brightness
//! shares them); `AccessibilitySettings::load` reads the same keys so the
//! hooks cover all eight settings. `publish` mirrors the values into process
//! globals, so any code can ask `accessibility::reduce_motion()` without
//! being handed the settings.

use super::prefs::Prefs;

pub mod actions;
pub mod app_rotors;
pub mod banner;
pub mod braille_out;
pub mod dwell;
pub mod earcons;
pub mod guided;
pub mod reader;
pub mod shortcut;
pub mod shortcut_panel;
pub mod speech;
pub mod spoken;
pub mod switch_panels;
pub mod switch_scan;
pub mod tree;
pub mod voice_control;
pub mod walk;
pub mod zoom;

/// Display Zoom: geometry factor for "Standard" and "Larger".
pub const ZOOMS: [(&str, f32); 2] = [("Standard", 1.0), ("Larger", 1.15)];
/// Touch Accommodations: how long a press must last to count as a hold (ms).
pub const HOLD_MS: [(&str, u32); 5] = [("Off", 0), ("0.5 s", 500), ("1 s", 1000), ("1.5 s", 1500), ("2 s", 2000)];
/// Reduce Motion: an open, close or switch becomes a cross-fade this long.
pub const REDUCED_MS: f64 = 150.0;
/// Screen Reader speaking rate choices, in words per minute (the engine's `-s` option).
pub const SPEECH_RATES: [(&str, u32); 3] = [("Slow", 140), ("Normal", 180), ("Fast", 240)];
/// Screen Reader pitch choices (the engine's `-p` option, 0 to 99).
pub const SPEECH_PITCHES: [(&str, u32); 3] = [("Low", 30), ("Normal", 50), ("High", 70)];
/// Screen Reader voice choices: the label and the engine voice name (empty is the engine's default voice).
pub const SPEECH_VOICES: [(&str, &str); 4] = [("System Default", ""), ("English (US)", "en-us"), ("English (UK)", "en-gb"), ("Icelandic", "is")];

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
    /// Dim Flashing Lights: apps that read the setting (the `accessibility_display` service) dim flagged sequences.
    pub dim_flashing_lights: bool,
    /// Guided Access: a supervised app may ask for a single-app session (`guided`). Off until the user turns it on.
    pub guided_access: bool,
    /// Screen Reader: a keyboard cursor walks the visible Settings page and speaks each element (`reader`).
    pub screen_reader: bool,
    /// Sticky Keys: a modifier tapped once holds for the next key, tapped twice it locks (`sticky_keys`, read by `HwKeys`).
    pub sticky_keys: bool,
    /// Dwell Control: index into `dwell::DELAYS_MS` (0 is Off). The shell's input loop clicks a resting pointer (`dwell_delay_ms`).
    pub dwell: usize,
    /// Slow Keys: index into `slow_keys::DELAYS_MS` (0 is Off). A key counts only when held for the delay (`slow_delay_ms`).
    pub slow: usize,
    /// Mouse Keys: the number keypad moves the pointer and the 5 key clicks (`mouse_keys`, read by the input loop).
    pub mouse_keys: bool,
    /// Live Captions: the sound the computer plays is turned into captions on screen (`live_captions`, off by default).
    pub live_captions: bool,
    /// index into `HOLD_MS`
    pub hold: usize,
    /// Cursor Size, 0..1 from the Settings slider: the pointer draws at 1x up to 2x (`cursor_scale`)
    pub cursor_size: f32,
    /// the features Option+Cmd+F5 switches, chosen in the Accessibility Shortcut panel (`shortcut::PREF`), panel order
    pub shortcut: Vec<shortcut::Feature>,
    /// Screen Reader speaking rate in words per minute, from `speech::MIN_RATE` to `speech::MAX_RATE` (`SPEECH_RATES` offers three)
    pub speech_rate: u32,
    /// Screen Reader pitch, 0 to `speech::MAX_PITCH` (`SPEECH_PITCHES` offers three)
    pub speech_pitch: u32,
    /// Screen Reader voice: an engine voice name from `SPEECH_VOICES`, or empty for the engine's default
    pub speech_voice: String,
}

impl AccessibilitySettings {
    pub const DEFAULT: Self = AccessibilitySettings { text_scale: 1.0, bold_text: false, zoom: 0, reduce_motion: false, increase_contrast: false, reduce_transparency: false, button_shapes: false, dim_flashing_lights: false, guided_access: false, screen_reader: false, sticky_keys: false, dwell: 0, slow: 0, mouse_keys: false, live_captions: false, hold: 0, cursor_size: 0.0, shortcut: Vec::new(), speech_rate: 180, speech_pitch: 50, speech_voice: String::new() };

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
            dim_flashing_lights: flag("a11y_dim_flashing_lights", d.dim_flashing_lights),
            guided_access: flag("a11y_guided_access", d.guided_access),
            screen_reader: flag("a11y_screen_reader", d.screen_reader),
            sticky_keys: flag("a11y_sticky_keys", d.sticky_keys),
            dwell: index("a11y_dwell", dwell::DELAYS_MS.len()).unwrap_or(d.dwell),
            slow: index("a11y_slow_keys", super::slow_keys::DELAYS_MS.len()).unwrap_or(d.slow),
            mouse_keys: flag("a11y_mouse_keys", d.mouse_keys),
            live_captions: flag("a11y_live_captions", d.live_captions),
            hold: index("a11y_hold", HOLD_MS.len()).unwrap_or(d.hold),
            cursor_size: p.get_parsed::<f32>("a11y_cursor_size").map_or(d.cursor_size, |v| v.clamp(0.0, 1.0)),
            shortcut: shortcut::parse(p.get(shortcut::PREF).unwrap_or("")),
            speech_rate: p.get_parsed::<u32>("a11y_speech_rate").map_or(d.speech_rate, |v| v.clamp(speech::MIN_RATE, speech::MAX_RATE)),
            speech_pitch: p.get_parsed::<u32>("a11y_speech_pitch").map_or(d.speech_pitch, |v| v.min(speech::MAX_PITCH)),
            speech_voice: p.get("a11y_speech_voice").filter(|v| v.is_empty() || bifrost_core::speech_engine::voice_name_ok(v)).unwrap_or("").to_string(),
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
            ("a11y_dim_flashing_lights", flag(self.dim_flashing_lights)),
            ("a11y_guided_access", flag(self.guided_access)),
            ("a11y_screen_reader", flag(self.screen_reader)),
            ("a11y_sticky_keys", flag(self.sticky_keys)),
            ("a11y_dwell", &self.dwell.to_string()),
            ("a11y_slow_keys", &self.slow.to_string()),
            ("a11y_mouse_keys", flag(self.mouse_keys)),
            ("a11y_live_captions", flag(self.live_captions)),
            ("a11y_hold", &self.hold.to_string()),
            ("a11y_cursor_size", &format!("{:.3}", self.cursor_size)),
            (shortcut::PREF, &shortcut::encode(&self.shortcut)),
            ("a11y_speech_rate", &self.speech_rate.to_string()),
            ("a11y_speech_pitch", &self.speech_pitch.to_string()),
            ("a11y_speech_voice", &self.speech_voice),
        ]);
        if let Err(e) = result {
            eprintln!("springboard: settings.ini: {e}");
        }
    }

    pub fn zoom_factor(&self) -> f32 {
        ZOOMS[self.zoom.min(ZOOMS.len() - 1)].1
    }

    /// The Slow Keys acceptance delay in milliseconds, 0 when Slow Keys is off.
    pub fn slow_ms(&self) -> u64 {
        super::slow_keys::delay_for_index(self.slow)
    }

    /// The dwell delay in milliseconds, 0 when Dwell Control is off.
    pub fn dwell_ms(&self) -> u64 {
        dwell::delay_for_index(self.dwell)
    }

    pub fn hold_ms(&self) -> u32 {
        HOLD_MS[self.hold.min(HOLD_MS.len() - 1)].1
    }

    /// The voice the Screen Reader speaks with: rate, pitch and voice from Settings.
    pub fn speech_config(&self) -> speech::Config {
        speech::Config { rate: self.speech_rate, pitch: self.speech_pitch, voice: self.speech_voice.clone() }
    }

    /// The pointer's scale: 1x at Normal, 2x at Large (the Cursor Size slider).
    pub fn cursor_scale(&self) -> f32 {
        1.0 + self.cursor_size.clamp(0.0, 1.0)
    }

    /// Makes these the values the hooks report.
    pub fn publish(&self) {
        STICKY_KEYS.store(self.sticky_keys, std::sync::atomic::Ordering::Relaxed);
        DWELL_MS.store(self.dwell_ms(), std::sync::atomic::Ordering::Relaxed);
        SLOW_MS.store(self.slow_ms(), std::sync::atomic::Ordering::Relaxed);
        MOUSE_KEYS.store(self.mouse_keys, std::sync::atomic::Ordering::Relaxed);
        bifrost_render::appearance::publish(bifrost_render::appearance::Appearance {
            text_scale: self.text_scale, zoom: self.zoom_factor(), hold_ms: self.hold_ms(),
            bold: self.bold_text, reduce_motion: self.reduce_motion,
            increase_contrast: self.increase_contrast, reduce_transparency: self.reduce_transparency,
            button_shapes: self.button_shapes,
        });
        // The window server polls this value (docs/window-server.md); the directory exists only on the device.
        let _ = bifrost_render::appearance::display_options().write_to(bifrost_render::appearance::DISPLAY_FILE);
    }
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Sticky Keys as the last `publish` set it. The key loop keeps `HwKeys` in step with this.
static STICKY_KEYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether Settings > Accessibility > Sticky Keys is on.
pub fn sticky_keys() -> bool {
    STICKY_KEYS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Dwell Control's delay in milliseconds as the last `publish` set it (0 is off). The input loop reads this.
static DWELL_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The Dwell Control delay in milliseconds, or 0 when Dwell Control is off.
pub fn dwell_delay_ms() -> u64 {
    DWELL_MS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Mouse Keys as the last `publish` set it. The input loop keeps `mouse_keys::MouseKeys` in step with this.
static MOUSE_KEYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether Settings > Accessibility > Keyboard > Mouse Keys is on.
pub fn mouse_keys() -> bool {
    MOUSE_KEYS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Slow Keys' acceptance delay in milliseconds as the last `publish` set it (0 is off). The input loop reads this.
static SLOW_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The Slow Keys acceptance delay in milliseconds, or 0 when Slow Keys is off.
pub fn slow_delay_ms() -> u64 {
    SLOW_MS.load(std::sync::atomic::Ordering::Relaxed)
}

pub use bifrost_render::appearance::{text_scale, bold_text, display_zoom, reduce_motion, increase_contrast, reduce_transparency, button_shapes, touch_hold_ms, contrast, contrast_on, button_shape_radius, anim_ms, long_press_ms};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let mut p = Prefs::memory();
        let a = AccessibilitySettings { zoom: 1, reduce_motion: true, increase_contrast: true, reduce_transparency: true, button_shapes: true, dim_flashing_lights: true, screen_reader: true, hold: 3, ..Default::default() };
        a.save(&mut p);
        assert_eq!(AccessibilitySettings::load(&p), a);
        assert_eq!(a.hold_ms(), 1500);
        assert!((a.zoom_factor() - 1.15).abs() < 1e-6);
    }

    #[test]
    fn sticky_keys_round_trips_and_is_off_by_default() {
        let mut p = Prefs::memory();
        assert!(!AccessibilitySettings::load(&p).sticky_keys);
        let a = AccessibilitySettings { sticky_keys: true, ..Default::default() };
        a.save(&mut p);
        assert!(AccessibilitySettings::load(&p).sticky_keys);
        // the global the key loop reads: set directly, because `publish` also writes the window server's display file
        STICKY_KEYS.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(sticky_keys());
        STICKY_KEYS.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn live_captions_round_trips_and_is_off_by_default() {
        let mut p = Prefs::memory();
        assert!(!AccessibilitySettings::load(&p).live_captions);
        let a = AccessibilitySettings { live_captions: true, ..Default::default() };
        a.save(&mut p);
        assert!(AccessibilitySettings::load(&p).live_captions);
        assert_eq!(p.get("a11y_live_captions"), Some("1"));
    }

    #[test]
    fn mouse_keys_round_trips_and_is_off_by_default() {
        let mut p = Prefs::memory();
        assert!(!AccessibilitySettings::load(&p).mouse_keys);
        let a = AccessibilitySettings { mouse_keys: true, ..Default::default() };
        a.save(&mut p);
        assert!(AccessibilitySettings::load(&p).mouse_keys);
    }

    #[test]
    fn slow_keys_round_trips_and_maps_to_a_delay() {
        let mut p = Prefs::memory();
        assert_eq!(AccessibilitySettings::load(&p).slow_ms(), 0);
        let a = AccessibilitySettings { slow: 2, ..Default::default() };
        a.save(&mut p);
        let b = AccessibilitySettings::load(&p);
        assert_eq!(b.slow, 2);
        assert_eq!(b.slow_ms(), 500);
        p.set("a11y_slow_keys", "99").unwrap();
        assert_eq!(AccessibilitySettings::load(&p).slow, 0);
    }

    #[test]
    fn dwell_round_trips_maps_to_a_delay_and_falls_back() {
        let mut p = Prefs::memory();
        assert_eq!(AccessibilitySettings::load(&p).dwell_ms(), 0);
        let a = AccessibilitySettings { dwell: 2, ..Default::default() };
        a.save(&mut p);
        let b = AccessibilitySettings::load(&p);
        assert_eq!(b.dwell, 2);
        assert_eq!(b.dwell_ms(), 1000);
        // an index past the last choice is ignored, so the default (Off) holds
        p.set("a11y_dwell", "99").unwrap();
        assert_eq!(AccessibilitySettings::load(&p).dwell, 0);
    }

    #[test]
    fn speech_rate_pitch_and_voice_round_trip_and_fall_back() {
        let mut p = Prefs::memory();
        let a = AccessibilitySettings { speech_rate: 240, speech_pitch: 70, speech_voice: "is".into(), ..Default::default() };
        a.save(&mut p);
        let b = AccessibilitySettings::load(&p);
        assert_eq!((b.speech_rate, b.speech_pitch, b.speech_voice.as_str()), (240, 70, "is"));
        assert_eq!(b.speech_config(), speech::Config { rate: 240, pitch: 70, voice: "is".into() });
        // out-of-range rate and pitch are clamped, a voice that is not a plain word is dropped
        p.set("a11y_speech_rate", "9000").unwrap();
        p.set("a11y_speech_pitch", "500").unwrap();
        p.set("a11y_speech_voice", "-bad voice").unwrap();
        let c = AccessibilitySettings::load(&p);
        assert_eq!((c.speech_rate, c.speech_pitch, c.speech_voice.as_str()), (speech::MAX_RATE, speech::MAX_PITCH, ""));
        // a missing key keeps the default: 180 words a minute, pitch 50, the engine's own voice
        let d = AccessibilitySettings::load(&Prefs::memory());
        assert_eq!(d.speech_config(), speech::Config::default());
    }

    #[test]
    fn every_speech_choice_is_inside_the_engine_range() {
        for (_, r) in SPEECH_RATES {
            assert!((speech::MIN_RATE..=speech::MAX_RATE).contains(&r));
        }
        for (_, p) in SPEECH_PITCHES {
            assert!(p <= speech::MAX_PITCH);
        }
        for (_, v) in SPEECH_VOICES {
            assert!(v.is_empty() || bifrost_core::speech_engine::voice_name_ok(v));
        }
    }

    #[test]
    fn cursor_size_and_shortcut_choice_round_trip_and_clamp() {
        let mut p = Prefs::memory();
        let a = AccessibilitySettings { cursor_size: 0.5, shortcut: vec![shortcut::Feature::ReduceMotion, shortcut::Feature::ScreenReader], ..Default::default() };
        a.save(&mut p);
        let b = AccessibilitySettings::load(&p);
        assert!((b.cursor_size - 0.5).abs() < 1e-3);
        assert_eq!(b.shortcut, vec![shortcut::Feature::ScreenReader, shortcut::Feature::ReduceMotion]);
        assert!((b.cursor_scale() - 1.5).abs() < 1e-3);
        p.set("a11y_cursor_size", "9").unwrap();
        let c = AccessibilitySettings::load(&p);
        assert_eq!(c.cursor_size, 1.0);
    }

    #[test]
    fn display_toggles_round_trip_through_prefs() {
        let mut p = Prefs::memory();
        let mut a = AccessibilitySettings::DEFAULT;
        a.reduce_transparency = true;
        a.increase_contrast = true;
        a.save(&mut p);
        let b = AccessibilitySettings::load(&p);
        assert!(b.reduce_transparency && b.increase_contrast);
        assert!(!AccessibilitySettings::load(&Prefs::memory()).increase_contrast);
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

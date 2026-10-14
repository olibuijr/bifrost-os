//! Paste control (`UIPasteControl`): a button the person taps to place pasteboard contents into the app. The tap is the
//! consent, so the paste reads the pasteboard without an access prompt, and the control is enabled only while the
//! pasteboard holds something it accepts.
//!
//! This module is the control's model: its look (display mode, label), the accepted types and the enabled state. The
//! read itself is in `bifrost-sdk` (`paste_control::paste`), which takes a [`Tap`] as proof that the person pressed it.

use crate::text_drag::TEXT_TYPE;
use crate::uttype;

pub const DEFAULT_LABEL: &str = "Paste";

/// What the button shows (`UIPasteControl.Configuration` display choices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    IconAndLabel,
    IconOnly,
    LabelOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    pub display: Display,
    pub label: String,
    /// The UTTypes the control takes. A pasteboard item conforming to any of them enables the control.
    pub accepted_types: Vec<String>,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration { display: Display::IconAndLabel, label: DEFAULT_LABEL.to_string(), accepted_types: vec![TEXT_TYPE.to_string()] }
    }
}

impl Configuration {
    /// A configuration that accepts the given types and otherwise has the default look.
    pub fn accepting(types: &[&str]) -> Self {
        Configuration { accepted_types: types.iter().map(|t| t.to_string()).collect(), ..Self::default() }
    }

    /// Whether the button shows an icon.
    pub fn shows_icon(&self) -> bool {
        self.display != Display::LabelOnly
    }

    /// The text the button shows, or None when the display mode hides it.
    pub fn visible_label(&self) -> Option<&str> {
        match self.display {
            Display::IconOnly => None,
            _ => Some(self.label.as_str()),
        }
    }

    /// `isEnabled`: some type on the pasteboard conforms to an accepted type. `available` lists the pasteboard's types.
    pub fn is_enabled(&self, available: &[String]) -> bool {
        self.accepted_types.iter().any(|want| available.iter().any(|have| uttype::conforms(have, want)))
    }
}

/// Proof that the person tapped the control. The toolkit makes one when a press on the control is delivered; a paste
/// needs one, and no other call reads the pasteboard on the control's behalf.
#[derive(Debug, PartialEq, Eq)]
pub struct Tap(());

impl Tap {
    /// Called by the event path when the control receives a tap.
    pub fn delivered() -> Self {
        Tap(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn default_accepts_plain_text_and_labels_paste() {
        let c = Configuration::default();
        assert_eq!(c.label, "Paste");
        assert_eq!(c.accepted_types, vec![TEXT_TYPE.to_string()]);
        assert!(c.shows_icon());
        assert_eq!(c.visible_label(), Some("Paste"));
    }

    #[test]
    fn display_modes_hide_the_right_part() {
        let mut c = Configuration::default();
        c.display = Display::IconOnly;
        assert_eq!(c.visible_label(), None);
        assert!(c.shows_icon());
        c.display = Display::LabelOnly;
        assert_eq!(c.visible_label(), Some("Paste"));
        assert!(!c.shows_icon());
    }

    #[test]
    fn enabled_only_when_an_accepted_type_is_present() {
        let c = Configuration::default();
        assert!(!c.is_enabled(&[]));
        assert!(!c.is_enabled(&types(&["public.png"])));
        assert!(c.is_enabled(&types(&["public.png", TEXT_TYPE])));
    }

    #[test]
    fn subtypes_of_an_accepted_type_enable_the_control() {
        let c = Configuration::accepting(&["public.text"]);
        assert!(c.is_enabled(&types(&[TEXT_TYPE])), "plain text conforms to public.text");
    }

    #[test]
    fn accepting_replaces_the_types_and_keeps_the_look() {
        let c = Configuration::accepting(&["public.png"]);
        assert_eq!(c.accepted_types, vec!["public.png".to_string()]);
        assert_eq!(c.label, DEFAULT_LABEL);
    }
}

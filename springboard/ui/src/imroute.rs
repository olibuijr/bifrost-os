//! Where a key or mouse event goes while a text input session is active. Three delivery modes decide how the provider
//! sees key events: it handles every key down itself, it gets key bindings as commands and plain text as text, or it
//! gets every key down as text with its modifiers. The mouse goes to the provider only over the marked text, and only
//! when it asks for mouse events. The model returns the destination; the shell does the delivery.

use crate::keys::{KeyCombo, Shortcuts};
use std::ops::Range;

/// How key down events reach the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventMode {
    /// The provider sees each key down first and answers whether it handled it. When it does not, the caller sends the
    /// event to the field.
    Handle,
    /// A key down that maps to a shortcut becomes a command. Other keys that produce text become text. Everything else
    /// goes to the field.
    KeyBinding,
    /// The provider gets every key down as text with its key and modifiers.
    TextData,
}

/// The destination of one key down event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The provider gets the event (`handle`, or text with key and modifiers).
    Provider,
    /// The provider gets the shortcut's command id (`didCommand`).
    Command(u32),
    /// The provider gets plain text (`inputText`).
    Text(char),
    /// The field gets the event; the provider does not see it.
    Field,
}

/// Decides where a key down goes. `text` is the character the key produces, if any. Command shortcuts win over text,
/// and a shortcut with Super, Control or Alt is never text.
pub fn route_key(mode: EventMode, shortcuts: &Shortcuts, combo: &KeyCombo, text: Option<char>) -> Route {
    match mode {
        EventMode::Handle | EventMode::TextData => Route::Provider,
        EventMode::KeyBinding => {
            if let Some(command) = shortcuts.command_for(combo) {
                return Route::Command(command);
            }
            match text {
                Some(c) if !combo.cmd && !combo.ctrl && !combo.alt => Route::Text(c),
                _ => Route::Field,
            }
        }
    }
}

/// The destination of one mouse down event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseRoute {
    Provider,
    Field,
}

/// A mouse down on character `index`. The provider gets it only when it handles mouse events, a composition is marked,
/// and the index lies inside the marked range (start included, end excluded).
pub fn route_mouse(handles_mouse: bool, marked: Option<Range<usize>>, index: usize) -> MouseRoute {
    match marked {
        Some(r) if handles_mouse && index >= r.start && index < r.end => MouseRoute::Provider,
        _ => MouseRoute::Field,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Key;

    fn combo(key: Key) -> KeyCombo {
        KeyCombo { cmd: false, ctrl: false, alt: false, shift: false, key }
    }

    #[test]
    fn text_data_and_handle_modes_send_every_key_to_the_provider() {
        let mut shortcuts = Shortcuts::default();
        shortcuts.bind(KeyCombo { cmd: true, ..combo(Key::Char('s')) }, 5).unwrap();
        let save = KeyCombo { cmd: true, ..combo(Key::Char('s')) };
        assert_eq!(route_key(EventMode::TextData, &shortcuts, &save, Some('s')), Route::Provider);
        assert_eq!(route_key(EventMode::Handle, &shortcuts, &combo(Key::Left), None), Route::Provider);
    }

    #[test]
    fn key_binding_mode_turns_a_bound_shortcut_into_a_command() {
        let mut shortcuts = Shortcuts::default();
        let save = KeyCombo { cmd: true, ..combo(Key::Char('s')) };
        shortcuts.bind(save, 5).unwrap();
        // Even when the key also produces a character, the shortcut is a command.
        assert_eq!(route_key(EventMode::KeyBinding, &shortcuts, &save, Some('s')), Route::Command(5));
    }

    #[test]
    fn key_binding_mode_sends_plain_text_as_text() {
        let shortcuts = Shortcuts::default();
        assert_eq!(route_key(EventMode::KeyBinding, &shortcuts, &combo(Key::Char('a')), Some('a')), Route::Text('a'));
        assert_eq!(route_key(EventMode::KeyBinding, &shortcuts, &combo(Key::Space), Some(' ')), Route::Text(' '));
    }

    #[test]
    fn key_binding_mode_sends_modified_or_non_text_keys_to_the_field() {
        let shortcuts = Shortcuts::default();
        let ctrl_a = KeyCombo { ctrl: true, ..combo(Key::Char('a')) };
        assert_eq!(route_key(EventMode::KeyBinding, &shortcuts, &ctrl_a, Some('a')), Route::Field);
        assert_eq!(route_key(EventMode::KeyBinding, &shortcuts, &combo(Key::Return), None), Route::Field);
        assert_eq!(route_key(EventMode::KeyBinding, &shortcuts, &combo(Key::Left), None), Route::Field);
    }

    #[test]
    fn mouse_reaches_the_provider_only_inside_the_marked_range() {
        // Marked range 2..5: index 2 is inside, index 5 is just past the end.
        assert_eq!(route_mouse(true, Some(2..5), 2), MouseRoute::Provider);
        assert_eq!(route_mouse(true, Some(2..5), 4), MouseRoute::Provider);
        assert_eq!(route_mouse(true, Some(2..5), 5), MouseRoute::Field);
        assert_eq!(route_mouse(true, Some(2..5), 1), MouseRoute::Field);
    }

    #[test]
    fn mouse_needs_mouse_handling_and_a_composition() {
        assert_eq!(route_mouse(false, Some(2..5), 3), MouseRoute::Field);
        assert_eq!(route_mouse(true, None, 3), MouseRoute::Field);
    }
}

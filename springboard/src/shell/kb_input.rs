//! Keyboard choices from Settings > Keyboard, as the hardware-key path applies them: the key repeat timing, modifier
//! key remapping, the function keys, Tab navigation, the shell shortcuts that can be switched off, and the text
//! replacements a hardware keyboard types.
//!
//! Settings publishes a fresh [`KeyPrefs`] with [`set`] and the input loop reads it on every key event, so a change
//! applies at once (the same pattern as `bare::pointer_prefs`). The rules here are pure and host-tested.

use super::hwkeys::Shortcut;
use super::keyboard::Out;
use std::sync::Mutex;

/// evdev codes of the keys the modifier choices remap.
pub const KEY_CAPS_LOCK: u16 = 58;
pub const KEY_LEFT_CTRL: u16 = 29;
pub const KEY_LEFT_ALT: u16 = 56;
pub const KEY_LEFT_META: u16 = 125;
const KEY_ESC: u16 = 1;

/// Settings > Keyboard > Modifier Keys choices, in menu order. Entry 0 is the key's own action; the others name the
/// evdev code the key sends instead (`Some(0)` sends nothing).
pub const REMAP_TARGETS: [(&str, Option<u16>); 7] = [
    ("Default", None),
    ("Caps Lock", Some(KEY_CAPS_LOCK)),
    ("Control", Some(KEY_LEFT_CTRL)),
    ("Option", Some(KEY_LEFT_ALT)),
    ("Command", Some(KEY_LEFT_META)),
    ("Escape", Some(KEY_ESC)),
    ("No Action", Some(0)),
];

/// The modifier keys that can be remapped, in the order Settings lists them: their labels.
pub const REMAPPABLE: [&str; 4] = ["Caps Lock", "Control", "Option", "Command"];

/// Where a remapped key's choice sits: one index into [`REMAP_TARGETS`] per remappable key, in [`REMAPPABLE`] order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Remap {
    pub caps_lock: usize,
    pub control: usize,
    pub option: usize,
    pub command: usize,
}

impl Remap {
    /// The index of the choice for the `slot`th remappable key (0 Caps Lock, 1 Control, 2 Option, 3 Command).
    pub fn slot(&self, slot: usize) -> usize {
        match slot {
            0 => self.caps_lock,
            1 => self.control,
            2 => self.option,
            _ => self.command,
        }
    }

    /// The key code the input path sees for a physical `code`: remapped keys send their choice, others pass through.
    pub fn apply(&self, code: u16) -> u16 {
        let slot = match code {
            KEY_CAPS_LOCK => 0,
            KEY_LEFT_CTRL => 1,
            KEY_LEFT_ALT => 2,
            KEY_LEFT_META => 3,
            _ => return code,
        };
        REMAP_TARGETS.get(self.slot(slot)).and_then(|t| t.1).unwrap_or(code)
    }
}

/// The evdev codes of F1 to F12, in order.
const FKEYS: [u16; 12] = [59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88];
const FKEY_NAMES: [&str; 12] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];

/// The name of the function key with evdev code `code` ("F1" to "F12"), if it is one.
pub fn function_key(code: u16) -> Option<&'static str> {
    FKEYS.iter().position(|&c| c == code).map(|i| FKEY_NAMES[i])
}

/// The evdev code of the function key called `name` ("F1" to "F12").
pub fn function_key_code(name: &str) -> Option<u16> {
    FKEY_NAMES.iter().position(|&n| n == name).map(|i| FKEYS[i])
}

/// The shell shortcuts Settings > Keyboard Shortcuts can switch off: (action, keys). Bit `i` of
/// `KeyPrefs::shortcuts_off` turns off entry `i`.
pub const BINDINGS: [(&str, &str); 10] = [
    ("Search", "Cmd+Space"),
    ("Switch Application", "Cmd+Tab"),
    ("Close Window", "Cmd+W"),
    ("Minimize Window", "Cmd+M"),
    ("Full Screen", "Ctrl+Cmd+F"),
    ("Tile Window", "Ctrl+Cmd+Arrows"),
    ("Cycle Windows", "Cmd+`"),
    ("Next Input Source", "Ctrl+Space"),
    ("Settings", "Cmd+,"),
    ("Home", "Cmd"),
];

/// The index into [`BINDINGS`] of a shortcut the user can switch off; None for the keys that are always on.
pub fn binding_of(sc: Shortcut) -> Option<usize> {
    Some(match sc {
        Shortcut::QuickSearch => 0,
        Shortcut::AppSwitch => 1,
        Shortcut::Close => 2,
        Shortcut::Minimize => 3,
        Shortcut::FullScreen => 4,
        Shortcut::Tile(_) => 5,
        Shortcut::CycleWindow => 6,
        Shortcut::InputSource => 7,
        Shortcut::Settings => 8,
        Shortcut::Home => 9,
        _ => return None,
    })
}

/// Whether the shortcut `sc` still runs, given the switched-off set `off`.
pub fn enabled(sc: Shortcut, off: u32) -> bool {
    binding_of(sc).map_or(true, |i| off & (1 << i) == 0)
}

/// The key repeat limits Settings allows: the hold time in ms and the repeats per second.
pub const REPEAT_DELAY_RANGE: (u32, u32) = (100, 2000);
pub const REPEAT_RATE_RANGE: (u32, u32) = (1, 100);

/// Everything the hardware-key path reads from Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPrefs {
    /// hold time before a held key repeats
    pub repeat_delay_ms: u32,
    /// repeats per second once repeating
    pub repeat_rate: u32,
    pub remap: Remap,
    /// F1 to F12 type their standard keys instead of acting as the shell's keys
    pub function_standard: bool,
    /// Tab moves focus between controls outside a terminal
    pub tab_navigation: bool,
    /// bit `i` set: the shell shortcut `BINDINGS[i]` is off
    pub shortcuts_off: u32,
}

impl KeyPrefs {
    pub const DEFAULT: KeyPrefs = KeyPrefs {
        repeat_delay_ms: 400,
        repeat_rate: 30,
        remap: Remap { caps_lock: 0, control: 0, option: 0, command: 0 },
        function_standard: false,
        tab_navigation: true,
        shortcuts_off: 0,
    };
}

static PREFS: Mutex<KeyPrefs> = Mutex::new(KeyPrefs::DEFAULT);

/// Publishes the choices the input path uses from now on.
pub fn set(p: KeyPrefs) {
    if let Ok(mut g) = PREFS.lock() {
        *g = p;
    }
}

/// The choices now in force (the defaults if the lock is poisoned).
pub fn get() -> KeyPrefs {
    PREFS.lock().map(|g| *g).unwrap_or(KeyPrefs::DEFAULT)
}

/// Builds the word a hardware keyboard has typed since the last break, to find a text replacement when a word ends.
/// A word is letters, digits and apostrophes. Any click, a return, or a key that is not a character ends it.
#[derive(Debug, Default)]
pub struct WordBuffer {
    word: String,
}

/// A text replacement due now: erase `erase` characters before the key just typed, then type `phrase`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    pub erase: usize,
    pub phrase: String,
}

impl WordBuffer {
    /// Forgets the word (a pointer press, a new field).
    pub fn clear(&mut self) {
        self.word.clear();
    }

    /// Feeds one output the hardware keyboard types, before it is delivered. A word ended by a space or a
    /// punctuation character is looked up with `expand`; a match returns the replacement to apply first.
    pub fn feed(&mut self, out: &Out, expand: impl Fn(&str) -> Option<String>) -> Option<Expansion> {
        match out {
            Out::Text(t) => {
                let mut chars = t.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else {
                    self.clear();
                    return None;
                };
                if c.is_alphanumeric() || c == '\'' {
                    self.word.push(c);
                    return None;
                }
                let word = std::mem::take(&mut self.word);
                if word.is_empty() {
                    return None;
                }
                let phrase = expand(&word)?;
                Some(Expansion { erase: word.chars().count(), phrase })
            }
            Out::Backspace => {
                self.word.pop();
                None
            }
            _ => {
                self.clear();
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_sends_the_chosen_code_and_passes_others_through() {
        let none = Remap::default();
        assert_eq!(none.apply(KEY_CAPS_LOCK), KEY_CAPS_LOCK, "Default keeps the key");
        let r = Remap { caps_lock: 2, control: 1, option: 5, command: 6 };
        assert_eq!(r.apply(KEY_CAPS_LOCK), KEY_LEFT_CTRL, "Caps Lock acts as Control");
        assert_eq!(r.apply(KEY_LEFT_CTRL), KEY_CAPS_LOCK, "Control acts as Caps Lock");
        assert_eq!(r.apply(KEY_LEFT_ALT), KEY_ESC, "Option acts as Escape");
        assert_eq!(r.apply(KEY_LEFT_META), 0, "Command does nothing");
        assert_eq!(r.apply(30), 30, "other keys pass through");
    }

    #[test]
    fn remap_out_of_range_keeps_the_key() {
        let r = Remap { caps_lock: 99, ..Remap::default() };
        assert_eq!(r.apply(KEY_CAPS_LOCK), KEY_CAPS_LOCK);
    }

    #[test]
    fn function_keys_map_both_ways() {
        assert_eq!(function_key(59), Some("F1"));
        assert_eq!(function_key(88), Some("F12"));
        assert_eq!(function_key(30), None);
        assert_eq!(function_key_code("F11"), Some(87));
        assert_eq!(function_key_code("F13"), None);
    }

    #[test]
    fn switched_off_shortcuts_do_not_run() {
        assert!(enabled(Shortcut::QuickSearch, 0));
        assert!(!enabled(Shortcut::QuickSearch, 1 << binding_of(Shortcut::QuickSearch).unwrap()));
        assert!(enabled(Shortcut::Back, u32::MAX), "Escape is not a switchable shortcut");
        let left = Shortcut::Tile(super::super::desk::Tile::Left);
        assert!(enabled(left, 1 << 4), "full screen's bit does not switch tiling off");
        assert!(!enabled(left, 1 << 5), "tiling has its own bit");
    }

    #[test]
    fn bindings_cover_every_switchable_shortcut_once() {
        let all = [
            Shortcut::QuickSearch,
            Shortcut::AppSwitch,
            Shortcut::Close,
            Shortcut::Minimize,
            Shortcut::FullScreen,
            Shortcut::CycleWindow,
            Shortcut::InputSource,
            Shortcut::Settings,
            Shortcut::Home,
        ];
        let mut seen: Vec<usize> = all.iter().filter_map(|s| binding_of(*s)).collect();
        seen.push(binding_of(Shortcut::Tile(super::super::desk::Tile::Center)).unwrap());
        seen.sort_unstable();
        assert_eq!(seen, (0..BINDINGS.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_typed_word_expands_when_a_space_ends_it() {
        let table = |w: &str| (w == "brb").then(|| "be right back".to_string());
        let mut b = WordBuffer::default();
        for c in "brb".chars() {
            assert_eq!(b.feed(&Out::Text(c.to_string()), table), None);
        }
        assert_eq!(
            b.feed(&Out::Text(" ".into()), table),
            Some(Expansion { erase: 3, phrase: "be right back".into() })
        );
        assert_eq!(b.feed(&Out::Text(" ".into()), table), None, "the word is spent");
    }

    #[test]
    fn backspace_edits_the_word_and_a_break_clears_it() {
        let table = |w: &str| (w == "ab").then(|| "x".to_string());
        let mut b = WordBuffer::default();
        b.feed(&Out::Text("a".into()), table);
        b.feed(&Out::Text("c".into()), table);
        b.feed(&Out::Backspace, table);
        b.feed(&Out::Text("b".into()), table);
        assert_eq!(b.feed(&Out::Text(".".into()), table), Some(Expansion { erase: 2, phrase: "x".into() }));
        b.feed(&Out::Text("a".into()), table);
        b.feed(&Out::Enter, table);
        assert_eq!(b.feed(&Out::Text("b".into()), table), None, "Return ends the word");
    }

    #[test]
    fn multi_character_text_is_not_a_word_key() {
        let mut b = WordBuffer::default();
        b.feed(&Out::Text("hi".into()), |_| None);
        assert_eq!(b.feed(&Out::Text(" ".into()), |w| (w == "hi").then(|| "x".into())), None, "the buffer was cleared");
    }

    #[test]
    fn repeat_defaults_match_the_shell() {
        assert_eq!(KeyPrefs::DEFAULT.repeat_delay_ms, 400);
        assert_eq!(KeyPrefs::DEFAULT.repeat_rate, 30);
        assert!(KeyPrefs::DEFAULT.tab_navigation);
    }

    #[test]
    fn set_and_get_round_trip() {
        let p = KeyPrefs { repeat_rate: 55, ..KeyPrefs::DEFAULT };
        set(p);
        assert_eq!(get(), p);
        set(KeyPrefs::DEFAULT);
    }
}

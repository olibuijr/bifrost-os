//! evdev key code to the character it types on the session's keyboard layout: app menu chords, type-to-select in
//! menus and the Dock stack, and the key a synthesized character event presses. The tables are the installer's
//! (`installer::keymaps`, generated from xkeyboard-config), the same ones the shell types into its own fields with.

use crate::installer::keymaps::{key_levels, layout_index};
use std::sync::atomic::{AtomicUsize, Ordering};

const KEY_SPACE: u16 = 57;

/// The active layout (an index into the installer's `LAYOUTS`); the shell's input loop sets it together with the
/// hardware keyboard's layout (`set_layout`). U.S. (index 0) until then.
static LAYOUT: AtomicUsize = AtomicUsize::new(0);

/// Follows the layout the hardware keyboard types with (`HwKeys::set_layout`).
pub fn set_layout(layout: usize) {
    LAYOUT.store(layout, Ordering::Relaxed);
}

fn us() -> usize {
    layout_index("us").unwrap_or(0)
}

/// The character the key `code` types on the active layout, with or without Shift. None for keys that type nothing
/// and for dead keys (a mark waiting for a letter).
pub fn char_for(code: u16, shift: bool) -> Option<char> {
    char_in(LAYOUT.load(Ordering::Relaxed), code, shift)
}

/// The character the key `code` types on the layout `layout` (an index into the installer's `LAYOUTS`).
pub fn char_in(layout: usize, code: u16, shift: bool) -> Option<char> {
    if code == KEY_SPACE {
        return Some(' ');
    }
    let c = key_levels(layout, code)?[usize::from(shift)];
    (c != '\0' && !('\u{300}'..='\u{36f}').contains(&c)).then_some(c)
}

/// The character a key equivalent (Cmd/Ctrl chord) reads from the key `code`: the active layout's, except that a
/// letter of a non-Latin layout (Cyrillic) reads as the U.S. letter on that key, so Cmd+C still copies there.
pub fn chord_char_for(code: u16) -> Option<char> {
    chord_char_in(LAYOUT.load(Ordering::Relaxed), code)
}

fn chord_char_in(layout: usize, code: u16) -> Option<char> {
    let c = char_in(layout, code, false)?;
    if c.is_ascii() || !c.is_alphabetic() || is_latin(c) {
        return Some(c);
    }
    match char_in(us(), code, false) {
        Some(u) if u.is_ascii_alphabetic() => Some(u),
        _ => Some(c),
    }
}

/// Latin letters beyond ASCII (ö, ð, þ, æ, é ...): Latin-1 Supplement through Latin Extended-B.
fn is_latin(c: char) -> bool {
    ('\u{c0}'..='\u{24f}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(id: &str) -> usize {
        layout_index(id).unwrap()
    }

    #[test]
    fn letters_digits_and_punctuation_on_us() {
        let us = layout("us");
        assert_eq!(char_in(us, 30, false), Some('a'));
        assert_eq!(char_in(us, 30, true), Some('A'));
        assert_eq!(char_in(us, 50, false), Some('m'));
        assert_eq!(char_in(us, 2, false), Some('1'));
        assert_eq!(char_in(us, 2, true), Some('!'));
        assert_eq!(char_in(us, 11, false), Some('0'));
        assert_eq!(char_in(us, 12, false), Some('-'));
        assert_eq!(char_in(us, 13, false), Some('='));
        assert_eq!(char_in(us, 52, false), Some('.'));
        assert_eq!(char_in(us, 53, false), Some('/'));
        assert_eq!(char_in(us, 57, false), Some(' '));
        assert_eq!(char_in(us, 1, false), None);
        assert_eq!(char_in(us, 28, false), None, "Enter types nothing");
    }

    #[test]
    fn the_icelandic_layout_types_its_own_characters() {
        let is = layout("is");
        // `-` is the key right of `ö` (the U.S. `=` key), not the U.S. minus key
        assert_eq!(char_in(is, 13, false), Some('-'));
        assert_eq!(char_in(is, 12, false), Some('ö'));
        assert_eq!(char_in(is, 53, false), Some('þ'));
        assert_eq!(char_in(is, 26, false), Some('ð'));
        assert_eq!(char_in(is, 39, false), Some('æ'));
        assert_eq!(char_in(is, 43, false), Some('+'));
        assert_eq!(char_in(is, 52, false), Some('.'));
        assert_eq!(char_in(is, 52, true), Some(':'));
        assert_eq!(char_in(is, 8, true), Some('/'));
        assert_eq!(char_in(is, 30, false), Some('a'));
        assert_eq!(char_in(is, 57, false), Some(' '));
        // the acute key is a dead key: it types nothing by itself
        assert_eq!(char_in(is, 40, false), None);
    }

    #[test]
    fn a_name_typed_on_icelandic_reads_back_through_the_icelandic_table() {
        // e2e8-i on the Icelandic board: e 2 e 8 - i
        let keys = [18u16, 3, 18, 9, 13, 23];
        let typed: String = keys.iter().filter_map(|&k| char_in(layout("is"), k, false)).collect();
        assert_eq!(typed, "e2e8-i");
        let on_us: String = keys.iter().filter_map(|&k| char_in(layout("us"), k, false)).collect();
        assert_eq!(on_us, "e2e8=i", "the U.S. table misreads the Icelandic minus");
    }

    #[test]
    fn key_equivalents_follow_the_layout_and_keep_latin_letters() {
        let is = layout("is");
        assert_eq!(chord_char_in(is, 46), Some('c'));
        assert_eq!(chord_char_in(is, 13), Some('-'));
        assert_eq!(chord_char_in(is, 12), Some('ö'), "an Icelandic letter is not read as the U.S. minus");
        assert_eq!(chord_char_in(layout("ru"), 46), Some('c'), "Cmd+C copies on a Cyrillic layout");
        assert_eq!(chord_char_in(layout("us"), 46), Some('c'));
    }
}

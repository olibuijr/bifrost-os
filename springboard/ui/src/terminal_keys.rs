//! Control-key bindings for terminals (`Terminal` line editing and job control):
//! which Ctrl and Alt chords send which bytes to the pty, and what a
//! readline-like shell does with them.
//!
//! A terminal emulator does not implement line editing itself: it sends bytes
//! and the shell (readline, zle, fish) acts on them. This module is the one
//! place that maps chords to bytes, so the hardware keyboard, the on-screen
//! Ctrl key and `term send_keys` all send the same thing. It matches the
//! reference platform's Terminal: Cmd+C/V stay copy/paste while Ctrl+C sends
//! SIGINT (`\x03`), Ctrl+A/E move to the start/end of the line, and so on.
//!
//! [`control_byte`] maps Ctrl+key to its byte, [`alt_bytes`] maps Alt+key to
//! its ESC-prefixed bytes, and [`CONTROL_BINDINGS`] documents every binding
//! with the readline action it drives.

use super::keys::{Key, KeyCombo};

/// The byte Ctrl+`c` sends: letters (case-insensitive) become 1-26,
/// `@` and Space become NUL, `[` `\\` `]` become ESC FS GS, `^` becomes RS
/// and `_` becomes US (undo in readline). `None` for keys Ctrl does not
/// combine with (digits other than the shifted `@`, most punctuation,
/// function and navigation keys).
pub fn control_byte(c: char) -> Option<u8> {
    Some(match c {
        'a' | 'A' => 1,
        'b' | 'B' => 2,
        'c' | 'C' => 3,
        'd' | 'D' => 4,
        'e' | 'E' => 5,
        'f' | 'F' => 6,
        'g' | 'G' => 7,
        'h' | 'H' => 8,
        'i' | 'I' => 9,
        'j' | 'J' => 10,
        'k' | 'K' => 11,
        'l' | 'L' => 12,
        'm' | 'M' => 13,
        'n' | 'N' => 14,
        'o' | 'O' => 15,
        'p' | 'P' => 16,
        'q' | 'Q' => 17,
        'r' | 'R' => 18,
        's' | 'S' => 19,
        't' | 'T' => 20,
        'u' | 'U' => 21,
        'v' | 'V' => 22,
        'w' | 'W' => 23,
        'x' | 'X' => 24,
        'y' | 'Y' => 25,
        'z' | 'Z' => 26,
        '@' | ' ' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' => 31,
        _ => return None,
    })
}

/// Every Ctrl binding: the key label, the byte sent, and the readline (or
/// tty) action a shell takes. Tab, LF, CR and ESC are the same bytes the
/// Tab, Enter and Escape keys already send; they are listed so help can show
/// the whole set in one place.
pub const CONTROL_BINDINGS: &[(&str, u8, &str)] = &[
    ("Ctrl+Space / Ctrl+@", 0, "set mark (NUL)"),
    ("Ctrl+A", 1, "beginning of line"),
    ("Ctrl+B", 2, "backward one character"),
    ("Ctrl+C", 3, "interrupt (SIGINT)"),
    ("Ctrl+D", 4, "delete character / end of input"),
    ("Ctrl+E", 5, "end of line"),
    ("Ctrl+F", 6, "forward one character"),
    ("Ctrl+G", 7, "abort (bell)"),
    ("Ctrl+H", 8, "delete backward (Backspace)"),
    ("Ctrl+I / Tab", 9, "complete"),
    ("Ctrl+J", 10, "accept line (line feed)"),
    ("Ctrl+K", 11, "kill to end of line"),
    ("Ctrl+L", 12, "clear screen"),
    ("Ctrl+M / Enter", 13, "accept line (carriage return)"),
    ("Ctrl+N", 14, "next history line"),
    ("Ctrl+O", 15, "accept line and fetch next history line"),
    ("Ctrl+P", 16, "previous history line"),
    ("Ctrl+Q", 17, "resume output (XON)"),
    ("Ctrl+R", 18, "reverse search history"),
    ("Ctrl+S", 19, "forward search history / pause output (XOFF)"),
    ("Ctrl+T", 20, "transpose characters"),
    ("Ctrl+U", 21, "kill whole line"),
    ("Ctrl+V", 22, "quoted insert (literal next character)"),
    ("Ctrl+W", 23, "kill word backward"),
    ("Ctrl+X", 24, "extended-command prefix"),
    ("Ctrl+Y", 25, "yank killed text"),
    ("Ctrl+Z", 26, "suspend (SIGTSTP)"),
    ("Ctrl+[ / Escape", 27, "escape / meta prefix"),
    ("Ctrl+\\", 28, "quit (SIGQUIT)"),
    ("Ctrl+]", 29, "character search"),
    ("Ctrl+^", 30, "record separator"),
    ("Ctrl+_", 31, "undo"),
];

/// The bytes Alt (Option as Meta) + `c` sends: ESC followed by the key
/// itself. Case is preserved, so Alt+F sends `ESC f` (forward word) while
/// Alt+Shift+F sends `ESC F`. `None` for control characters, which Alt does
/// not combine with.
pub fn alt_bytes(c: char) -> Option<[u8; 2]> {
    if c.is_control() || !c.is_ascii() {
        return None;
    }
    Some([0x1b, c as u8])
}

/// The byte Alt+Backspace sends after ESC: DEL, which readline reads as
/// backward-kill-word.
pub const ALT_BACKSPACE: u8 = 0x7f;

/// The bytes Alt+Left / Alt+Right send: word backward / forward, the same as
/// Alt+B / Alt+F.
pub fn alt_word_bytes(right: bool) -> [u8; 2] {
    [0x1b, if right { b'f' } else { b'b' }]
}

/// The byte a `KeyCombo` with only Ctrl held sends to the pty. Shift is
/// ignored for letters (Ctrl+Shift+A is still `\x01`) but lets shifted
/// symbols through (Ctrl+Shift+2 is Ctrl+@, NUL). Cmd, Alt, function keys,
/// navigation keys, Tab and Space are never control bytes here: Tab and
/// Space are shell shortcuts (completion stays on Tab, the input source stays
/// on Ctrl+Space) and the rest are not control keys. `None` for all of those.
pub fn ctrl_bytes(k: KeyCombo) -> Option<u8> {
    if !k.ctrl || k.cmd || k.alt {
        return None;
    }
    match k.key {
        Key::Char(c) => control_byte(c).or_else(|| k.shift.then(|| control_byte_shifted(c)).flatten()),
        Key::Space | Key::Tab => None,
        _ => None,
    }
}

/// The shifted symbol of a key, for [`ctrl_bytes`]: Shift+2 types `@`, so
/// Ctrl+Shift+2 is Ctrl+@ (NUL). Letters are handled by [`control_byte`]
/// itself (case-insensitive).
fn control_byte_shifted(c: char) -> Option<u8> {
    Some(match c {
        '2' => 0, // @
        _ => return None,
    })
}

/// The bytes a `KeyCombo` with only Alt (Option as Meta, not AltGr) held
/// sends to the pty: ESC plus the key. Shift is kept (Alt+Shift+F is
/// `ESC F`). Cmd, Ctrl, function keys and arrows other than Left/Right are
/// not Alt combinations here (`alt_arrows` covers word jumps separately).
/// Backspace becomes `ESC DEL` (backward-kill-word). `None` for all others.
pub fn alt_combo_bytes(k: KeyCombo) -> Option<Vec<u8>> {
    if !k.alt || k.ctrl || k.cmd {
        return None;
    }
    match k.key {
        Key::Char(c) => {
            let c = if k.shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
            alt_bytes(c).map(|b| b.to_vec())
        }
        Key::Space => Some(vec![0x1b, b' ']),
        Key::Delete => Some(vec![0x1b, ALT_BACKSPACE]),
        _ => None,
    }
}

/// The bytes Alt+Left / Alt+Right send: word backward / forward. `None`
/// unless the combo is exactly Alt plus an arrow (Shift extends in fields,
/// but a terminal has no selection, so Shift+Alt+arrow is the same jump).
pub fn alt_arrows(k: KeyCombo) -> Option<[u8; 2]> {
    if !k.alt || k.ctrl || k.cmd {
        return None;
    }
    match k.key {
        Key::Left => Some(alt_word_bytes(false)),
        Key::Right => Some(alt_word_bytes(true)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(s: &str) -> KeyCombo {
        KeyCombo::parse(s).unwrap()
    }

    #[test]
    fn control_letters_are_one_to_twenty_six_case_insensitive() {
        for (i, c) in ('a'..='z').enumerate() {
            assert_eq!(control_byte(c), Some((i + 1) as u8));
            assert_eq!(control_byte(c.to_ascii_uppercase()), Some((i + 1) as u8));
        }
    }

    #[test]
    fn control_symbols_map_and_digits_do_not() {
        assert_eq!(control_byte(' '), Some(0));
        assert_eq!(control_byte('@'), Some(0));
        assert_eq!(control_byte('['), Some(27));
        assert_eq!(control_byte('\\'), Some(28));
        assert_eq!(control_byte(']'), Some(29));
        assert_eq!(control_byte('^'), Some(30));
        assert_eq!(control_byte('_'), Some(31));
        for c in ['1', '2', '-', '=', ';', '\'', ',', '.', '/', '`'] {
            assert_eq!(control_byte(c), None, "{c:?}");
        }
    }

    #[test]
    fn bindings_cover_every_control_byte_once() {
        let mut seen = CONTROL_BINDINGS.iter().map(|(_, b, _)| *b).collect::<Vec<_>>();
        seen.sort_unstable();
        assert_eq!(seen, (0..=31).collect::<Vec<_>>());
    }

    #[test]
    fn ctrl_combos_send_bytes_but_leave_shortcuts_and_cmd_alone() {
        assert_eq!(ctrl_bytes(ctrl("ctrl+a")), Some(1));
        assert_eq!(ctrl_bytes(ctrl("ctrl+E")), Some(5));
        assert_eq!(ctrl_bytes(ctrl("ctrl+shift+a")), Some(1), "shift does not change letters");
        assert_eq!(ctrl_bytes(ctrl("ctrl+shift+2")), Some(0), "Ctrl+Shift+2 is Ctrl+@ (NUL)");
        assert_eq!(ctrl_bytes(ctrl("ctrl+space")), None, "the input source keeps Ctrl+Space");
        assert_eq!(ctrl_bytes(ctrl("ctrl+tab")), None, "the app switcher keeps Ctrl+Tab");
        assert_eq!(ctrl_bytes(ctrl("cmd+a")), None);
        assert_eq!(ctrl_bytes(ctrl("alt+a")), None);
        assert_eq!(ctrl_bytes(ctrl("ctrl+f1")), None);
        assert_eq!(ctrl_bytes(ctrl("ctrl+left")), None);
        assert_eq!(ctrl_bytes(ctrl("ctrl+1")), None);
    }

    #[test]
    fn alt_combos_send_esc_plus_key_and_keep_case() {
        assert_eq!(alt_combo_bytes(KeyCombo::parse("alt+f").unwrap()), Some(vec![0x1b, b'f']));
        assert_eq!(alt_combo_bytes(KeyCombo::parse("alt+shift+f").unwrap()), Some(vec![0x1b, b'F']));
        assert_eq!(alt_combo_bytes(KeyCombo::parse("alt+d").unwrap()), Some(vec![0x1b, b'd']));
        assert_eq!(alt_combo_bytes(KeyCombo::parse("alt+backspace").unwrap()), Some(vec![0x1b, 0x7f]));
        assert_eq!(alt_combo_bytes(KeyCombo::parse("alt+space").unwrap()), Some(vec![0x1b, b' ']));
        assert_eq!(alt_combo_bytes(KeyCombo::parse("ctrl+alt+f").unwrap()), None);
        assert_eq!(alt_combo_bytes(KeyCombo::parse("alt+f1").unwrap()), None);
    }

    #[test]
    fn alt_arrows_jump_by_word() {
        assert_eq!(alt_arrows(KeyCombo::parse("alt+left").unwrap()), Some([0x1b, b'b']));
        assert_eq!(alt_arrows(KeyCombo::parse("alt+right").unwrap()), Some([0x1b, b'f']));
        assert_eq!(alt_arrows(KeyCombo::parse("alt+up").unwrap()), None);
        assert_eq!(alt_arrows(KeyCombo::parse("ctrl+alt+left").unwrap()), None, "the screen reader keeps Ctrl+Alt+arrows");
    }
}

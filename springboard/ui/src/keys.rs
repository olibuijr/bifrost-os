//! Key equivalents (`NativeMenuItem.keyEquivalent`, `NativeKeyCommand`): parsing, display and a conflict-checked registry.
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Return,
    Escape,
    Tab,
    Space,
    /// The key above Return (backspace).
    Delete,
    /// The forward-delete key (Fn+Delete on a laptop keyboard).
    ForwardDelete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

/// A key with modifiers. `cmd` is the primary shortcut modifier (Super on this desktop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl KeyCombo {
    pub fn new(key: Key) -> Self {
        KeyCombo { cmd: false, ctrl: false, alt: false, shift: false, key }
    }
    /// Parses "cmd+shift+z", "Ctrl+S", "cmd+,", "alt+F4", "escape". Modifiers and keys are case-insensitive; a letter key
    /// is normalised to lower case (Shift is a modifier, not a different key). `None` for anything unrecognised.
    pub fn parse(s: &str) -> Option<KeyCombo> {
        let parts: Vec<&str> = s.split('+').collect();
        // "cmd++" means Cmd and the plus key: an empty piece between separators is a literal '+'.
        let mut tokens: Vec<String> = Vec::new();
        let mut i = 0;
        while i < parts.len() {
            if parts[i].is_empty() && i + 1 < parts.len() && parts[i + 1].is_empty() {
                tokens.push("+".into());
                i += 2;
            } else if parts[i].is_empty() && i == parts.len() - 1 && !tokens.is_empty() {
                i += 1;
            } else {
                tokens.push(parts[i].trim().to_string());
                i += 1;
            }
        }
        let (key_tok, mods) = tokens.split_last()?;
        let mut c = KeyCombo::new(parse_key(key_tok)?);
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "super" | "meta" => c.cmd = true,
                "ctrl" | "control" => c.ctrl = true,
                "alt" | "opt" | "option" => c.alt = true,
                "shift" => c.shift = true,
                _ => return None,
            }
        }
        Some(c)
    }

    /// The combination a chord names, from the evdev code of the key and the modifiers held (`ctrl`, `shift`, `alt`,
    /// `cmd`): a letter, a digit, Space, Backspace, forward Delete, the four arrows, Home, End, Page Up and Page Down.
    /// `None` for any other key. The physical key is named, whatever the layout types.
    pub fn from_evdev(code: u32, ctrl: bool, shift: bool, alt: bool, cmd: bool) -> Option<KeyCombo> {
        const ROWS: [(u32, &str); 3] = [(16, "qwertyuiop"), (30, "asdfghjkl"), (44, "zxcvbnm")];
        let key = match code {
            14 => Key::Delete,
            111 => Key::ForwardDelete,
            57 => Key::Space,
            102 => Key::Home,
            103 => Key::Up,
            104 => Key::PageUp,
            105 => Key::Left,
            106 => Key::Right,
            107 => Key::End,
            108 => Key::Down,
            109 => Key::PageDown,
            2..=10 => Key::Char(char::from(b'1' + (code - 2) as u8)),
            11 => Key::Char('0'),
            _ => Key::Char(ROWS.iter().find_map(|(start, chars)| code.checked_sub(*start).and_then(|i| chars.chars().nth(i as usize)))?),
        };
        Some(KeyCombo { cmd, ctrl, alt, shift, key })
    }
}

fn parse_key(t: &str) -> Option<Key> {
    let l = t.to_ascii_lowercase();
    Some(match l.as_str() {
        "return" | "enter" => Key::Return,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "delete" | "backspace" => Key::Delete,
        "forwarddelete" | "forward_delete" => Key::ForwardDelete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" => Key::PageUp,
        "pagedown" | "page_down" => Key::PageDown,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        _ => {
            if let Some(n) = l.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                return (1..=12).contains(&n).then_some(Key::F(n));
            }
            let mut ch = t.chars();
            match (ch.next(), ch.next()) {
                (Some(c), None) if !c.is_control() => Key::Char(c.to_ascii_lowercase()),
                _ => return None,
            }
        }
    })
}

impl fmt::Display for KeyCombo {
    /// Golden Gate menu notation: ⌃⌥⇧⌘ then the key, e.g. "⇧⌘Z".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl { f.write_str("⌃")?; }
        if self.alt { f.write_str("⌥")?; }
        if self.shift { f.write_str("⇧")?; }
        if self.cmd { f.write_str("⌘")?; }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::Return => f.write_str("↩"),
            Key::Escape => f.write_str("⎋"),
            Key::Tab => f.write_str("⇥"),
            Key::Space => f.write_str("Space"),
            Key::Delete => f.write_str("⌫"),
            Key::ForwardDelete => f.write_str("⌦"),
            Key::Left => f.write_str("←"),
            Key::Right => f.write_str("→"),
            Key::Up => f.write_str("↑"),
            Key::Down => f.write_str("↓"),
            Key::Home => f.write_str("↖"),
            Key::End => f.write_str("↘"),
            Key::PageUp => f.write_str("⇞"),
            Key::PageDown => f.write_str("⇟"),
            Key::F(n) => write!(f, "F{n}"),
        }
    }
}

/// Which command already owns a combination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub combo: KeyCombo,
    pub existing: u32,
}

/// Maps key combinations to command ids and refuses duplicates.
#[derive(Default)]
pub struct Shortcuts {
    map: HashMap<KeyCombo, u32>,
}

impl Shortcuts {
    pub fn bind(&mut self, combo: KeyCombo, command: u32) -> Result<(), Conflict> {
        match self.map.get(&combo) {
            Some(&existing) if existing != command => Err(Conflict { combo, existing }),
            _ => {
                self.map.insert(combo, command);
                Ok(())
            }
        }
    }
    pub fn command_for(&self, combo: &KeyCombo) -> Option<u32> {
        self.map.get(combo).copied()
    }
    /// The combination shown beside a menu item.
    pub fn combo_for(&self, command: u32) -> Option<KeyCombo> {
        self.map.iter().filter(|(_, c)| **c == command).map(|(k, _)| *k).min_by_key(|k| format!("{k}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_keys_and_case() {
        let c = KeyCombo::parse("Cmd+Shift+Z").unwrap();
        assert!(c.cmd && c.shift && !c.ctrl && !c.alt);
        assert_eq!(c.key, Key::Char('z'));
        assert_eq!(KeyCombo::parse("ctrl+S"), KeyCombo::parse("Control+s"));
        assert_eq!(KeyCombo::parse("alt+F4").unwrap().key, Key::F(4));
        assert_eq!(KeyCombo::parse("escape"), Some(KeyCombo::new(Key::Escape)));
        assert_eq!(KeyCombo::parse("cmd+,").unwrap().key, Key::Char(','));
        assert_eq!(KeyCombo::parse("cmd++").unwrap().key, Key::Char('+'));
        assert_eq!(KeyCombo::parse("+"), Some(KeyCombo::new(Key::Char('+'))), "a bare plus is the plus key");
    }

    #[test]
    fn evdev_chords_name_letters_digits_and_the_editing_keys() {
        let c = |code, ctrl, shift, alt, cmd| KeyCombo::from_evdev(code, ctrl, shift, alt, cmd);
        assert_eq!(c(30, false, false, false, true), KeyCombo::parse("cmd+a"));
        assert_eq!(c(44, false, true, false, true), KeyCombo::parse("cmd+shift+z"));
        assert_eq!(c(2, true, false, false, false), KeyCombo::parse("ctrl+1"));
        assert_eq!(c(11, false, false, false, false), KeyCombo::parse("0"));
        assert_eq!(c(14, false, false, true, false), KeyCombo::parse("alt+backspace"));
        assert_eq!(c(111, false, false, false, false), KeyCombo::parse("forwarddelete"));
        assert_eq!(c(105, false, true, true, false), KeyCombo::parse("alt+shift+left"));
        assert_eq!(c(102, false, false, false, true), KeyCombo::parse("cmd+home"));
        assert_eq!(c(109, false, true, false, false), KeyCombo::parse("shift+pagedown"));
        assert_eq!(c(1, false, false, false, false), None, "Escape is not an editing key");
    }

    #[test]
    fn rejects_nonsense() {
        for bad in ["", "cmd+", "hyper+x", "cmd+f13", "cmd+f0", "cmd+toolong"] {
            assert_eq!(KeyCombo::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn displays_in_golden_gate_menu_notation() {
        assert_eq!(KeyCombo::parse("cmd+shift+z").unwrap().to_string(), "⇧⌘Z");
        assert_eq!(KeyCombo::parse("ctrl+alt+shift+cmd+q").unwrap().to_string(), "⌃⌥⇧⌘Q");
        assert_eq!(KeyCombo::parse("cmd+return").unwrap().to_string(), "⌘↩");
        assert_eq!(KeyCombo::parse("f5").unwrap().to_string(), "F5");
    }

    #[test]
    fn shortcuts_refuse_a_second_owner_but_allow_rebinding_the_same_command() {
        let mut s = Shortcuts::default();
        let save = KeyCombo::parse("cmd+s").unwrap();
        assert_eq!(s.bind(save, 1), Ok(()));
        assert_eq!(s.bind(save, 1), Ok(()), "binding the same command twice is fine");
        assert_eq!(s.bind(save, 2), Err(Conflict { combo: save, existing: 1 }));
        assert_eq!(s.command_for(&save), Some(1));
        assert_eq!(s.combo_for(1), Some(save));
        assert_eq!(s.combo_for(2), None);
        assert_eq!(s.command_for(&KeyCombo::parse("cmd+shift+s").unwrap()), None, "modifiers are part of the identity");
    }
}

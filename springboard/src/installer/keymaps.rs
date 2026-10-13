//! Keyboard layouts for the installer: evdev key codes to characters, with shift, AltGr, Caps Lock and
//! dead-key composition. The tables are generated from xkeyboard-config (see tools/gen_keymaps.py).

use super::keymaps_data::{COMPOSE, KEYMAPS, LAYOUTS};

pub const KEY_LSHIFT: u16 = 42;
pub const KEY_RSHIFT: u16 = 54;
pub const KEY_RALT: u16 = 100;
pub const KEY_CAPS: u16 = 58;

/// What a key press does to a text field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    Char(char),
    Backspace,
    Enter,
    Tab,
    BackTab,
    Escape,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub id: &'static str,
    pub name: &'static str,
    pub xkb_layout: &'static str,
    pub xkb_variant: &'static str,
    pub console_keymap: &'static str,
}

pub fn layouts() -> impl Iterator<Item = Layout> {
    LAYOUTS.iter().map(|&(id, name, xkb_layout, xkb_variant, console_keymap)| Layout { id, name, xkb_layout, xkb_variant, console_keymap })
}

/// The four characters a key types on a layout: plain, Shift, AltGr and Shift+AltGr (`'\0'` where it types none).
/// Read by the keyboard viewer, which draws a board from the same tables the typing uses.
pub fn key_levels(layout: usize, code: u16) -> Option<[char; 4]> {
    KEYMAPS.get(layout)?.iter().find(|k| k.0 == code).map(|k| k.1)
}

pub fn layout_index(id: &str) -> Option<usize> {
    LAYOUTS.iter().position(|l| l.0 == id)
}

/// The layout for an xkb layout and variant pair, as `keyboard.env` names them (`is` and empty, `us` and `intl`).
pub fn layout_for_xkb(layout: &str, variant: &str) -> Option<usize> {
    LAYOUTS.iter().position(|l| l.2 == layout && l.3 == variant)
}

/// The layout written to `/etc/bifrost/keyboard.env`, read from `text` (`XKB_DEFAULT_LAYOUT` and `XKB_DEFAULT_VARIANT`).
pub fn layout_from_keyboard_env(text: &str) -> Option<usize> {
    let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::to_string);
    layout_for_xkb(&value("XKB_DEFAULT_LAYOUT")?, &value("XKB_DEFAULT_VARIANT").unwrap_or_default())
}

/// The layout the running system uses: the session's `XKB_DEFAULT_*` variables, else `/etc/bifrost/keyboard.env`, else U.S.
pub fn configured_layout() -> usize {
    if let Ok(layout) = std::env::var("XKB_DEFAULT_LAYOUT") {
        let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
        if let Some(i) = layout_for_xkb(&layout, &variant) {
            return i;
        }
    }
    std::fs::read_to_string("/etc/bifrost/keyboard.env").ok().and_then(|t| layout_from_keyboard_env(&t)).unwrap_or(0)
}

/// Key state for one keyboard: modifiers, the active layout and a pending dead key.
#[derive(Clone, Debug, Default)]
pub struct Keyboard {
    layout: usize,
    shift: u8,
    altgr: bool,
    caps: bool,
    dead: Option<char>,
}

impl Keyboard {
    pub fn new(layout: usize) -> Self {
        Keyboard { layout: layout.min(KEYMAPS.len() - 1), ..Default::default() }
    }

    pub fn set_layout(&mut self, layout: usize) {
        self.layout = layout.min(KEYMAPS.len() - 1);
        self.dead = None;
    }

    pub fn layout(&self) -> usize {
        self.layout
    }

    fn lookup(&self, code: u16, shift: bool, altgr: bool) -> Option<char> {
        let level = usize::from(shift) + 2 * usize::from(altgr);
        KEYMAPS[self.layout].iter().find(|k| k.0 == code).map(|k| k.1[level]).filter(|c| *c != '\0')
    }

    /// Feeds one key event; returns what a key-down types, if anything. Modifier keys return None.
    pub fn feed(&mut self, code: u16, down: bool) -> Option<Typed> {
        match code {
            KEY_LSHIFT | KEY_RSHIFT => {
                self.shift = if down { self.shift.saturating_add(1) } else { self.shift.saturating_sub(1) };
                return None;
            }
            KEY_RALT => {
                self.altgr = down;
                return None;
            }
            KEY_CAPS => {
                if down {
                    self.caps = !self.caps;
                }
                return None;
            }
            _ => {}
        }
        if !down {
            return None;
        }
        self.press(code, self.shift > 0, self.altgr)
    }

    /// Types one key-down given the Shift and AltGr state the caller holds (the shell counts its own modifiers).
    pub fn press(&mut self, code: u16, shifted: bool, altgr: bool) -> Option<Typed> {
        let named = match code {
            14 => Some(Typed::Backspace),
            28 | 96 => Some(Typed::Enter),
            1 => Some(Typed::Escape),
            15 if shifted => Some(Typed::BackTab),
            15 => Some(Typed::Tab),
            103 => Some(Typed::Up),
            108 => Some(Typed::Down),
            105 => Some(Typed::Left),
            106 => Some(Typed::Right),
            _ => None,
        };
        if named.is_some() {
            self.dead = None;
            return named;
        }
        if code == 57 {
            return Some(Typed::Char(self.dead.take().map_or(' ', |m| mark_alone(m))));
        }
        let mut c = self.lookup(code, shifted, altgr)?;
        // Caps Lock flips the case of letters only
        if self.caps && !altgr && c.is_alphabetic() {
            c = self.lookup(code, !shifted, false).unwrap_or(c);
        }
        if is_mark(c) {
            // a dead key: a second press of the same mark types it by itself
            return match self.dead.take() {
                Some(m) if m == c => Some(Typed::Char(mark_alone(c))),
                _ => {
                    self.dead = Some(c);
                    None
                }
            };
        }
        Some(Typed::Char(match self.dead.take() {
            Some(m) => compose(m, c).unwrap_or(c),
            None => c,
        }))
    }
}

fn is_mark(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

fn mark_alone(m: char) -> char {
    match m {
        '\u{300}' => '`',
        '\u{301}' => '´',
        '\u{302}' => '^',
        '\u{303}' => '~',
        '\u{308}' => '¨',
        '\u{30a}' => '°',
        '\u{30c}' => 'ˇ',
        '\u{327}' => '¸',
        _ => ' ',
    }
}

fn compose(mark: char, base: char) -> Option<char> {
    COMPOSE.iter().find(|c| c.0 == mark && c.1 == base).map(|c| c.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb(id: &str) -> Keyboard {
        Keyboard::new(layout_index(id).unwrap())
    }

    fn typed(k: &mut Keyboard, codes: &[u16]) -> String {
        let mut s = String::new();
        for &c in codes {
            if let Some(Typed::Char(ch)) = k.feed(c, true) {
                s.push(ch);
            }
            k.feed(c, false);
        }
        s
    }

    #[test]
    fn us_typing_and_shift() {
        let mut k = kb("us");
        assert_eq!(typed(&mut k, &[35, 18, 38, 38, 24]), "hello");
        k.feed(KEY_LSHIFT, true);
        assert_eq!(typed(&mut k, &[2, 30]), "!A");
        k.feed(KEY_LSHIFT, false);
        assert_eq!(typed(&mut k, &[2]), "1");
    }

    #[test]
    fn icelandic_has_its_own_letters() {
        let mut k = kb("is");
        // ö is where '-' is on a US board (code 12), ð where '[' is (26), þ at '/' (53), æ at ';' (39)
        assert_eq!(typed(&mut k, &[12, 26, 53, 39]), "öðþæ");
        k.feed(KEY_LSHIFT, true);
        assert_eq!(typed(&mut k, &[3]), "\"");
    }

    #[test]
    fn german_swaps_y_and_z_and_altgr_types_euro() {
        let mut k = kb("de");
        assert_eq!(typed(&mut k, &[44, 21]), "yz");
        k.feed(KEY_RALT, true);
        assert_eq!(typed(&mut k, &[18]), "€");
    }

    #[test]
    fn dead_keys_compose_and_space_types_the_mark() {
        let mut k = kb("us-intl");
        // the apostrophe key is dead acute on US international
        assert_eq!(typed(&mut k, &[40, 18]), "é");
        assert_eq!(typed(&mut k, &[40, 57]), "´");
    }

    #[test]
    fn caps_lock_flips_letters_only() {
        let mut k = kb("us");
        k.feed(KEY_CAPS, true);
        k.feed(KEY_CAPS, false);
        assert_eq!(typed(&mut k, &[30, 2]), "A1");
    }

    #[test]
    fn every_layout_types_a_full_alphabet() {
        for (i, l) in layouts().enumerate() {
            let mut k = Keyboard::new(i);
            let mut letters: Vec<char> = (1..=255u16).filter_map(|c| match k.feed(c, true) { Some(Typed::Char(ch)) if ch.is_alphabetic() => Some(ch), _ => None }).collect();
            letters.sort();
            letters.dedup();
            assert!(letters.len() >= 20, "{} types only {} letters", l.name, letters.len());
        }
    }

    #[test]
    fn the_installed_layout_is_read_from_keyboard_env() {
        assert_eq!(layout_from_keyboard_env("XKB_DEFAULT_LAYOUT=is\nXKB_DEFAULT_VARIANT=\n"), layout_index("is"));
        assert_eq!(layout_from_keyboard_env("XKB_DEFAULT_LAYOUT=us\nXKB_DEFAULT_VARIANT=intl\n"), layout_index("us-intl"));
        assert_eq!(layout_from_keyboard_env("XKB_DEFAULT_LAYOUT=xx\nXKB_DEFAULT_VARIANT=\n"), None, "an unknown layout falls back");
        assert_eq!(layout_from_keyboard_env(""), None);
    }
}

//! Hardware-keyboard shortcuts: tracks the modifier keys from evdev key
//! events and turns chords into shell actions. Cmd/Meta/Ctrl+Space opens
//! Search, Alt/Meta/Ctrl+Tab the app switcher, Esc is Back, Meta alone is
//! Home, and Ctrl+C/V/X/A/Z are the clipboard and undo, which text controls
//! take as `Out::Named`.

use super::keyboard::Out;
use super::desk::Tile;
use super::{AppId, Effect, Screen, Shell};
use crate::installer::keymaps::{Keyboard, Typed};
use bifrost_proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};

const KEY_ESC: u16 = 1;
const KEY_TAB: u16 = 15;
const KEY_W: u16 = 17;
const KEY_Q: u16 = 16;
const KEY_H: u16 = 35;
const KEY_F: u16 = 33;
const KEY_GRAVE: u16 = 41;
const KEY_UP: u16 = 103;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_DOWN: u16 = 108;
const KEY_Y: u16 = 21;
const KEY_LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_Z: u16 = 44;
const KEY_X: u16 = 45;
const KEY_C: u16 = 46;
const KEY_V: u16 = 47;
const KEY_M: u16 = 50;
const KEY_COMMA: u16 = 51;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_SPACE: u16 = 57;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;

/// Term key equivalents, taken only while Term is in front and only with Cmd held (Ctrl chords stay control characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermKey {
    Copy,
    SelectAll,
    Find,
    FindNext,
    FindPrev,
    ClearScrollback,
    Bigger,
    Smaller,
    ActualSize,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    /// Cmd+1 .. Cmd+9, zero based
    Tab(u8),
}

impl TermKey {
    /// The Term action of a Cmd chord: the evdev key code and whether Shift is held.
    pub fn from_chord(code: u16, shift: bool) -> Option<TermKey> {
        Some(match (code, shift) {
            (46, false) => TermKey::Copy,
            (30, false) => TermKey::SelectAll,
            (33, false) => TermKey::Find,
            (34, false) => TermKey::FindNext,
            (34, true) => TermKey::FindPrev,
            (37, false) => TermKey::ClearScrollback,
            // Cmd+= and Cmd++ (Shift+=) both enlarge
            (13, _) => TermKey::Bigger,
            (12, false) => TermKey::Smaller,
            (11, false) => TermKey::ActualSize,
            (20, false) => TermKey::NewTab,
            (17, false) => TermKey::CloseTab,
            (27, true) => TermKey::NextTab,
            (26, true) => TermKey::PrevTab,
            (2..=10, false) => TermKey::Tab((code - 2) as u8),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Term(TermKey),
    QuickSearch,
    AppSwitch,
    /// Esc
    Back,
    /// the Meta (Super) key pressed and released alone
    Home,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Undo,
    Redo,
    /// Ctrl+W: close the front desktop window
    Close,
    /// Ctrl+M: minimize the front desktop window
    Minimize,
    /// Cmd+, : the app's Settings (the App menu's Settings… key equivalent)
    Settings,
    /// Ctrl+Cmd+F: full screen (zoom) for the front desktop window
    FullScreen,
    /// Ctrl+Cmd+arrows: tile the front desktop window (left, right, fill with Up, centre with Down)
    Tile(Tile),
    /// Cmd+`: bring the window behind the front one forward (the Window menu's Next Window)
    CycleWindow,
    /// Ctrl+Alt+Right (true) and Ctrl+Alt+Left (false): the screen reader's cursor moves to the next or previous element
    ReaderStep(bool),
}

impl Shortcut {
    /// The key a text control receives for an editing shortcut.
    pub fn out(self) -> Option<Out> {
        Some(Out::Named(match self {
            Shortcut::Copy => "Copy",
            Shortcut::Cut => "Cut",
            Shortcut::Paste => "Paste",
            Shortcut::SelectAll => "SelectAll",
            Shortcut::Undo => "Undo",
            Shortcut::Redo => "Redo",
            Shortcut::Term(_) | Shortcut::QuickSearch | Shortcut::AppSwitch | Shortcut::Back | Shortcut::Home | Shortcut::Close | Shortcut::Minimize | Shortcut::Settings | Shortcut::FullScreen | Shortcut::Tile(_) | Shortcut::CycleWindow | Shortcut::ReaderStep(_) => return None,
        }))
    }
}

/// Modifier state; feed it every key event.
#[derive(Default)]
pub struct HwKeys {
    ctrl: u8,
    shift: u8,
    /// Every Alt key, Right Alt (AltGr) included
    alt: u8,
    /// Right Alt only: AltGr selects the third and fourth characters of a key
    altgr: u8,
    meta: u8,
    /// Meta went down and nothing else has been pressed since
    meta_alone: bool,
    /// The installed keyboard layout: what a key types
    text: Keyboard,
}

impl HwKeys {
    /// Forgets every held modifier. Releases that arrive while the screen is
    /// off or the device is gone never reach `key`, so the counters would
    /// stick and turn plain keys into chords. The keyboard layout is kept.
    pub fn reset(&mut self) {
        let layout = self.text.layout();
        *self = Self { text: Keyboard::new(layout), ..Self::default() };
    }

    /// Sets the keyboard layout (an index into the installer's `LAYOUTS`) that text keys type with.
    pub fn set_layout(&mut self, layout: usize) {
        self.text.set_layout(layout);
    }

    /// True while any Ctrl, Alt or Meta key is held (the app switcher commits when the last one is released).
    pub fn mods_held(&self) -> bool {
        self.ctrl + self.alt + self.meta > 0
    }

    /// Feeds a key event; returns the shortcut a key-down completes.
    pub fn key(&mut self, code: u16, down: bool) -> Option<Shortcut> {
        self.key_in(code, down, false)
    }

    /// `key`, with Term's Cmd chords taken first when `term` (Term is in front).
    pub fn key_in(&mut self, code: u16, down: bool, term: bool) -> Option<Shortcut> {
        if code == KEY_RIGHTALT {
            self.altgr = if down { self.altgr.saturating_add(1) } else { self.altgr.saturating_sub(1) };
        }
        let count = match code {
            KEY_LEFTCTRL | KEY_RIGHTCTRL => Some(&mut self.ctrl),
            KEY_LEFTSHIFT | KEY_RIGHTSHIFT => Some(&mut self.shift),
            KEY_LEFTALT | KEY_RIGHTALT => Some(&mut self.alt),
            KEY_LEFTMETA | KEY_RIGHTMETA => Some(&mut self.meta),
            _ => None,
        };
        if let Some(n) = count {
            *n = if down { n.saturating_add(1) } else { n.saturating_sub(1) };
            if matches!(code, KEY_LEFTMETA | KEY_RIGHTMETA) {
                if down {
                    self.meta_alone = self.meta == 1 && self.ctrl + self.shift + self.alt == 0;
                } else if self.meta == 0 && std::mem::take(&mut self.meta_alone) {
                    return Some(Shortcut::Home);
                }
            } else {
                self.meta_alone = false;
            }
            return None;
        }
        self.meta_alone = false;
        if !down {
            return None;
        }
        let (ctrl, alt, meta) = (self.ctrl > 0, self.alt > 0, self.meta > 0);
        if term && meta && !ctrl && !alt {
            if let Some(k) = TermKey::from_chord(code, self.shift > 0) {
                return Some(Shortcut::Term(k));
            }
        }
        match code {
            KEY_SPACE if (meta || ctrl) && !alt => Some(Shortcut::QuickSearch),
            KEY_TAB if alt || meta || ctrl => Some(Shortcut::AppSwitch),
            KEY_ESC if !ctrl && !alt && !meta => Some(Shortcut::Back),
            // Cmd on a Mac keyboard is Meta here, so it works for edits too
            KEY_Z if (ctrl || meta) && !alt => Some(if self.shift > 0 { Shortcut::Redo } else { Shortcut::Undo }),
            KEY_Y if (ctrl || meta) && !alt && self.shift == 0 => Some(Shortcut::Redo),
            // Ctrl+Q (quit the shell) is deliberately not bound. Ctrl+W and Ctrl+M are
            // not bound: they reach terminals as readline kill-word and Enter. Cmd/Super+W and
            // Cmd/Super+M close and minimize the front desktop window; terminals never see them,
            // and where no desktop window is open `shell` lets the key fall through unchanged.
            // Cmd+Q quits the app and Cmd+H hides it; a window is one app here, so they close and minimise it
            KEY_W | KEY_Q if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::Close),
            KEY_M | KEY_H if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::Minimize),
            KEY_F if meta && ctrl && !alt => Some(Shortcut::FullScreen),
            KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN if meta && ctrl && !alt && self.shift == 0 => Some(Shortcut::Tile(match code {
                KEY_LEFT => Tile::Left,
                KEY_RIGHT => Tile::Right,
                KEY_UP => Tile::Fill,
                _ => Tile::Center,
            })),
            KEY_GRAVE if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::CycleWindow),
            KEY_LEFT | KEY_RIGHT if ctrl && alt && !meta && self.shift == 0 => Some(Shortcut::ReaderStep(code == KEY_RIGHT)),
            KEY_COMMA if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::Settings),
            KEY_C | KEY_X | KEY_V | KEY_A if (ctrl || meta) && !alt && self.shift == 0 => Some(match code {
                KEY_C => Shortcut::Copy,
                KEY_X => Shortcut::Cut,
                KEY_V => Shortcut::Paste,
                _ => Shortcut::SelectAll,
            }),
            _ => None,
        }
    }
}

/// Keys a Ctrl or Cmd chord can name, by evdev code with their US legends (the physical key, whatever the layout).
const TEXT_KEYS: &[(u16, char, char)] = &[
    (2, '1', '!'), (3, '2', '@'), (4, '3', '#'), (5, '4', '$'), (6, '5', '%'), (7, '6', '^'), (8, '7', '&'), (9, '8', '*'), (10, '9', '('), (11, '0', ')'),
    (12, '-', '_'), (13, '=', '+'), (16, 'q', 'Q'), (17, 'w', 'W'), (18, 'e', 'E'), (19, 'r', 'R'), (20, 't', 'T'), (21, 'y', 'Y'), (22, 'u', 'U'), (23, 'i', 'I'),
    (24, 'o', 'O'), (25, 'p', 'P'), (26, '[', '{'), (27, ']', '}'), (30, 'a', 'A'), (31, 's', 'S'), (32, 'd', 'D'), (33, 'f', 'F'), (34, 'g', 'G'), (35, 'h', 'H'),
    (36, 'j', 'J'), (37, 'k', 'K'), (38, 'l', 'L'), (39, ';', ':'), (40, '\'', '"'), (41, '`', '~'), (43, '\\', '|'), (44, 'z', 'Z'), (45, 'x', 'X'), (46, 'c', 'C'),
    (47, 'v', 'V'), (48, 'b', 'B'), (49, 'n', 'N'), (50, 'm', 'M'), (51, ',', '<'), (52, '.', '>'), (53, '/', '?'), (57, ' ', ' '),
];

impl HwKeys {
    /// The CHORD_* bits of a Ctrl or Cmd chord over a letter, digit or punctuation key. None when
    /// no Ctrl or Meta is held, or the key is not one a chord can name.
    pub fn chord_mods(&self, code: u16) -> Option<u32> {
        // Backspace is a chord key too: Cmd+Backspace deletes in the apps
        if self.ctrl == 0 && self.meta == 0 || (code != 14 && !TEXT_KEYS.iter().any(|k| k.0 == code)) {
            return None;
        }
        let mut mods = 0;
        for (held, bit) in [(self.ctrl, CHORD_CTRL), (self.shift, CHORD_SHIFT), (self.alt, CHORD_ALT), (self.meta, CHORD_META)] {
            if held > 0 {
                mods |= bit;
            }
        }
        Some(mods)
    }

    /// What a key-down types into a text control or the terminal: characters, Backspace, Enter
    /// and the terminal's named keys. None while Ctrl, Alt or Meta is held (those are shortcuts).
    pub fn typed(&mut self, code: u16) -> Option<Out> {
        // Left Alt, Ctrl and Meta are shortcuts; AltGr types the layout's third level
        if self.ctrl > 0 || self.meta > 0 || self.alt > self.altgr {
            return None;
        }
        Some(match self.text.press(code, self.shift > 0, self.altgr > 0)? {
            Typed::Char(c) => Out::Text(c.to_string()),
            Typed::Backspace => Out::Backspace,
            Typed::Enter => Out::Enter,
            Typed::Tab | Typed::BackTab => Out::Named("Tab"),
            Typed::Escape => Out::Named("Escape"),
            Typed::Up => Out::Named("Up"),
            Typed::Down => Out::Named("Down"),
            Typed::Left => Out::Named("Left"),
            Typed::Right => Out::Named("Right"),
        })
    }
}

/// Key auto-repeat: after `REPEAT_DELAY_MS` held, the last typed key repeats at 30 Hz. A pure state
/// machine (the evdev decoder drops the kernel's own repeats), cancelled by a release or another key.
#[derive(Default)]
pub struct KeyRepeat {
    held: Option<(u16, Out, f64)>,
}

pub const REPEAT_DELAY_MS: f64 = 400.0;
const REPEAT_PERIOD_MS: f64 = 1000.0 / 30.0;

impl KeyRepeat {
    /// A key went down; `out` is what it types (None for keys that do not repeat, which still cancel).
    pub fn press(&mut self, code: u16, out: Option<Out>, t: f64) {
        self.held = out.filter(|o| *o != Out::Named("Escape")).map(|o| (code, o, t + REPEAT_DELAY_MS));
    }

    pub fn release(&mut self, code: u16) {
        if self.held.as_ref().is_some_and(|h| h.0 == code) {
            self.held = None;
        }
    }

    pub fn deadline(&self) -> Option<f64> {
        self.held.as_ref().map(|h| h.2)
    }

    /// The repeats due by `t` (at most a few, so a stalled loop does not flood a field).
    pub fn due(&mut self, t: f64) -> Vec<Out> {
        let Some((_, out, next)) = self.held.as_mut() else { return vec![] };
        let mut v = vec![];
        while t >= *next && v.len() < 4 {
            v.push(out.clone());
            *next += REPEAT_PERIOD_MS;
        }
        if t >= *next {
            *next = t + REPEAT_PERIOD_MS;
        }
        v
    }
}

impl Shell {
    /// A key from a physical keyboard that the app on screen handles itself (Deildu, Files); true if taken.
    /// Power, volume and wake stay with the shell.
    pub fn hw_key(&mut self, code: u16, down: bool, t: f64) -> bool {
        const KEEP: [u16; 4] = [114, 115, 116, 143];
        if !down || !self.screen_on || self.anim.is_some() || KEEP.contains(&code) {
            return false;
        }
        let used = match self.screen {
            Screen::App(AppId::Deildu) => self.deildu.as_mut().is_some_and(|d| d.hw_key(code, t)),
            Screen::App(AppId::Files) => self.files.as_mut().is_some_and(|f| f.hw_key(code, t)),
            _ => false,
        };
        if used {
            self.last_input = t;
            self.dirty = true;
        }
        used
    }

    /// Types a physical-keyboard key into the terminal, a hosted app's text field or the focused
    /// native text field of the app on screen (Search, Browser, Settings, Files, Deildu); elsewhere it is ignored. The effects are the field's (a Wi-Fi join, say).
    /// A Ctrl or Cmd chord the shell did not take goes to the external app in front. False when
    /// the screen is not an external app, so the key falls through as before.
    pub fn hw_chord(&mut self, code: u16, mods: u32) -> bool {
        // Option+Cmd+D (KEY_D): Automatically Hide and Show for the dock
        if code == 32 && mods == (CHORD_META | CHORD_ALT) && self.dock_toggle_autohide() {
            return true;
        }
        if !self.screen_on || !matches!(self.screen, Screen::App(AppId::External(_))) {
            return false;
        }
        self.ext_chord(u32::from(code), mods)
    }

    pub fn hw_type(&mut self, out: Out, t: f64) -> Vec<Effect> {
        if !self.screen_on {
            return vec![];
        }
        // with a panel open, typing goes to its name field and nowhere else
        #[cfg(feature = "desktop-wayland")]
        if self.doc_panel.is_some() {
            if let Out::Text(text) = &out {
                self.doc_type(text);
            }
            return vec![];
        }
        // the login window opens on a tick; the shell may not have drawn since the lock, so open it for this key
        if self.screen == Screen::Lock {
            self.login_ensure(t);
        }
        let fx = match self.screen {
            // an open Search takes the typing, over whatever screen it was opened on
            _ if self.quicksearch.is_shown() => self.quicksearch_key(out, t),
            Screen::App(AppId::Terminal | AppId::External(_)) => {
                self.type_out(out, t);
                vec![]
            }
            Screen::Lock if self.login_open() => self.login_key(&out, t),
            Screen::Lock if matches!(out, Out::Enter) && self.lock_pad.is_none() && self.anim.is_none() => {
                self.begin_unlock(0.0, t);
                vec![]
            }
            Screen::App(app) if self.anim.is_none() => self.hw_type_app(app, out, t),
            _ => return vec![],
        };
        self.last_input = t;
        self.dirty = true;
        fx
    }

    fn hw_type_app(&mut self, app: AppId, out: Out, t: f64) -> Vec<Effect> {
        match app {
            AppId::Settings => {
                if let Some(s) = self.settings.as_mut() {
                    return s.key(&self.ui, out, t);
                }
            }
            AppId::Browser => {
                if let Some(b) = self.browser.as_mut() {
                    b.key(&self.ui, out, t);
                }
            }
            AppId::Files => {
                if let Some(f) = self.files.as_mut() {
                    f.key(&self.ui, out, t);
                }
            }
            AppId::Deildu => {
                if let Some(d) = self.deildu.as_mut() {
                    d.hw_text(&self.ui, out);
                }
            }
            _ => {}
        }
        vec![]
    }

    /// Esc: closes the topmost thing (Control Center, Search, the switcher), else steps back in
    /// the app on screen, else leaves it. The terminal and hosted apps get the Escape key itself.
    fn hw_back(&mut self, t: f64) -> Vec<Effect> {
        #[cfg(feature = "desktop-wayland")]
        if let Some(fx) = self.desktop_menu_key(1, t) {
            return fx;
        }
        if self.cc_active() {
            self.cc.hide();
            return vec![];
        }
        if self.center_active() {
            self.notify.center.hide();
            self.dirty = true;
            return vec![];
        }
        if self.quicksearch.is_shown() {
            self.quicksearch.hide();
            return vec![];
        }
        match self.screen {
            Screen::Switcher => return self.assistive_action(super::assistive_touch::Action::Home, t),
            Screen::App(AppId::Terminal | AppId::External(_)) => self.type_out(Out::Named("Escape"), t),
            Screen::App(app) => {
                let stepped = match app {
                    AppId::Settings => self.settings.as_mut().is_some_and(|s| s.back_page(t)),
                    AppId::Files => self.files.as_mut().is_some_and(|f| f.hw_back(t)),
                    AppId::Browser => match self.browser.as_mut() {
                        Some(b) if b.keyboard_shown() => {
                            b.key(&self.ui, Out::Hide, t);
                            true
                        }
                        _ => false,
                    },
                    _ => false,
                };
                if !stepped {
                    self.go_home(t);
                }
            }
            Screen::Home | Screen::Lock => {}
        }
        vec![]
    }

    /// Runs a shortcut. Search, the switcher, Back and Home are the shell's own; the terminal
    /// takes Paste, and the editing shortcuts go to the focused native text field.
    /// Ctrl+W / Ctrl+M act only on the desktop. Elsewhere `shell` drops the
    /// shortcut so the key falls through exactly as it did before.
    /// True while Term is the front app and takes the Cmd key equivalents.
    pub fn term_front(&self) -> bool {
        self.screen_on && self.screen == Screen::App(AppId::Terminal) && !self.is_locked()
    }
    pub fn hw_window_shortcut(&self, sc: Shortcut) -> bool {
        cfg!(feature = "desktop-wayland") && matches!(sc, Shortcut::Close | Shortcut::Minimize) && self.desk_active() && !self.is_locked()
    }
    /// Keys while a desktop menu is open (arrows, Return); None when no menu is open or on release.
    pub fn hw_menu_key(&mut self, code: u16, down: bool, t: f64) -> Option<Vec<Effect>> {
        // the open or save panel takes its own keys; the rest fall through to `hw_type`, which types the file name
        #[cfg(feature = "desktop-wayland")]
        if self.doc_panel.is_some() {
            return (!down || self.doc_key(code, t)).then(Vec::new);
        }
        // the preview panel takes Esc, Space and the arrows while it is up
        if self.quick_look_key(code, down).is_some() {
            return Some(vec![]);
        }
        #[cfg(feature = "desktop-wayland")]
        if self.desktop_menu.is_some() {
            return Some(if down { self.desktop_menu_key(code, t).unwrap_or_default() } else { vec![] });
        }
        let _ = (code, down, t);
        None
    }

    pub fn hw_shortcut(&mut self, sc: Shortcut, t: f64) -> Vec<Effect> {
        if !self.screen_on || self.screen == Screen::Lock {
            return vec![];
        }
        let in_terminal = matches!(self.screen, Screen::App(AppId::Terminal | AppId::External(_)));
        let fx = match sc {
            Shortcut::QuickSearch => {
                self.open_quicksearch_hw(t);
                vec![]
            }
            Shortcut::AppSwitch => {
                if self.screen != Screen::Switcher {
                    self.open_app_switcher(t);
                }
                vec![]
            }
            #[cfg(feature = "desktop-wayland")]
            Shortcut::Close | Shortcut::Minimize if self.hw_window_shortcut(sc) => self.desk_window_close_or_minimize(sc == Shortcut::Minimize),
            #[cfg(feature = "desktop-wayland")]
            Shortcut::Settings if self.desk_active() => {
                self.desk_open(AppId::Settings, t);
                vec![]
            }
            #[cfg(feature = "desktop-wayland")]
            #[cfg(feature = "desktop-wayland")]
            Shortcut::Tile(tile) if self.desk_active() && !self.is_locked() => {
                self.desk_tile_front(tile);
                vec![]
            }
            #[cfg(feature = "desktop-wayland")]
            Shortcut::CycleWindow if self.desk_active() && !self.is_locked() && !in_terminal => {
                self.desk_cycle_window();
                self.cmdtab_release(t);
                vec![]
            }
            Shortcut::ReaderStep(forward) if self.screen_reader_on() => self.reader_step(forward, t),
            Shortcut::FullScreen if self.desk_active() && !self.is_locked() => {
                if let Some(i) = self.desk.as_ref().and_then(|d| d.front().and_then(|w| d.index_of(w.app))) {
                    self.desk_zoom_window(i);
                }
                vec![]
            }
            Shortcut::Back => self.hw_back(t),
            Shortcut::Home => self.assistive_action(super::assistive_touch::Action::Home, t),
            Shortcut::Term(k) => self.terminal_hw_key(k, t),
            Shortcut::Paste if self.screen == Screen::App(AppId::Terminal) => {
                self.type_out(Out::Named("Paste"), t);
                vec![]
            }
            sc if !in_terminal => sc.out().map_or(vec![], |out| self.hw_type(out, t)),
            _ => vec![],
        };
        self.dirty = true;
        fx
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_takes_cmd_chords_only_while_in_front_and_leaves_ctrl_alone() {
        let mut k = HwKeys::default();
        let in_term = |k: &mut HwKeys, mods: &[u16], key: u16| {
            for m in mods {
                k.key(*m, true);
            }
            let r = k.key_in(key, true, true);
            k.key(key, false);
            for m in mods {
                k.key(*m, false);
            }
            r
        };
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 33), Some(Shortcut::Term(TermKey::Find)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 34), Some(Shortcut::Term(TermKey::FindNext)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], 34), Some(Shortcut::Term(TermKey::FindPrev)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 37), Some(Shortcut::Term(TermKey::ClearScrollback)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 13), Some(Shortcut::Term(TermKey::Bigger)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], 13), Some(Shortcut::Term(TermKey::Bigger)), "Cmd++");
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 12), Some(Shortcut::Term(TermKey::Smaller)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 11), Some(Shortcut::Term(TermKey::ActualSize)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 20), Some(Shortcut::Term(TermKey::NewTab)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 17), Some(Shortcut::Term(TermKey::CloseTab)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], 27), Some(Shortcut::Term(TermKey::NextTab)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], 26), Some(Shortcut::Term(TermKey::PrevTab)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 2), Some(Shortcut::Term(TermKey::Tab(0))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], 10), Some(Shortcut::Term(TermKey::Tab(8))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], KEY_C), Some(Shortcut::Term(TermKey::Copy)));
        assert_eq!(in_term(&mut k, &[KEY_LEFTMETA], KEY_A), Some(Shortcut::Term(TermKey::SelectAll)));
        // Ctrl chords are never Term's: Ctrl+F, Ctrl+K and Ctrl+T stay control characters for the pty
        for key in [33, 37, 20, 34] {
            assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], key), None);
        }
        // outside Term the same Cmd chords fall through to the app
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.key_in(33, true, false), None);
        assert_eq!(k.key_in(KEY_C, true, false), Some(Shortcut::Copy));
    }

    #[test]
    fn ctrl_and_cmd_chords_name_the_key_and_held_modifiers() {
        let mut k = HwKeys::default();
        assert_eq!(k.chord_mods(49), None, "no modifier, no chord");
        k.key(KEY_LEFTCTRL, true);
        assert_eq!(k.chord_mods(49), Some(CHORD_CTRL), "Ctrl+N");
        k.key(KEY_LEFTSHIFT, true);
        assert_eq!(k.chord_mods(49), Some(CHORD_CTRL | CHORD_SHIFT));
        k.key(KEY_LEFTSHIFT, false);
        k.key(KEY_LEFTCTRL, false);
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.chord_mods(33), Some(CHORD_META), "Cmd+F");
        assert_eq!(k.chord_mods(14), Some(CHORD_META), "Cmd+Backspace");
        // Cmd+Space is Search: the shell takes shortcuts before it asks for a chord
    }

    #[test]
    fn keys_type_characters_with_shift_and_not_in_chords() {
        let mut k = HwKeys::default();
        assert_eq!(k.typed(30), Some(Out::Text("a".into())));
        assert_eq!(k.typed(2), Some(Out::Text("1".into())));
        k.key(KEY_LEFTSHIFT, true);
        assert_eq!(k.typed(30), Some(Out::Text("A".into())));
        assert_eq!(k.typed(2), Some(Out::Text("!".into())));
        k.key(KEY_LEFTSHIFT, false);
        k.key(KEY_LEFTCTRL, true);
        assert_eq!(k.typed(30), None, "Ctrl+A is a shortcut, not text");
    }

    #[test]
    fn editing_and_arrow_keys_map_to_terminal_keys() {
        let mut k = HwKeys::default();
        assert_eq!(k.typed(14), Some(Out::Backspace));
        assert_eq!(k.typed(28), Some(Out::Enter));
        assert_eq!(k.typed(57), Some(Out::Text(" ".into())));
        assert_eq!(k.typed(103), Some(Out::Named("Up")));
        assert_eq!(k.typed(15), Some(Out::Named("Tab")));
        assert_eq!(k.typed(116), None, "power is not text");
    }

    fn chord(k: &mut HwKeys, mods: &[u16], key: u16) -> Option<Shortcut> {
        for &m in mods {
            k.key(m, true);
        }
        let r = k.key(key, true);
        k.key(key, false);
        for &m in mods.iter().rev() {
            k.key(m, false);
        }
        r
    }

    #[test]
    fn chords_map_to_shortcuts() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_SPACE), Some(Shortcut::QuickSearch));
        assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_TAB), Some(Shortcut::AppSwitch));
        assert_eq!(chord(&mut k, &[KEY_RIGHTMETA], KEY_TAB), Some(Shortcut::AppSwitch));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_C), Some(Shortcut::Copy));
        assert_eq!(chord(&mut k, &[KEY_RIGHTCTRL], KEY_V), Some(Shortcut::Paste));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_X), Some(Shortcut::Cut));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_A), Some(Shortcut::SelectAll));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_Z), Some(Shortcut::Undo));
        // the screen reader's cursor: Ctrl+Alt+Right forward, Ctrl+Alt+Left back
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTALT], KEY_RIGHT), Some(Shortcut::ReaderStep(true)));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTALT], KEY_LEFT), Some(Shortcut::ReaderStep(false)));
        assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_RIGHT), None, "Alt alone is not the cursor");
    }

    #[test]
    fn plain_keys_and_other_chords_do_nothing() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[], KEY_C), None);
        assert_eq!(chord(&mut k, &[], KEY_SPACE), None);
        assert_eq!(chord(&mut k, &[], KEY_TAB), None);
                assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_C), None);
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_SPACE), Some(Shortcut::QuickSearch));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_TAB), Some(Shortcut::AppSwitch));
    }

    #[test]
    fn redo_is_ctrl_shift_z_or_ctrl_y() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTSHIFT], KEY_Z), Some(Shortcut::Redo));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_RIGHTSHIFT], KEY_Z), Some(Shortcut::Redo));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_Y), Some(Shortcut::Redo));
        assert_eq!(chord(&mut k, &[], KEY_Y), None);
        assert_eq!(Shortcut::Redo.out(), Some(Out::Named("Redo")));
    }

    #[test]
    fn escape_is_back_and_meta_alone_is_home() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[], KEY_ESC), Some(Shortcut::Back));
        assert_eq!(k.key(KEY_LEFTMETA, true), None);
        assert_eq!(k.key(KEY_LEFTMETA, false), Some(Shortcut::Home));
        k.key(KEY_LEFTMETA, true);
        k.key(KEY_TAB, true);
        k.key(KEY_TAB, false);
        assert_eq!(k.key(KEY_LEFTMETA, false), None, "a chord is not a lone press");
        k.key(KEY_LEFTSHIFT, true);
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.key(KEY_LEFTMETA, false), None);
    }

    #[test]
    fn reset_drops_stuck_modifiers() {
        let mut k = HwKeys::default();
        k.key(KEY_LEFTCTRL, true);
        k.reset();
        assert_eq!(k.key(KEY_C, true), None, "a release lost to screen-off must not leave ctrl held");
    }

    #[test]
    fn modifiers_release_and_two_sides_count_separately() {
        let mut k = HwKeys::default();
        k.key(KEY_LEFTCTRL, true);
        k.key(KEY_RIGHTCTRL, true);
        k.key(KEY_LEFTCTRL, false);
        assert_eq!(k.key(KEY_C, true), Some(Shortcut::Copy), "the right ctrl is still down");
        k.key(KEY_RIGHTCTRL, false);
        assert_eq!(k.key(KEY_C, true), None);
        k.key(KEY_LEFTCTRL, false);
        assert_eq!(k.key(KEY_C, true), None, "stray releases never underflow");
    }

    #[test]
    fn repeat_waits_then_runs_at_thirty_hertz_until_released() {
        let mut r = KeyRepeat::default();
        r.press(30, Some(Out::Text("a".into())), 0.0);
        assert_eq!(r.deadline(), Some(REPEAT_DELAY_MS));
        assert!(r.due(399.0).is_empty());
        assert_eq!(r.due(400.0).len(), 1);
        assert_eq!(r.due(400.0 + 2.0 * REPEAT_PERIOD_MS).len(), 2);
        r.release(31);
        assert!(r.deadline().is_some(), "another key's release does not cancel");
        r.release(30);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn repeat_is_cancelled_by_another_key_and_skips_escape() {
        let mut r = KeyRepeat::default();
        r.press(30, Some(Out::Text("a".into())), 0.0);
        r.press(42, None, 10.0);
        assert_eq!(r.deadline(), None);
        r.press(1, Some(Out::Named("Escape")), 0.0);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn meta_w_and_m_close_and_minimize_without_touching_ctrl_keys() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_W), Some(Shortcut::Close));
        assert_eq!(chord(&mut k, &[KEY_RIGHTMETA], KEY_M), Some(Shortcut::Minimize));
        assert_eq!(chord(&mut k, &[KEY_RIGHTMETA], KEY_COMMA), Some(Shortcut::Settings), "Cmd+, opens Settings");
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_COMMA), None, "Ctrl+, is the terminal's");
        // Ctrl+W / Ctrl+M stay plain keys: terminals use them (kill-word, Enter)
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_Q), Some(Shortcut::Close));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_H), Some(Shortcut::Minimize));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_W), None);
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_M), None);
        assert_eq!(chord(&mut k, &[], KEY_W), None);
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_W), None);
    }

    #[test]
    fn editing_shortcuts_become_named_keys() {
        assert_eq!(Shortcut::Copy.out(), Some(Out::Named("Copy")));
        assert_eq!(Shortcut::Undo.out(), Some(Out::Named("Undo")));
        assert_eq!(Shortcut::QuickSearch.out(), None);
    }

    #[test]
    fn typed_keys_follow_the_installed_layout() {
        use crate::installer::keymaps::layout_index;
        let mut k = HwKeys::default();
        k.set_layout(layout_index("is").unwrap());
        assert_eq!(k.typed(12), Some(Out::Text("ö".into())), "ö is where - is on a US board");
        k.key(KEY_LEFTSHIFT, true);
        assert_eq!(k.typed(12), Some(Out::Text("Ö".into())));
        k.key(KEY_LEFTSHIFT, false);
        assert_eq!(k.typed(53), Some(Out::Text("þ".into())));
        assert_eq!(k.typed(39), Some(Out::Text("æ".into())));
        assert_eq!(k.typed(14), Some(Out::Backspace), "named keys do not depend on the layout");
    }

    #[test]
    fn dead_acute_composes_with_the_next_letter_and_survives_key_up() {
        use crate::installer::keymaps::layout_index;
        let mut k = HwKeys::default();
        k.set_layout(layout_index("is").unwrap());
        assert_eq!(k.typed(40), None, "the acute is a dead key: it waits for a letter");
        k.key(40, false);
        assert_eq!(k.typed(30), Some(Out::Text("á".into())));
    }

    #[test]
    fn altgr_types_the_third_level_and_left_alt_stays_a_shortcut() {
        use crate::installer::keymaps::layout_index;
        let mut k = HwKeys::default();
        k.set_layout(layout_index("de").unwrap());
        k.key(KEY_RIGHTALT, true);
        assert_eq!(k.typed(18), Some(Out::Text("€".into())), "AltGr+E is the euro sign");
        k.key(KEY_RIGHTALT, false);
        k.key(KEY_LEFTALT, true);
        assert_eq!(k.typed(18), None, "Alt+E is not text");
    }

    #[test]
    fn reset_keeps_the_installed_layout() {
        use crate::installer::keymaps::layout_index;
        let mut k = HwKeys::default();
        k.set_layout(layout_index("is").unwrap());
        k.key(KEY_LEFTCTRL, true);
        k.reset();
        assert_eq!(k.typed(12), Some(Out::Text("ö".into())));
    }
}

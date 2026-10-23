//! Hardware-keyboard shortcuts: tracks the modifier keys from evdev key
//! events and turns chords into shell actions. Cmd/Meta+Space opens
//! Search, Ctrl+Space switches the input source, Alt/Meta/Ctrl+Tab the app switcher, Esc is Back, Meta alone is
//! Home (a tablet's hardware keyboard only; on the desktop a lone Cmd does nothing), and Ctrl+C/V/X/A/Z are the clipboard and undo, which text controls
//! take as `Out::Named`.

use super::keyboard::Out;
use super::sticky_keys::{Modifier, StickyKeys};
use super::desk::Tile;
use super::photos::commands::PhotoCommand;
use super::{AppId, Effect, Screen, Shell};
use crate::installer::keymaps::{Keyboard, Typed};
use bifrost_proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};

const KEY_ESC: u16 = 1;
const KEY_TAB: u16 = 15;
const KEY_W: u16 = 17;
const KEY_Q: u16 = 16;
const KEY_H: u16 = 35;
const KEY_F: u16 = 33;
const KEY_8: u16 = 9;
const KEY_MINUS: u16 = 12;
const KEY_EQUAL: u16 = 13;
const KEY_D: u16 = 32;
const KEY_GRAVE: u16 = 41;
const KEY_3: u16 = 4;
const KEY_4: u16 = 5;
const KEY_5: u16 = 6;
const KEY_BACKSPACE: u16 = 14;
const KEY_ENTER: u16 = 28;
const KEY_UP: u16 = 103;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_DOWN: u16 = 108;
const KEY_HOME: u16 = 102;
const KEY_PAGEUP: u16 = 104;
const KEY_PAGEDOWN: u16 = 109;
const KEY_END: u16 = 107;
const KEY_DELETE: u16 = 111;
const KEY_Y: u16 = 21;
const KEY_LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_Z: u16 = 44;
const KEY_X: u16 = 45;
const KEY_C: u16 = 46;
const KEY_V: u16 = 47;
const KEY_M: u16 = 50;
const KEY_COMMA: u16 = 51;
const KEY_F5: u16 = 63;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_CAPSLOCK: u16 = 58;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_SPACE: u16 = 57;
const KEY_SLASH: u16 = 53;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;
const KEY_BRIGHTNESSDOWN: u16 = 224;
const KEY_BRIGHTNESSUP: u16 = 225;
const KEY_KBDILLUMDOWN: u16 = 229;
const KEY_KBDILLUMUP: u16 = 230;

/// Term key equivalents, taken only while Term is in front. Cmd chords (Meta)
/// drive the chrome (tabs, find, zoom); plain Ctrl and Alt chords become the
/// control bytes and ESC-prefixed bytes the pty expects
/// (`bifrost_ui::terminal_keys`), so readline and job control all work:
/// Ctrl+A/E line start/end, Ctrl+C/Z interrupt/suspend, Alt+F/B word jumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermKey {
    Copy,
    /// Edit > Copy Special > Text with Colors (no key equivalent; the Edit menu runs it)
    CopyStyled,
    /// Edit > Copy Special > Without Background Colour
    CopyStyledPlain,
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
    /// Cmd+E: Use Selection for Find
    UseSelectionForFind,
    /// A Ctrl chord's byte for the pty (Ctrl+A is 1, Ctrl+E is 5, ...).
    Ctrl(u8),
    /// An Alt chord's byte after ESC (Alt+F is `ESC f`).
    Alt(u8),
    /// Alt+Left (false) / Alt+Right (true): word backward / forward (`ESC b` / `ESC f`).
    AltWord(bool),
    /// A navigation key for the pty: "Home", "End", "Delete", "PageUp" or "PageDown".
    Named(&'static str),
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
            (18, false) => TermKey::UseSelectionForFind,
            _ => return None,
        })
    }
}

/// The TermKey a plain Ctrl chord sends to the pty while Term is in front:
/// the control byte of the physical key's US legend (Shift lets shifted
/// symbols through, so Ctrl+Shift+2 is Ctrl+@). Ctrl+Backspace deletes a
/// word (`0x17`, as in other terminals) and Ctrl+Enter is CR. None for keys
/// Ctrl does not combine with (digits, Tab, function keys).
fn term_ctrl_key(code: u16, shift: bool) -> Option<TermKey> {
    if code == KEY_BACKSPACE {
        return Some(TermKey::Ctrl(0x17));
    }
    if code == KEY_ENTER {
        return Some(TermKey::Ctrl(b'\r'));
    }
    let (_, lower, upper) = TEXT_KEYS.iter().find(|k| k.0 == code)?;
    bifrost_ui::terminal_keys::control_byte(*lower)
        .or_else(|| shift.then(|| bifrost_ui::terminal_keys::control_byte(*upper)).flatten())
        .map(TermKey::Ctrl)
}

/// The TermKey a plain Alt (Left Alt, not AltGr) chord sends to the pty while
/// Term is in front: ESC plus the key, with Shift kept (Alt+Shift+F is
/// `ESC F`). Alt+Backspace is `ESC DEL` (kill word backward) and Alt+Left /
/// Alt+Right jump a word (`ESC b` / `ESC f`). None for keys Alt does not
/// combine with (digits stay digits: they fall through as before).
fn term_alt_key(code: u16, shift: bool) -> Option<TermKey> {
    if code == KEY_BACKSPACE {
        return Some(TermKey::Alt(bifrost_ui::terminal_keys::ALT_BACKSPACE));
    }
    if code == KEY_LEFT {
        return Some(TermKey::AltWord(false));
    }
    if code == KEY_RIGHT {
        return Some(TermKey::AltWord(true));
    }
    let (_, lower, upper) = TEXT_KEYS.iter().find(|k| k.0 == code)?;
    let c = if shift { *upper } else { *lower };
    // Letters, Space and the brackety symbols Alt combines with as Meta;
    // digits and most punctuation fall through untouched.
    if c.is_ascii_alphabetic() || c == ' ' || matches!(c, '[' | ']' | '\\') {
        return bifrost_ui::terminal_keys::alt_bytes(c).map(|b| TermKey::Alt(b[1]));
    }
    None
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
    /// Cmd+Q: quit the front app. An installed SDK app is sent `Quit` (it saves its state and exits), a window-server app
    /// closes every window, and a built-in window closes (`Shell::desk_quit_front`).
    Quit,
    /// Cmd+, : the app's Settings (the App menu's Settings… key equivalent)
    Settings,
    /// Ctrl+Cmd+F: full screen (zoom) for the front desktop window
    FullScreen,
    /// Ctrl+Cmd+arrows: tile the front desktop window (left, right, fill with Up, centre with Down)
    Tile(Tile),
    /// Cmd+`: bring the window behind the front one forward (the Window menu's Next Window)
    CycleWindow,
    /// Ctrl+Space: the next enabled input source (`kb_layouts::cycle`); the binary moves the hardware layout with it
    InputSource,
    /// Ctrl+Alt+Right (true) and Ctrl+Alt+Left (false): the screen reader's cursor moves to the next or previous element
    ReaderStep(bool),
    /// Ctrl+Alt+Down (true) and Ctrl+Alt+Up (false): the screen reader's next or previous rotor
    ReaderRotor(bool),
    /// Ctrl+Alt+Space: the screen reader presses the element under its cursor
    ReaderActivate,
    /// Option+Escape: speaks the selected text of the focused field (`accessibility::spoken`)
    SpeakSelection,
    /// Option+Cmd+F5: switches the features chosen for the Accessibility Shortcut (`accessibility::shortcut`)
    AccessibilityShortcut,
    /// Ctrl+Cmd+D: Look Up the selection in Dictionary (the Services menu's Look Up in Dictionary)
    LookUp,
    /// Shift+Cmd+5: the screenshot toolbar, a capture session over the screen
    Capture,
    /// Shift+Cmd+3: the whole screen is saved as a picture at once
    CaptureScreen,
    /// Shift+Cmd+4: a crosshair to drag a selection (Space turns it into a window pick); no toolbar
    CaptureSelection,
    /// a Photos menu command or its key equivalent, taken while Photos is in front (`photos::commands`)
    Photo(PhotoCommand),
    /// Option+Cmd+8: Screen Zoom on or off (`accessibility::zoom`)
    ZoomToggle,
    /// Cmd+Shift+A: opens the assistant panel (`assistant_panel`); with the assistant off, Settings > Assistant instead
    Assistant,
    /// Option+Cmd+= (true) zooms the screen in one step, Option+Cmd+- (false) zooms it out
    ZoomStep(bool),
    /// Shift+Cmd+/: opens the Help menu with its search field ready for typing
    HelpSearch,
    /// Ctrl+Cmd+Q: locks the screen (the System menu's Lock Screen)
    Lock,
    /// Option+Cmd+Esc: the Force Quit Applications window (the System menu's Force Quit…)
    ForceQuit,
    /// Shift+Cmd+Q: asks to log out (the System menu's Log Out)
    LogOut,
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
            Shortcut::Term(_) | Shortcut::QuickSearch | Shortcut::AppSwitch | Shortcut::Back | Shortcut::Home | Shortcut::Close | Shortcut::Quit | Shortcut::Minimize | Shortcut::Settings | Shortcut::FullScreen | Shortcut::Tile(_) | Shortcut::CycleWindow | Shortcut::InputSource | Shortcut::ReaderStep(_) | Shortcut::ReaderRotor(_) | Shortcut::ReaderActivate | Shortcut::SpeakSelection | Shortcut::AccessibilityShortcut | Shortcut::Capture | Shortcut::CaptureScreen | Shortcut::CaptureSelection | Shortcut::ZoomToggle | Shortcut::ZoomStep(_) | Shortcut::LookUp | Shortcut::Photo(_) | Shortcut::HelpSearch | Shortcut::Lock | Shortcut::ForceQuit | Shortcut::LogOut | Shortcut::Assistant => return None,
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
    /// Caps Lock is on (each press of the key turns it over)
    caps: bool,
    /// Meta went down and nothing else has been pressed since
    meta_alone: bool,
    /// The desktop form factor: a lone Cmd press does nothing (it is the Home key of a tablet only), see `set_desktop`
    desktop: bool,
    /// The installed keyboard layout: what a key types
    text: Keyboard,
    /// Sticky Keys: the latches, tapped on modifier presses and taken by the next non-modifier press
    sticky: StickyKeys,
    /// The non-modifier key that holds sticky modifiers, and those modifiers; they count as held until it is released
    sticky_held: Option<(u16, Vec<Modifier>)>,
}

impl HwKeys {
    /// Forgets every held modifier. Releases that arrive while the screen is
    /// off or the device is gone never reach `key`, so the counters would
    /// stick and turn plain keys into chords. The keyboard layout is kept,
    /// and Sticky Keys stays on with its latches cleared.
    pub fn reset(&mut self) {
        let layout = self.text.layout();
        let mut sticky = self.sticky.clone();
        let on = sticky.enabled();
        sticky.set_enabled(false);
        sticky.set_enabled(on);
        let desktop = self.desktop;
        *self = Self { text: Keyboard::new(layout), sticky, desktop, ..Self::default() };
    }

    /// Chooses the form factor behaviour of a lone Cmd (Super) press. On a tablet it is the Home key (the default here, so the
    /// hardware keyboard of a tablet keeps it); on the desktop it does nothing, because the Cmd key is a modifier first and a
    /// chord the front app takes (Cmd+A in a text field, Cmd+Space) must never send the windows to the Dock.
    pub fn set_desktop(&mut self, desktop: bool) {
        self.desktop = desktop;
        if desktop {
            self.meta_alone = false;
        }
    }

    /// A non-modifier key went down that never reached `key_front` (a switch, an event tap or the app in front took it) while
    /// Cmd was held: the Cmd release that follows is the end of a chord, not a lone press.
    pub fn cancel_lone_meta(&mut self) {
        self.meta_alone = false;
    }

    /// Turns Sticky Keys on or off. Turning it off releases every latch and any modifier a sticky key still holds.
    pub fn set_sticky_keys(&mut self, on: bool) {
        self.sticky_release();
        self.sticky.set_enabled(on);
    }

    /// Whether Sticky Keys is on.
    pub fn sticky_keys(&self) -> bool {
        self.sticky.enabled()
    }

    /// A non-modifier key goes down: the latched modifiers count as held until that key is released.
    fn sticky_press(&mut self, code: u16) {
        self.sticky_release();
        let mods = self.sticky.take_for_key();
        if mods.is_empty() {
            return;
        }
        for m in &mods {
            let n = self.counter(*m);
            *n = n.saturating_add(1);
        }
        self.sticky_held = Some((code, mods));
    }

    /// The key that held sticky modifiers is released (or another key replaces it): those modifiers drop.
    fn sticky_release(&mut self) {
        if let Some((_, mods)) = self.sticky_held.take() {
            for m in mods {
                let n = self.counter(m);
                *n = n.saturating_sub(1);
            }
        }
    }

    fn counter(&mut self, m: Modifier) -> &mut u8 {
        match m {
            Modifier::Shift => &mut self.shift,
            Modifier::Ctrl => &mut self.ctrl,
            Modifier::Alt => &mut self.alt,
            Modifier::Super => &mut self.meta,
        }
    }

    /// Sets the keyboard layout (an index into the installer's `LAYOUTS`) that text keys type with.
    pub fn set_layout(&mut self, layout: usize) {
        self.text.set_layout(layout);
    }

    /// True while a Shift key is held (Photos extends the selection to the last tapped photo with it).
    pub fn shift_held(&self) -> bool {
        self.shift > 0
    }

    /// True while a Ctrl key is held.
    pub fn ctrl_held(&self) -> bool {
        self.ctrl > 0
    }

    /// True while a Meta (Cmd) key is held.
    pub fn meta_held(&self) -> bool {
        self.meta > 0
    }

    /// True while an Alt key is held.
    pub fn alt_held(&self) -> bool {
        self.alt > 0
    }

    /// The modifier keys held now, as `NSEvent.modifierFlags`: every press the shell forwards to an app carries them, so a
    /// click can be a Shift-click or a Cmd-click. AltGr types characters and is not Option; Caps Lock counts while it is on.
    pub fn modifier_flags(&self) -> bifrost_proto::ModifierFlags {
        bifrost_proto::ModifierFlags::from_held(self.shift > 0, self.ctrl > 0, self.alt > self.altgr, self.meta > 0, self.caps)
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
        self.key_front(code, down, term, false)
    }

    /// `key`, with the key equivalents of the front app taken first: Term's Cmd chords while `term`, and Photos'
    /// commands while `photos`.
    pub fn key_front(&mut self, code: u16, down: bool, term: bool, photos: bool) -> Option<Shortcut> {
        // Sticky Keys: a modifier press taps its latch; a text key takes the latched modifiers until it is released
        match Modifier::of_code(code) {
            Some(m) => {
                if down {
                    self.sticky.tap_modifier(m);
                }
            }
            None if down => self.sticky_press(code),
            None => {
                if self.sticky_held.as_ref().is_some_and(|(held, _)| *held == code) {
                    self.sticky_release();
                }
            }
        }
        if code == KEY_CAPSLOCK && down {
            self.caps = !self.caps;
        }
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
                    self.meta_alone = !self.desktop && self.meta == 1 && self.ctrl + self.shift + self.alt == 0;
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
        // While Term is in front, plain Ctrl and Alt chords go to the pty as
        // control bytes (`bifrost_ui::terminal_keys`), not to editing
        // shortcuts: Ctrl+A/E move, Ctrl+C/Z signal, Alt+F/B jump words.
        // Kept for the shell: Ctrl+Space (input source), any Tab chord (the
        // app switcher) and any Ctrl+Alt chord (the screen reader).
        if term && ctrl && !meta && !alt && code != KEY_SPACE && code != KEY_TAB {
            if let Some(k) = term_ctrl_key(code, self.shift > 0) {
                return Some(Shortcut::Term(k));
            }
        }
        if term && alt && self.alt > self.altgr && !ctrl && !meta && code != KEY_TAB {
            if let Some(k) = term_alt_key(code, self.shift > 0) {
                return Some(Shortcut::Term(k));
            }
        }
        // Navigation keys with no modifiers reach the pty while Term is in
        // front (Home/End line start/end, Delete, PageUp/PageDown).
        if term && !ctrl && !alt && !meta {
            let name = match code {
                KEY_HOME => Some("Home"),
                KEY_END => Some("End"),
                KEY_DELETE => Some("Delete"),
                KEY_PAGEUP => Some("PageUp"),
                KEY_PAGEDOWN => Some("PageDown"),
                _ => None,
            };
            if let Some(name) = name {
                return Some(Shortcut::Term(TermKey::Named(name)));
            }
        }
        if photos && meta && !ctrl {
            if let Some(c) = PhotoCommand::from_chord(code, self.shift > 0, alt) {
                return Some(Shortcut::Photo(c));
            }
        }
        match code {
            KEY_SPACE if meta && !alt => Some(Shortcut::QuickSearch),
            // Ctrl+Space switches the input source (the input menu's next keyboard), as on the reference platform
            KEY_SPACE if ctrl && !alt => Some(Shortcut::InputSource),
            // Cmd+Shift+A opens the assistant panel (its chord is switched off in Settings > Assistant)
            KEY_A if meta && self.shift > 0 && !ctrl && !alt => Some(Shortcut::Assistant),
            KEY_TAB if alt || meta || ctrl => Some(Shortcut::AppSwitch),
            KEY_ESC if !ctrl && !alt && !meta => Some(Shortcut::Back),
            // Cmd on a Mac keyboard is Meta here, so it works for edits too
            KEY_Z if (ctrl || meta) && !alt => Some(if self.shift > 0 { Shortcut::Redo } else { Shortcut::Undo }),
            // Ctrl+Y redoes; Cmd+Y is not Redo (that is Shift+Cmd+Z) and stays the front app's, the Browser's Show All History
            KEY_Y if ctrl && !meta && !alt && self.shift == 0 => Some(Shortcut::Redo),
            // Ctrl+Q (quit the shell) is deliberately not bound. Ctrl+W and Ctrl+M are
            // not bound: they reach terminals as readline kill-word and Enter. Cmd/Super+W and
            // Cmd/Super+M close and minimize the front desktop window; terminals never see them,
            // and where no desktop window is open `shell` lets the key fall through unchanged.
            // Cmd+H hides the app; a window is one app here, so it minimises the window
            // Ctrl+Cmd+Q locks the screen; the plain Cmd+Q below quits the front app (Cmd+W only closes its window)
            KEY_Q if meta && ctrl && !alt && self.shift == 0 => Some(Shortcut::Lock),
            KEY_Q if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::Quit),
            // Shift+Cmd+Q asks to log out (Cmd+Q alone quits the front app)
            KEY_Q if meta && !ctrl && !alt && self.shift > 0 => Some(Shortcut::LogOut),
            KEY_W if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::Close),
            // Shift+Cmd+W closes the window of an app that keeps plain Cmd+W for its own Close Tab (`Shell::app_owns_close`)
            KEY_W if meta && !ctrl && !alt && self.shift > 0 => Some(Shortcut::Close),
            KEY_M | KEY_H if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::Minimize),
            KEY_F if meta && ctrl && !alt => Some(Shortcut::FullScreen),
            // Option+Cmd+8 turns Screen Zoom on and off; Option+Cmd+= and Option+Cmd+- zoom in and out
            KEY_8 if meta && alt && !ctrl && self.shift == 0 => Some(Shortcut::ZoomToggle),
            KEY_EQUAL | KEY_MINUS if meta && alt && !ctrl && self.shift == 0 => Some(Shortcut::ZoomStep(code == KEY_EQUAL)),
            KEY_D if meta && ctrl && !alt && self.shift == 0 => Some(Shortcut::LookUp),
            KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN if meta && ctrl && !alt && self.shift == 0 => Some(Shortcut::Tile(match code {
                KEY_LEFT => Tile::Left,
                KEY_RIGHT => Tile::Right,
                KEY_UP => Tile::Fill,
                _ => Tile::Center,
            })),
            KEY_GRAVE if meta && !ctrl && !alt && self.shift == 0 => Some(Shortcut::CycleWindow),
            KEY_LEFT | KEY_RIGHT if ctrl && alt && !meta && self.shift == 0 => Some(Shortcut::ReaderStep(code == KEY_RIGHT)),
            KEY_UP | KEY_DOWN if ctrl && alt && !meta && self.shift == 0 => Some(Shortcut::ReaderRotor(code == KEY_DOWN)),
            KEY_SPACE if ctrl && alt && !meta && self.shift == 0 => Some(Shortcut::ReaderActivate),
            KEY_ESC if alt && !ctrl && !meta && self.shift == 0 => Some(Shortcut::SpeakSelection),
            // Option+Cmd+Esc opens Force Quit Applications, from any app (the shell reads the keyboard itself)
            KEY_ESC if alt && meta && !ctrl && self.shift == 0 => Some(Shortcut::ForceQuit),
            KEY_F5 if meta && alt && !ctrl && self.shift == 0 => Some(Shortcut::AccessibilityShortcut),
            KEY_3 if meta && self.shift > 0 && !ctrl && !alt => Some(Shortcut::CaptureScreen),
            KEY_4 if meta && self.shift > 0 && !ctrl && !alt => Some(Shortcut::CaptureSelection),
            KEY_5 if meta && self.shift > 0 && !ctrl && !alt => Some(Shortcut::Capture),
            KEY_SLASH if meta && self.shift > 0 && !ctrl && !alt => Some(Shortcut::HelpSearch),
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
        // Backspace, forward Delete, Up and Down are chord keys too: Cmd+Backspace deletes in the apps, Cmd+Up goes to the
        // enclosing folder in Files and Cmd+Down opens the item (neither arrow types anything while Cmd is held).
        // Option+Backspace and Option+Delete are chords as well (they delete a word); AltGr alone types.
        let option_delete = matches!(code, KEY_BACKSPACE | KEY_DELETE) && self.alt > self.altgr;
        if self.ctrl == 0 && self.meta == 0 && !option_delete || (!matches!(code, KEY_BACKSPACE | KEY_DELETE | KEY_UP | KEY_DOWN) && !TEXT_KEYS.iter().any(|k| k.0 == code)) {
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

    /// The CHORD_* bits of a navigation key (an arrow, Home, End, Page Up, Page Down or forward Delete) that goes to a text
    /// editor in front (the manifest's `edit_keys`, or an SDK app with a text field focused): every modifier held is named, so
    /// the editor can extend the selection (Shift), move by word (Alt or Ctrl) and by line (Cmd). The keys that are not
    /// arrows go even with no modifier, as `Some(0)`, since the apps never got them otherwise; an arrow with none held is
    /// `None` (it types as before). None for any other key.
    pub fn nav_mods(&self, code: u16) -> Option<u32> {
        let ends = matches!(code, KEY_HOME | KEY_END | KEY_PAGEUP | KEY_PAGEDOWN | KEY_DELETE);
        if !ends && !matches!(code, KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN) {
            return None;
        }
        let held = [(self.ctrl > 0, CHORD_CTRL), (self.shift > 0, CHORD_SHIFT), (self.alt > self.altgr, CHORD_ALT), (self.meta > 0, CHORD_META)];
        let mods = held.iter().filter(|(on, _)| *on).fold(0u32, |all, (_, bit)| all | *bit);
        (mods != 0 || ends).then_some(mods)
    }

    /// The movement and delete keys of a shell text field that `typed` leaves out, because a modifier is held or because they type
    /// nothing: Home and End; Option or Ctrl with Left and Right (by word); Cmd with Left and Right (the ends); Shift with any of
    /// them (extend the selection) and with the plain arrows; forward Delete; Option or Ctrl with Backspace and Delete (a word);
    /// Cmd+Backspace (everything before the caret). The field reads them as `Out::Named`. None for any other key.
    pub fn field_edit(&self, code: u16) -> Option<Out> {
        let (shift, word, line) = (self.shift > 0, self.ctrl > 0 || self.alt > self.altgr, self.meta > 0);
        let pick = |plain: &'static str, extend: &'static str| Out::Named(if shift { extend } else { plain });
        Some(match code {
            KEY_HOME => pick("Home", "ShiftHome"),
            KEY_END => pick("End", "ShiftEnd"),
            KEY_LEFT if line => pick("Home", "ShiftHome"),
            KEY_RIGHT if line => pick("End", "ShiftEnd"),
            KEY_LEFT if word => pick("WordLeft", "ShiftWordLeft"),
            KEY_RIGHT if word => pick("WordRight", "ShiftWordRight"),
            KEY_LEFT if shift => Out::Named("ShiftLeft"),
            KEY_RIGHT if shift => Out::Named("ShiftRight"),
            KEY_DELETE if word => Out::Named("DeleteWordForward"),
            KEY_DELETE => Out::Named("DeleteForward"),
            KEY_BACKSPACE if line => Out::Named("DeleteLine"),
            KEY_BACKSPACE if word => Out::Named("DeleteWord"),
            _ => return None,
        })
    }

    /// What a key-down types into a text control or the terminal: characters, Backspace, Enter
    /// and the terminal's named keys. None while Ctrl, Alt or Meta is held (those are shortcuts).
    pub fn typed(&mut self, code: u16) -> Option<Out> {
        // Left Alt, Ctrl and Meta are shortcuts; AltGr types the layout's third level
        if self.ctrl > 0 || self.meta > 0 || self.alt > self.altgr {
            return None;
        }
        // F1 to F12 type their standard keys only when Settings asks for it
        if let Some(name) = super::kb_input::function_key(code) {
            return super::kb_input::get().function_standard.then_some(Out::Named(name));
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

/// Key auto-repeat: after the delay (Settings > Keyboard > Key Repeat; 400 ms and 30 Hz by default) held, the last
/// typed key repeats. A pure state machine (the evdev decoder drops the kernel's own repeats), cancelled by a
/// release or another key.
pub struct KeyRepeat {
    held: Option<(u16, Out, f64)>,
    delay_ms: f64,
    period_ms: f64,
}

pub const REPEAT_DELAY_MS: f64 = 400.0;
const REPEAT_PERIOD_MS: f64 = 1000.0 / 30.0;

impl Default for KeyRepeat {
    fn default() -> Self {
        KeyRepeat { held: None, delay_ms: REPEAT_DELAY_MS, period_ms: REPEAT_PERIOD_MS }
    }
}

impl KeyRepeat {
    /// The timing Settings chose: the hold time in ms and the repeats per second. Applies from the next press.
    pub fn set_timing(&mut self, delay_ms: f64, rate: u32) {
        self.delay_ms = delay_ms;
        self.period_ms = 1000.0 / f64::from(rate.max(1));
    }

    /// A key went down; `out` is what it types (None for keys that do not repeat, which still cancel).
    pub fn press(&mut self, code: u16, out: Option<Out>, t: f64) {
        self.held = out.filter(|o| *o != Out::Named("Escape")).map(|o| (code, o, t + self.delay_ms));
    }

    /// Ends the repeat in progress, whatever key it is (a card alert took the held key).
    pub fn cancel(&mut self) {
        self.held = None;
    }

    pub fn release(&mut self, code: u16) {
        if self.held.as_ref().is_some_and(|h| h.0 == code) {
            self.held = None;
        }
    }

    pub fn deadline(&self) -> Option<f64> {
        self.held.as_ref().map(|h| h.2)
    }

    /// A held letter that has accents, once its delay is up, opens its accent list (`popup`: the caller can open one)
    /// instead of repeating. Returns the letter and ends the hold, so `due` repeats nothing for it.
    pub fn accent_hold(&mut self, t: f64, popup: bool) -> Option<char> {
        if !popup {
            return None;
        }
        let (_, out, next) = self.held.as_ref()?;
        if t < *next {
            return None;
        }
        let Out::Text(text) = out else { return None };
        let mut chars = text.chars();
        let letter = match (chars.next(), chars.next()) {
            (Some(c), None) => c,
            _ => return None,
        };
        if bifrost_ui::compose_method::candidates_for(letter).is_empty() {
            return None;
        }
        self.held = None;
        Some(letter)
    }

    /// The repeats due by `t` (at most a few, so a stalled loop does not flood a field).
    pub fn due(&mut self, t: f64) -> Vec<Out> {
        let period = self.period_ms;
        let Some((_, out, next)) = self.held.as_mut() else { return vec![] };
        let mut v = vec![];
        while t >= *next && v.len() < 4 {
            v.push(out.clone());
            *next += period;
        }
        if t >= *next {
            *next = t + period;
        }
        v
    }
}

impl Shell {
    /// A hardware media key (play, pause, next, previous, stop, fast forward, rewind). It goes to the owner of the
    /// Now Playing session when that owner enabled the command, and works with the screen off. True when sent.
    pub fn hw_media_key(&mut self, code: u16, down: bool, t: f64) -> bool {
        if !down {
            return false;
        }
        let used = super::services::now_playing::media_key(code);
        if used {
            self.last_input = t;
        }
        used
    }

    /// The keyboard backlight the machine has: its level 0..=1 and how many steps the hardware offers (`steps` 0 =
    /// none). A sync at start; no HUD, no effect.
    pub fn set_keyboard_backlight(&mut self, level: f32, steps: u32) {
        self.kbd_backlight = (steps > 0).then(|| (level.clamp(0.0, 1.0), steps));
    }

    /// The keyboard backlight level 0..=1, None on a machine without one.
    pub fn keyboard_backlight(&self) -> Option<f32> {
        self.kbd_backlight.map(|(l, _)| l)
    }

    /// A brightness key (screen brightness down/up, keyboard backlight down/up): one step, the same HUD as the
    /// volume keys when the screen shows something other than Control Center (whose slider moves instead), and the
    /// effect for the device. Works on every screen and never wakes it (with the screen off the press is taken and ignored). None for any other key; a key release or a
    /// keyboard backlight key on a machine without one is taken and does nothing.
    pub fn hw_brightness_key(&mut self, code: u16, down: bool, t: f64) -> Option<Vec<Effect>> {
        use bifrost_ui::osd::Level;
        let (kind, up) = match code {
            KEY_BRIGHTNESSDOWN => (Level::Brightness, false),
            KEY_BRIGHTNESSUP => (Level::Brightness, true),
            KEY_KBDILLUMDOWN => (Level::KeyboardBrightness, false),
            KEY_KBDILLUMUP => (Level::KeyboardBrightness, true),
            _ => return None,
        };
        // a dark screen keeps its backlight off: the press is taken and does nothing
        if !down || !self.screen_on {
            return Some(vec![]);
        }
        let (now, steps, floor) = match kind {
            Level::KeyboardBrightness => {
                let (level, steps) = self.kbd_backlight?;
                (level, steps, 0)
            }
            // the panel never goes black from a key: the lowest step stays lit
            _ => (self.info.brightness, 16, 1),
        };
        let at = (now * steps as f32).round() as i64;
        let level = ((at + if up { 1 } else { -1 }).clamp(floor, steps as i64) as f32) / steps as f32;
        let effect = match kind {
            Level::KeyboardBrightness => {
                self.kbd_backlight = Some((level, steps));
                Effect::KeyboardBacklight(level)
            }
            _ => {
                self.info.brightness = level;
                Effect::Brightness(level)
            }
        };
        if self.screen_on && !self.cc_active() {
            self.hud.show_level(kind, level, t);
        }
        self.dirty = true;
        Some(vec![effect])
    }

    /// A key from a physical keyboard that the app on screen handles itself (Deildu, Files); true if taken.
    /// Power, volume and wake stay with the shell.
    pub fn hw_key(&mut self, code: u16, down: bool, t: f64) -> bool {
        const KEEP: [u16; 4] = [114, 115, 116, 143];
        if !down || !self.screen_on || self.anim.is_some() || KEEP.contains(&code) || self.card_alert_up() {
            return false;
        }
        // Space in an open capture session turns the selection into a window pick and back
        if code == KEY_SPACE {
            if let Some(c) = self.capture.as_mut().filter(|c| c.pending.is_none()) {
                c.toggle_pick();
                self.dirty = true;
                return true;
            }
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

    /// Overview and the Apps grid from the keyboard while the shell has the keys: F3 and Ctrl+Up open or close Overview, F4 the Apps grid (the window server binds the
    /// same chords for the times an app holds the keyboard, `wm/src/keybindings.rs`), and while Overview or the Apps grid is open
    /// its own keys (Esc, Return, Tab, the arrows) are taken here and nothing else is typed. True when the key was used. Modifier,
    /// volume and brightness keys always pass.
    pub fn hw_overview_key(&mut self, code: u16, down: bool, mods: super::overview::KeyMods, alt: bool, t: f64) -> bool {
        const PASS: [u16; 14] = [29, 42, 54, 56, 97, 100, 125, 126, 114, 115, 116, 143, 224, 225];
        if !down || !self.screen_on || alt || PASS.contains(&code) {
            return false;
        }
        // an alert holds the keyboard and Secure Keyboard Entry keeps the shell's keys out of the Terminal: neither is for Overview
        if self.card_alert_up() || self.secure_entry_up() {
            return false;
        }
        // the Apps grid takes only its navigation keys: letters go on to `hw_type`, which types them into its search field
        let grid_key = self.overview.grid.visible() && matches!(code, KEY_ESC | KEY_BACKSPACE | KEY_ENTER | KEY_UP | KEY_LEFT | KEY_RIGHT | KEY_DOWN);
        let open = self.overview.open || grid_key;
        let bound = !mods.cmd && !mods.shift && (matches!(code, super::overview::KEY_F3 | super::overview::KEY_F4) && !mods.ctrl || code == super::overview::KEY_UP && mods.ctrl);
        if !(open || bound) {
            return false;
        }
        self.overview_key(code, down, mods, t)
    }

    /// True while a layer of the shell itself takes the typing (`hw_type` serves it before any app): the sheets for
    /// authentication, volumes, packages and network login (and the access, USB and firewall prompts, which swallow keys),
    /// the Apps grid, an open or save panel, the compose sheet, the character viewer, the help viewer, Search and the
    /// assistant. The app behind such a layer must not get the keys as chords, and the layer's own field gets them.
    pub fn shell_takes_typing(&self) -> bool {
        self.auth_showing()
            || self.volume_showing()
            || self.package_showing()
            || self.net_auth_showing()
            || self.access_showing()
            || self.usb_showing()
            || self.fw_showing()
            || self.overview.grid.visible()
            || self.doc_panel_open()
            || self.compose_showing()
            || self.char_viewer.is_some()
            || self.quicksearch.is_shown()
            || self.assistant.is_shown()
            || self.help_viewer.is_some()
            || self.card_alert_up()
            // the layers `modal_up` lists that hold the pointer: the keyboard viewer is left out, as it is a palette whose touches
            // reach the desktop and whose purpose is to show the keys that go on to the app behind it
            || self.power_menu.is_some()
            || self.pick_sheet.is_some()
            || self.photo_pick_sheet.is_some()
            || self.trust_sheet.is_some()
            || self.a11y_panel.is_some()
            || self.switch_panel_shown()
            || self.print_showing()
    }

    /// True while a password is being entered: an app's password sheet, an encrypted volume's, a network login, a secure field
    /// of Settings (the Wi-Fi password), or the Terminal in front with Secure Keyboard Entry on. Nothing else may see these keys.
    pub fn secure_entry_up(&self) -> bool {
        self.auth_showing()
            || self.volume_showing()
            || self.net_auth_showing()
            || (self.screen == Screen::App(AppId::Terminal) && self.terminal_ui.secure)
            || (self.screen == Screen::App(AppId::Settings) && self.settings.as_ref().is_some_and(|s| s.secret_field_focused()))
    }

    /// Escape closes the power menu, the picker, photo and trust sheets and the print sheet: layers `shell_takes_typing` holds
    /// the keyboard for, so the key must not reach the app behind them. True when one was up.
    fn modal_escape(&mut self) -> bool {
        let closed = if self.power_menu.take().is_some() {
            true
        } else if let Some(sheet) = self.pick_sheet.take() {
            self.pick_sheet = sheet.tapped(&self.ui, super::contact_pick_sheet::Tap::Cancel);
            true
        } else if let Some(sheet) = self.photo_pick_sheet.take() {
            sheet.cancel();
            true
        } else if self.trust_sheet.is_some() {
            use super::trust_sheet::Hit;
            if self.trust_sheet.as_mut().is_some_and(|s| s.press(Hit::Cancel) || s.press(Hit::Done)) {
                self.trust_sheet = None;
            }
            true
        } else {
            self.print_sheet_escape()
        };
        if closed {
            self.drawn = super::Drawn::Nothing;
            self.dirty = true;
        }
        closed
    }

    #[cfg(feature = "desktop-wayland")]
    fn print_sheet_escape(&mut self) -> bool {
        self.print_sheet.take().is_some()
    }

    #[cfg(not(feature = "desktop-wayland"))]
    fn print_sheet_escape(&mut self) -> bool {
        false
    }

    #[cfg(feature = "desktop-wayland")]
    fn doc_panel_open(&self) -> bool {
        self.doc_panel.is_some()
    }

    #[cfg(not(feature = "desktop-wayland"))]
    fn doc_panel_open(&self) -> bool {
        false
    }

    /// Types a physical-keyboard key into the terminal, a hosted app's text field or the focused
    /// native text field of the app on screen (Search, Browser, Settings, Files, Deildu); elsewhere it is ignored. The effects are the field's (a Wi-Fi join, say).
    /// A Ctrl or Cmd chord the shell did not take goes to the external app in front. False when
    /// the screen is not an external app, so the key falls through as before.
    /// Key equivalents of the shell's own windows that `hw_chord` (which serves the SDK app in front) never sees: Cmd+[ goes
    /// Back in Settings, as the back control does (Golden Gate's System Settings binds Cmd+[ to Back). True when the chord
    /// was used; false leaves it to `hw_chord`.
    pub fn hw_builtin_chord(&mut self, code: u16, mods: u32, t: f64) -> bool {
        const KEY_LEFTBRACE: u16 = 26;
        if !self.screen_on || self.anim.is_some() || self.shell_takes_typing() || self.card_alert_up() {
            return false;
        }
        if code == KEY_LEFTBRACE && mods == CHORD_META && self.screen == Screen::App(AppId::Settings) {
            let stepped = self.settings.as_mut().is_some_and(|s| s.back_page(t));
            if stepped {
                self.last_input = t;
                self.dirty = true;
            }
            return stepped;
        }
        false
    }

    pub fn hw_chord(&mut self, code: u16, mods: u32) -> bool {
        // Option+Cmd+D (KEY_D): Automatically Hide and Show for the dock
        if code == 32 && mods == (CHORD_META | CHORD_ALT) && self.dock_toggle_autohide() {
            return true;
        }
        // a shell layer that takes the typing keeps its keys from the app behind it
        if self.shell_takes_typing() {
            return false;
        }
        if !self.screen_on || !matches!(self.screen, Screen::App(AppId::External(_))) {
            return false;
        }
        self.ext_chord(u32::from(code), mods)
    }

    /// A navigation key with its modifiers (`HwKeys::nav_mods`) for a text editor in front: the app that declares `edit_keys`,
    /// or any SDK app with a text field focused (it reported the field's caret), gets it as a chord on the evdev key. False
    /// when the app in front is not one (or no app is), so the key goes on as an ordinary arrow.
    pub fn hw_nav(&mut self, code: u16, mods: u32) -> bool {
        if !self.screen_on || self.shell_takes_typing() {
            return false;
        }
        self.ext_takes_edit_keys() && self.ext_chord(u32::from(code), mods)
    }

    pub fn hw_type(&mut self, out: Out, t: f64) -> Vec<Effect> {
        if !self.screen_on {
            return vec![];
        }
        // the password sheet of an app's local authentication takes the typing before anything else
        if self.auth_showing() {
            self.auth_type(out, t);
            return vec![];
        }
        // the unlock sheet for an encrypted volume takes the typing too
        if self.volume_showing() {
            self.volume_type(out, t);
            return vec![];
        }
        // the package install sheet takes the typing (Escape closes it) and never installs on Return
        if self.package_showing() {
            self.package_type(out, t);
            return vec![];
        }
        // the network login sheet takes the typing for its fields
        if self.net_auth_showing() {
            self.net_auth_type(out, t);
            return vec![];
        }
        // the account access sheet, the USB prompt and the firewall question take touches only: typing waits for the answer
        if self.access_showing() || self.usb_showing() || self.fw_showing() {
            return vec![];
        }
        // a system alert holds the keyboard: Return presses its bold button when a key may (never a permission), every other key is used up (the app behind
        // it gets none)
        if self.card_alert_up() {
            return if matches!(out, Out::Enter) { self.card_alert_key(false, t) } else { vec![] };
        }
        // the power menu, the picker, photo and trust sheets, the print sheet and the Accessibility Shortcut panel have no field: typing is used up
        // (Escape closes them in `hw_shortcut`), not sent to the app behind
        if self.power_menu.is_some() || self.pick_sheet.is_some() || self.photo_pick_sheet.is_some() || self.trust_sheet.is_some() || self.print_showing() || self.a11y_panel.is_some() {
            return vec![];
        }
        // the Apps grid is modal: typing goes to its search field, its own keys move and open, nothing reaches the app
        if self.overview_typed(&out, t) {
            return vec![];
        }
        // Keyboard Navigation off: Tab does not move focus, though a terminal still gets it for completion
        if matches!(out, Out::Named("Tab")) && !super::kb_input::get().tab_navigation && !matches!(self.screen, Screen::App(AppId::Terminal | AppId::External(_))) {
            return vec![];
        }
        // with a panel open, typing goes to its name or search field and nowhere else: text, Backspace, the caret keys
        // (Left, Right, Shift and Option with them, Home, End, Delete) and Select All, Cut, Copy and Paste
        #[cfg(feature = "desktop-wayland")]
        if self.doc_panel.is_some() {
            self.doc_edit(&out);
            return vec![];
        }
        // the compose sheet takes the typing: it edits the post text
        #[cfg(feature = "desktop-wayland")]
        if self.compose.is_some() {
            self.compose_type(out);
            return vec![];
        }
        // the character viewer takes the typing for its search, unless a Search field or the assistant panel is open
        if self.char_viewer.is_some() && !self.quicksearch.is_shown() && !self.assistant.is_shown() {
            return self.char_viewer_key(out);
        }
        // any other sheet or panel on the overlay (id 15) holds the keyboard: typing is used up, not sent to the app behind
        if self.modal_layer.is_some() {
            return vec![];
        }
        // the login window opens on a tick; the shell may not have drawn since the lock, so open it for this key
        if self.screen == Screen::Lock {
            self.login_ensure(t);
        }
        let fx = match self.screen {
            // an open Search takes the typing, over whatever screen it was opened on
            // an open assistant panel takes the typing too, as Search does
            _ if self.assistant.is_shown() => self.assistant_key(out, t),
            _ if self.quicksearch.is_shown() => self.compose_search_key(out, t),
            Screen::App(AppId::Terminal | AppId::External(_)) => {
                self.type_out(out, t);
                vec![]
            }
            Screen::Lock if self.login_open() || self.login_backend.is_some() => {
                self.login_show(t);
                self.login_key(&out, t)
            }
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
            AppId::AppStore => {
                if let Some(c) = self.store.as_mut() {
                    c.key(out);
                }
            }
            AppId::Photos => {
                if let Some(p) = self.photos.as_mut() {
                    p.key(&self.ui, out, t);
                }
            }
            _ => {}
        }
        vec![]
    }

    /// Esc: closes the topmost thing (Control Center, Search, the switcher), else steps back in
    /// the app on screen, else leaves it. The terminal and hosted apps get the Escape key itself.
    fn hw_back(&mut self, t: f64) -> Vec<Effect> {
        // the open Quick Look panel is the topmost thing: Escape closes it (and tells the app that opened it) before
        // anything under it hears the key
        if self.quick_look_key(1, true).is_some() {
            return vec![];
        }
        // so is a card of the System menu (Force Quit, About, Log Out)
        #[cfg(feature = "desktop-wayland")]
        if let Some(fx) = self.utility_key(1, true, t) {
            return fx;
        }
        #[cfg(feature = "desktop-wayland")]
        if let Some(fx) = self.desktop_menu_key(1, t) {
            return fx;
        }
        // Overview or the Apps grid is a modal layer over the windows: Escape closes it before anything under it
        if self.overview_escape(t) {
            return vec![];
        }
        if self.cc_active() {
            self.cc.hide();
            self.dirty = true;
            return vec![];
        }
        if self.center_active() {
            self.notify.center.hide();
            self.dirty = true;
            return vec![];
        }
        if self.assistant.is_shown() {
            self.assistant_hide();
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
                    AppId::Photos => match self.photos.as_mut() {
                        Some(p) => p.hw_back(&self.ui, t),
                        None => false,
                    },
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

    /// True while a text field of an app on screen takes typing, so a text replacement may be typed (not in a terminal).
    pub fn replacements_apply(&self) -> bool {
        self.screen_on && !self.is_locked() && matches!(self.screen, Screen::App(app) if !matches!(app, AppId::Terminal | AppId::External(_)))
    }

    /// Runs a shortcut. Search, the switcher, Back and Home are the shell's own; the terminal
    /// takes Paste, and the editing shortcuts go to the focused native text field.
    /// Ctrl+W / Ctrl+M act only on the desktop. Elsewhere `shell` drops the
    /// shortcut so the key falls through exactly as it did before.
    /// True while Term is the front app and takes the Cmd key equivalents.
    pub fn term_front(&self) -> bool {
        self.screen_on && self.screen == Screen::App(AppId::Terminal) && !self.is_locked()
    }
    /// True while Photos is the front app: its menus show, and its Cmd key equivalents are taken.
    pub fn photos_front(&self) -> bool {
        self.screen_on && self.screen == Screen::App(AppId::Photos) && !self.is_locked()
    }
    /// The modifier keys held now (`HwKeys::modifier_flags`): presses forwarded to external apps carry them. The shell calls
    /// it on every key event.
    pub fn set_key_mods(&mut self, mods: bifrost_proto::ModifierFlags) {
        self.key_mods = mods;
    }
    /// Tells Photos whether Shift is held, so a tap on the grid extends the selection. The shell calls it on every key event.
    pub fn set_photos_shift(&mut self, held: bool) {
        if let Some(p) = self.photos.as_mut() {
            p.set_shift(held);
        }
    }
    pub fn hw_window_shortcut(&self, sc: Shortcut) -> bool {
        cfg!(feature = "desktop-wayland") && matches!(sc, Shortcut::Close | Shortcut::Minimize | Shortcut::Quit) && self.desk_active() && !self.is_locked()
    }
    /// Whether the shell runs `sc` itself. Cmd+W, Cmd+M and Cmd+Q act on the desktop window, so they run only with a desktop
    /// window to act on and when the app in front did not take the chord (`app_chord`, from `app_owns_chord`) through an
    /// enabled menu item of its own; every other shortcut is the shell's always.
    pub fn shortcut_is_shells(&self, sc: Shortcut, app_chord: bool) -> bool {
        !matches!(sc, Shortcut::Close | Shortcut::Minimize | Shortcut::Quit) || (self.hw_window_shortcut(sc) && !app_chord)
    }
    /// True when the SDK app in front published an enabled menu item with the key equivalent `code` + `mods` (`CHORD_*`). Key
    /// equivalents go to the focused app first: the binary leaves Cmd+W, Cmd+M and Cmd+Q to `hw_chord` (which sends the app its
    /// menu action) when this is true, and closes, minimizes or quits itself only when the app does not take the chord.
    pub fn app_owns_chord(&self, code: u16, mods: u32) -> bool {
        matches!(self.screen, Screen::App(AppId::External(_)))
            && self.ext_fg.and_then(|n| self.ext.get(n as usize)).and_then(Option::as_ref).is_some_and(|h| super::desktop_bar::chord_in_menus(&h.app_menus, code, mods))
    }
    /// True when the app in front keeps plain Cmd+W (a tabbed app's Close Tab), so the File menu moves Close Window to Shift+Cmd+W.
    pub fn app_owns_close(&self) -> bool {
        self.app_owns_chord(KEY_W, CHORD_META)
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
        // a card of the System menu takes Esc, Return and (Force Quit) the arrows
        #[cfg(feature = "desktop-wayland")]
        if let Some(fx) = self.utility_key(code, down, t) {
            return Some(fx);
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
        // a card alert holds the keyboard: every shortcut but Escape (which answers it) and the lock is used up, not run on
        // the app that raised the alert or sent to the app behind it
        if self.card_alert_up() && !matches!(sc, Shortcut::Back | Shortcut::Lock) {
            return vec![];
        }
        // Secure Keyboard Entry: while the Terminal is in front, only its own keys run; the shell's shortcuts
        // (switcher, search, input source, Home, Back, window keys) never see them. Escape still answers a card alert.
        if self.screen == Screen::App(AppId::Terminal) && self.terminal_ui.secure && !(matches!(sc, Shortcut::Back) && self.card_alert_up()) && !matches!(sc, Shortcut::Term(_) | Shortcut::Paste | Shortcut::Lock | Shortcut::ForceQuit) {
            return vec![];
        }
        let fx = match sc {
            Shortcut::QuickSearch => {
                self.open_quicksearch_hw(t);
                vec![]
            }
            // the binary moves the hardware layout to the new source before the next key is typed; the menu bar
            // shows the new label
            Shortcut::InputSource => {
                let source = super::kb_layouts::cycle();
                self.input_layout_selected(source);
                self.show_input_source(source, t);
                vec![]
            }
            Shortcut::Assistant => self.assistant_hotkey(t),
            // Ctrl+Cmd+Q: the same lock as the System menu's Lock Screen row
            Shortcut::Lock => self.lock_screen(),
            // Option+Cmd+Esc and Shift+Cmd+Q: the cards of the System menu's Force Quit… and Log Out rows
            #[cfg(feature = "desktop-wayland")]
            Shortcut::ForceQuit if self.desk_active() && !self.is_locked() => {
                self.utility_open(super::utility_window::Kind::ForceQuit);
                vec![]
            }
            #[cfg(feature = "desktop-wayland")]
            Shortcut::LogOut if self.desk_active() && !self.is_locked() => {
                self.utility_open(super::utility_window::Kind::LogOut);
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
            Shortcut::Quit if self.hw_window_shortcut(sc) => self.desk_quit_front(),
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
            Shortcut::ReaderStep(forward) if self.reader_active() || self.switch_scan_on() => self.reader_step(forward, t),
            Shortcut::ReaderRotor(forward) if self.screen_reader_on() => self.reader_rotor(forward, t),
            Shortcut::ReaderActivate if self.reader_active() || self.switch_scan_on() => self.reader_activate(t),
            Shortcut::SpeakSelection => {
                self.speak_selection();
                vec![]
            }
            Shortcut::AccessibilityShortcut => {
                self.accessibility_shortcut();
                vec![]
            }
            Shortcut::ZoomToggle if !self.is_locked() => {
                self.screen_zoom.toggle();
                self.force_full_redraw();
                vec![]
            }
            Shortcut::ZoomStep(bigger) if !self.is_locked() => {
                self.screen_zoom.step(bigger);
                self.force_full_redraw();
                vec![]
            }
            Shortcut::FullScreen if self.desk_active() && !self.is_locked() => {
                // Ctrl+Cmd+F: the front window takes the whole screen in its own space, or leaves it
                if let Some((app, full)) = self.desk.as_ref().and_then(|d| d.front().map(|w| (w.app, w.full))) {
                    self.desk_set_full_screen(app, !full);
                }
                vec![]
            }
            // Shift+Cmd+5 opens the screenshot toolbar; Escape closes it before it goes Back
            // Shift+Cmd+/ opens the Help menu with its search field focused
            #[cfg(feature = "desktop-wayland")]
            Shortcut::HelpSearch if self.desk_active() && !self.is_locked() => {
                self.help_search_open();
                vec![]
            }
            Shortcut::Capture if !self.is_locked() => {
                self.capture_open(super::capture::Session::with_prefs(self.capture_prefs));
                vec![]
            }
            // Shift+Cmd+3 saves the whole screen at once; Shift+Cmd+4 opens the crosshair (Space picks a window)
            Shortcut::CaptureScreen if !self.is_locked() => {
                self.capture_screen_now(t);
                vec![]
            }
            Shortcut::CaptureSelection if !self.is_locked() => {
                self.capture_open(super::capture::Session::crosshair(self.capture_prefs));
                vec![]
            }
            // an alert is modal over everything and holds the keyboard: Escape answers it as its cancel button would
            Shortcut::Back if self.card_alert_up() => self.card_alert_key(true, t),
            // Escape closes the other layers that hold the keyboard before it reaches the app behind
            Shortcut::Back if self.modal_escape() => vec![],
            Shortcut::Back if self.capture.is_some() => {
                self.capture = None;
                vec![]
            }
            Shortcut::LookUp if self.desk_active() && !self.is_locked() => self.run_service(bifrost_ui::services_menu::Service::LookUp, t),
            // Escape closes the Look Up popover first, then the Accessibility Shortcut panel, then the character viewer, before it goes Back
            Shortcut::Back if self.visual_search_escape() => vec![],
            Shortcut::Back if self.look_up_escape() => vec![],
            // Escape hides the Accessibility Shortcut panel, then the character viewer, before it goes Back
            Shortcut::Back if self.a11y_panel_escape() => vec![],
            // Escape hides the Switch Control panel before it goes Back
            Shortcut::Back if self.switch_panel_escape() => vec![],
            // Escape hides the character viewer first, before it goes Back
            Shortcut::Back if self.char_viewer_escape() => vec![],
            Shortcut::Back => self.hw_back(t),
            // a lone Cmd is not Home on the desktop (a stray release must never send the windows to the Dock)
            Shortcut::Home if self.form.is_desktop() => vec![],
            Shortcut::Home => self.assistive_action(super::assistive_touch::Action::Home, t),
            Shortcut::Term(k) => self.terminal_hw_key(k, t),
            Shortcut::Photo(c) => self.photo_command(c, t),
            Shortcut::Paste if self.screen == Screen::App(AppId::Terminal) => {
                self.type_out(Out::Named("Paste"), t);
                vec![]
            }
            // an SDK app takes the editing shortcuts too (Cmd+A, C, X, V, Z): `hw_type` hands them to its focused view as the
            // standard commands, once an open sheet or panel of the shell has had its say
            sc if !in_terminal || matches!(self.screen, Screen::App(AppId::External(_))) => sc.out().map_or(vec![], |out| self.hw_type(out, t)),
            _ => vec![],
        };
        self.dirty = true;
        fx
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// QA 0.10.20: Cmd+[ did nothing in Settings; it goes Back like the back control, and only Cmd+[ does.
    #[test]
    fn cmd_left_bracket_goes_back_in_settings() {
        use crate::shell::tests::{settle, shell, unlock};
        let mut sh = shell();
        unlock(&mut sh);
        sh.open(AppId::Settings, 1000.0);
        settle(&mut sh, 1000.0);
        let (ui, st) = (&sh.ui, sh.settings.as_mut().expect("Settings is open"));
        // the single column, where About is pushed on the root (the split view replaces the detail pane instead)
        st.allow_split(ui, false);
        st.open_page(ui, "About", 3000.0);
        st.tick(ui, 5000.0);
        let depth = st.stack().len();
        assert!(depth >= 2, "About is a page above the root: {:?}", st.stack());
        assert!(!sh.hw_builtin_chord(26, CHORD_META | CHORD_SHIFT, 6000.0), "Shift+Cmd+[ is not Back");
        assert!(!sh.hw_builtin_chord(27, CHORD_META, 6000.0), "Cmd+] is not Back");
        assert!(sh.hw_builtin_chord(26, CHORD_META, 6000.0), "Cmd+[ goes Back");
        assert_eq!(sh.settings.as_ref().unwrap().stack().len(), depth - 1);
    }

    #[test]
    fn term_takes_cmd_chords_only_while_in_front_and_ctrl_reaches_the_pty() {
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
        // Plain Ctrl chords are Term's too now: they carry the control byte
        // for the pty (readline, job control), never an editing shortcut.
        // A=1 (line start), E=5 (line end), C=3 (interrupt), Z=26 (suspend).
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], 30), Some(Shortcut::Term(TermKey::Ctrl(1))), "Ctrl+A");
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], 18), Some(Shortcut::Term(TermKey::Ctrl(5))), "Ctrl+E");
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_C), Some(Shortcut::Term(TermKey::Ctrl(3))), "Ctrl+C interrupts");
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_A), Some(Shortcut::Term(TermKey::Ctrl(1))), "Ctrl+A, not SelectAll");
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_X), Some(Shortcut::Term(TermKey::Ctrl(24))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_V), Some(Shortcut::Term(TermKey::Ctrl(22))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_Z), Some(Shortcut::Term(TermKey::Ctrl(26))), "Ctrl+Z suspends, not Undo");
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL, KEY_LEFTSHIFT], KEY_Z), Some(Shortcut::Term(TermKey::Ctrl(26))), "Ctrl+Shift+Z is the same byte");
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_LEFTSHIFT), None, "bare Shift is not a chord");
        // The shell keeps its own Ctrl chords: input source, the app switcher.
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_SPACE), Some(Shortcut::InputSource));
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], KEY_TAB), Some(Shortcut::AppSwitch));
        assert_eq!(in_term(&mut k, &[KEY_LEFTCTRL], 2), None, "Ctrl+1 is not a control byte");
        // Plain Alt chords send ESC plus the key (word jumps); AltGr still types.
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT], 33), Some(Shortcut::Term(TermKey::Alt(b'f'))), "Alt+F forward word");
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT], 30), Some(Shortcut::Term(TermKey::Alt(b'a'))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT, KEY_LEFTSHIFT], 33), Some(Shortcut::Term(TermKey::Alt(b'F'))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT], KEY_LEFT), Some(Shortcut::Term(TermKey::AltWord(false))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT], KEY_RIGHT), Some(Shortcut::Term(TermKey::AltWord(true))));
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT], 2), None, "Alt+1 falls through");
        assert_eq!(in_term(&mut k, &[KEY_LEFTALT], KEY_TAB), Some(Shortcut::AppSwitch));
        // Navigation keys with no modifiers reach the pty.
        assert_eq!(in_term(&mut k, &[], KEY_HOME), Some(Shortcut::Term(TermKey::Named("Home"))));
        assert_eq!(in_term(&mut k, &[], KEY_END), Some(Shortcut::Term(TermKey::Named("End"))));
        assert_eq!(in_term(&mut k, &[], KEY_DELETE), Some(Shortcut::Term(TermKey::Named("Delete"))));
        // outside Term the same chords are the app's shortcuts again
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.key_in(33, true, false), None);
        assert_eq!(k.key_in(KEY_C, true, false), Some(Shortcut::Copy));
        k.key(KEY_C, false);
        k.key(KEY_LEFTMETA, false);
        k.key(KEY_LEFTCTRL, true);
        assert_eq!(k.key_in(KEY_C, true, false), Some(Shortcut::Copy), "Ctrl+C outside Term is Copy");
        k.key(KEY_C, false);
        k.key(KEY_LEFTCTRL, false);
        k.key(KEY_LEFTALT, true);
        assert_eq!(k.key_in(33, true, false), None, "Alt+F outside Term is not the terminal's");
        k.key(33, false);
        k.key(KEY_LEFTALT, false);
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
        assert_eq!(k.chord_mods(KEY_UP), Some(CHORD_META), "Cmd+Up");
        assert_eq!(k.chord_mods(KEY_DOWN), Some(CHORD_META), "Cmd+Down");
        assert_eq!(k.chord_mods(KEY_DELETE), Some(CHORD_META), "Cmd+Delete");
        assert_eq!(k.chord_mods(KEY_LEFT), None, "the other arrows are not chord keys");
        k.key(KEY_LEFTMETA, false);
        assert_eq!(k.chord_mods(KEY_BACKSPACE), None, "plain Backspace types");
        k.key(KEY_LEFTALT, true);
        assert_eq!(k.chord_mods(KEY_BACKSPACE), Some(CHORD_ALT), "Option+Backspace deletes a word");
        assert_eq!(k.chord_mods(KEY_DELETE), Some(CHORD_ALT), "Option+Delete deletes a word forward");
        assert_eq!(k.chord_mods(49), None, "Option and a letter type (AltGr and the layout's third level)");
        k.key(KEY_LEFTALT, false);
        k.key(KEY_RIGHTALT, true);
        assert_eq!(k.chord_mods(KEY_BACKSPACE), None, "AltGr alone is not Option");
        k.key(KEY_RIGHTALT, false);
        // Cmd+Space is Search: the shell takes shortcuts before it asks for a chord
    }

    #[test]
    fn a_shell_field_reads_the_movement_and_delete_keys() {
        let mut k = HwKeys::default();
        assert_eq!(k.field_edit(KEY_HOME), Some(Out::Named("Home")));
        assert_eq!(k.field_edit(KEY_END), Some(Out::Named("End")));
        assert_eq!(k.field_edit(KEY_DELETE), Some(Out::Named("DeleteForward")));
        assert_eq!(k.field_edit(KEY_LEFT), None, "a plain arrow is `typed`");
        assert_eq!(k.field_edit(KEY_BACKSPACE), None, "so is a plain Backspace");
        assert_eq!(k.field_edit(49), None);
        k.key(KEY_LEFTSHIFT, true);
        assert_eq!(k.field_edit(KEY_LEFT), Some(Out::Named("ShiftLeft")));
        assert_eq!(k.field_edit(KEY_RIGHT), Some(Out::Named("ShiftRight")));
        assert_eq!(k.field_edit(KEY_HOME), Some(Out::Named("ShiftHome")));
        k.key(KEY_LEFTALT, true);
        assert_eq!(k.field_edit(KEY_LEFT), Some(Out::Named("ShiftWordLeft")), "Shift+Option+Left");
        k.key(KEY_LEFTSHIFT, false);
        assert_eq!(k.field_edit(KEY_LEFT), Some(Out::Named("WordLeft")), "Option+Left");
        assert_eq!(k.field_edit(KEY_RIGHT), Some(Out::Named("WordRight")));
        assert_eq!(k.field_edit(KEY_BACKSPACE), Some(Out::Named("DeleteWord")), "Option+Backspace");
        assert_eq!(k.field_edit(KEY_DELETE), Some(Out::Named("DeleteWordForward")), "Option+Delete");
        k.key(KEY_LEFTALT, false);
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.field_edit(KEY_LEFT), Some(Out::Named("Home")), "Cmd+Left");
        assert_eq!(k.field_edit(KEY_RIGHT), Some(Out::Named("End")), "Cmd+Right");
        assert_eq!(k.field_edit(KEY_BACKSPACE), Some(Out::Named("DeleteLine")), "Cmd+Backspace");
        k.key(KEY_LEFTMETA, false);
        k.key(KEY_RIGHTALT, true);
        assert_eq!(k.field_edit(KEY_LEFT), None, "AltGr is not Option");
    }

    #[test]
    fn navigation_keys_name_their_modifiers_for_text_editors() {
        let mut k = HwKeys::default();
        assert_eq!(k.nav_mods(KEY_LEFT), None, "a plain arrow types as before");
        assert_eq!(k.nav_mods(KEY_HOME), Some(0), "Home and End go to an editor alone");
        assert_eq!(k.nav_mods(KEY_END), Some(0));
        assert_eq!(k.nav_mods(KEY_PAGEUP), Some(0), "Page Up, Page Down and forward Delete go to an editor alone too");
        assert_eq!(k.nav_mods(KEY_PAGEDOWN), Some(0));
        assert_eq!(k.nav_mods(KEY_DELETE), Some(0));
        assert_eq!(k.nav_mods(49), None, "other keys are not navigation keys");
        k.key(KEY_LEFTSHIFT, true);
        assert_eq!(k.nav_mods(KEY_LEFT), Some(CHORD_SHIFT));
        assert_eq!(k.nav_mods(KEY_HOME), Some(CHORD_SHIFT));
        k.key(KEY_LEFTALT, true);
        assert_eq!(k.nav_mods(KEY_RIGHT), Some(CHORD_SHIFT | CHORD_ALT));
        k.key(KEY_LEFTALT, false);
        k.key(KEY_LEFTSHIFT, false);
        assert_eq!(k.nav_mods(KEY_RIGHT), None);
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.nav_mods(KEY_UP), Some(CHORD_META));
        assert_eq!(k.nav_mods(KEY_END), Some(CHORD_META));
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
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_5), Some(Shortcut::Capture), "Shift-Cmd-5 opens the toolbar");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_SLASH), Some(Shortcut::HelpSearch), "Shift-Cmd-/ opens the Help search");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_3), Some(Shortcut::CaptureScreen), "Shift-Cmd-3 saves the whole screen");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_4), Some(Shortcut::CaptureSelection), "Shift-Cmd-4 opens the crosshair");
        assert_eq!(chord(&mut k, &[KEY_RIGHTMETA, KEY_RIGHTSHIFT], KEY_4), Some(Shortcut::CaptureSelection), "with the right-hand keys too");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_3), None, "Cmd-3 alone is not a shortcut");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_4), None, "Cmd-4 alone is not a shortcut");
        assert_eq!(chord(&mut k, &[KEY_LEFTSHIFT], KEY_4), None, "Shift-4 types a symbol");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_LEFTALT], KEY_4), None, "extra modifiers are not the chord");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_5), None, "Cmd-5 alone is not a shortcut");
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
        // Spoken Content: Option+Escape speaks the selection (Escape alone is Back)
        assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_ESC), Some(Shortcut::SpeakSelection));
        assert_eq!(chord(&mut k, &[], KEY_ESC), Some(Shortcut::Back));
        // the rotor keys and the press key on the same chord family; Ctrl+Space keeps the input source
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTALT], KEY_DOWN), Some(Shortcut::ReaderRotor(true)));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTALT], KEY_UP), Some(Shortcut::ReaderRotor(false)));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTALT], KEY_SPACE), Some(Shortcut::ReaderActivate));
        // Look Up: Ctrl+Cmd+D (Services > Look Up in Dictionary on the selection)
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_D), Some(Shortcut::LookUp));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTALT], KEY_F5), Some(Shortcut::AccessibilityShortcut), "Option+Cmd+F5");
        // Screen Zoom: Option+Cmd+8 toggles, Option+Cmd+= zooms in, Option+Cmd+- zooms out
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTALT], KEY_8), Some(Shortcut::ZoomToggle));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTALT], KEY_EQUAL), Some(Shortcut::ZoomStep(true)));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTALT], KEY_MINUS), Some(Shortcut::ZoomStep(false)));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_SPACE), Some(Shortcut::InputSource));
        assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_RIGHT), None, "Alt alone is not the cursor");
    }

    #[test]
    fn plain_keys_and_other_chords_do_nothing() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[], KEY_C), None);
        assert_eq!(chord(&mut k, &[], KEY_SPACE), None);
        assert_eq!(chord(&mut k, &[], KEY_TAB), None);
                assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_C), None);
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_SPACE), Some(Shortcut::InputSource));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_SPACE), Some(Shortcut::QuickSearch), "Cmd wins over Ctrl");
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_TAB), Some(Shortcut::AppSwitch));
    }

    #[test]
    fn redo_is_ctrl_shift_z_or_ctrl_y() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTSHIFT], KEY_Z), Some(Shortcut::Redo));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_RIGHTSHIFT], KEY_Z), Some(Shortcut::Redo));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_Y), Some(Shortcut::Redo));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_Y), None, "Cmd+Y reaches the app (Show All History)");
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

    /// On the desktop the Cmd key is a modifier only: a lone press and release is not Home, so a chord the window server
    /// or the front app consumes (whose letter the shell never sees) cannot send the windows to the Dock.
    #[test]
    fn on_the_desktop_a_lone_cmd_does_nothing() {
        let mut k = HwKeys::default();
        k.set_desktop(true);
        assert_eq!(k.key(KEY_LEFTMETA, true), None);
        assert_eq!(k.key(KEY_LEFTMETA, false), None, "no Home on the desktop");
        assert_eq!(k.key(KEY_RIGHTMETA, true), None);
        assert_eq!(k.key(KEY_RIGHTMETA, false), None);
        // the other shortcuts are unchanged
        assert_eq!(chord(&mut k, &[], KEY_ESC), Some(Shortcut::Back));
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_SPACE), Some(Shortcut::QuickSearch));
        // reset keeps the form factor
        k.reset();
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.key(KEY_LEFTMETA, false), None);
        // and a tablet keeps its Home key
        k.set_desktop(false);
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.key(KEY_LEFTMETA, false), Some(Shortcut::Home));
    }

    /// A chord whose letter the shell never receives (the window server took it, or an event tap or Switch Control used it up)
    /// leaves only the Cmd press and release; `cancel_lone_meta` and the letter reaching `key` both end the lone-press claim.
    #[test]
    fn a_chord_the_app_consumes_is_never_a_lone_cmd() {
        for desktop in [false, true] {
            let mut k = HwKeys::default();
            k.set_desktop(desktop);
            // the letter reached the key path (an app-owned chord such as Cmd+A in a text field)
            k.key(KEY_LEFTMETA, true);
            k.key(KEY_A, true);
            k.key(KEY_A, false);
            assert_eq!(k.key(KEY_LEFTMETA, false), None, "Cmd+A is a chord (desktop {desktop})");
            // the letter was used up before it got here
            k.key(KEY_LEFTMETA, true);
            k.cancel_lone_meta();
            assert_eq!(k.key(KEY_LEFTMETA, false), None, "a consumed key cancels the lone press (desktop {desktop})");
            // a second Cmd press starts afresh: on a tablet it is Home again
            k.key(KEY_LEFTMETA, true);
            let up = k.key(KEY_LEFTMETA, false);
            assert_eq!(up, if desktop { None } else { Some(Shortcut::Home) });
        }
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
    fn modifier_flags_follow_the_held_keys_for_clicks() {
        use bifrost_proto::ModifierFlags as F;
        let mut k = HwKeys::default();
        assert!(k.modifier_flags().is_empty());
        k.key(KEY_LEFTSHIFT, true);
        assert_eq!(k.modifier_flags(), F::SHIFT);
        k.key(KEY_LEFTMETA, true);
        assert_eq!(k.modifier_flags(), F::SHIFT | F::COMMAND, "Shift-Cmd-click");
        k.key(KEY_LEFTSHIFT, false);
        k.key(KEY_LEFTMETA, false);
        k.key(KEY_LEFTCTRL, true);
        k.key(KEY_LEFTALT, true);
        assert_eq!(k.modifier_flags(), F::CONTROL | F::OPTION);
        k.key(KEY_LEFTCTRL, false);
        k.key(KEY_LEFTALT, false);
        k.key(KEY_RIGHTALT, true);
        assert!(k.modifier_flags().is_empty(), "AltGr types characters and is not Option");
        k.key(KEY_RIGHTALT, false);
        k.key(KEY_CAPSLOCK, true);
        k.key(KEY_CAPSLOCK, false);
        assert_eq!(k.modifier_flags(), F::CAPS_LOCK, "Caps Lock counts while it is on");
        k.key(KEY_CAPSLOCK, true);
        assert!(k.modifier_flags().is_empty());
    }

    #[test]
    fn a_held_letter_with_accents_opens_its_list_instead_of_repeating() {
        let mut r = KeyRepeat::default();
        r.press(18, Some(Out::Text("e".into())), 0.0);
        assert_eq!(r.accent_hold(399.0, true), None, "the delay is not up");
        assert_eq!(r.accent_hold(400.0, false), None, "no list can open here, so the key repeats");
        assert_eq!(r.accent_hold(400.0, true), Some('e'));
        assert!(r.due(10_000.0).is_empty(), "the hold ended the repeat");
        r.press(49, Some(Out::Text("x".into())), 0.0);
        assert_eq!(r.accent_hold(400.0, true), None, "x has no accents, so it repeats");
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
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_Q), Some(Shortcut::Quit), "Cmd+Q quits the app, it is not Close");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA], KEY_H), Some(Shortcut::Minimize));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_W), None);
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL], KEY_M), None);
        assert_eq!(chord(&mut k, &[], KEY_W), None);
        // Shift+Cmd+W closes the window of an app that keeps plain Cmd+W for its own Close Tab
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_W), Some(Shortcut::Close));
        assert_eq!(chord(&mut k, &[KEY_LEFTCTRL, KEY_LEFTSHIFT], KEY_W), None, "Ctrl+Shift+W stays a plain key");
        assert_eq!(chord(&mut k, &[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_Q), Some(Shortcut::LogOut), "Shift+Cmd+Q asks to log out, it is not a quit");
    }

    /// Option+Cmd+Esc opens Force Quit Applications; Option+Esc alone still speaks the selection and Esc alone goes Back.
    #[test]
    fn option_command_escape_is_force_quit() {
        let mut k = HwKeys::default();
        assert_eq!(chord(&mut k, &[KEY_LEFTALT, KEY_LEFTMETA], KEY_ESC), Some(Shortcut::ForceQuit));
        assert_eq!(chord(&mut k, &[KEY_LEFTALT], KEY_ESC), Some(Shortcut::SpeakSelection));
        assert_eq!(chord(&mut k, &[], KEY_ESC), Some(Shortcut::Back));
        assert_eq!(chord(&mut k, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_ESC), None, "Shift makes it another chord");
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

    #[test]
    fn sticky_shift_capitalises_one_key_then_lets_go() {
        let mut k = HwKeys::default();
        k.set_sticky_keys(true);
        k.key(KEY_LEFTSHIFT, true);
        k.key(KEY_LEFTSHIFT, false);
        k.key(KEY_A, true);
        assert_eq!(k.typed(KEY_A), Some(Out::Text("A".into())));
        assert!(k.shift_held(), "the latched shift is held for the key that took it");
        k.key(KEY_A, false);
        assert!(!k.shift_held());
        k.key(KEY_A, true);
        assert_eq!(k.typed(KEY_A), Some(Out::Text("a".into())), "the one-shot latch is spent");
        k.key(KEY_A, false);
    }

    #[test]
    fn sticky_ctrl_locked_by_two_taps_chords_every_key_until_a_third_tap() {
        let mut k = HwKeys::default();
        k.set_sticky_keys(true);
        for _ in 0..2 {
            k.key(KEY_LEFTCTRL, true);
            k.key(KEY_LEFTCTRL, false);
        }
        k.key(KEY_C, true);
        assert_eq!(k.chord_mods(KEY_C), Some(CHORD_CTRL));
        k.key(KEY_C, false);
        k.key(KEY_C, true);
        assert_eq!(k.chord_mods(KEY_C), Some(CHORD_CTRL), "locked Ctrl stays for the next key");
        k.key(KEY_C, false);
        k.key(KEY_LEFTCTRL, true);
        k.key(KEY_LEFTCTRL, false);
        k.key(KEY_X, true);
        assert_eq!(k.chord_mods(KEY_X), None, "the third tap unlocks Ctrl");
        k.key(KEY_X, false);
    }

    #[test]
    fn sticky_keys_off_ignores_taps_and_turning_it_off_releases_latches() {
        let mut k = HwKeys::default();
        k.key(KEY_LEFTMETA, true);
        k.key(KEY_LEFTMETA, false);
        k.key(KEY_A, true);
        assert!(!k.mods_held(), "taps do nothing while Sticky Keys is off");
        k.key(KEY_A, false);
        k.set_sticky_keys(true);
        k.key(KEY_LEFTSHIFT, true);
        k.key(KEY_LEFTSHIFT, false);
        k.set_sticky_keys(false);
        assert!(!k.sticky_keys());
        let b: u16 = 48;
        k.key(b, true);
        assert!(!k.shift_held());
        k.key(b, false);
    }

    #[test]
    fn reset_keeps_sticky_keys_on_but_clears_latches() {
        let mut k = HwKeys::default();
        k.set_sticky_keys(true);
        k.key(KEY_LEFTALT, true);
        k.key(KEY_LEFTALT, false);
        k.reset();
        assert!(k.sticky_keys());
        k.key(KEY_A, true);
        assert!(!k.mods_held());
        k.key(KEY_A, false);
    }
}


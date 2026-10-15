//! Keyboard and mouse as game input: the state an app reads from a keyboard and a mouse, fed from raw evdev records.
//! Held keys and buttons are kept as sets and report each edge once. Relative motion and wheel steps are summed
//! between frames and taken once with [`MouseProfile::take_frame`], which is how a game reads a mouse.
//!
//! Pure: no device, no thread. The shell's event loop decides when to feed these models. The shell's own shortcut
//! handling still reads the keyboard separately (`springboard/src/shell/hwkeys.rs`), so the two do not share state yet.

use std::collections::BTreeSet;

/// `EV_KEY`: key and button records.
pub const EV_KEY: u16 = 0x01;
/// `EV_REL`: relative axis records (mouse motion and wheel steps).
pub const EV_REL: u16 = 0x02;

const KEY_MAX: u16 = 0xff;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;

/// A mouse button: the codes from `BTN_LEFT` to `BTN_FORWARD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Side,
    Extra,
}

impl MouseButton {
    /// The button a raw key code names, or `None` for any other code.
    pub fn from_code(code: u16) -> Option<MouseButton> {
        match code {
            0x110 => Some(MouseButton::Left),
            0x111 => Some(MouseButton::Right),
            0x112 => Some(MouseButton::Middle),
            0x113 => Some(MouseButton::Side),
            0x114 => Some(MouseButton::Extra),
            _ => None,
        }
    }

    /// The raw key code of this button.
    pub fn code(self) -> u16 {
        match self {
            MouseButton::Left => 0x110,
            MouseButton::Right => 0x111,
            MouseButton::Middle => 0x112,
            MouseButton::Side => 0x113,
            MouseButton::Extra => 0x114,
        }
    }
}

/// The name of a keyboard key that games commonly bind, or `None` for a code outside that set.
pub fn key_name(code: u16) -> Option<&'static str> {
    match code {
        1 => Some("escape"),
        15 => Some("tab"),
        17 => Some("w"),
        28 => Some("return"),
        29 => Some("control"),
        30 => Some("a"),
        31 => Some("s"),
        32 => Some("d"),
        42 => Some("shift"),
        56 => Some("alt"),
        57 => Some("space"),
        103 => Some("up"),
        105 => Some("left"),
        106 => Some("right"),
        108 => Some("down"),
        _ => None,
    }
}

/// A change to the input state, reported once per edge or per motion record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A keyboard key went down or up.
    Key { code: u16, pressed: bool },
    /// A mouse button went down or up.
    Button { button: MouseButton, pressed: bool },
    /// Relative motion from one record, in device units.
    Motion { dx: i32, dy: i32 },
    /// Wheel steps from one record, positive to the right and up.
    Wheel { x: i32, y: i32 },
}

/// Motion and wheel steps summed since the last [`MouseProfile::take_frame`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub dx: i32,
    pub dy: i32,
    pub wheel_x: i32,
    pub wheel_y: i32,
}

/// The keys held down on a keyboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardProfile {
    held: BTreeSet<u16>,
}

impl KeyboardProfile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one raw record. Returns the change it caused. Key repeats, unknown values, non-key records and a key
    /// already in the state it reports all return `None`.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32) -> Option<Change> {
        if ty != EV_KEY || code > KEY_MAX {
            return None;
        }
        let pressed = match value {
            0 => false,
            1 => true,
            _ => return None,
        };
        if pressed == self.held.contains(&code) {
            return None;
        }
        if pressed {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
        Some(Change::Key { code, pressed })
    }

    pub fn is_pressed(&self, code: u16) -> bool {
        self.held.contains(&code)
    }

    /// The held key codes, in ascending order.
    pub fn held(&self) -> impl Iterator<Item = u16> + '_ {
        self.held.iter().copied()
    }

    /// Releases every held key, for when the window loses focus. Returns one release change per key.
    pub fn release_all(&mut self) -> Vec<Change> {
        let released: Vec<u16> = self.held.iter().copied().collect();
        self.held.clear();
        released.into_iter().map(|code| Change::Key { code, pressed: false }).collect()
    }
}

/// The buttons held on a mouse, and the motion and wheel steps summed since the last frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MouseProfile {
    held: BTreeSet<MouseButton>,
    frame: Frame,
}

impl MouseProfile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one raw record. Button edges and non-zero motion or wheel steps return a change. Anything else returns `None`.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32) -> Option<Change> {
        match ty {
            EV_KEY => self.feed_button(code, value),
            EV_REL => self.feed_relative(code, value),
            _ => None,
        }
    }

    fn feed_button(&mut self, code: u16, value: i32) -> Option<Change> {
        let button = MouseButton::from_code(code)?;
        let pressed = match value {
            0 => false,
            1 => true,
            _ => return None,
        };
        if pressed == self.held.contains(&button) {
            return None;
        }
        if pressed {
            self.held.insert(button);
        } else {
            self.held.remove(&button);
        }
        Some(Change::Button { button, pressed })
    }

    fn feed_relative(&mut self, code: u16, value: i32) -> Option<Change> {
        if value == 0 {
            return None;
        }
        match code {
            REL_X => {
                self.frame.dx = self.frame.dx.saturating_add(value);
                Some(Change::Motion { dx: value, dy: 0 })
            }
            REL_Y => {
                self.frame.dy = self.frame.dy.saturating_add(value);
                Some(Change::Motion { dx: 0, dy: value })
            }
            REL_WHEEL => {
                self.frame.wheel_y = self.frame.wheel_y.saturating_add(value);
                Some(Change::Wheel { x: 0, y: value })
            }
            REL_HWHEEL => {
                self.frame.wheel_x = self.frame.wheel_x.saturating_add(value);
                Some(Change::Wheel { x: value, y: 0 })
            }
            _ => None,
        }
    }

    pub fn is_pressed(&self, button: MouseButton) -> bool {
        self.held.contains(&button)
    }

    /// Returns the motion and wheel steps summed since the last call, and starts the next frame at zero.
    pub fn take_frame(&mut self) -> Frame {
        std::mem::take(&mut self.frame)
    }

    /// Releases every held button, for when the window loses focus. Returns one release change per button.
    pub fn release_all(&mut self) -> Vec<Change> {
        let released: Vec<MouseButton> = self.held.iter().copied().collect();
        self.held.clear();
        released.into_iter().map(|button| Change::Button { button, pressed: false }).collect()
    }
}

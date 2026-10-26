//! See `wl/mod.rs` for this module's contract and owner.
//!
//! Input of the direct path: `wl_seat` pointer and keyboard, converted to the SDK's `Input` events (touch-style pointer
//! events, key events, right-click as `Input::Context`). Wheel and trackpad scrolling has no `Input` variant yet, so it
//! goes out as [`ScrollEvent`]s, which the app feeds to its scroll physics (`bifrost_ui::scroll::Scroller`) and starts a
//! kinetic fling on [`ScrollEvent::Stop`].
//!
//! The state machine is pure and host-tested. The `Dispatch` impl for `wl_pointer` and `wl_keyboard` is the glue: the
//! connection's state type implements [`SeatHandler`] and delegates with `wayland_client::delegate_dispatch!`, or
//! calls [`Seat::pointer_event`] and [`Seat::keyboard_event`] itself.
//!
//! Known limits (todo_left in docs/graphics/g1-sdk-wl-input-frame.md): printable keys use a built-in US layout, not the
//! compositor's xkb keymap, which is read and kept as text (`Seat::keymap_text`) for a later xkb pass; the keymap is
//! read with a read-only `read` of the fd instead of `mmap`, so no unsafe code is needed.
//!
//! Key codes follow the shell's app protocol (`apphost.rs`): printable keys are `KeyInput::Text`, Backspace 8, Tab 9,
//! Enter 13, Escape 1, Home 36, Left 37, Up 38, Right 39, Down 40, End 35, PageUp 33, PageDown 34 and Delete 46, and
//! Ctrl/Alt/Meta chords on printable keys are `KeyInput::chord` with the evdev key.

use crate::proto::sys::now_ms;
use crate::proto::{KeyInput, ModifierFlags, TouchKind, CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
use crate::Input;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use wayland_client::protocol::{wl_keyboard, wl_pointer, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};

/// evdev `BTN_LEFT`, the primary button (touch down, drag, up).
pub const BTN_LEFT: u32 = 0x110;
/// evdev `BTN_RIGHT`, the secondary button (`Input::Context`).
pub const BTN_RIGHT: u32 = 0x111;

const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const KEY_LEFTCTRL: u32 = 29;
const KEY_RIGHTCTRL: u32 = 97;
const KEY_LEFTALT: u32 = 56;
const KEY_RIGHTALT: u32 = 100;
const KEY_LEFTMETA: u32 = 125;
const KEY_RIGHTMETA: u32 = 126;
const KEY_CAPSLOCK: u32 = 58;
const KEY_KPENTER: u32 = 96;
const KEY_HOME: u32 = 102;
const KEY_UP: u32 = 103;
const KEY_PAGEUP: u32 = 104;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_END: u32 = 107;
const KEY_DOWN: u32 = 108;
const KEY_PAGEDOWN: u32 = 109;
const KEY_DELETE: u32 = 111;

/// Keymap files larger than this are refused (a real keymap is a few tens of kilobytes).
const MAX_KEYMAP_BYTES: u64 = 1 << 20;

/// Printable keys of the built-in US layout: evdev code, unshifted and shifted character.
const PRINTABLE: &[(u32, char, char)] = &[
    (2, '1', '!'),
    (3, '2', '@'),
    (4, '3', '#'),
    (5, '4', '$'),
    (6, '5', '%'),
    (7, '6', '^'),
    (8, '7', '&'),
    (9, '8', '*'),
    (10, '9', '('),
    (11, '0', ')'),
    (12, '-', '_'),
    (13, '=', '+'),
    (16, 'q', 'Q'),
    (17, 'w', 'W'),
    (18, 'e', 'E'),
    (19, 'r', 'R'),
    (20, 't', 'T'),
    (21, 'y', 'Y'),
    (22, 'u', 'U'),
    (23, 'i', 'I'),
    (24, 'o', 'O'),
    (25, 'p', 'P'),
    (26, '[', '{'),
    (27, ']', '}'),
    (30, 'a', 'A'),
    (31, 's', 'S'),
    (32, 'd', 'D'),
    (33, 'f', 'F'),
    (34, 'g', 'G'),
    (35, 'h', 'H'),
    (36, 'j', 'J'),
    (37, 'k', 'K'),
    (38, 'l', 'L'),
    (39, ';', ':'),
    (40, '\'', '"'),
    (41, '`', '~'),
    (43, '\\', '|'),
    (44, 'z', 'Z'),
    (45, 'x', 'X'),
    (46, 'c', 'C'),
    (47, 'v', 'V'),
    (48, 'b', 'B'),
    (49, 'n', 'N'),
    (50, 'm', 'M'),
    (51, ',', '<'),
    (52, '.', '>'),
    (53, '/', '?'),
    (57, ' ', ' '),
];

/// Modifier state, from the compositor's `wl_keyboard.modifiers` bits (the standard xkb order: Shift, Lock, Control,
/// Mod1 = Alt, Mod4 = Super).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub caps: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    /// From the depressed, latched and locked masks of `wl_keyboard.modifiers`.
    pub fn from_masks(depressed: u32, latched: u32, locked: u32) -> Mods {
        let held = depressed | latched;
        Mods {
            shift: held & 1 != 0,
            caps: locked & 2 != 0,
            ctrl: held & 4 != 0,
            alt: held & 8 != 0,
            meta: held & 64 != 0,
        }
    }

    /// The state as `NSEvent.modifierFlags` (Caps Lock counts while it is locked on).
    pub fn flags(self) -> ModifierFlags {
        ModifierFlags::from_held(self.shift, self.ctrl, self.alt, self.meta, self.caps)
    }

    /// The `CHORD_*` bits of the Ctrl, Shift, Alt and Meta state.
    pub fn chord_bits(self) -> u32 {
        let mut bits = 0;
        if self.ctrl {
            bits |= CHORD_CTRL;
        }
        if self.shift {
            bits |= CHORD_SHIFT;
        }
        if self.alt {
            bits |= CHORD_ALT;
        }
        if self.meta {
            bits |= CHORD_META;
        }
        bits
    }
}

/// The printable character for an evdev key with `mods`, or None for keys that are not printable.
pub fn printable(code: u32, mods: Mods) -> Option<char> {
    let &(_, plain, shifted) = PRINTABLE.iter().find(|(c, _, _)| *c == code)?;
    let upper = plain.is_ascii_lowercase() && (mods.shift ^ mods.caps);
    Some(match () {
        _ if plain == shifted => plain,
        _ if upper => shifted,
        _ if mods.shift && !plain.is_ascii_lowercase() => shifted,
        _ => plain,
    })
}

/// The `KeyInput` for a key press, or None for keys that send nothing (modifiers, unknown keys). Printable keys are
/// text; with Ctrl, Alt or Meta held they are chords on the evdev key. Shift+Tab, and Backspace or forward Delete with Ctrl,
/// Alt or Meta, are chords too, so the modifier reaches the app (focus steps back; Cmd+Backspace deletes to the line start).
/// Other keys use the shell's key codes.
pub fn translate_key(code: u32, mods: Mods) -> Option<KeyInput> {
    let chord_held = mods.ctrl || mods.alt || mods.meta;
    if printable(code, Mods::default()).is_some() && chord_held {
        return Some(KeyInput::chord(code, mods.chord_bits()));
    }
    if (code == KEY_TAB && mods.shift) || (matches!(code, KEY_BACKSPACE | KEY_DELETE) && chord_held) {
        return Some(KeyInput::chord(code, mods.chord_bits()));
    }
    if let Some(c) = printable(code, mods) {
        return Some(KeyInput::Text(c.to_string()));
    }
    let named = match code {
        KEY_ESC => 1,
        KEY_BACKSPACE => 8,
        KEY_TAB => 9,
        KEY_ENTER | KEY_KPENTER => 13,
        KEY_END => 35,
        KEY_HOME => 36,
        KEY_LEFT => 37,
        KEY_UP => 38,
        KEY_RIGHT => 39,
        KEY_DOWN => 40,
        KEY_PAGEUP => 33,
        KEY_PAGEDOWN => 34,
        KEY_DELETE => 46,
        // Modifiers arrive as `wl_keyboard.modifiers`, never as keys.
        KEY_LEFTSHIFT | KEY_RIGHTSHIFT | KEY_LEFTCTRL | KEY_RIGHTCTRL | KEY_LEFTALT | KEY_RIGHTALT | KEY_LEFTMETA
        | KEY_RIGHTMETA | KEY_CAPSLOCK => return None,
        _ => return None,
    };
    Some(KeyInput::Code(named))
}

/// Whether a held key repeats (text and editing keys; chords and Escape do not).
pub fn is_repeatable(key: &KeyInput) -> bool {
    match key {
        KeyInput::Text(_) => true,
        KeyInput::Code(c) => matches!(c, 8 | 9 | 13 | 33..=40 | 46),
    }
}

/// Wayland's pointer timestamp (ms, the monotonic clock) to the SDK's event time, the same rule as the shell path:
/// `now` minus the age of the event, or `now` when the age is not plausible (a wrapped or future timestamp).
pub fn event_time(now: f64, time_ms: u32) -> f64 {
    let ago = (now as u64 as u32).wrapping_sub(time_ms) as f64;
    if ago < 1000.0 {
        now - ago
    } else {
        now
    }
}

/// The axis of a scroll event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAxis {
    Vertical,
    Horizontal,
}

/// Scrolling from the pointer or a trackpad. Times are in the SDK's event time (ms, as [`Input::Touch`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollEvent {
    /// A continuous scroll of `px` surface pixels, with the pointer at (`x`, `y`).
    Delta { t: f64, x: f32, y: f32, axis: ScrollAxis, px: f32 },
    /// Wheel detents: one notch is 1.0 (from `axis_value120`, 120 units per notch).
    Detent { t: f64, axis: ScrollAxis, steps: f32 },
    /// The finger sequence ended on this axis: the app starts kinetic scrolling with its last velocity.
    Stop { t: f64, axis: ScrollAxis },
}

/// A held key that repeats while it stays down.
#[derive(Clone, Debug, PartialEq)]
struct Repeat {
    code: u32,
    input: KeyInput,
    /// When the next repeat is due, in the SDK's event time (ms).
    due: f64,
}

/// The pointer and keyboard of one seat, as the window sees them.
#[derive(Debug, Default)]
pub struct Seat {
    out: Vec<Input>,
    scrolls: Vec<ScrollEvent>,
    pointer: Option<(f32, f32)>,
    /// The left button is down: pointer motion is a touch drag.
    left_down: bool,
    mods: Mods,
    /// The compositor's repeat rate (keys per second) and delay (ms); rate 0 turns repeat off.
    repeat_rate: i32,
    repeat_delay: i32,
    repeat: Option<Repeat>,
    keymap: Option<String>,
}

/// Implemented by the connection's state so the seat's `Dispatch` impls can reach it.
pub trait SeatHandler {
    fn seat(&mut self) -> &mut Seat;
}

impl Seat {
    /// A seat with no pointer position, no modifiers and repeat off until the compositor says otherwise.
    pub fn new() -> Seat {
        Seat::default()
    }

    /// The pointer position in surface pixels, while the pointer is over the window.
    pub fn pointer_position(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    /// The current modifier state.
    pub fn mods(&self) -> Mods {
        self.mods
    }

    /// The keymap text the compositor sent, for the xkb pass (todo_left).
    pub fn keymap_text(&self) -> Option<&str> {
        self.keymap.as_deref()
    }

    /// Takes the `Input` events since the last call, in order.
    pub fn take_inputs(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.out)
    }

    /// Takes the scroll events since the last call, in order.
    pub fn take_scrolls(&mut self) -> Vec<ScrollEvent> {
        std::mem::take(&mut self.scrolls)
    }

    /// Pointer enters the window at (`x`, `y`).
    pub fn pointer_enter(&mut self, x: f64, y: f64) {
        self.pointer = Some((x as f32, y as f32));
    }

    /// Pointer leaves the window. A drag in progress is cancelled.
    pub fn pointer_leave(&mut self) {
        if self.left_down {
            self.left_down = false;
            let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
            self.touch(TouchKind::Cancel, x, y, now_ms());
        }
        self.pointer = None;
    }

    /// Pointer motion. With the left button down this is a touch move.
    pub fn pointer_motion(&mut self, time_ms: u32, x: f64, y: f64) {
        let (x, y) = (x as f32, y as f32);
        self.pointer = Some((x, y));
        if self.left_down {
            self.touch(TouchKind::Move, x, y, event_time(now_ms(), time_ms));
        }
    }

    /// A pointer button changed. The left button is a touch (down, up); the right button opens the context menu at
    /// the pointer on press. Other buttons do nothing.
    pub fn pointer_button(&mut self, time_ms: u32, button: u32, pressed: bool) {
        let t = event_time(now_ms(), time_ms);
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        match (button, pressed) {
            (BTN_LEFT, true) => {
                self.left_down = true;
                self.touch(TouchKind::Down, x, y, t);
            }
            (BTN_LEFT, false) => {
                if self.left_down {
                    self.left_down = false;
                    self.touch(TouchKind::Up, x, y, t);
                }
            }
            (BTN_RIGHT, true) => self.out.push(Input::Context { x, y }),
            _ => {}
        }
    }

    /// A continuous scroll on one axis, in surface pixels.
    pub fn pointer_axis(&mut self, time_ms: u32, axis: ScrollAxis, px: f64) {
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        let t = event_time(now_ms(), time_ms);
        self.scrolls.push(ScrollEvent::Delta { t, x, y, axis, px: px as f32 });
    }

    /// A discrete wheel step in `axis_value120` units (120 per notch).
    pub fn pointer_axis_value120(&mut self, axis: ScrollAxis, value120: i32) {
        self.scrolls.push(ScrollEvent::Detent { t: now_ms(), axis, steps: value120_to_steps(value120) });
    }

    /// The finger sequence on `axis` ended: kinetic scrolling starts.
    pub fn pointer_axis_stop(&mut self, time_ms: u32, axis: ScrollAxis) {
        self.scrolls.push(ScrollEvent::Stop { t: event_time(now_ms(), time_ms), axis });
    }

    /// Keyboard focus arrived: nothing is held yet.
    pub fn keyboard_enter(&mut self) {
        self.repeat = None;
    }

    /// Keyboard focus left: held keys stop repeating.
    pub fn keyboard_leave(&mut self) {
        self.repeat = None;
    }

    /// Reads the keymap the compositor sent. The fd is read to its end (at most `MAX_KEYMAP_BYTES`) and kept as text.
    pub fn keymap(&mut self, fd: OwnedFd, size: u32) -> io::Result<()> {
        let file = File::from(fd);
        let mut bytes = Vec::new();
        file.take(u64::from(size).min(MAX_KEYMAP_BYTES)).read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string();
        self.keymap = Some(text);
        Ok(())
    }

    /// A key changed (evdev code). A press produces its input and starts the repeat; a release stops the repeat of
    /// that key.
    pub fn key(&mut self, time_ms: u32, key: u32, pressed: bool) {
        if !pressed {
            if self.repeat.as_ref().is_some_and(|r| r.code == key) {
                self.repeat = None;
            }
            return;
        }
        let Some(k) = translate_key(key, self.mods) else {
            return;
        };
        let now = event_time(now_ms(), time_ms);
        if self.repeat_rate > 0 && is_repeatable(&k) {
            self.repeat = Some(Repeat { code: key, input: k.clone(), due: now + f64::from(self.repeat_delay.max(0)) });
        }
        self.out.push(Input::Key(k));
    }

    /// The modifier masks changed.
    pub fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32) {
        self.mods = Mods::from_masks(depressed, latched, locked);
    }

    /// The compositor's repeat rate (keys per second) and delay (ms).
    pub fn repeat_info(&mut self, rate: i32, delay: i32) {
        self.repeat_rate = rate;
        self.repeat_delay = delay;
    }

    /// When the held key repeats next, in the SDK's event time (ms), or None when nothing repeats.
    pub fn next_repeat_at(&self) -> Option<f64> {
        self.repeat.as_ref().map(|r| r.due)
    }

    /// Emits the repeat of the held key when it is due at `now` (SDK event time, ms), and schedules the next one. At
    /// most one repeat per call, so a late wake does not flood the app.
    pub fn repeat_due(&mut self, now: f64) -> Option<Input> {
        let interval = 1000.0 / f64::from(self.repeat_rate.max(1));
        let r = self.repeat.as_mut()?;
        if now < r.due {
            return None;
        }
        r.due = now + interval;
        Some(Input::Key(r.input.clone()))
    }

    /// Applies one `wl_pointer` event.
    pub fn pointer_event(&mut self, event: wl_pointer::Event) {
        use wl_pointer::Event;
        match event {
            Event::Enter { surface_x, surface_y, .. } => self.pointer_enter(surface_x, surface_y),
            Event::Leave { .. } => self.pointer_leave(),
            Event::Motion { time, surface_x, surface_y } => self.pointer_motion(time, surface_x, surface_y),
            Event::Button { time, button, state, .. } => self.pointer_button(time, button, button_pressed(state)),
            Event::Axis { time, axis, value } => {
                if let Some(a) = scroll_axis(axis) {
                    self.pointer_axis(time, a, value);
                }
            }
            Event::AxisValue120 { axis, value120 } => {
                if let Some(a) = scroll_axis(axis) {
                    self.pointer_axis_value120(a, value120);
                }
            }
            Event::AxisStop { time, axis } => {
                if let Some(a) = scroll_axis(axis) {
                    self.pointer_axis_stop(time, a);
                }
            }
            _ => {}
        }
    }

    /// Applies one `wl_keyboard` event.
    pub fn keyboard_event(&mut self, event: wl_keyboard::Event) {
        use wl_keyboard::Event;
        match event {
            Event::Keymap { fd, size, .. } => {
                // A keymap we cannot read leaves the built-in layout in place; the app keeps running.
                let _ = self.keymap(fd, size);
            }
            Event::Enter { .. } => self.keyboard_enter(),
            Event::Leave { .. } => self.keyboard_leave(),
            Event::Key { time, key, state, .. } => self.key(time, key, key_pressed(state)),
            Event::Modifiers { mods_depressed, mods_latched, mods_locked, .. } => {
                self.modifiers(mods_depressed, mods_latched, mods_locked)
            }
            Event::RepeatInfo { rate, delay } => self.repeat_info(rate, delay),
            _ => {}
        }
    }

    fn touch(&mut self, kind: TouchKind, x: f32, y: f32, t: f64) {
        // a press carries the modifiers held at the time, as the shell's touches do (`NSEvent.modifierFlags`)
        self.out.push(Input::Touch { kind, id: 0, x, y, t, mods: self.mods.flags() });
    }
}

/// `axis_value120` units to notches.
pub fn value120_to_steps(value120: i32) -> f32 {
    value120 as f32 / 120.0
}

fn button_pressed(state: WEnum<wl_pointer::ButtonState>) -> bool {
    matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed))
}

fn key_pressed(state: WEnum<wl_keyboard::KeyState>) -> bool {
    matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed))
}

fn scroll_axis(axis: WEnum<wl_pointer::Axis>) -> Option<ScrollAxis> {
    match axis {
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => Some(ScrollAxis::Vertical),
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Some(ScrollAxis::Horizontal),
        _ => None,
    }
}

impl<D> Dispatch<wl_pointer::WlPointer, (), D> for Seat
where
    D: Dispatch<wl_pointer::WlPointer, ()> + SeatHandler,
{
    fn event(state: &mut D, _proxy: &wl_pointer::WlPointer, event: wl_pointer::Event, _data: &(), _conn: &Connection, _qh: &QueueHandle<D>) {
        state.seat().pointer_event(event);
    }
}

impl<D> Dispatch<wl_keyboard::WlKeyboard, (), D> for Seat
where
    D: Dispatch<wl_keyboard::WlKeyboard, ()> + SeatHandler,
{
    fn event(state: &mut D, _proxy: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _data: &(), _conn: &Connection, _qh: &QueueHandle<D>) {
        state.seat().keyboard_event(event);
    }
}

/// Asks the seat for the pointer and keyboard it advertises. Returns the objects made; a seat without the capability
/// gives None for it. Called from the connection when `wl_seat.capabilities` arrives.
pub fn bind_devices<D>(
    seat: &wl_seat::WlSeat,
    capabilities: WEnum<wl_seat::Capability>,
    qh: &QueueHandle<D>,
) -> (Option<wl_pointer::WlPointer>, Option<wl_keyboard::WlKeyboard>)
where
    D: Dispatch<wl_pointer::WlPointer, ()> + Dispatch<wl_keyboard::WlKeyboard, ()> + 'static,
{
    // wl_seat capability bits: pointer 1, keyboard 2 (touch 4 is not used here).
    let bits = match capabilities {
        WEnum::Value(c) => u32::from(c.bits()),
        WEnum::Unknown(u) => u,
    };
    let pointer = (bits & 1 != 0).then(|| seat.get_pointer(qh, ()));
    let keyboard = (bits & 2 != 0).then(|| seat.get_keyboard(qh, ()));
    (pointer, keyboard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Option<KeyInput> {
        Some(KeyInput::Text(s.to_string()))
    }

    #[test]
    fn printable_keys_follow_the_us_layout() {
        let plain = Mods::default();
        let shift = Mods { shift: true, ..Mods::default() };
        assert_eq!(printable(30, plain), Some('a'));
        assert_eq!(printable(30, shift), Some('A'));
        assert_eq!(printable(2, plain), Some('1'));
        assert_eq!(printable(2, shift), Some('!'));
        assert_eq!(printable(53, shift), Some('?'));
        assert_eq!(printable(57, plain), Some(' '));
        // Escape is not printable.
        assert_eq!(printable(KEY_ESC, plain), None);
    }

    #[test]
    fn caps_lock_shifts_letters_only() {
        let caps = Mods { caps: true, ..Mods::default() };
        assert_eq!(printable(16, caps), Some('Q'));
        assert_eq!(printable(2, caps), Some('1'));
        let both = Mods { caps: true, shift: true, ..Mods::default() };
        assert_eq!(printable(16, both), Some('q'));
        assert_eq!(printable(2, both), Some('!'));
    }

    #[test]
    fn printable_keys_are_text_and_named_keys_are_codes() {
        let plain = Mods::default();
        assert_eq!(translate_key(23, plain), text("i"));
        assert_eq!(translate_key(57, plain), text(" "));
        assert_eq!(translate_key(KEY_BACKSPACE, plain), Some(KeyInput::Code(8)));
        assert_eq!(translate_key(KEY_TAB, plain), Some(KeyInput::Code(9)));
        assert_eq!(translate_key(KEY_ENTER, plain), Some(KeyInput::Code(13)));
        assert_eq!(translate_key(KEY_KPENTER, plain), Some(KeyInput::Code(13)));
        assert_eq!(translate_key(KEY_ESC, plain), Some(KeyInput::Code(1)));
        assert_eq!(translate_key(KEY_LEFT, plain), Some(KeyInput::Code(37)));
        assert_eq!(translate_key(KEY_UP, plain), Some(KeyInput::Code(38)));
        assert_eq!(translate_key(KEY_RIGHT, plain), Some(KeyInput::Code(39)));
        assert_eq!(translate_key(KEY_DOWN, plain), Some(KeyInput::Code(40)));
        assert_eq!(translate_key(KEY_DELETE, plain), Some(KeyInput::Code(46)));
        assert_eq!(translate_key(KEY_HOME, plain), Some(KeyInput::Code(36)));
        assert_eq!(translate_key(KEY_END, plain), Some(KeyInput::Code(35)));
        assert_eq!(translate_key(KEY_PAGEUP, plain), Some(KeyInput::Code(33)));
        assert_eq!(translate_key(KEY_PAGEDOWN, plain), Some(KeyInput::Code(34)));
    }

    #[test]
    fn modifier_keys_and_unknown_keys_send_nothing() {
        let plain = Mods::default();
        assert_eq!(translate_key(KEY_LEFTSHIFT, plain), None);
        assert_eq!(translate_key(KEY_LEFTCTRL, plain), None);
        assert_eq!(translate_key(KEY_CAPSLOCK, plain), None);
        assert_eq!(translate_key(999, plain), None);
    }

    #[test]
    fn ctrl_chords_use_the_evdev_key_and_the_chord_bits() {
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        // Ctrl+Z is evdev 44 with CHORD_CTRL: the same chord the shell sends for Undo.
        assert_eq!(translate_key(44, ctrl), Some(KeyInput::chord(44, CHORD_CTRL)));
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::default() };
        assert_eq!(translate_key(44, ctrl_shift), Some(KeyInput::chord(44, CHORD_CTRL | CHORD_SHIFT)));
        let alt_meta = Mods { alt: true, meta: true, ..Mods::default() };
        assert_eq!(translate_key(31, alt_meta).and_then(|k| k.as_chord()), Some((31, CHORD_ALT | CHORD_META)));
        // Chords on named keys stay plain codes.
        assert_eq!(translate_key(KEY_LEFT, ctrl), Some(KeyInput::Code(37)));
    }

    #[test]
    fn shift_tab_and_modified_deletes_keep_their_modifiers() {
        let shift = Mods { shift: true, ..Mods::default() };
        // Shift+Tab is the chord on Tab, which the views read as "focus back"
        assert_eq!(translate_key(KEY_TAB, shift), Some(KeyInput::chord(KEY_TAB, CHORD_SHIFT)));
        assert_eq!(translate_key(KEY_TAB, Mods::default()), Some(KeyInput::Code(9)), "plain Tab is unchanged");
        // Cmd+Backspace and Cmd+Delete reach the app as chords with CHORD_META
        let meta = Mods { meta: true, ..Mods::default() };
        assert_eq!(translate_key(KEY_BACKSPACE, meta), Some(KeyInput::chord(KEY_BACKSPACE, CHORD_META)));
        assert_eq!(translate_key(KEY_DELETE, meta), Some(KeyInput::chord(KEY_DELETE, CHORD_META)));
        let alt = Mods { alt: true, ..Mods::default() };
        assert_eq!(translate_key(KEY_BACKSPACE, alt), Some(KeyInput::chord(KEY_BACKSPACE, CHORD_ALT)));
        assert_eq!(translate_key(KEY_BACKSPACE, shift), Some(KeyInput::Code(8)), "Shift alone does not make Backspace a chord");
        assert_eq!(translate_key(KEY_BACKSPACE, Mods::default()), Some(KeyInput::Code(8)));
    }

    #[test]
    fn modifier_masks_use_the_standard_xkb_bits() {
        let m = Mods::from_masks(1 | 4, 0, 0);
        assert!(m.shift && m.ctrl && !m.alt && !m.caps && !m.meta);
        // Alt and Super latched (held), Lock locked: caps is the lock, not a held modifier.
        let m = Mods::from_masks(64, 8, 2);
        assert!(m.alt && m.caps && m.meta && !m.shift && !m.ctrl);
        assert_eq!(Mods::from_masks(0, 0, 0), Mods::default());
    }

    #[test]
    fn repeatable_keys_are_text_and_editing_keys() {
        assert!(is_repeatable(&KeyInput::Text("a".into())));
        assert!(is_repeatable(&KeyInput::Code(8)));
        assert!(is_repeatable(&KeyInput::Code(38)));
        assert!(!is_repeatable(&KeyInput::Code(1)));
        assert!(!is_repeatable(&KeyInput::chord(44, CHORD_CTRL)));
    }

    #[test]
    fn event_time_subtracts_the_age_of_the_event() {
        // An event 40 ms old, at now = 10_000 ms.
        assert_eq!(event_time(10_000.0, 9_960), 9_960.0);
        // Same millisecond: now.
        assert_eq!(event_time(10_000.0, 10_000), 10_000.0);
        // A timestamp from the future (wrapped as a huge age) falls back to now.
        assert_eq!(event_time(10_000.0, 10_500), 10_000.0);
    }

    #[test]
    fn value120_is_one_notch_per_120() {
        assert_eq!(value120_to_steps(120), 1.0);
        assert_eq!(value120_to_steps(-240), -2.0);
        assert_eq!(value120_to_steps(30), 0.25);
        assert_eq!(value120_to_steps(0), 0.0);
    }

    #[test]
    fn left_button_is_a_touch_down_move_up() {
        let mut seat = Seat::new();
        seat.pointer_enter(100.0, 50.0);
        seat.pointer_button(0, BTN_LEFT, true);
        seat.pointer_motion(0, 110.0, 55.0);
        // Hover without a button is not an input.
        seat.pointer_button(0, BTN_LEFT, false);
        seat.pointer_motion(0, 200.0, 200.0);
        let ev = seat.take_inputs();
        let kinds: Vec<TouchKind> = ev
            .iter()
            .map(|e| match e {
                Input::Touch { kind, id, .. } => {
                    assert_eq!(*id, 0);
                    *kind
                }
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(kinds, vec![TouchKind::Down, TouchKind::Move, TouchKind::Up]);
        match &ev[1] {
            Input::Touch { x, y, .. } => assert_eq!((*x, *y), (110.0, 55.0)),
            _ => unreachable!(),
        }
        assert_eq!(seat.pointer_position(), Some((200.0, 200.0)));
    }

    #[test]
    fn right_button_opens_the_context_menu_at_the_pointer() {
        let mut seat = Seat::new();
        seat.pointer_enter(12.0, 34.0);
        seat.pointer_button(0, BTN_RIGHT, true);
        seat.pointer_button(0, BTN_RIGHT, false);
        assert_eq!(seat.take_inputs(), vec![Input::Context { x: 12.0, y: 34.0 }]);
    }

    #[test]
    fn leaving_during_a_drag_cancels_the_touch() {
        let mut seat = Seat::new();
        seat.pointer_enter(5.0, 6.0);
        seat.pointer_button(0, BTN_LEFT, true);
        seat.pointer_leave();
        let ev = seat.take_inputs();
        assert_eq!(ev.len(), 2);
        assert!(matches!(ev[1], Input::Touch { kind: TouchKind::Cancel, x, y, .. } if x == 5.0 && y == 6.0));
        assert_eq!(seat.pointer_position(), None);
    }

    #[test]
    fn scroll_events_carry_deltas_detents_and_the_kinetic_stop() {
        let mut seat = Seat::new();
        seat.pointer_enter(20.0, 30.0);
        seat.pointer_axis(0, ScrollAxis::Vertical, 15.5);
        seat.pointer_axis_value120(ScrollAxis::Vertical, 240);
        seat.pointer_axis_stop(0, ScrollAxis::Vertical);
        let s = seat.take_scrolls();
        assert_eq!(s.len(), 3);
        assert!(matches!(s[0], ScrollEvent::Delta { x, y, axis: ScrollAxis::Vertical, px, .. } if x == 20.0 && y == 30.0 && px == 15.5));
        assert!(matches!(s[1], ScrollEvent::Detent { axis: ScrollAxis::Vertical, steps, .. } if steps == 2.0));
        assert!(matches!(s[2], ScrollEvent::Stop { axis: ScrollAxis::Vertical, .. }));
        assert!(seat.take_scrolls().is_empty());
    }

    #[test]
    fn key_press_and_release_produce_one_input_each_and_repeat_only_while_held() {
        let mut seat = Seat::new();
        seat.repeat_info(25, 500);
        seat.key(0, 30, true); // 'a'
        assert_eq!(seat.take_inputs(), vec![Input::Key(KeyInput::Text("a".into()))]);
        let first_due = seat.next_repeat_at().unwrap();
        // Nothing before the delay.
        assert_eq!(seat.repeat_due(first_due - 1.0), None);
        assert_eq!(seat.repeat_due(first_due), Some(Input::Key(KeyInput::Text("a".into()))));
        // The next repeat is one interval (40 ms at 25 Hz) later.
        assert_eq!(seat.next_repeat_at(), Some(first_due + 40.0));
        seat.key(0, 30, false);
        assert_eq!(seat.next_repeat_at(), None);
        assert_eq!(seat.repeat_due(first_due + 1000.0), None);
    }

    #[test]
    fn a_late_wake_repeats_once_and_reschedules_from_now() {
        let mut seat = Seat::new();
        seat.repeat_info(10, 300);
        seat.key(0, KEY_BACKSPACE, true);
        seat.take_inputs();
        let due = seat.next_repeat_at().unwrap();
        // Woken two seconds late: one repeat, the next 100 ms after the wake.
        let wake = due + 2000.0;
        assert_eq!(seat.repeat_due(wake), Some(Input::Key(KeyInput::Code(8))));
        assert_eq!(seat.next_repeat_at(), Some(wake + 100.0));
        assert_eq!(seat.repeat_due(wake + 99.0), None);
    }

    #[test]
    fn repeat_is_off_at_rate_zero_and_for_chords() {
        let mut seat = Seat::new();
        seat.key(0, 30, true);
        assert_eq!(seat.next_repeat_at(), None);
        seat.repeat_info(30, 400);
        seat.modifiers(4, 0, 0); // Ctrl
        seat.key(0, 46, true); // Ctrl+C
        assert_eq!(seat.take_inputs().last(), Some(&Input::Key(KeyInput::chord(46, CHORD_CTRL))));
        assert_eq!(seat.next_repeat_at(), None);
    }

    #[test]
    fn leaving_focus_stops_the_repeat() {
        let mut seat = Seat::new();
        seat.repeat_info(20, 100);
        seat.key(0, 30, true);
        seat.keyboard_leave();
        assert_eq!(seat.next_repeat_at(), None);
    }

    #[test]
    fn modifiers_change_the_next_key() {
        let mut seat = Seat::new();
        seat.modifiers(1, 0, 0); // Shift
        seat.key(0, 16, true);
        assert_eq!(seat.take_inputs(), vec![Input::Key(KeyInput::Text("Q".into()))]);
        assert!(seat.mods().shift);
    }

    #[test]
    fn keymap_is_read_from_the_fd_and_kept_as_text() {
        let dir = std::env::temp_dir().join(format!("bifrost-input-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keymap.xkb");
        let body = "xkb_keymap { };\0\0";
        std::fs::write(&path, body).unwrap();
        let file = File::open(&path).unwrap();
        let mut seat = Seat::new();
        seat.keymap(OwnedFd::from(file), body.len() as u32).unwrap();
        assert_eq!(seat.keymap_text(), Some("xkb_keymap { };"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_click_carries_the_modifiers_the_compositor_reported() {
        let mut seat = Seat::new();
        seat.pointer_enter(10.0, 20.0);
        // Shift (bit 0) and Super (bit 6) held
        seat.modifiers(1 | 64, 0, 0);
        seat.pointer_button(5, BTN_LEFT, true);
        let mods = seat.take_inputs().into_iter().find_map(|i| match i {
            Input::Touch { kind: TouchKind::Down, mods, .. } => Some(mods),
            _ => None,
        });
        assert_eq!(mods, Some(ModifierFlags::SHIFT | ModifierFlags::COMMAND));
        seat.modifiers(0, 0, 0);
        seat.pointer_button(6, BTN_LEFT, false);
        assert!(matches!(seat.take_inputs().as_slice(), [Input::Touch { kind: TouchKind::Up, mods, .. }] if mods.is_empty()));
    }

    #[test]
    fn a_keymap_larger_than_the_limit_is_cut_short() {
        let dir = std::env::temp_dir().join(format!("bifrost-input-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.xkb");
        std::fs::write(&path, vec![b'a'; (MAX_KEYMAP_BYTES as usize) + 100]).unwrap();
        let file = File::open(&path).unwrap();
        let mut seat = Seat::new();
        seat.keymap(OwnedFd::from(file), u32::MAX).unwrap();
        assert_eq!(seat.keymap_text().unwrap().len(), MAX_KEYMAP_BYTES as usize);
        std::fs::remove_dir_all(&dir).ok();
    }
}

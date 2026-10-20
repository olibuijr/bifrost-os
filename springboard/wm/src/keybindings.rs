//! Compositor key bindings: Super+Space opens the search panel, Ctrl+Up and F3 Mission Control, Alt+Tab cycles the focus,
//! Alt+F4 closes the focused window and Ctrl+Super+arrows tile it. The shell only gets keys while one of its surfaces has
//! focus, so the compositor takes these chords before any client sees them. Super+A is deliberately not bound: Cmd+A is
//! Select All and belongs to the app (the app list launcher it used to start was a leftover of the old window manager).
//!
//! [`action`] is pure (keysym and modifiers in, action out) and host-tested. [`WmState::run_action`] performs one.

use std::{io::Write, net::TcpStream, time::Duration};

use smithay::{desktop::Window, input::keyboard::ModifiersState, utils::SERIAL_COUNTER};

use crate::{state::WmState, tiling::Arrow, wm::WindowId};

/// X11 keysym values (`xkbcommon-keysyms.h`).
pub mod keysym {
    pub const SPACE: u32 = 0x0020;
    pub const TAB: u32 = 0xff09;
    pub const F3: u32 = 0xffc0;
    pub const F4: u32 = 0xffc1;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
}

/// The shell's control port (`control.rs`, one line per connection).
pub const CONTROL_ADDR: &str = "127.0.0.1:7777";
/// The line sent to the shell for the search panel.
pub const QUICKSEARCH_LINE: &[u8] = b"quicksearch\n";
/// The line sent to the shell for Mission Control.
pub const OVERVIEW_LINE: &[u8] = b"overview\n";

/// The modifier keys held with a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub logo: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl From<&ModifiersState> for Mods {
    fn from(m: &ModifiersState) -> Self { Mods { logo: m.logo, alt: m.alt, ctrl: m.ctrl, shift: m.shift } }
}

/// What a compositor key binding does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Super+Space: sends `quicksearch` to the shell control port.
    QuickSearch,
    /// Ctrl+Up and F3: Mission Control, sends `overview` to the shell control port.
    MissionControl,
    /// Alt+Tab: focuses the next window.
    NextWindow,
    /// Alt+F4: closes the focused toplevel.
    CloseWindow,
    /// Ctrl+Super+arrow: Left and Right tile to a half, Up maximizes, Down centres at the previous size.
    Tile(Arrow),
}

/// The action a chord is bound to. The modifiers must match exactly: Super+Shift+Space is not the search panel.
pub fn action(sym: u32, mods: Mods) -> Option<Action> {
    let none = Mods::default();
    let super_only = mods == Mods { logo: true, ..none };
    let alt_only = mods == Mods { alt: true, ..none };
    let ctrl_only = mods == Mods { ctrl: true, ..none };
    let ctrl_super = mods == Mods { logo: true, ctrl: true, ..none };
    match sym {
        keysym::SPACE if super_only => Some(Action::QuickSearch),
        keysym::UP if ctrl_only => Some(Action::MissionControl),
        keysym::F3 if mods == none => Some(Action::MissionControl),
        keysym::TAB if alt_only => Some(Action::NextWindow),
        keysym::F4 if alt_only => Some(Action::CloseWindow),
        keysym::LEFT if ctrl_super => Some(Action::Tile(Arrow::Left)),
        keysym::RIGHT if ctrl_super => Some(Action::Tile(Arrow::Right)),
        keysym::UP if ctrl_super => Some(Action::Tile(Arrow::Up)),
        keysym::DOWN if ctrl_super => Some(Action::Tile(Arrow::Down)),
        _ => None,
    }
}

/// The window Alt+Tab focuses: the lowest of the shown windows (`stack` is bottom to top), which comes to the top so
/// repeated presses walk through all of them. With fewer than two there is nothing to switch to.
pub fn next_window(stack: &[WindowId]) -> Option<WindowId> { if stack.len() >= 2 { stack.first().copied() } else { None } }

fn tell_shell(line: &[u8]) {
    let Ok(addr) = CONTROL_ADDR.parse() else { return };
    match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
        Ok(mut s) => {
            if let Err(e) = s.write_all(line) {
                eprintln!("bifrost-wm: shell control port: {e}");
            }
        }
        Err(e) => eprintln!("bifrost-wm: shell control port: {e}"),
    }
}

impl WmState {
    /// Performs a key binding's action.
    pub fn run_action(&mut self, action: Action) {
        match action {
            Action::QuickSearch => tell_shell(QUICKSEARCH_LINE),
            Action::MissionControl => tell_shell(OVERVIEW_LINE),
            Action::NextWindow => {
                let shown: Vec<WindowId> = self.wm.stack().into_iter().filter(|id| self.wm.visible(*id)).collect();
                if let Some(id) = next_window(&shown) {
                    self.focus(Some(id), SERIAL_COUNTER.next_serial());
                }
            }
            Action::CloseWindow => {
                let window: Option<Window> = self.wm.focused().and_then(|id| self.window(id));
                if let Some(t) = window.as_ref().and_then(|w| w.toplevel()) {
                    t.send_close();
                }
            }
            Action::Tile(arrow) => {
                let Some(id) = self.wm.focused() else { return };
                let rect = match crate::tiling::for_arrow(arrow) {
                    Some(layout) => self.wm.tile(id, layout),
                    None => self.wm.restore_centre(id),
                };
                if rect.is_some() {
                    self.apply_frames();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPER: Mods = Mods { logo: true, alt: false, ctrl: false, shift: false };
    const ALT: Mods = Mods { logo: false, alt: true, ctrl: false, shift: false };
    const CTRL_SUPER: Mods = Mods { logo: true, alt: false, ctrl: true, shift: false };

    #[test]
    fn super_space_is_quicksearch() { assert_eq!(action(keysym::SPACE, SUPER), Some(Action::QuickSearch)); }

    #[test]
    fn super_a_is_select_all_and_stays_with_the_app() {
        assert_eq!(action(0x0061, SUPER), None, "Cmd+A must reach the front app");
        assert_eq!(action(0x0041, SUPER), None);
    }

    #[test]
    fn ctrl_up_and_f3_are_mission_control() {
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        assert_eq!(action(keysym::UP, ctrl), Some(Action::MissionControl));
        assert_eq!(action(keysym::F3, Mods::default()), Some(Action::MissionControl));
        assert_eq!(action(keysym::UP, Mods::default()), None, "a plain Up is the app's");
        assert_eq!(action(keysym::UP, Mods { shift: true, ..ctrl }), None);
        assert_eq!(action(keysym::UP, CTRL_SUPER), Some(Action::Tile(Arrow::Up)), "Ctrl+Super+Up still tiles");
        assert_eq!(action(keysym::F3, Mods { ctrl: true, ..Mods::default() }), None);
    }

    #[test]
    fn mission_control_is_one_line_for_the_control_port() { assert_eq!(OVERVIEW_LINE, b"overview\n"); }

    #[test]
    fn alt_tab_cycles_and_alt_f4_closes() {
        assert_eq!(action(keysym::TAB, ALT), Some(Action::NextWindow));
        assert_eq!(action(keysym::F4, ALT), Some(Action::CloseWindow));
    }

    #[test]
    fn ctrl_super_arrows_tile() {
        assert_eq!(action(keysym::LEFT, CTRL_SUPER), Some(Action::Tile(Arrow::Left)));
        assert_eq!(action(keysym::RIGHT, CTRL_SUPER), Some(Action::Tile(Arrow::Right)));
        assert_eq!(action(keysym::UP, CTRL_SUPER), Some(Action::Tile(Arrow::Up)));
        assert_eq!(action(keysym::DOWN, CTRL_SUPER), Some(Action::Tile(Arrow::Down)));
    }

    #[test]
    fn extra_or_missing_modifiers_do_not_match() {
        assert_eq!(action(keysym::SPACE, Mods { shift: true, ..SUPER }), None);
        assert_eq!(action(keysym::SPACE, Mods::default()), None);
        assert_eq!(action(keysym::TAB, Mods { shift: true, ..ALT }), None);
        assert_eq!(action(keysym::LEFT, SUPER), None);
        assert_eq!(action(keysym::LEFT, ALT), None);
        assert_eq!(action(keysym::F4, SUPER), None);
    }

    #[test]
    fn other_keys_are_not_bound() { assert_eq!(action(0x62, SUPER), None); }

    #[test]
    fn alt_tab_needs_two_windows_and_takes_the_lowest() {
        assert_eq!(next_window(&[]), None);
        assert_eq!(next_window(&[4]), None);
        assert_eq!(next_window(&[4, 7, 9]), Some(4));
    }

    #[test]
    fn quicksearch_is_one_line_for_the_control_port() { assert_eq!(QUICKSEARCH_LINE, b"quicksearch\n"); }
}

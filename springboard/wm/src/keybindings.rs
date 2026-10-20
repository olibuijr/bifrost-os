//! Compositor key bindings: Super+Space opens the search panel, Ctrl+Up and F3 Overview, F4 the Apps grid,
//! Alt+Tab cycles the focus, Alt+F4 closes the focused window, Ctrl+Super+arrows tile it and Ctrl+Super+F toggles full
//! screen, Alt+Super+Esc the Force Quit window and Shift+Super+Q the Log Out card. The shell only gets keys while one of its surfaces has focus, so the compositor takes these chords before any
//! client sees them. Super+A is deliberately not bound: Cmd+A is Select All and belongs to the app (the app list
//! launcher it used to start was a leftover of the old window manager).
//!
//! [`action`] is pure (keysym and modifiers in, action out) and host-tested. [`WmState::run_action`] performs one.

use std::{io::Write, os::unix::net::UnixStream, path::{Path, PathBuf}, time::Duration};

use smithay::{desktop::Window, input::keyboard::ModifiersState, utils::SERIAL_COUNTER};

use crate::{state::WmState, tiling::Arrow, wm::WindowId};

/// X11 keysym values (`xkbcommon-keysyms.h`).
pub mod keysym {
    pub const SPACE: u32 = 0x0020;
    pub const F: u32 = 0x0066;
    pub const TAB: u32 = 0xff09;
    pub const ESCAPE: u32 = 0xff1b;
    /// `q`; with Shift held the modified keysym is the capital.
    pub const Q: u32 = 0x0071;
    pub const Q_SHIFTED: u32 = 0x0051;
    pub const F3: u32 = 0xffc0;
    pub const F4: u32 = 0xffc1;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
}

/// The shell's control socket (`bare/control.rs`, one line per connection). A Unix socket owned by the account this
/// window server runs as; the shell checks the peer's uid, so only root and that account are served.
pub const CONTROL_SOCKET: &str = "/run/bifrost/shell-control.sock";
/// The line sent to the shell for the search panel.
pub const QUICKSEARCH_LINE: &[u8] = b"quicksearch\n";
/// The line sent to the shell for Overview.
pub const OVERVIEW_LINE: &[u8] = b"overview\n";
/// The line sent to the shell for the Apps grid.
pub const APPGRID_LINE: &[u8] = b"appgrid\n";
/// The line sent to the shell for the Force Quit window.
pub const FORCEQUIT_LINE: &[u8] = b"forcequit\n";
/// The line sent to the shell for the Log Out card.
pub const LOGOUT_LINE: &[u8] = b"logout\n";

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
    /// Super+Space: sends `quicksearch` to the shell control socket.
    QuickSearch,
    /// Ctrl+Up and F3: Overview, sends `overview` to the shell control socket.
    MissionControl,
    /// F4: the Apps grid, sends `appgrid` to the shell control socket.
    AppGrid,
    /// Alt+Tab: focuses the next window.
    NextWindow,
    /// Alt+F4: closes the focused toplevel.
    CloseWindow,
    /// Ctrl+Super+arrow: Left and Right tile to a half, Up maximizes, Down centres at the previous size.
    Tile(Arrow),
    /// Ctrl+Super+F: the focused window goes full screen, or leaves it.
    FullScreen,
    /// Alt+Super+Esc: the Force Quit window, sends `forcequit` to the shell control socket. The shell gets no key while a
    /// hung app has the focus, so the compositor must take the chord.
    ForceQuit,
    /// Shift+Super+Q: the Log Out card, sends `logout` to the shell control socket.
    LogOut,
}

/// The action a chord is bound to. The modifiers must match exactly: Super+Shift+Space is not the search panel.
pub fn action(sym: u32, mods: Mods) -> Option<Action> {
    let none = Mods::default();
    let super_only = mods == Mods { logo: true, ..none };
    let alt_only = mods == Mods { alt: true, ..none };
    let ctrl_only = mods == Mods { ctrl: true, ..none };
    let ctrl_super = mods == Mods { logo: true, ctrl: true, ..none };
    let alt_super = mods == Mods { logo: true, alt: true, ..none };
    let shift_super = mods == Mods { logo: true, shift: true, ..none };
    match sym {
        keysym::SPACE if super_only => Some(Action::QuickSearch),
        keysym::UP if ctrl_only => Some(Action::MissionControl),
        keysym::F3 if mods == none => Some(Action::MissionControl),
        keysym::F4 if mods == none => Some(Action::AppGrid),
        keysym::TAB if alt_only => Some(Action::NextWindow),
        keysym::F4 if alt_only => Some(Action::CloseWindow),
        keysym::F if ctrl_super => Some(Action::FullScreen),
        keysym::ESCAPE if alt_super => Some(Action::ForceQuit),
        // Shift makes the modified keysym the capital; accept the lower case too in case a layout reports the base symbol
        keysym::Q | keysym::Q_SHIFTED if shift_super => Some(Action::LogOut),
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

/// The socket to use: `SPRINGBOARD_CONTROL_SOCKET` when it is an absolute path (as in the shell), else `CONTROL_SOCKET`.
fn control_socket() -> PathBuf {
    control_socket_from(std::env::var("SPRINGBOARD_CONTROL_SOCKET").ok().as_deref())
}

fn control_socket_from(v: Option<&str>) -> PathBuf {
    match v.map(str::trim) {
        Some(p) if p.starts_with('/') && p.len() > 1 => PathBuf::from(p),
        _ => PathBuf::from(CONTROL_SOCKET),
    }
}

/// Writes one request line to the shell's control socket.
fn send_line(socket: &Path, line: &[u8]) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket)?;
    s.set_write_timeout(Some(Duration::from_millis(200)))?;
    s.write_all(line)
}

fn tell_shell(line: &[u8]) {
    if let Err(e) = send_line(&control_socket(), line) {
        eprintln!("bifrost-wm: shell control socket: {e}");
    }
}

/// Whether the compositor's own chords may run: not while a session lock is held.
pub fn chords_enabled(locked: bool) -> bool { !locked }

impl WmState {
    /// Performs a key binding's action. While the session is locked every chord is inert: the windows behind the lock
    /// screen must not be closed, tiled or re-focused by someone at the keyboard.
    pub fn run_action(&mut self, action: Action) {
        if !chords_enabled(self.locked) {
            return;
        }
        match action {
            Action::QuickSearch => tell_shell(QUICKSEARCH_LINE),
            Action::MissionControl => tell_shell(OVERVIEW_LINE),
            Action::AppGrid => tell_shell(APPGRID_LINE),
            Action::ForceQuit => tell_shell(FORCEQUIT_LINE),
            Action::LogOut => tell_shell(LOGOUT_LINE),
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
                // the same placement the shell's green-button menu asks for, and the client is resized to match
                self.place_window(id, crate::tiling::placement_for_arrow(arrow));
            }
            Action::FullScreen => {
                let Some(id) = self.wm.focused() else { return };
                self.set_full_screen(id, !self.wm.is_full_screen(id));
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
    fn mission_control_is_one_line_for_the_control_socket() { assert_eq!(OVERVIEW_LINE, b"overview\n"); }

    #[test]
    fn f4_is_the_apps_grid_and_alt_f4_still_closes() {
        assert_eq!(action(keysym::F4, Mods::default()), Some(Action::AppGrid));
        assert_eq!(action(keysym::F4, ALT), Some(Action::CloseWindow));
        assert_eq!(action(keysym::F4, Mods { ctrl: true, ..Mods::default() }), None);
        assert_eq!(APPGRID_LINE, b"appgrid\n");
    }

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
    fn ctrl_super_f_toggles_full_screen() {
        assert_eq!(action(keysym::F, CTRL_SUPER), Some(Action::FullScreen));
        assert_eq!(action(keysym::F, SUPER), None);
        assert_eq!(action(keysym::F, Mods { shift: true, ..CTRL_SUPER }), None);
    }

    #[test]
    fn alt_super_escape_is_force_quit() {
        let alt_super = Mods { logo: true, alt: true, ..Mods::default() };
        assert_eq!(action(keysym::ESCAPE, alt_super), Some(Action::ForceQuit));
        assert_eq!(action(keysym::ESCAPE, Mods::default()), None, "a plain Esc is the app's");
        assert_eq!(action(keysym::ESCAPE, SUPER), None);
        assert_eq!(action(keysym::ESCAPE, ALT), None);
        assert_eq!(action(keysym::ESCAPE, Mods { ctrl: true, ..alt_super }), None);
        assert_eq!(FORCEQUIT_LINE, b"forcequit\n");
    }

    #[test]
    fn shift_super_q_is_log_out_and_ctrl_super_q_is_not() {
        let shift_super = Mods { logo: true, shift: true, ..Mods::default() };
        // with Shift held the modified keysym is the capital Q
        assert_eq!(action(keysym::Q_SHIFTED, shift_super), Some(Action::LogOut));
        assert_eq!(action(keysym::Q, shift_super), Some(Action::LogOut));
        assert_eq!(action(keysym::Q, SUPER), None, "Cmd+Q quits the app, it is not a log out");
        assert_eq!(action(keysym::Q_SHIFTED, Mods { ctrl: true, ..shift_super }), None);
        assert_eq!(action(keysym::Q, CTRL_SUPER), None, "Ctrl+Cmd+Q is the lock, the shell's own chord");
        assert_eq!(LOGOUT_LINE, b"logout\n");
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
    fn chords_are_inert_while_the_session_is_locked() {
        assert!(chords_enabled(false));
        assert!(!chords_enabled(true), "Alt+F4 must not close windows behind the lock screen");
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
    fn quicksearch_is_one_line_for_the_control_socket() { assert_eq!(QUICKSEARCH_LINE, b"quicksearch\n"); }

    #[test]
    fn the_control_socket_is_the_shells_unix_socket_not_a_tcp_port() {
        assert_eq!(CONTROL_SOCKET, "/run/bifrost/shell-control.sock");
        assert_eq!(control_socket_from(None), PathBuf::from(CONTROL_SOCKET));
        assert_eq!(control_socket_from(Some("/tmp/emu/c.sock")), PathBuf::from("/tmp/emu/c.sock"));
        assert_eq!(control_socket_from(Some("127.0.0.1:7777")), PathBuf::from(CONTROL_SOCKET), "an address is not a socket path");
    }

    #[test]
    fn a_binding_writes_its_line_to_the_socket() {
        use std::io::Read;
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("bifrost-wm-control-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("c.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        for line in [QUICKSEARCH_LINE, OVERVIEW_LINE, APPGRID_LINE, FORCEQUIT_LINE, LOGOUT_LINE] {
            send_line(&sock, line).unwrap();
            let (mut conn, _) = listener.accept().unwrap();
            let mut got = Vec::new();
            conn.read_to_end(&mut got).unwrap();
            assert_eq!(got, line);
        }
        assert!(send_line(&dir.join("missing.sock"), OVERVIEW_LINE).is_err(), "no shell, no panic: an error to log");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Clipboard control for tools and for the shell: `zwlr_data_control_manager_v1` and `ext_data_control_manager_v1`.
//!
//! The ordinary clipboard of the seat (`wl_data_device`) lets only the client with keyboard focus set the selection
//! (Smithay denies the request of any other client), so a command line tool (`wl-copy`, `wl-paste`) and the shell
//! itself, which has no keyboard-focused surface of its own for every copy, cannot reach it. The data-control protocols
//! lift that for the clients that may be trusted with the whole clipboard:
//!
//! - **The shell.** It connected through the privileged socket (`privileged.rs`) and owns the selection: what an app
//!   copies is offered to every program, and what a program copies is read into the shared pasteboard
//!   (`src/shell/pasteboard_wl.rs`).
//! - **Unsandboxed programs of the compositor's own user** on the ordinary socket: `wl-copy`, `wl-paste`, a terminal,
//!   `pbcopy` and `pbpaste`. A sandboxed app (native packages run in `bwrap`) runs as the same account and gets the raw
//!   socket, because the sandbox proxy (`docs/wayland-filter-design.md`) is design only. So a same-user client is
//!   admitted only when its mount namespace is the window server's (`state::shares_mount_ns`, read from the
//!   `SO_PEERCRED` pid on accept); a sandbox always has a namespace of its own and never sees the globals. Such an app
//!   still copies and pastes through the ordinary focused `wl_data_device`.
//!
//! `BIFROST_WM_DATA_CONTROL=off` removes both globals. The selection itself is kept by Smithay: this module only
//! decides who may bind.

use smithay::{
    delegate_data_control, delegate_ext_data_control,
    reexports::wayland_server::{Client, DisplayHandle},
    wayland::selection::{ext_data_control, wlr_data_control},
};

use crate::{
    privileged::is_privileged,
    state::{ClientState, WmState},
    text_input::own_uid,
};

/// Environment variable that turns the globals off.
pub const POLICY_ENV: &str = "BIFROST_WM_DATA_CONTROL";

/// Whether the setting leaves the globals on. Unset, empty and unknown values leave them on.
pub fn enabled(value: Option<&str>) -> bool {
    !matches!(value.map(|v| v.trim().to_ascii_lowercase()).as_deref(), Some("off" | "0" | "no" | "false" | "none"))
}

/// Whether a client may bind: it is privileged (the shell), runs as root (which could read the window server's memory
/// anyway), or runs as the compositor's own user outside any app sandbox (`same_mount_ns`: its mount namespace is the
/// window server's). Unknown credentials admit nobody.
pub fn admits(privileged: bool, client_uid: Option<u32>, own_uid: Option<u32>, same_mount_ns: bool) -> bool {
    privileged || client_uid == Some(0) || (same_mount_ns && matches!((client_uid, own_uid), (Some(c), Some(o)) if c == o))
}

fn filter(own: Option<u32>) -> impl for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static {
    move |client: &Client| {
        let data = client.get_data::<ClientState>();
        admits(is_privileged(client), data.and_then(|data| data.peer_uid), own, data.is_some_and(|data| data.shares_mount_ns))
    }
}

/// The two data-control globals (none when switched off).
pub struct DataControl {
    wlr: Option<wlr_data_control::DataControlState>,
    ext: Option<ext_data_control::DataControlState>,
}

impl DataControl {
    pub fn new(dh: &DisplayHandle) -> Self {
        if !enabled(std::env::var(POLICY_ENV).ok().as_deref()) {
            return DataControl { wlr: None, ext: None };
        }
        let own = own_uid();
        DataControl {
            wlr: Some(wlr_data_control::DataControlState::new::<WmState, _>(dh, None, filter(own))),
            ext: Some(ext_data_control::DataControlState::new::<WmState, _>(dh, None, filter(own))),
        }
    }

    /// Whether the globals exist.
    pub fn active(&self) -> bool {
        self.wlr.is_some() && self.ext.is_some()
    }
}

impl wlr_data_control::DataControlHandler for WmState {
    fn data_control_state(&self) -> &wlr_data_control::DataControlState {
        self.data_control.wlr.as_ref().expect("a data-control request needs the global that was created")
    }
}

impl ext_data_control::DataControlHandler for WmState {
    fn data_control_state(&self) -> &ext_data_control::DataControlState {
        self.data_control.ext.as_ref().expect("a data-control request needs the global that was created")
    }
}

delegate_data_control!(WmState);
delegate_ext_data_control!(WmState);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_only_turns_the_globals_off_when_it_says_so() {
        assert!(enabled(None));
        assert!(enabled(Some("")));
        assert!(enabled(Some("on")));
        assert!(enabled(Some("typo")));
        assert!(!enabled(Some("off")));
        assert!(!enabled(Some(" OFF\n")));
        assert!(!enabled(Some("0")));
    }

    #[test]
    fn the_shell_and_the_users_own_programs_may_bind() {
        assert!(admits(true, Some(0), Some(1000), true), "the privileged shell runs as root");
        assert!(admits(false, Some(1000), Some(1000), true), "wl-paste of the same user");
        assert!(admits(false, Some(0), Some(1000), true), "the shell on the ordinary socket, as root");
        assert!(!admits(false, Some(1001), Some(1000), true), "another user");
        assert!(!admits(false, None, Some(1000), true), "credentials that could not be read");
        assert!(!admits(false, Some(1000), None, true), "an unknown own uid admits nobody");
    }

    #[test]
    fn a_sandboxed_app_of_the_same_user_may_not_bind() {
        assert!(!admits(false, Some(1000), Some(1000), false), "same uid, own mount namespace: a bwrap'd native app");
        assert!(admits(true, Some(1000), Some(1000), false), "the privileged shell does not depend on the namespace");
        assert!(admits(false, Some(0), Some(1000), false), "root is admitted as before");
    }
}

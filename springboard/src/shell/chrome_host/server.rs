//! The shell's end of the chrome socket: a `HostLink` polled from the shell loop, the latest model of each kind kept
//! for clients that connect later, and the messages the chrome clients send back.
//!
//! The model producers (`menubar.rs`, `dock.rs`, `panels.rs`) call `ChromeServer::publish` with what the shell's
//! state says now; a model is broadcast to every client at once and stored so a new client gets it on its `Hello`.
//! `ShowPanel` is a command, not state, so it is broadcast and not stored.
//!
//! Owner: wave G2 agent `chrome-core`. The shell loop calls `start_if_requested`, adds `fds()` to its poll set and calls
//! `service()` after each wait.

use bifrost_chrome::link::{ClientId, HostLink};
use bifrost_chrome::model::{FromChrome, ToChrome};
use std::os::fd::RawFd;

/// Whether a model is kept for clients that connect later (state), or only broadcast (a command).
pub fn is_state(m: &ToChrome) -> bool {
    !matches!(m, ToChrome::ShowPanel(_))
}

/// The latest model of each kind: at most one entry per variant, in the order each kind first arrived.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Snapshot {
    models: Vec<ToChrome>,
}

impl Snapshot {
    /// Keeps `m` as the latest of its kind when it is state.
    pub fn record(&mut self, m: &ToChrome) {
        if !is_state(m) {
            return;
        }
        let kind = std::mem::discriminant(m);
        match self.models.iter_mut().find(|old| std::mem::discriminant(*old) == kind) {
            Some(slot) => *slot = m.clone(),
            None => self.models.push(m.clone()),
        }
    }

    /// The models a new client needs, in the order they were first recorded.
    pub fn models(&self) -> &[ToChrome] {
        &self.models
    }
}

/// The shell's chrome server: the socket, the snapshot, and nothing else.
pub struct ChromeServer {
    host: HostLink,
    snapshot: Snapshot,
}

impl ChromeServer {
    /// Binds the chrome socket.
    pub fn start() -> std::io::Result<ChromeServer> {
        Ok(ChromeServer::from_host(HostLink::bind()?))
    }

    fn from_host(host: HostLink) -> ChromeServer {
        ChromeServer { host, snapshot: Snapshot::default() }
    }

    /// The socket's fd, for the shell's poll set.
    pub fn fds(&self) -> Vec<RawFd> {
        self.host.fds()
    }

    /// Serves what is waiting without blocking: a `Hello` gets the current models, and every other message is
    /// returned for the shell to act on.
    pub fn service(&mut self) -> Vec<(ClientId, FromChrome)> {
        let mut rest = Vec::new();
        for (id, msg) in self.host.poll() {
            match msg {
                FromChrome::Hello { .. } => {
                    for m in self.snapshot.models().to_vec() {
                        self.host.send(id, &m);
                    }
                }
                other => rest.push((id, other)),
            }
        }
        rest
    }

    /// Sends a model to every client and keeps it for the clients that connect later.
    pub fn publish(&mut self, m: ToChrome) {
        self.host.broadcast(&m);
        self.snapshot.record(&m);
    }
}

/// Starts the chrome server when `chrome_host::requested()`. A socket that cannot be bound is logged and the shell
/// keeps drawing its own menu bar, Dock and panels.
pub fn start_if_requested() -> Option<ChromeServer> {
    if !super::requested() {
        return None;
    }
    match ChromeServer::start() {
        Ok(server) => Some(server),
        Err(e) => {
            eprintln!("bifrost: chrome socket unavailable, the shell draws its own chrome: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_chrome::model::{ControlModule, DockModel, DockPosition, MenuBarModel, PanelsModel};

    fn dock(size: f32) -> ToChrome {
        ToChrome::Dock(DockModel { items: vec![], size, magnified: 0.0, position: DockPosition::Bottom, autohide: false })
    }

    fn appearance(dark: bool) -> ToChrome {
        ToChrome::Appearance { dark, accent: 0x0a84ff, reduce_motion: false, reduce_transparency: false }
    }

    #[test]
    fn a_new_snapshot_is_empty() {
        assert!(Snapshot::default().models().is_empty());
    }

    #[test]
    fn one_model_per_kind_and_the_latest_wins() {
        let mut snap = Snapshot::default();
        snap.record(&dock(48.0));
        snap.record(&ToChrome::MenuBar(MenuBarModel { app_name: "Files".into(), menus: vec![], status: vec![] }));
        snap.record(&dock(64.0));
        assert_eq!(snap.models().len(), 2);
        assert_eq!(snap.models()[0], dock(64.0), "the dock is replaced in place");
        assert!(matches!(snap.models()[1], ToChrome::MenuBar(_)));
    }

    #[test]
    fn show_panel_is_a_command_and_is_not_kept() {
        let mut snap = Snapshot::default();
        snap.record(&ToChrome::ShowPanel("control".into()));
        assert!(snap.models().is_empty());
        assert!(!is_state(&ToChrome::ShowPanel(String::new())));
        assert!(is_state(&appearance(true)));
    }

    #[test]
    fn appearance_and_panels_are_kept_separately() {
        let mut snap = Snapshot::default();
        snap.record(&appearance(false));
        snap.record(&ToChrome::Panels(PanelsModel::default()));
        snap.record(&appearance(true));
        assert_eq!(snap.models(), &[appearance(true), ToChrome::Panels(PanelsModel::default())]);
    }

    #[test]
    fn a_panels_update_replaces_only_the_panels() {
        let mut snap = Snapshot::default();
        snap.record(&dock(48.0));
        snap.record(&ToChrome::Panels(PanelsModel::default()));
        let controls = ToChrome::Panels(PanelsModel {
            notifications: vec![],
            controls: vec![ControlModule { id: 1, title: "Wi-Fi".into(), symbol: "wifi".into(), on: true, value: -1.0 }],
        });
        snap.record(&controls);
        assert_eq!(snap.models(), &[dock(48.0), controls]);
    }

    #[test]
    fn the_snapshot_replays_in_first_arrival_order() {
        let mut snap = Snapshot::default();
        snap.record(&appearance(true));
        snap.record(&dock(40.0));
        snap.record(&appearance(false));
        assert_eq!(snap.models(), &[appearance(false), dock(40.0)]);
    }

    #[test]
    fn the_start_switch_is_off_by_default() {
        // BIFROST_CHROME is not set in the test environment; the shell then keeps its own chrome.
        if std::env::var("BIFROST_CHROME").map(|v| v == "1").unwrap_or(false) {
            return;
        }
        assert!(start_if_requested().is_none());
    }
}

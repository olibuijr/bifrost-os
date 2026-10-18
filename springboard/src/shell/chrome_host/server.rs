//! The shell's end of the chrome socket: a `HostLink` polled from the shell loop, the latest model of each kind kept
//! for clients that connect later, and the messages the chrome clients send back.
//!
//! The model producers (`menubar.rs`, `dock.rs`, `panels.rs`) call `ChromeServer::publish` with what the shell's
//! state says now; a model is broadcast to every client at once and stored so a new client gets it on its `Hello`.
//! `ShowPanel` is a command, not state, so it is broadcast and not stored.
//!
//! The menu bar and the Dock are produced here; the Appearance and Panels models have no producer yet (IPC-8).
//! Owner: wave G2 agent `chrome-core`. The shell loop calls `start_if_requested`, adds `fds()` to its poll set and calls
//! `service()` after each wait.

use bifrost_chrome::link::{ClientId, HostLink};
#[cfg(feature = "desktop-wayland")]
use crate::shell::{Effect, Shell};
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

    /// Whether `m` is not the model of its kind that is kept now: a model is sent again only when it changed.
    #[cfg(feature = "desktop-wayland")]
    pub fn changed(&self, m: &ToChrome) -> bool {
        !self.models.contains(m)
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
    /// `MenuBarModel.generation` of the bar the clients have; bumped whenever a changed bar is published.
    #[cfg(feature = "desktop-wayland")]
    bar_generation: u32,
    /// When (monotonic ms) the shell's bar was last compared with the clients' (`BAR_CHECK_MS` apart).
    #[cfg(any(test, feature = "desktop-wayland"))]
    bar_checked_at: f64,
}

/// How often the shell's menu bar is rebuilt and compared with what the clients have. Building it walks every menu
/// (rows, shortcut parsing), so it is not done on every frame and input event (IPC-9); a click forces a check.
#[cfg(any(test, feature = "desktop-wayland"))]
const BAR_CHECK_MS: f64 = 200.0;

impl ChromeServer {
    /// Binds the chrome socket.
    pub fn start() -> std::io::Result<ChromeServer> {
        Ok(ChromeServer::from_host(HostLink::bind()?))
    }

    fn from_host(host: HostLink) -> ChromeServer {
        ChromeServer {
            host,
            snapshot: Snapshot::default(),
            #[cfg(feature = "desktop-wayland")]
            bar_generation: 0,
            #[cfg(any(test, feature = "desktop-wayland"))]
            bar_checked_at: f64::NEG_INFINITY,
        }
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

    /// Whether the bar is due for a rebuild at `t` (ms); true at most once per `BAR_CHECK_MS`.
    #[cfg(any(test, feature = "desktop-wayland"))]
    fn bar_due(&mut self, t: f64) -> bool {
        if t - self.bar_checked_at < BAR_CHECK_MS {
            return false;
        }
        self.bar_checked_at = t;
        true
    }

    /// Publishes the menu bar model of the shell's current menus when it differs from the one the clients have,
    /// stamped with the next generation. True when it published.
    #[cfg(feature = "desktop-wayland")]
    fn publish_menu_bar_now(&mut self, shell: &Shell) -> bool {
        let mut bar = super::menubar::model(shell);
        bar.generation = self.bar_generation;
        if !self.snapshot.changed(&ToChrome::MenuBar(bar.clone())) {
            return false;
        }
        self.bar_generation = self.bar_generation.wrapping_add(1);
        bar.generation = self.bar_generation;
        self.publish(ToChrome::MenuBar(bar));
        true
    }

    /// Publishes the Dock model of the shell's Dock when it differs from the one the clients have.
    #[cfg(feature = "desktop-wayland")]
    fn publish_dock_now(&mut self, shell: &Shell) {
        if let Some(dock) = shell.chrome_dock_model() {
            let m = ToChrome::Dock(dock);
            if self.snapshot.changed(&m) {
                self.publish(m);
            }
        }
    }

    /// Whether a `MenuAction` still means what the user saw: the clicked bar is the one the clients have, and the
    /// shell's bar has not changed since (a changed bar is published at once and the click is dropped), because
    /// the ids are positions in the bar (IPC-7).
    #[cfg(feature = "desktop-wayland")]
    fn action_is_current(&mut self, shell: &Shell, generation: u32) -> bool {
        if generation != self.bar_generation {
            return false;
        }
        !self.publish_menu_bar_now(shell)
    }

    /// One pass of the chrome socket in the shell loop: runs the menu items the clients chose on the shell, then
    /// publishes the menu bar. Returns the effects the shell must run; other client messages are not acted on yet.
    #[cfg(feature = "desktop-wayland")]
    pub fn serve_shell(&mut self, shell: &mut Shell, t: f64) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (_, msg) in self.service() {
            match msg {
                FromChrome::MenuAction { menu, item, generation } => {
                    if self.action_is_current(shell, generation) {
                        effects.extend(super::menubar::menu_action(shell, menu, item, t));
                    }
                }
                // the Dock client's clicks run through the same changes the shell's own Dock uses (IPC-8)
                m @ (FromChrome::DockActivate { .. } | FromChrome::DockCommand { .. }) => effects.extend(shell.chrome_dock_message(&m, t)),
                _ => {}
            }
        }
        if self.bar_due(t) {
            self.publish_menu_bar_now(shell);
            self.publish_dock_now(shell);
        }
        effects
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

    fn test_host(tag: &str) -> (HostLink, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("bcsrv-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        (HostLink::bind_at(&dir.join("bifrost-chrome.sock")).expect("bind"), dir)
    }

    fn dock(size: f32) -> ToChrome {
        ToChrome::Dock(DockModel { items: vec![], size, magnified: 0.0, position: DockPosition::Bottom, autohide: false, icon_variant: 0, icon_tint: [0; 3], icon_gen: 0 })
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
        snap.record(&ToChrome::MenuBar(MenuBarModel { app_name: "Files".into(), menus: vec![], status: vec![], generation: 0 }));
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

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_menu_bar_is_sent_again_only_when_it_changes() {
        let mut snap = Snapshot::default();
        let bar = |name: &str| ToChrome::MenuBar(MenuBarModel { app_name: name.into(), menus: vec![], status: vec![], generation: 0 });
        assert!(snap.changed(&bar("Files")));
        snap.record(&bar("Files"));
        assert!(!snap.changed(&bar("Files")));
        assert!(snap.changed(&bar("Notes")));
    }

    #[test]
    fn the_bar_is_rebuilt_at_most_once_per_interval() {
        let (host, _dir) = test_host("due");
        let mut server = ChromeServer::from_host(host);
        let due = (0..100).filter(|i| server.bar_due(1000.0 + f64::from(*i) * 0.5)).count();
        assert_eq!(due, 1, "100 idle passes within 50 ms rebuild the bar once");
        assert!(server.bar_due(1000.0 + BAR_CHECK_MS));
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

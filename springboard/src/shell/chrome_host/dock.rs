//! The shell side of the Dock client (`bifrost-chrome`'s `dock`): the `DockModel` the shell publishes from its own Dock
//! state (`desktop_dock.rs`, `desk.rs`), and the `DockActivate` / `DockCommand` messages the client sends back, which
//! the shell runs through the same `Change`s its own Dock uses.
//!
//! Owner: wave G2 agent `chrome-dock`. Pure key mapping is host-tested below.

use super::super::desktop_dock::{Change, Position};
use super::super::{apps, Effect, Shell};
use bifrost_chrome::model::{DockItem, DockModel, DockPosition, FromChrome};

/// Maps the shell's Dock position onto the chrome model's.
pub fn chrome_position(p: Position) -> DockPosition {
    match p {
        Position::Bottom => DockPosition::Bottom,
        Position::Left => DockPosition::Left,
        Position::Right => DockPosition::Right,
    }
}

/// Splits a `move:<target key>:<after>` command into the target's key and whether the item goes after it. The key is
/// the app's icon name, or `linux:<app id>` for a native client, so it may contain colons: the side is the last field.
pub fn split_move(command: &str) -> Option<(&str, bool)> {
    let rest = command.strip_prefix("move:")?;
    let (key, after) = rest.rsplit_once(':')?;
    match after {
        "0" => Some((key, false)),
        "1" => Some((key, true)),
        _ => None,
    }
}

/// The command a `DockCommand` asks for, decoded: the Pin toggles only when it would change the state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Show,
    Quit,
    /// Pin (`keep`) or unpin (`remove`); the shell's `Pin` toggles, so only the change is run.
    Pin,
    Move,
    Ignored,
}

/// Decodes a command for an item that is (`pinned`) or is not kept in the Dock.
pub fn decode_command(command: &str, pinned: bool) -> Command {
    match command {
        "show" => Command::Show,
        "quit" => Command::Quit,
        "keep" if !pinned => Command::Pin,
        "remove" if pinned => Command::Pin,
        c if c.starts_with("move:") => Command::Move,
        _ => Command::Ignored,
    }
}

impl Shell {
    /// The Dock as the chrome client shows it: the list `desk_dock` lays out, without the Trash and without minimized
    /// windows (those are the window server's to animate). None before the desktop is up.
    pub fn chrome_dock_model(&self) -> Option<DockModel> {
        self.desk.as_ref()?;
        let m = self.desk_metrics();
        let (list, _, _) = self.desk_dock(&m);
        let prefs = self.dock_preferences();
        let items = list
            .iter()
            .filter(|&&entry| entry != super::super::desk::DOCK_TRASH && super::super::desktop_dock::dock_min_index(entry).is_none())
            .filter_map(|&entry| {
                let app_id = self.dock_app_key(entry)?;
                let title = match apps().get(entry) {
                    Some(a) => a.name.to_string(),
                    None => self.dock_linux_name(entry),
                };
                Some(DockItem {
                    app_id,
                    title,
                    // built-in icons are drawn by the shell, so the client shows a placeholder for them
                    icon_path: self.dock_icon_path(entry),
                    running: self.dock_item_running(entry),
                    active: false,
                    badge: String::new(),
                    progress: -1.0,
                    pinned: self.dock_section(entry) == 0,
                })
            })
            .collect();
        Some(DockModel {
            items,
            size: prefs.size,
            magnified: if prefs.magnification { prefs.magnified } else { prefs.size },
            position: chrome_position(prefs.position),
            autohide: prefs.autohide,
        })
    }

    /// The icon file of a Dock entry the client can read: an installed app's icon. Built-in apps have none.
    fn dock_icon_path(&self, entry: usize) -> String {
        #[cfg(feature = "desktop-wayland")]
        if let Some(super::super::AppId::External(slot)) = apps().get(entry).map(|a| a.id) {
            if let Some(p) = super::super::apphost::installed().get(slot as usize) {
                return p.icon_path().to_string_lossy().into_owned();
            }
        }
        let _ = entry;
        String::new()
    }

    /// The Dock list entry of a client's item key, if the Dock still has it.
    pub fn chrome_dock_entry(&self, key: &str) -> Option<usize> {
        if let Some(name) = key.strip_prefix("linux:") {
            #[cfg(feature = "desktop-wayland")]
            {
                let pos = self.desk.as_ref()?.linux_apps.iter().position(|a| a.app_id == name)?;
                return Some(apps().len() + 1 + pos);
            }
            #[cfg(not(feature = "desktop-wayland"))]
            {
                let _ = name;
                return None;
            }
        }
        apps().iter().position(|a| a.icon == key)
    }

    /// Applies one message from the Dock client. Returns the effects the shell must run; a message for an item the
    /// Dock no longer has is ignored.
    pub fn chrome_dock_message(&mut self, msg: &FromChrome, t: f64) -> Vec<Effect> {
        match msg {
            FromChrome::DockActivate { app_id } => match self.chrome_dock_entry(app_id) {
                Some(entry) => self.dock_change(Change::Open(entry), t),
                None => vec![],
            },
            FromChrome::DockCommand { app_id, command } => {
                let Some(entry) = self.chrome_dock_entry(app_id) else { return vec![] };
                let pinned = self.dock_section(entry) == 0;
                match decode_command(command, pinned) {
                    Command::Show => self.dock_change(Change::Open(entry), t),
                    Command::Quit => self.dock_change(Change::Quit(entry), t),
                    Command::Pin => self.dock_change(Change::Pin(entry), t),
                    Command::Move => match split_move(command).and_then(|(key, after)| Some((self.chrome_dock_entry(key)?, after))) {
                        Some((target, after)) => {
                            self.dock_reorder(entry, target, after);
                            vec![]
                        }
                        None => vec![],
                    },
                    Command::Ignored => vec![],
                }
            }
            // the window server aims minimize animations at these; the shell has nothing to do with them
            FromChrome::MinimizeTarget { .. } | FromChrome::Hello { .. } => vec![],
            FromChrome::MenuAction { .. } | FromChrome::ControlChanged { .. } | FromChrome::NotificationAction { .. } => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_command_carries_the_target_key_and_side() {
        assert_eq!(split_move("move:bifrost.settings:1"), Some(("bifrost.settings", true)));
        assert_eq!(split_move("move:linux:firefox:0"), Some(("linux:firefox", false)), "a key may contain colons");
        assert_eq!(split_move("move:bifrost.settings:2"), None);
        assert_eq!(split_move("move:bifrost.settings"), None);
        assert_eq!(split_move("show"), None);
    }

    #[test]
    fn pin_commands_only_change_the_pin() {
        assert_eq!(decode_command("keep", false), Command::Pin);
        assert_eq!(decode_command("keep", true), Command::Ignored, "already kept: no toggle back");
        assert_eq!(decode_command("remove", true), Command::Pin);
        assert_eq!(decode_command("remove", false), Command::Ignored);
    }

    #[test]
    fn show_quit_and_moves_decode_regardless_of_the_pin() {
        assert_eq!(decode_command("show", false), Command::Show);
        assert_eq!(decode_command("show", true), Command::Show);
        assert_eq!(decode_command("quit", false), Command::Quit);
        assert_eq!(decode_command("move:x:1", true), Command::Move);
        assert_eq!(decode_command("explode", true), Command::Ignored);
    }

    #[test]
    fn positions_map_one_to_one() {
        assert_eq!(chrome_position(Position::Bottom), DockPosition::Bottom);
        assert_eq!(chrome_position(Position::Left), DockPosition::Left);
        assert_eq!(chrome_position(Position::Right), DockPosition::Right);
    }
}

//! Routing of system transport requests to the app's remote command handlers. The menu bar Now Playing menu, the
//! Control Center tile and the hardware media keys all produce a [`Command`]; the shell sends it only to the app that
//! owns the current session, and the app passes it through [`route`] to its `CommandCenter`. A switched-off action or
//! one with no handler is refused, and the app then leaves its state unchanged.

use crate::now_playing::Command;
use bifrost_ui::remote_command::{CommandCenter, CommandError, HandlerToken, RemoteAction};

/// The remote action a shell command maps to. Skips have no `RemoteAction`: they use `SkipIntervalCommand` instead,
/// so they map to `None`.
pub fn action_for(command: Command) -> Option<RemoteAction> {
    Some(match command {
        Command::Play => RemoteAction::Play,
        Command::Pause => RemoteAction::Pause,
        Command::Stop => RemoteAction::Stop,
        Command::Toggle => RemoteAction::TogglePlayPause,
        Command::Next => RemoteAction::NextTrack,
        Command::Previous => RemoteAction::PreviousTrack,
        Command::Seek(_) => RemoteAction::ChangePlaybackPosition,
        Command::SkipForward | Command::SkipBackward => return None,
    })
}

/// The shell command a hardware media key asks for, by the key's name as the input layer reports it. `None` for keys
/// that are not transport keys.
pub fn media_key(name: &str) -> Option<Command> {
    Some(match name {
        "play" => Command::Play,
        "pause" => Command::Pause,
        "play_pause" => Command::Toggle,
        "stop" => Command::Stop,
        "next" => Command::Next,
        "previous" => Command::Previous,
        "fast_forward" => Command::SkipForward,
        "rewind" => Command::SkipBackward,
        _ => return None,
    })
}

/// Sends a shell command to the app's handlers. Returns the handler that took it, or the reason it was refused.
pub fn route(center: &CommandCenter, command: Command) -> Result<HandlerToken, CommandError> {
    let action = action_for(command).ok_or(CommandError::NoHandler)?;
    center.dispatch(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routed_command_reaches_the_newest_handler_for_its_action() {
        let mut center = CommandCenter::new();
        let _old = center.add_handler(RemoteAction::Play);
        let newest = center.add_handler(RemoteAction::Play);
        assert_eq!(route(&center, Command::Play), Ok(newest));
    }

    #[test]
    fn a_switched_off_action_is_refused_as_disabled() {
        let mut center = CommandCenter::new();
        center.add_handler(RemoteAction::Pause);
        center.set_enabled(RemoteAction::Pause, false);
        assert_eq!(route(&center, Command::Pause), Err(CommandError::Disabled));
    }

    #[test]
    fn an_action_without_a_handler_is_refused() {
        let center = CommandCenter::new();
        assert_eq!(route(&center, Command::Next), Err(CommandError::NoHandler));
    }

    #[test]
    fn skips_are_not_remote_actions() {
        let mut center = CommandCenter::new();
        center.add_handler(RemoteAction::Play);
        assert_eq!(action_for(Command::SkipForward), None);
        assert_eq!(route(&center, Command::SkipBackward), Err(CommandError::NoHandler));
    }

    #[test]
    fn media_keys_map_to_transport_commands() {
        assert_eq!(media_key("play_pause"), Some(Command::Toggle));
        assert_eq!(media_key("rewind"), Some(Command::SkipBackward));
        assert_eq!(media_key("volume_up"), None, "not a transport key");
    }

    #[test]
    fn seek_routes_to_the_position_handler() {
        assert_eq!(action_for(Command::Seek(1000.0)), Some(RemoteAction::ChangePlaybackPosition));
    }
}

//! Transport intents for the Now Playing session: play or pause, next track and previous track as App Intents.
//! The shell lists them in Search with the other shortcuts (see `intents.rs`) and runs one by sending the matching
//! `now_playing` command to the app that owns the session, the same command the Control Center tile sends.
//! Nothing here touches a socket or the UI; the mapping is pure so it can be tested on its own.

use super::intents::ManifestIntent;
use bifrost_ui::media_session::RemoteCommand;

/// The transport intents: id, title, and the command each one sends to the session owner.
pub const TRANSPORT: [(&str, &str, RemoteCommand); 3] = [
    ("media-play-pause", "Play or Pause", RemoteCommand::TogglePlayPause),
    ("media-next", "Next Track", RemoteCommand::Next),
    ("media-previous", "Previous Track", RemoteCommand::Previous),
];

/// The command an intent id sends, or `None` when the id is not a transport intent.
pub fn command_for_intent(id: &str) -> Option<RemoteCommand> {
    TRANSPORT.iter().find(|(intent, _, _)| *intent == id).map(|(_, _, command)| *command)
}

/// The command name the `now_playing` service uses for `command` in its events.
pub fn command_name(command: RemoteCommand) -> &'static str {
    match command {
        RemoteCommand::Play => "play",
        RemoteCommand::Pause => "pause",
        RemoteCommand::Stop => "stop",
        RemoteCommand::TogglePlayPause => "toggle",
        RemoteCommand::Next => "next",
        RemoteCommand::Previous => "previous",
        RemoteCommand::SkipForward => "skip_forward",
        RemoteCommand::SkipBackward => "skip_backward",
        RemoteCommand::SeekTo => "seek",
    }
}

/// The transport intents in the manifest form Search reads, all offered as shortcuts with no parameters.
pub fn manifest_intents() -> Vec<ManifestIntent> {
    TRANSPORT
        .iter()
        .map(|(id, title, _)| ManifestIntent { id: id.to_string(), title: title.to_string(), description: String::new(), shortcut: true, params: Vec::new() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_transport_intent_maps_to_its_command() {
        assert_eq!(command_for_intent("media-play-pause"), Some(RemoteCommand::TogglePlayPause));
        assert_eq!(command_for_intent("media-next"), Some(RemoteCommand::Next));
        assert_eq!(command_for_intent("media-previous"), Some(RemoteCommand::Previous));
        assert_eq!(command_for_intent("new-note"), None);
        assert_eq!(command_for_intent(""), None);
    }

    #[test]
    fn command_names_match_the_service_table() {
        assert_eq!(command_name(RemoteCommand::TogglePlayPause), "toggle");
        assert_eq!(command_name(RemoteCommand::SeekTo), "seek");
        assert_eq!(command_name(RemoteCommand::SkipBackward), "skip_backward");
        for command in RemoteCommand::ALL {
            assert!(!command_name(command).is_empty());
        }
    }

    #[test]
    fn transport_intents_are_runnable_shortcuts() {
        let intents = manifest_intents();
        assert_eq!(intents.len(), TRANSPORT.len());
        assert!(intents.iter().all(|intent| intent.runnable()));
        assert_eq!(intents[1].title, "Next Track");
    }
}

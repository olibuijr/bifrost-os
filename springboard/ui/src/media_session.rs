//! Local Now Playing sessions as a pure model: `MediaSession` (identifier, primary flags, content, playback state),
//! `MediaPlaybackSnapshot`, `MediaSessionError`, and the playback command table an app declares (`RemoteCommand`,
//! `CommandState`). An app keeps one session, sets its content and playback, and declares each command as unsupported,
//! disabled or enabled. The system checks a command with [`MediaSession::dispatch`] before it runs it. Publishing to the
//! lock screen, delivering the command to the app and the asynchronous primary requests stay with the caller.

use crate::media_content::{MediaContent, MediaDuration};

/// The playback commands a session can declare (`play(_:)`, `pause(_:)`, `stop(_:)`, `togglePlayPause(_:)`,
/// `next(_:)`, `previous(_:)`, `skipForward(preferredIntervals:_:)`, `skipBackward(preferredIntervals:_:)`,
/// `seekToPosition(_:)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RemoteCommand {
    Play,
    Pause,
    Stop,
    TogglePlayPause,
    Next,
    Previous,
    SkipForward,
    SkipBackward,
    SeekTo,
}

impl RemoteCommand {
    /// Every command, in the order the table stores them.
    pub const ALL: [RemoteCommand; 9] = [
        RemoteCommand::Play,
        RemoteCommand::Pause,
        RemoteCommand::Stop,
        RemoteCommand::TogglePlayPause,
        RemoteCommand::Next,
        RemoteCommand::Previous,
        RemoteCommand::SkipForward,
        RemoteCommand::SkipBackward,
        RemoteCommand::SeekTo,
    ];

    fn index(self) -> usize {
        match self {
            RemoteCommand::Play => 0,
            RemoteCommand::Pause => 1,
            RemoteCommand::Stop => 2,
            RemoteCommand::TogglePlayPause => 3,
            RemoteCommand::Next => 4,
            RemoteCommand::Previous => 5,
            RemoteCommand::SkipForward => 6,
            RemoteCommand::SkipBackward => 7,
            RemoteCommand::SeekTo => 8,
        }
    }
}

/// What the app declares for one command. `Unsupported` means the app has no handler, so the system never offers it.
/// `Disabled` means the handler exists but cannot run now (for example, Next on the last item of a queue).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandState {
    #[default]
    Unsupported,
    Disabled,
    Enabled,
}

/// The playback state a session publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

/// `MediaPlaybackSnapshot`: the state and the position in milliseconds that the session publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaPlaybackSnapshot {
    pub state: PlaybackState,
    pub position_ms: u64,
}

impl MediaPlaybackSnapshot {
    /// `rate`: 1 while playing and 0 otherwise. Paused and stopped content does not advance.
    pub fn rate(&self) -> f32 {
        match self.state {
            PlaybackState::Playing => 1.0,
            PlaybackState::Paused | PlaybackState::Stopped => 0.0,
        }
    }
}

/// How many media seconds pass per second a skip command is held. Holding forward or backward moves the position at
/// this rate.
pub const HOLD_SEEK_RATE: u64 = 4;

/// The position reached by holding a skip command for `held_ms` from `from_ms`. Forward holds stop at the end of
/// finite content. Backward holds stop at zero. Live and continuous content have no end to stop at.
pub fn held_seek_position(from_ms: u64, held_ms: u64, forward: bool, duration: MediaDuration) -> u64 {
    let travelled = held_ms.saturating_mul(HOLD_SEEK_RATE);
    if forward {
        within(duration, from_ms.saturating_add(travelled))
    } else {
        from_ms.saturating_sub(travelled)
    }
}

/// Caps a position at the end of finite content. Live and continuous content keep the position as given.
fn within(duration: MediaDuration, position_ms: u64) -> u64 {
    match duration {
        MediaDuration::Finite(seconds) => position_ms.min((seconds * 1000.0) as u64),
        MediaDuration::Continuous | MediaDuration::Live => position_ms,
    }
}

/// Why a session operation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaSessionError {
    /// The session was created or marked as unable to become the app's primary session.
    NotPrimaryCapable,
    /// The app did not declare the command.
    CommandUnsupported,
    /// The app declared the command but it cannot run now.
    CommandDisabled,
    /// The identifier is empty after trimming.
    EmptyId,
}

/// A local Now Playing session. Only the caller's own state is stored here; nothing is sent to the system.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaSession {
    id: String,
    can_become_application_primary: bool,
    application_primary: bool,
    system_primary: bool,
    content: Option<MediaContent>,
    state: PlaybackState,
    position_ms: u64,
    commands: [CommandState; 9],
}

impl MediaSession {
    /// `init(_:)`: a session with a trimmed identifier, able to become the app's primary session, with no content,
    /// stopped at zero, and every command unsupported. An empty identifier is refused.
    pub fn new(id: &str) -> Result<Self, MediaSessionError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(MediaSessionError::EmptyId);
        }
        Ok(MediaSession {
            id: id.to_string(),
            can_become_application_primary: true,
            application_primary: false,
            system_primary: false,
            content: None,
            state: PlaybackState::Stopped,
            position_ms: 0,
            commands: [CommandState::Unsupported; 9],
        })
    }

    /// `id`: the unique identifier of the session.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `canBecomeApplicationPrimary`.
    pub fn can_become_application_primary(&self) -> bool {
        self.can_become_application_primary
    }

    /// Sets whether the session may be made the app's primary session. Turning it off does not demote a session
    /// that is already primary.
    pub fn set_can_become_application_primary(&mut self, allowed: bool) {
        self.can_become_application_primary = allowed;
    }

    /// `isApplicationPrimary`.
    pub fn is_application_primary(&self) -> bool {
        self.application_primary
    }

    /// `isSystemPrimary`.
    pub fn is_system_primary(&self) -> bool {
        self.system_primary
    }

    /// `requestToBecomeApplicationPrimary()`: makes the session the app's primary session. Refused when the session
    /// was marked as not able to become primary.
    pub fn request_to_become_application_primary(&mut self) -> Result<(), MediaSessionError> {
        if !self.can_become_application_primary {
            return Err(MediaSessionError::NotPrimaryCapable);
        }
        self.application_primary = true;
        Ok(())
    }

    /// Records the system's answer to `requestToBecomeSystemPrimary()`. The grant is decided by the shell, so this
    /// model only stores it.
    pub fn set_system_primary(&mut self, granted: bool) {
        self.system_primary = granted;
    }

    /// The metadata the session describes, if any.
    pub fn content(&self) -> Option<&MediaContent> {
        self.content.as_ref()
    }

    /// Replaces the content. `None` clears it.
    pub fn set_content(&mut self, content: Option<MediaContent>) {
        self.content = content;
    }

    /// Publishes the playback state and position. For content with a finite duration the position is capped at that
    /// duration; for live or continuous content it is kept as given.
    pub fn set_playback(&mut self, state: PlaybackState, position_ms: u64) {
        self.state = state;
        self.position_ms = self.clamp_position(position_ms);
    }

    fn clamp_position(&self, position_ms: u64) -> u64 {
        within(self.duration(), position_ms)
    }

    /// The duration of the content, or `Continuous` when there is none.
    fn duration(&self) -> MediaDuration {
        self.content.as_ref().map_or(MediaDuration::Continuous, |c| c.duration)
    }

    /// Moves the published position as a held skip command does, keeping the playback state. Holding forward or
    /// backward for `held_ms` moves the position by [`HOLD_SEEK_RATE`] times that time.
    pub fn seek_held(&mut self, forward: bool, held_ms: u64) {
        self.position_ms = held_seek_position(self.position_ms, held_ms, forward, self.duration());
    }

    /// The state and position the session currently publishes.
    pub fn snapshot(&self) -> MediaPlaybackSnapshot {
        MediaPlaybackSnapshot { state: self.state, position_ms: self.position_ms }
    }

    /// Declares the state of one command.
    pub fn set_command(&mut self, command: RemoteCommand, state: CommandState) {
        self.commands[command.index()] = state;
    }

    /// The declared state of one command.
    pub fn command_state(&self, command: RemoteCommand) -> CommandState {
        self.commands[command.index()]
    }

    /// Checks whether a command from the system may run. `Ok` means the caller should deliver it to the app's handler.
    pub fn dispatch(&self, command: RemoteCommand) -> Result<(), MediaSessionError> {
        match self.command_state(command) {
            CommandState::Unsupported => Err(MediaSessionError::CommandUnsupported),
            CommandState::Disabled => Err(MediaSessionError::CommandDisabled),
            CommandState::Enabled => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_content::MediaType;

    fn song(seconds: f64) -> MediaContent {
        MediaContent::new("track-1", "Song", MediaType::Audio, MediaDuration::Finite(seconds)).expect("valid content")
    }

    #[test]
    fn a_new_session_is_stopped_with_every_command_unsupported() {
        let s = MediaSession::new(" main ").expect("a session");
        assert_eq!(s.id(), "main", "the identifier is trimmed");
        assert!(s.content().is_none());
        assert_eq!(s.snapshot(), MediaPlaybackSnapshot { state: PlaybackState::Stopped, position_ms: 0 });
        assert_eq!(s.dispatch(RemoteCommand::Play), Err(MediaSessionError::CommandUnsupported));
        assert_eq!(MediaSession::new("  "), Err(MediaSessionError::EmptyId));
    }

    #[test]
    fn a_disabled_command_is_refused_until_it_is_enabled() {
        let mut s = MediaSession::new("main").unwrap();
        s.set_command(RemoteCommand::Play, CommandState::Enabled);
        s.set_command(RemoteCommand::Next, CommandState::Disabled);
        assert_eq!(s.dispatch(RemoteCommand::Play), Ok(()));
        assert_eq!(s.dispatch(RemoteCommand::Next), Err(MediaSessionError::CommandDisabled));
        s.set_command(RemoteCommand::Next, CommandState::Enabled);
        assert_eq!(s.dispatch(RemoteCommand::Next), Ok(()));
        assert_eq!(s.command_state(RemoteCommand::Pause), CommandState::Unsupported);
    }

    #[test]
    fn position_is_capped_at_a_finite_duration_only() {
        let mut s = MediaSession::new("main").unwrap();
        s.set_content(Some(song(200.0)));
        s.set_playback(PlaybackState::Playing, 250_000);
        assert_eq!(s.snapshot().position_ms, 200_000, "capped at 200 s");
        s.set_playback(PlaybackState::Paused, 1_500);
        assert_eq!(s.snapshot(), MediaPlaybackSnapshot { state: PlaybackState::Paused, position_ms: 1_500 });

        let mut live = MediaSession::new("radio").unwrap();
        live.set_content(MediaContent::new("r", "Rás", MediaType::Audio, MediaDuration::Live).ok());
        live.set_playback(PlaybackState::Playing, 9_999_000);
        assert_eq!(live.snapshot().position_ms, 9_999_000, "live content has no end to cap at");
    }

    #[test]
    fn clearing_the_content_keeps_the_published_playback() {
        let mut s = MediaSession::new("main").unwrap();
        s.set_content(Some(song(60.0)));
        s.set_playback(PlaybackState::Playing, 30_000);
        s.set_content(None);
        assert!(s.content().is_none());
        assert_eq!(s.snapshot(), MediaPlaybackSnapshot { state: PlaybackState::Playing, position_ms: 30_000 });
    }

    #[test]
    fn only_a_capable_session_becomes_the_application_primary() {
        let mut s = MediaSession::new("main").unwrap();
        assert!(s.can_become_application_primary());
        assert_eq!(s.request_to_become_application_primary(), Ok(()));
        assert!(s.is_application_primary());

        let mut locked = MediaSession::new("side").unwrap();
        locked.set_can_become_application_primary(false);
        assert_eq!(locked.request_to_become_application_primary(), Err(MediaSessionError::NotPrimaryCapable));
        assert!(!locked.is_application_primary());
        assert!(!locked.is_system_primary());
        locked.set_system_primary(true);
        assert!(locked.is_system_primary(), "the system grant is stored as given");
    }
}

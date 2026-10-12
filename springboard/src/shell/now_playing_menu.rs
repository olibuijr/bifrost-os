//! The menu bar's Now Playing extra. It shows while an app has a Now Playing session published
//! (`services/now_playing.rs`): the title and artist, the position, and Previous, Play or Pause, Next rows. A row sends
//! its remote command to the session's owner, and a row the owner did not enable is greyed out. The menu has no
//! artwork: the service carries none (see `docs/parity/wire6-menubar-now-playing.md`).
use super::services::now_playing::{self, Progress};
use crate::shell::music::nowplaying::Target as Media;
use bifrost_ui::media_content::MediaDuration;
use bifrost_ui::media_session::{CommandState, MediaSession, RemoteCommand};

/// What the menu bar extra and its menu show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Panel {
    /// The menu bar label: the item's title.
    pub label: String,
    /// The title, then ` - ` and the artist and album when there are any.
    pub title: String,
    /// `1:05 of 3:20`, `1:05` for an open item, or `Live`.
    pub position: String,
    /// Previous, Play or Pause (the playback state picks which), Next. The flag is whether the owner enabled it.
    pub transport: Vec<(RemoteCommand, &'static str, bool)>,
}

/// A time as `m:ss`, or `h:mm:ss` from an hour on. Whole seconds, rounded down.
pub(crate) fn time_text(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The panel for a session and the position the tile shows. None when the session has no content.
pub(crate) fn panel_for(session: &MediaSession, progress: Progress) -> Option<Panel> {
    let content = session.content()?;
    let title = content.title.clone();
    let subtitle = content.subtitle();
    let heading = if subtitle.is_empty() { title.clone() } else { format!("{title} - {subtitle}") };
    let position = match (&content.duration, progress.duration_ms) {
        (MediaDuration::Live, _) => "Live".to_string(),
        (_, Some(total)) => format!("{} of {}", time_text(progress.position_ms), time_text(total)),
        (_, None) => time_text(progress.position_ms),
    };
    let enabled = |command: RemoteCommand| session.command_state(command) == CommandState::Enabled;
    let toggle = now_playing::transport_for(Media::Play, progress.playing).unwrap_or(RemoteCommand::Play);
    let toggle_label = if progress.playing { "Pause" } else { "Play" };
    let transport = vec![
        (RemoteCommand::Previous, "Previous", enabled(RemoteCommand::Previous)),
        (toggle, toggle_label, enabled(toggle)),
        (RemoteCommand::Next, "Next", enabled(RemoteCommand::Next)),
    ];
    Some(Panel { label: title, title: heading, position, transport })
}

/// The panel for the session an app has published, or None when no app has one.
pub(crate) fn panel() -> Option<Panel> {
    let session = now_playing::current()?;
    let progress = now_playing::progress()?;
    panel_for(&session, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::media_content::{MediaContent, MediaType};
    use bifrost_ui::media_session::PlaybackState;

    #[test]
    fn times_read_as_minutes_and_seconds_with_hours_from_an_hour_on() {
        assert_eq!(time_text(0), "0:00");
        assert_eq!(time_text(59_999), "0:59", "whole seconds, rounded down");
        assert_eq!(time_text(65_000), "1:05");
        assert_eq!(time_text(3_723_000), "1:02:03");
    }

    #[test]
    fn a_playing_song_shows_its_title_artist_position_and_the_enabled_rows() {
        let mut session = MediaSession::new("app.music").expect("a session");
        let mut content = MediaContent::new("track-1", " Song ", MediaType::Audio, MediaDuration::Finite(200.0)).expect("content");
        content.artist = Some("Band".to_string());
        session.set_content(Some(content));
        session.set_playback(PlaybackState::Playing, 5_000);
        session.set_command(RemoteCommand::Pause, CommandState::Enabled);
        session.set_command(RemoteCommand::Next, CommandState::Enabled);
        session.set_command(RemoteCommand::Previous, CommandState::Disabled);
        let progress = Progress { playing: true, position_ms: 65_000, duration_ms: Some(200_000) };
        let p = panel_for(&session, progress).expect("a panel");
        assert_eq!(p.label, "Song");
        assert_eq!(p.title, "Song - Band");
        assert_eq!(p.position, "1:05 of 3:20");
        assert_eq!(
            p.transport,
            vec![
                (RemoteCommand::Previous, "Previous", false),
                (RemoteCommand::Pause, "Pause", true),
                (RemoteCommand::Next, "Next", true),
            ]
        );
    }

    #[test]
    fn a_paused_live_item_without_artist_offers_play_and_shows_live() {
        let mut session = MediaSession::new("app.radio").expect("a session");
        session.set_content(Some(MediaContent::new("radio", "Radio", MediaType::Audio, MediaDuration::Live).expect("content")));
        session.set_playback(PlaybackState::Paused, 0);
        session.set_command(RemoteCommand::Play, CommandState::Enabled);
        let progress = Progress { playing: false, position_ms: 0, duration_ms: None };
        let p = panel_for(&session, progress).expect("a panel");
        assert_eq!(p.title, "Radio", "no artist, so no separator");
        assert_eq!(p.position, "Live");
        assert_eq!(p.transport[1], (RemoteCommand::Play, "Play", true));
        assert_eq!(p.transport[0], (RemoteCommand::Previous, "Previous", false), "unsupported by the owner");
        assert_eq!(p.transport[2], (RemoteCommand::Next, "Next", false));
    }

    #[test]
    fn an_open_item_shows_only_the_position_it_has_reached() {
        let mut session = MediaSession::new("app.talk").expect("a session");
        session.set_content(Some(MediaContent::new("talk", "Talk", MediaType::Audio, MediaDuration::Continuous).expect("content")));
        let progress = Progress { playing: true, position_ms: 9_000, duration_ms: None };
        assert_eq!(panel_for(&session, progress).expect("a panel").position, "0:09");
    }

    #[test]
    fn a_session_without_content_has_no_panel() {
        let session = MediaSession::new("app.empty").expect("a session");
        let progress = Progress { playing: false, position_ms: 0, duration_ms: None };
        assert_eq!(panel_for(&session, progress), None);
    }
}

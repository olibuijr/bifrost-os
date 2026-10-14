//! Parity sdk-complete-22: headless tests for the app side of Now Playing. They check the published body, the
//! command check that runs before an app acts on a shell command, and the rate and kind fields. Nothing here
//! publishes to a service or touches a display.

use bifrost_app::now_playing::{accept, publish_body, Command, ContentKind, MediaContent, MediaDuration, MediaSession, MediaSessionError, MediaType, PlaybackState, RemoteCommand, CommandState};
use bifrost_core::json::Json;

fn episode(kind: ContentKind) -> MediaContent {
    MediaContent::new("ep-1", "Episode", MediaType::Audio, MediaDuration::Finite(1800.0))
        .expect("valid content")
        .with_kind(kind)
}

#[test]
fn a_paused_session_publishes_a_rate_of_zero_and_its_kind() {
    let mut session = MediaSession::new("main").unwrap();
    session.set_content(Some(episode(ContentKind::Podcast)));
    session.set_playback(PlaybackState::Paused, 600_000);
    let body = publish_body(&session).expect("a body");
    assert_eq!(body.get("rate").and_then(Json::number), Some(0.0));
    assert_eq!(body.str_field("kind").as_deref(), Some("podcast"));
    assert_eq!(body.get("position_ms").and_then(Json::number), Some(600_000.0));
}

#[test]
fn a_playing_session_publishes_a_rate_of_one() {
    let mut session = MediaSession::new("main").unwrap();
    session.set_content(Some(episode(ContentKind::Music)));
    session.set_playback(PlaybackState::Playing, 1_000);
    let body = publish_body(&session).expect("a body");
    assert_eq!(body.get("rate").and_then(Json::number), Some(1.0));
    assert_eq!(body.str_field("kind").as_deref(), Some("music"));
}

#[test]
fn content_without_a_kind_is_published_as_generic() {
    let mut session = MediaSession::new("main").unwrap();
    session.set_content(Some(MediaContent::new("x", "Thing", MediaType::Audio, MediaDuration::Continuous).unwrap()));
    let body = publish_body(&session).expect("a body");
    assert_eq!(body.str_field("kind").as_deref(), Some("generic"));
}

#[test]
fn every_shell_command_maps_to_its_session_command() {
    assert_eq!(Command::Play.remote(), RemoteCommand::Play);
    assert_eq!(Command::Pause.remote(), RemoteCommand::Pause);
    assert_eq!(Command::Stop.remote(), RemoteCommand::Stop);
    assert_eq!(Command::Toggle.remote(), RemoteCommand::TogglePlayPause);
    assert_eq!(Command::Next.remote(), RemoteCommand::Next);
    assert_eq!(Command::Previous.remote(), RemoteCommand::Previous);
    assert_eq!(Command::SkipForward.remote(), RemoteCommand::SkipForward);
    assert_eq!(Command::SkipBackward.remote(), RemoteCommand::SkipBackward);
    assert_eq!(Command::Seek(90_000.0).remote(), RemoteCommand::SeekTo);
}

#[test]
fn an_undeclared_shell_command_is_refused_before_the_app_runs_it() {
    let session = MediaSession::new("main").unwrap();
    assert_eq!(accept(&session, Command::Play), Err(MediaSessionError::CommandUnsupported));
}

#[test]
fn a_disabled_shell_command_is_refused_and_an_enabled_one_is_accepted() {
    let mut session = MediaSession::new("main").unwrap();
    session.set_command(RemoteCommand::Next, CommandState::Disabled);
    session.set_command(RemoteCommand::Play, CommandState::Enabled);
    assert_eq!(accept(&session, Command::Next), Err(MediaSessionError::CommandDisabled));
    assert_eq!(accept(&session, Command::Play), Ok(RemoteCommand::Play));
}

#[test]
fn an_accepted_seek_reports_the_seek_command() {
    let mut session = MediaSession::new("main").unwrap();
    session.set_command(RemoteCommand::SeekTo, CommandState::Enabled);
    assert_eq!(accept(&session, Command::Seek(12_000.0)), Ok(RemoteCommand::SeekTo));
}

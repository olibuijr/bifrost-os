//! The Podcasts app's connection to two shell services: the Now Playing session (Control Center's media tile) and the
//! Dock tile's download bar. This file holds the pure parts: what each service should show, and the SDK calls that
//! send it. `podcasts.rs` (`sync_services`) decides when to call, and only when the answer changes, never per frame.

use bifrost_app::dock_tile;
use bifrost_app::now_playing::{self, CommandState, MediaContent, MediaDuration, MediaSession, MediaType, PlaybackState, RemoteCommand};
use bifrost_app::Ctx;

/// The session id of the episode the app plays. One app, one session.
const SESSION: &str = "episode";
/// The title the tile shows when an episode has none.
const UNTITLED: &str = "Episode";

/// What the Control Center tile shows: the episode the app has playing or paused.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub url: String,
    pub title: String,
    /// the show's title
    pub artist: String,
    /// the episode's length in milliseconds, or 0 while it is not known
    pub duration_ms: f64,
    pub playing: bool,
}

/// What the tile should show for the current episode, or `None` when nothing is playing or the audio service has not
/// confirmed the episode yet. `episode` is the episode's (URL, title, show title).
pub fn shown(episode: Option<(&str, &str, &str)>, started: bool, paused: bool, duration_ms: f64) -> Option<Shown> {
    let (url, title, show) = episode?;
    if !started {
        return None;
    }
    let title = title.trim();
    Some(Shown {
        url: url.to_string(),
        title: if title.is_empty() { UNTITLED.to_string() } else { title.to_string() },
        artist: show.trim().to_string(),
        duration_ms: if duration_ms.is_finite() && duration_ms > 0.0 { duration_ms } else { 0.0 },
        playing: !paused,
    })
}

/// The Now Playing session for `shown`. It declares the commands this app handles (play, pause, toggle, stop, skip
/// back and forward, and seek); next and previous stay unsupported, so the tile does not offer them. `None` when the
/// content is not valid (an empty URL).
pub fn session(shown: &Shown, position_ms: f64) -> Option<MediaSession> {
    let duration = if shown.duration_ms > 0.0 { MediaDuration::Finite(shown.duration_ms / 1000.0) } else { MediaDuration::Continuous };
    let mut content = MediaContent::new(&shown.url, &shown.title, MediaType::Audio, duration).ok()?;
    content.artist = (!shown.artist.is_empty()).then(|| shown.artist.clone());
    let mut session = MediaSession::new(SESSION).ok()?;
    session.set_content(Some(content));
    let state = if shown.playing { PlaybackState::Playing } else { PlaybackState::Paused };
    session.set_playback(state, position_ms.max(0.0) as u64);
    for command in [
        RemoteCommand::Play,
        RemoteCommand::Pause,
        RemoteCommand::Stop,
        RemoteCommand::TogglePlayPause,
        RemoteCommand::SkipForward,
        RemoteCommand::SkipBackward,
        RemoteCommand::SeekTo,
    ] {
        session.set_command(command, CommandState::Enabled);
    }
    Some(session)
}

/// Publishes `shown` to the Now Playing service. Returns the request id, or `None` when the session is not valid.
pub fn publish(ctx: &Ctx, shown: &Shown, position_ms: f64) -> Option<u32> {
    now_playing::publish(ctx, &session(shown, position_ms)?)
}

/// What the Dock tile's download bar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Off,
    /// a sweep, while the download's size is not known
    Indeterminate,
    /// whole percent of the download done
    Percent(u8),
}

/// The bar for the episode download. `progress` is (bytes so far, total when the server said).
pub fn dock_bar(downloading: bool, progress: Option<(u64, Option<u64>)>) -> Bar {
    if !downloading {
        return Bar::Off;
    }
    match progress {
        Some((at, Some(total))) if total > 0 => Bar::Percent(((at.min(total) * 100) / total) as u8),
        _ => Bar::Indeterminate,
    }
}

/// Sends `bar` to the Dock tile. Returns the request id.
pub fn send_bar(ctx: &Ctx, bar: Bar) -> u32 {
    match bar {
        Bar::Off => dock_tile::set_progress(ctx, None),
        Bar::Indeterminate => dock_tile::set_indeterminate(ctx),
        Bar::Percent(p) => dock_tile::set_progress(ctx, Some(f32::from(p) / 100.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx() -> Ctx {
        use bifrost_app::kit::Font;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(bifrost_app::Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    const EPISODE: (&str, &str, &str) = ("https://example.com/ep1.mp3", "Episode one", "The Show");

    #[test]
    fn nothing_is_shown_until_the_service_confirms_the_episode() {
        assert_eq!(shown(None, true, false, 0.0), None, "no episode");
        assert_eq!(shown(Some(EPISODE), false, false, 0.0), None, "not started");
    }

    #[test]
    fn a_paused_episode_is_shown_as_paused() {
        let s = shown(Some(EPISODE), true, true, 0.0).unwrap();
        assert!(!s.playing);
        assert_eq!(s.artist, "The Show");
        assert_eq!(s.title, "Episode one");
    }

    #[test]
    fn a_blank_title_falls_back_and_an_unknown_length_is_zero() {
        let s = shown(Some(("u", "  ", "Show")), true, false, f64::NAN).unwrap();
        assert_eq!(s.title, "Episode");
        assert_eq!(s.duration_ms, 0.0);
    }

    #[test]
    fn the_session_declares_the_handled_commands_and_leaves_next_unsupported() {
        let s = session(&shown(Some(EPISODE), true, false, 600_000.0).unwrap(), 5_000.0).unwrap();
        for command in [RemoteCommand::Play, RemoteCommand::Pause, RemoteCommand::TogglePlayPause, RemoteCommand::SeekTo] {
            assert_eq!(s.command_state(command), CommandState::Enabled);
        }
        assert!(matches!(s.command_state(RemoteCommand::Next), CommandState::Unsupported));
        assert!(matches!(s.command_state(RemoteCommand::Previous), CommandState::Unsupported));
    }

    #[test]
    fn the_session_carries_the_length_and_the_playback_state() {
        let s = session(&shown(Some(EPISODE), true, true, 600_000.0).unwrap(), 5_000.0).unwrap();
        let content = s.content().unwrap();
        assert!(matches!(content.duration, MediaDuration::Finite(x) if x == 600.0));
        assert_eq!(content.artist.as_deref(), Some("The Show"));
        assert!(matches!(s.snapshot().state, PlaybackState::Paused));
        assert_eq!(s.snapshot().position_ms, 5_000);
    }

    #[test]
    fn an_unknown_length_publishes_as_continuous() {
        let s = session(&shown(Some(EPISODE), true, false, 0.0).unwrap(), -1.0).unwrap();
        assert!(matches!(s.content().unwrap().duration, MediaDuration::Continuous));
        assert_eq!(s.snapshot().position_ms, 0, "a negative position is clamped to the start");
    }

    #[test]
    fn the_dock_bar_is_off_unless_a_download_runs() {
        assert_eq!(dock_bar(false, Some((10, Some(100)))), Bar::Off);
    }

    #[test]
    fn the_dock_bar_sweeps_until_the_size_is_known_and_then_shows_the_percent() {
        assert_eq!(dock_bar(true, None), Bar::Indeterminate);
        assert_eq!(dock_bar(true, Some((10, None))), Bar::Indeterminate);
        assert_eq!(dock_bar(true, Some((10, Some(0)))), Bar::Indeterminate);
        assert_eq!(dock_bar(true, Some((50, Some(200)))), Bar::Percent(25));
    }

    #[test]
    fn the_dock_percent_is_clamped_to_the_whole_download() {
        assert_eq!(dock_bar(true, Some((300, Some(200)))), Bar::Percent(100));
        assert_eq!(dock_bar(true, Some((0, Some(200)))), Bar::Percent(0));
    }

    #[test]
    fn a_dock_bar_change_sends_one_dock_service_call() {
        let ctx = test_ctx();
        let id = send_bar(&ctx, Bar::Percent(40));
        match ctx.take_sent().as_slice() {
            [ToShell::ServiceCall { request, service, method, .. }] => {
                assert_eq!(*request, id);
                assert_eq!(service, "dock");
                assert_eq!(method, "progress");
            }
            other => panic!("expected one dock call, got {other:?}"),
        }
    }
}

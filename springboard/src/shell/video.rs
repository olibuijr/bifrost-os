//! Deildu's player: the media stack (hardware decode, ALSA audio) behind
//! the app's `Player` trait.

use crate::deildu::play::{PlayState, Player};
use crate::gfx::Image;
use crate::media::player::{Player as Media, State};
use std::io;
use std::path::Path;

pub struct MediaPlayer {
    w: i32,
    h: i32,
    /// pictures go to the GPU unconverted (see `Player::set_gpu`)
    gpu: bool,
    inner: Option<Media>,
}

impl MediaPlayer {
    pub fn new(w: i32, h: i32) -> Self {
        MediaPlayer { w, h, gpu: false, inner: None }
    }
}

/// The progress-screen line for a player state.
pub fn status_text(state: &State, position_ms: f64) -> String {
    match state {
        State::Playing => format!("Playing {}", clock(position_ms)),
        State::Paused => format!("Paused {}", clock(position_ms)),
        State::Buffering { .. } => "Waiting for data…".into(),
        State::Ended => "Finished".into(),
        State::Error(e) => format!("Playback failed: {e}"),
    }
}

fn clock(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Player for MediaPlayer {
    fn open(&mut self, path: &Path, _mime: &str) -> io::Result<()> {
        let mut m = Media::open(path)?;
        m.set_viewport(self.w as u32, self.h as u32);
        m.set_zero_copy(self.gpu);
        m.play();
        self.inner = Some(m);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut m) = self.inner.take() {
            m.stop();
        }
    }

    fn status(&self) -> String {
        match &self.inner {
            Some(m) => status_text(&m.state(), m.position_ms()),
            None => "Stopped".into(),
        }
    }

    fn frame(&mut self) -> Option<Image> {
        self.inner.as_mut().and_then(|m| m.take_frame())
    }

    fn picture(&mut self) -> Option<crate::media::Picture> {
        self.inner.as_mut().and_then(|m| m.take_picture())
    }

    fn set_gpu(&mut self, on: bool) {
        if self.gpu != on {
            self.gpu = on;
            if let Some(m) = &mut self.inner {
                m.set_zero_copy(on);
            }
        }
    }

    fn fd(&self) -> Option<std::ffi::c_int> {
        self.inner.as_ref().map(|m| m.fd())
    }

    fn play(&mut self) {
        if let Some(m) = &mut self.inner {
            m.play();
        }
    }

    fn pause(&mut self) {
        if let Some(m) = &mut self.inner {
            m.pause();
        }
    }

    fn seek_ms(&mut self, ms: f64) {
        if let Some(m) = &mut self.inner {
            m.seek_ms(ms);
        }
    }

    fn position_ms(&self) -> f64 {
        self.inner.as_ref().map_or(0.0, |m| m.position_ms())
    }

    fn duration_ms(&self) -> Option<f64> {
        self.inner.as_ref().and_then(|m| m.duration_ms())
    }

    fn state(&self) -> PlayState {
        match self.inner.as_ref().map(|m| m.state()) {
            None => PlayState::Idle,
            Some(State::Playing) => PlayState::Playing,
            Some(State::Paused) => PlayState::Paused,
            Some(State::Buffering { .. }) => PlayState::Buffering,
            Some(State::Ended) => PlayState::Ended,
            Some(State::Error(e)) => PlayState::Error(e),
        }
    }

    fn set_viewport(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        if let Some(m) = &mut self.inner {
            m.set_viewport(w.max(2) as u32, h.max(2) as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        assert_eq!(status_text(&State::Playing, 65_000.0), "Playing 1:05");
        assert_eq!(status_text(&State::Buffering { need: (0, 1) }, 0.0), "Waiting for data…");
        assert_eq!(status_text(&State::Error("x".into()), 0.0), "Playback failed: x");
        let mut p = MediaPlayer::new(1200, 1920);
        assert_eq!(p.status(), "Stopped");
        assert!(p.frame().is_none() && p.fd().is_none());
        assert_eq!((p.state(), p.duration_ms(), p.position_ms()), (PlayState::Idle, None, 0.0));
        p.set_viewport(1920, 1200);
        p.toggle();
        assert!(p.open(Path::new("/nonexistent.mp4"), "video/mp4").is_err());
    }
}

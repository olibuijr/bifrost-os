//! Screen stream pacing: the model behind a running screen stream. A stream starts only for an app that the person
//! allowed screen capture (`CaptureAccess::can_capture(Media::Screen)`) and whose config passes
//! `StreamConfig::validate`. Once running, `tick` delivers one frame each time the minimum frame interval has
//! passed. A frame is `Fresh` when the source changed since the last tick. Otherwise it is `Repeated`: the last
//! still from the screenshot path is sent again, so a consumer sees a steady cadence while the screen is still.
//! Frame times are seconds on the caller's clock. Pure: nothing is captured or encoded here.

use crate::capture_access::{CaptureAccess, Media};
use crate::capture_config::StreamConfig;
use crate::capture_error::StreamError;

/// Why a frame was delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// The source changed since the previous frame, or this is the first frame.
    Fresh,
    /// The source did not change; the previous still is delivered again.
    Repeated,
}

/// One frame the stream delivers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Counts the frames from 0 for this stream.
    pub index: u64,
    /// The time the frame is due, in seconds on the caller's clock.
    pub time: f64,
    pub kind: FrameKind,
}

/// A running screen stream. It is created by `ScreenStream::start` and paces its frames.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenStream {
    config: StreamConfig,
    interval: f64,
    next_due: f64,
    index: u64,
    running: bool,
    first_frame_sent: bool,
}

impl ScreenStream {
    /// Starts a stream at time `now` (seconds). The app must be allowed screen capture, and the config must pass
    /// `StreamConfig::validate`. A denied or undecided permission gives `UserDeclined`.
    pub fn start(access: &CaptureAccess, config: StreamConfig, now: f64) -> Result<Self, StreamError> {
        if !access.can_capture(Media::Screen) {
            return Err(StreamError::UserDeclined);
        }
        config.validate()?;
        if !now.is_finite() {
            return Err(StreamError::InvalidParameter);
        }
        let interval = config.min_frame_interval;
        Ok(ScreenStream { config, interval, next_due: now, index: 0, running: true, first_frame_sent: false })
    }

    pub fn config(&self) -> &StreamConfig {
        &self.config
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Frames delivered so far.
    pub fn frames_sent(&self) -> u64 {
        self.index
    }

    /// Stops the stream. Later ticks deliver nothing.
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Advances the stream to `now`. `source_changed` says whether the screen differs from the last frame. Returns
    /// a frame when one is due, otherwise `None`. A late tick delivers one frame and moves the schedule to
    /// `now`, so the stream never sends a burst of catch-up frames.
    /// Whether `tick(now, _)` would deliver a frame. The caller checks this first, so it draws the source only when
    /// a frame will be sent.
    pub fn is_due(&self, now: f64) -> bool {
        self.running && now.is_finite() && now >= self.next_due
    }

    pub fn tick(&mut self, now: f64, source_changed: bool) -> Option<Frame> {
        if !self.is_due(now) {
            return None;
        }
        let kind = if source_changed || !self.first_frame_sent { FrameKind::Fresh } else { FrameKind::Repeated };
        let frame = Frame { index: self.index, time: self.next_due, kind };
        self.index += 1;
        self.first_frame_sent = true;
        self.next_due += self.interval;
        if self.next_due <= now {
            self.next_due = now + self.interval;
        }
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_access::Status;

    fn allowed() -> CaptureAccess {
        let mut a = CaptureAccess::new();
        a.request(Media::Screen, true);
        a
    }

    fn config(interval: f64) -> StreamConfig {
        StreamConfig { min_frame_interval: interval, ..StreamConfig::default() }
    }

    #[test]
    fn a_stream_needs_screen_permission() {
        let a = CaptureAccess::new();
        assert_eq!(ScreenStream::start(&a, config(0.1), 0.0), Err(StreamError::UserDeclined));
        let mut denied = CaptureAccess::new();
        denied.request(Media::Screen, false);
        assert_eq!(denied.status(Media::Screen), Status::Denied);
        assert_eq!(ScreenStream::start(&denied, config(0.1), 0.0), Err(StreamError::UserDeclined));
    }

    #[test]
    fn an_invalid_config_is_refused() {
        let bad = StreamConfig { width: 0, ..StreamConfig::default() };
        assert_eq!(ScreenStream::start(&allowed(), bad, 0.0), Err(StreamError::InvalidParameter));
        assert_eq!(ScreenStream::start(&allowed(), config(0.1), f64::NAN), Err(StreamError::InvalidParameter));
    }

    #[test]
    fn the_first_frame_is_fresh_and_a_still_screen_repeats_frames() {
        let mut s = ScreenStream::start(&allowed(), config(0.1), 0.0).unwrap();
        let first = s.tick(0.0, false).unwrap();
        assert_eq!(first.kind, FrameKind::Fresh);
        assert_eq!(first.index, 0);
        let second = s.tick(0.1, false).unwrap();
        assert_eq!(second.kind, FrameKind::Repeated);
        assert!((second.time - 0.1).abs() < 1e-9);
        assert_eq!(s.frames_sent(), 2);
    }

    #[test]
    fn a_changed_source_gives_a_fresh_frame() {
        let mut s = ScreenStream::start(&allowed(), config(0.1), 0.0).unwrap();
        s.tick(0.0, false);
        assert_eq!(s.tick(0.1, true).unwrap().kind, FrameKind::Fresh);
    }

    #[test]
    fn no_frame_before_the_interval_passes() {
        let mut s = ScreenStream::start(&allowed(), config(0.5), 0.0).unwrap();
        assert!(s.tick(0.0, true).is_some());
        assert!(s.tick(0.2, true).is_none());
        assert!(s.tick(0.5, true).is_some());
    }

    #[test]
    fn a_late_tick_sends_one_frame_and_does_not_burst() {
        let mut s = ScreenStream::start(&allowed(), config(0.1), 0.0).unwrap();
        s.tick(0.0, true);
        // The caller was late: one frame now, then the schedule continues from now.
        assert!(s.tick(1.0, true).is_some());
        assert!(s.tick(1.0, true).is_none());
        assert!(s.tick(1.05, true).is_none());
        assert!(s.tick(1.2, true).is_some());
    }

    #[test]
    fn is_due_matches_what_tick_delivers() {
        let mut s = ScreenStream::start(&allowed(), config(0.1), 0.0).unwrap();
        assert!(s.is_due(0.0));
        s.tick(0.0, true);
        assert!(!s.is_due(0.05));
        assert!(s.is_due(0.1));
        assert!(!s.is_due(f64::NAN));
        s.stop();
        assert!(!s.is_due(5.0));
    }

    #[test]
    fn a_stopped_stream_delivers_nothing() {
        let mut s = ScreenStream::start(&allowed(), config(0.1), 0.0).unwrap();
        s.stop();
        assert!(!s.is_running());
        assert!(s.tick(1.0, true).is_none());
    }
}

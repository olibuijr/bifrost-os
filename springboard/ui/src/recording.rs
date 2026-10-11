//! Recording sessions (`Recorder`) and the trim range of a finished recording (`Trim`). The recorder keeps the state an
//! app can read: whether recording is available, whether a session runs, the microphone and camera choices, and an
//! event queue the owner drains. Times are seconds as `f64`; callers pass `now`. Nothing here captures a frame or
//! writes a file: the owner records, then reports back through `stop`, `discard` or `fail`.
//! The trim range is the model behind an edit sheet: it keeps the start and end handles inside the recording.

/// The camera used for a recording, when the camera is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraPosition {
    Front,
    Back,
}

/// A change the owner should hear about.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A session started with these choices.
    Started { microphone: bool, camera: Option<CameraPosition> },
    /// A session ended normally after this many seconds.
    Stopped { duration: f64 },
    /// A session ended and its recording was dropped.
    Discarded,
    /// A session ended because of a failure or because recording became unavailable.
    Failed(String),
}

/// Why a recorder call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// Recording is not available on this device or for this app right now.
    Unavailable,
    /// A session is already running.
    AlreadyRecording,
    /// No session is running.
    NotRecording,
    /// The camera choice cannot change while a session runs.
    Busy,
    /// The time passed in is not finite.
    InvalidTime,
}

/// What a finished session produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub duration: f64,
    pub microphone: bool,
    pub camera: Option<CameraPosition>,
}

#[derive(Clone, Debug)]
struct Session {
    started: f64,
    microphone: bool,
    camera: Option<CameraPosition>,
}

/// The recording state of one app.
#[derive(Clone, Debug)]
pub struct Recorder {
    available: bool,
    camera: Option<CameraPosition>,
    session: Option<Session>,
    events: Vec<Event>,
}

impl Recorder {
    pub fn new(available: bool) -> Self {
        Recorder { available, camera: None, session: None, events: Vec::new() }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn is_recording(&self) -> bool {
        self.session.is_some()
    }

    /// Whether the running session records the microphone. False when no session runs.
    pub fn is_microphone_enabled(&self) -> bool {
        match &self.session {
            Some(session) => session.microphone,
            None => false,
        }
    }

    /// Whether the camera is on for the next session (or the running one).
    pub fn is_camera_enabled(&self) -> bool {
        self.camera.is_some()
    }

    /// The camera choice: `None` means the camera is off.
    pub fn camera(&self) -> Option<CameraPosition> {
        self.camera
    }

    /// Sets the camera for the next session. Refused with `Busy` while a session runs.
    pub fn set_camera(&mut self, camera: Option<CameraPosition>) -> Result<(), RecordError> {
        if self.session.is_some() {
            return Err(RecordError::Busy);
        }
        self.camera = camera;
        Ok(())
    }

    /// Changes availability. Losing it while a session runs ends that session with a `Failed` event.
    pub fn set_available(&mut self, available: bool) {
        self.available = available;
        if !available && self.session.is_some() {
            self.session = None;
            self.events.push(Event::Failed("recording became unavailable".to_string()));
        }
    }

    /// Starts a session at time `now`, with or without the microphone.
    pub fn start(&mut self, now: f64, microphone: bool) -> Result<(), RecordError> {
        if !self.available {
            return Err(RecordError::Unavailable);
        }
        if self.session.is_some() {
            return Err(RecordError::AlreadyRecording);
        }
        if !now.is_finite() {
            return Err(RecordError::InvalidTime);
        }
        let camera = self.camera;
        self.session = Some(Session { started: now, microphone, camera });
        self.events.push(Event::Started { microphone, camera });
        Ok(())
    }

    /// Ends the session at time `now` and reports how long it ran. A time before the start counts as zero.
    pub fn stop(&mut self, now: f64) -> Result<Finished, RecordError> {
        if !now.is_finite() {
            return Err(RecordError::InvalidTime);
        }
        let session = self.session.take().ok_or(RecordError::NotRecording)?;
        let duration = (now - session.started).max(0.0);
        self.events.push(Event::Stopped { duration });
        Ok(Finished { duration, microphone: session.microphone, camera: session.camera })
    }

    /// Ends the session and drops what it recorded.
    pub fn discard(&mut self) -> Result<(), RecordError> {
        self.session.take().ok_or(RecordError::NotRecording)?;
        self.events.push(Event::Discarded);
        Ok(())
    }

    /// Ends the session because the owner's capture failed. Returns `false` when no session ran.
    pub fn fail(&mut self, reason: &str) -> bool {
        if self.session.take().is_none() {
            return false;
        }
        self.events.push(Event::Failed(reason.to_string()));
        true
    }

    /// The events since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

/// The shortest part a trim can keep, in seconds.
pub const MIN_TRIM: f64 = 0.1;

/// The kept part of a recording, in seconds from its first frame. The start and end handles stay at least `MIN_TRIM`
/// apart and inside the recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    start: f64,
    end: f64,
    length: f64,
}

impl Trim {
    /// The whole recording. `None` when the length is not finite or is shorter than `MIN_TRIM`.
    pub fn full(length: f64) -> Option<Trim> {
        Trim::new(0.0, length, length)
    }

    /// A trim from `start` to `end` of a recording `length` seconds long. `None` when a value is not finite, the
    /// start is before 0, the end is past the length, or the kept part is shorter than `MIN_TRIM`.
    pub fn new(start: f64, end: f64, length: f64) -> Option<Trim> {
        let finite = start.is_finite() && end.is_finite() && length.is_finite();
        if finite && start >= 0.0 && end <= length && end - start >= MIN_TRIM {
            Some(Trim { start, end, length })
        } else {
            None
        }
    }

    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn end(&self) -> f64 {
        self.end
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    /// The kept duration in seconds.
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    /// Moves the start handle to `t`, held between 0 and the end less `MIN_TRIM`. Returns `false` for a value that
    /// is not finite, and leaves the trim as it was.
    pub fn set_start(&mut self, t: f64) -> bool {
        if !t.is_finite() {
            return false;
        }
        self.start = t.max(0.0).min(self.end - MIN_TRIM);
        true
    }

    /// Moves the end handle to `t`, held between the start plus `MIN_TRIM` and the length. Returns `false` for a value
    /// that is not finite, and leaves the trim as it was.
    pub fn set_end(&mut self, t: f64) -> bool {
        if !t.is_finite() {
            return false;
        }
        self.end = t.min(self.length).max(self.start + MIN_TRIM);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_refused_when_unavailable_and_while_a_session_runs() {
        let mut rec = Recorder::new(false);
        assert_eq!(rec.start(0.0, false), Err(RecordError::Unavailable));
        rec.set_available(true);
        assert_eq!(rec.start(0.0, false), Ok(()));
        assert_eq!(rec.start(1.0, false), Err(RecordError::AlreadyRecording));
        assert!(rec.is_recording());
    }

    #[test]
    fn stop_reports_the_length_and_the_choices_and_queues_events() {
        let mut rec = Recorder::new(true);
        rec.set_camera(Some(CameraPosition::Front)).unwrap();
        rec.start(10.0, true).unwrap();
        assert!(rec.is_microphone_enabled());
        assert!(rec.is_camera_enabled());
        let done = rec.stop(12.5).unwrap();
        assert_eq!(
            done,
            Finished { duration: 2.5, microphone: true, camera: Some(CameraPosition::Front) }
        );
        assert!(!rec.is_recording());
        assert!(!rec.is_microphone_enabled());
        assert_eq!(
            rec.take_events(),
            vec![
                Event::Started { microphone: true, camera: Some(CameraPosition::Front) },
                Event::Stopped { duration: 2.5 },
            ]
        );
        assert!(rec.take_events().is_empty(), "events are drained once");
    }

    #[test]
    fn stop_without_a_session_is_refused_and_a_time_before_the_start_is_zero() {
        let mut rec = Recorder::new(true);
        assert_eq!(rec.stop(1.0), Err(RecordError::NotRecording));
        rec.start(5.0, false).unwrap();
        assert_eq!(rec.stop(f64::NAN), Err(RecordError::InvalidTime));
        assert!(rec.is_recording(), "a bad stop time keeps the session");
        assert_eq!(rec.stop(3.0).map(|f| f.duration), Ok(0.0));
    }

    #[test]
    fn discard_drops_the_session_and_reports_it() {
        let mut rec = Recorder::new(true);
        assert_eq!(rec.discard(), Err(RecordError::NotRecording));
        rec.start(0.0, false).unwrap();
        rec.discard().unwrap();
        assert!(!rec.is_recording());
        let events = rec.take_events();
        assert_eq!(events.last(), Some(&Event::Discarded));
    }

    #[test]
    fn camera_cannot_change_during_a_session_and_loss_of_availability_fails_it() {
        let mut rec = Recorder::new(true);
        rec.start(0.0, false).unwrap();
        assert_eq!(rec.set_camera(Some(CameraPosition::Back)), Err(RecordError::Busy));
        rec.set_available(false);
        assert!(!rec.is_recording());
        assert_eq!(
            rec.take_events().last(),
            Some(&Event::Failed("recording became unavailable".to_string()))
        );
        assert!(!rec.is_available());
        assert!(!rec.fail("late failure"), "no session is left to fail");
    }

    #[test]
    fn owner_failure_ends_the_session_with_a_reason() {
        let mut rec = Recorder::new(true);
        rec.start(0.0, true).unwrap();
        assert!(rec.fail("capture stopped"));
        assert!(!rec.is_recording());
        assert_eq!(rec.take_events().last(), Some(&Event::Failed("capture stopped".to_string())));
    }

    #[test]
    fn trim_validates_its_range() {
        assert!(Trim::full(0.05).is_none(), "shorter than the minimum");
        assert!(Trim::full(f64::NAN).is_none());
        assert_eq!(Trim::full(8.0).map(|t| t.duration()), Some(8.0));
        assert!(Trim::new(-1.0, 4.0, 8.0).is_none());
        assert!(Trim::new(2.0, 9.0, 8.0).is_none(), "end past the recording");
        assert!(Trim::new(2.0, 2.05, 8.0).is_none(), "kept part shorter than the minimum");
        assert!(Trim::new(2.0, 6.0, 8.0).is_some());
    }

    #[test]
    fn trim_handles_stay_apart_and_inside_the_recording() {
        let mut trim = Trim::new(2.0, 6.0, 8.0).unwrap();
        assert!(trim.set_start(7.0));
        assert!((trim.start() - (6.0 - MIN_TRIM)).abs() < 1e-9, "start is held MIN_TRIM before the end");
        assert!(trim.set_start(-3.0));
        assert_eq!(trim.start(), 0.0);
        assert!(trim.set_end(20.0));
        assert_eq!(trim.end(), 8.0, "end is held at the length");
        assert!(trim.set_end(0.0));
        assert!((trim.end() - MIN_TRIM).abs() < 1e-9, "end is held MIN_TRIM after the start");
        assert!(!trim.set_end(f64::INFINITY), "a value that is not finite is ignored");
        assert_eq!(trim.length(), 8.0);
    }
}

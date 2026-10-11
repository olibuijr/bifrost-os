//! Voice activity detection: a frame-by-frame decision on whether a mono signal holds speech, reported as the times
//! speech starts and ends. A speech run opens after a short run of loud frames and closes only after a longer run of
//! quiet frames, so a click does not start speech and a short pause does not end it. These are pure models over the
//! samples the caller provides; no audio device is opened here.

/// How quiet a signal may be and still count as speech. Higher sensitivity accepts quieter speech.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensitivityLevel {
    Low,
    Medium,
    High,
}

impl SensitivityLevel {
    /// The frame level (RMS, in dBFS) at or above which a frame counts as speech.
    pub fn threshold_db(self) -> f32 {
        match self {
            SensitivityLevel::Low => -35.0,
            SensitivityLevel::Medium => -45.0,
            SensitivityLevel::High => -55.0,
        }
    }
}

impl Default for SensitivityLevel {
    fn default() -> Self {
        SensitivityLevel::Medium
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetectionOptions {
    pub sensitivity: SensitivityLevel,
}

/// Consecutive loud frames needed to open a speech run.
pub const ONSET_FRAMES: u32 = 2;
/// Consecutive quiet frames needed to close a speech run.
pub const HANGOVER_FRAMES: u32 = 5;

/// A change of state. Times are in seconds from the first sample the detector was given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Speech starts at this time.
    Started(f64),
    /// Speech ends at this time, the first quiet frame of the run.
    Ended(f64),
}

#[derive(Clone, Debug)]
pub struct SpeechDetector {
    options: DetectionOptions,
    sample_rate: u32,
    elapsed: f64,
    speaking: bool,
    run: u32,
    run_start: f64,
}

impl SpeechDetector {
    /// A sample rate of zero is replaced by 16 kHz.
    pub fn new(options: DetectionOptions, sample_rate: u32) -> Self {
        SpeechDetector {
            options,
            sample_rate: if sample_rate == 0 { 16_000 } else { sample_rate },
            elapsed: 0.0,
            speaking: false,
            run: 0,
            run_start: 0.0,
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// RMS level of a frame in dBFS. An empty frame is silence and reads as negative infinity.
    pub fn level_db(frame: &[f32]) -> f32 {
        if frame.is_empty() {
            return f32::NEG_INFINITY;
        }
        let mean = frame.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / frame.len() as f64;
        (20.0 * mean.sqrt().log10()) as f32
    }

    /// Feeds one frame and returns the change of state it caused, if any. An empty frame is ignored.
    pub fn push(&mut self, frame: &[f32]) -> Option<Event> {
        if frame.is_empty() {
            return None;
        }
        let loud = Self::level_db(frame) >= self.options.sensitivity.threshold_db();
        let at = self.elapsed;
        self.elapsed += frame.len() as f64 / self.sample_rate as f64;
        if loud == self.speaking {
            self.run = 0;
            return None;
        }
        if self.run == 0 {
            self.run_start = at;
        }
        self.run += 1;
        let needed = if loud { ONSET_FRAMES } else { HANGOVER_FRAMES };
        if self.run < needed {
            return None;
        }
        self.speaking = loud;
        self.run = 0;
        Some(if loud {
            Event::Started(self.run_start)
        } else {
            Event::Ended(self.run_start)
        })
    }

    /// Feeds a whole signal in frames of `frame_len` samples. A trailing partial frame is not fed.
    pub fn process(&mut self, samples: &[f32], frame_len: usize) -> Vec<Event> {
        if frame_len == 0 {
            return Vec::new();
        }
        samples.chunks_exact(frame_len).filter_map(|f| self.push(f)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 ms at 16 kHz.
    const FRAME: usize = 160;

    fn tone(amp: f32, frames: usize) -> Vec<f32> {
        vec![amp; FRAME * frames]
    }

    fn medium() -> SpeechDetector {
        SpeechDetector::new(DetectionOptions::default(), 16_000)
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn silence_never_starts_speech() {
        let mut d = medium();
        assert!(d.process(&tone(0.0, 20), FRAME).is_empty());
        assert!(!d.is_speaking());
    }

    #[test]
    fn speech_starts_at_the_first_frame_of_the_loud_run_once_onset_is_met() {
        let mut d = medium();
        let mut signal = tone(0.0, 3);
        signal.extend(tone(0.5, 2));
        let events = d.process(&signal, FRAME);
        assert_eq!(events.len(), 1);
        match events[0] {
            Event::Started(t) => assert!(near(t, 0.03), "started at {t}"),
            other => panic!("expected a start, got {other:?}"),
        }
        assert!(d.is_speaking());
    }

    #[test]
    fn a_click_shorter_than_the_onset_is_ignored() {
        let mut d = medium();
        let mut signal = tone(0.5, 1);
        signal.extend(tone(0.0, 5));
        assert!(d.process(&signal, FRAME).is_empty());
        assert!(!d.is_speaking());
    }

    #[test]
    fn a_short_pause_inside_speech_does_not_end_it() {
        let mut d = medium();
        let mut signal = tone(0.5, 4);
        signal.extend(tone(0.0, 3));
        signal.extend(tone(0.5, 4));
        let events = d.process(&signal, FRAME);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Started(t) if near(t, 0.0)));
        assert!(d.is_speaking());
    }

    #[test]
    fn speech_ends_after_the_hangover_at_the_first_quiet_frame() {
        let mut d = medium();
        let mut signal = tone(0.5, 4);
        signal.extend(tone(0.0, 5));
        let events = d.process(&signal, FRAME);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Started(t) if near(t, 0.0)));
        assert!(matches!(events[1], Event::Ended(t) if near(t, 0.04)));
        assert!(!d.is_speaking());
    }

    #[test]
    fn sensitivity_decides_how_quiet_speech_can_be() {
        // RMS of 0.003 is about -50.5 dBFS: under the Medium threshold, over the High one
        let signal = tone(0.003, 4);
        let mut high = SpeechDetector::new(
            DetectionOptions {
                sensitivity: SensitivityLevel::High,
            },
            16_000,
        );
        let mut medium_detector = medium();
        assert_eq!(high.process(&signal, FRAME).len(), 1);
        assert!(medium_detector.process(&signal, FRAME).is_empty());
    }

    #[test]
    fn level_db_reads_known_frames() {
        assert_eq!(SpeechDetector::level_db(&[]), f32::NEG_INFINITY);
        assert!((SpeechDetector::level_db(&tone(0.1, 1)) - (-20.0)).abs() < 1e-4);
        assert!((SpeechDetector::level_db(&tone(1.0, 1))).abs() < 1e-4);
    }

    #[test]
    fn a_trailing_partial_frame_is_not_fed() {
        let mut d = medium();
        // 1.5 frames of loud audio: only one whole frame is fed, which is below the onset
        let signal = tone(0.5, 1).into_iter().chain(std::iter::repeat(0.5).take(FRAME / 2)).collect::<Vec<f32>>();
        assert!(d.process(&signal, FRAME).is_empty());
        assert!(!d.is_speaking());
    }
}

//! Speech synthesis models: an utterance (text plus delivery settings), a voice chosen by language tag, and a
//! synthesizer that speaks its queue in order and can pause, resume or stop. These are pure models; the audio
//! engine that turns text into sound is a separate service and is not part of this module.

use std::collections::VecDeque;

/// Speaking rate, from the slowest to the fastest voice. The default is the middle of the range.
pub const RATE_MIN: f32 = 0.0;
pub const RATE_MAX: f32 = 1.0;
pub const RATE_DEFAULT: f32 = 0.5;
/// Pitch multiplier. 1.0 is the voice's natural pitch.
pub const PITCH_MIN: f32 = 0.5;
pub const PITCH_MAX: f32 = 2.0;
pub const PITCH_DEFAULT: f32 = 1.0;
/// Output volume, 0 is silent and 1 is full.
pub const VOLUME_MIN: f32 = 0.0;
pub const VOLUME_MAX: f32 = 1.0;
pub const VOLUME_DEFAULT: f32 = 1.0;

/// Text to speak with the settings that shape it. Setters clamp into the ranges above, so a model value is always
/// one the engine accepts. A value that is not a number resets to the default.
#[derive(Clone, Debug, PartialEq)]
pub struct Utterance {
    pub text: String,
    rate: f32,
    pitch: f32,
    volume: f32,
    pre_delay: f32,
    post_delay: f32,
    /// BCP 47 tag of the wanted voice, such as `en-GB`. `None` lets the engine pick by the text's language.
    pub language: Option<String>,
}

impl Utterance {
    pub fn new(text: &str) -> Self {
        Utterance {
            text: text.into(),
            rate: RATE_DEFAULT,
            pitch: PITCH_DEFAULT,
            volume: VOLUME_DEFAULT,
            pre_delay: 0.0,
            post_delay: 0.0,
            language: None,
        }
    }
    pub fn rate(&self) -> f32 {
        self.rate
    }
    pub fn pitch(&self) -> f32 {
        self.pitch
    }
    pub fn volume(&self) -> f32 {
        self.volume
    }
    /// Seconds of silence before the utterance starts.
    pub fn pre_delay(&self) -> f32 {
        self.pre_delay
    }
    /// Seconds of silence after the utterance.
    pub fn post_delay(&self) -> f32 {
        self.post_delay
    }
    pub fn with_rate(mut self, rate: f32) -> Self {
        self.rate = clamp_or(rate, RATE_MIN, RATE_MAX, RATE_DEFAULT);
        self
    }
    pub fn with_pitch(mut self, pitch: f32) -> Self {
        self.pitch = clamp_or(pitch, PITCH_MIN, PITCH_MAX, PITCH_DEFAULT);
        self
    }
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = clamp_or(volume, VOLUME_MIN, VOLUME_MAX, VOLUME_DEFAULT);
        self
    }
    /// Negative and non-numeric delays become zero.
    pub fn with_pre_delay(mut self, seconds: f32) -> Self {
        self.pre_delay = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self
    }
    /// Negative and non-numeric delays become zero.
    pub fn with_post_delay(mut self, seconds: f32) -> Self {
        self.post_delay = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self
    }
    pub fn with_language(mut self, tag: &str) -> Self {
        self.language = Some(tag.into());
        self
    }
}

fn clamp_or(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// An installed voice: a stable identifier and the BCP 47 language it speaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Voice {
    pub identifier: String,
    pub language: String,
}

impl Voice {
    pub fn new(identifier: &str, language: &str) -> Self {
        Voice { identifier: identifier.into(), language: language.into() }
    }
}

/// Picks the voice for a language tag. An exact match (ignoring case and `_` versus `-`) wins. Otherwise the first
/// voice that shares the primary subtag is used, so `en-US` falls back to an `en-GB` voice. `None` when no voice fits.
pub fn select_voice<'a>(voices: &'a [Voice], language: &str) -> Option<&'a Voice> {
    let wanted = normalize_tag(language);
    if wanted.is_empty() {
        return None;
    }
    if let Some(v) = voices.iter().find(|v| normalize_tag(&v.language) == wanted) {
        return Some(v);
    }
    let primary = wanted.split('-').next().unwrap_or("");
    voices.iter().find(|v| normalize_tag(&v.language).split('-').next() == Some(primary))
}

/// Every voice whose language matches `language` by the same rule as `select_voice`, in the order given. An exact
/// match and a primary-subtag match are both listed, exact matches first. Empty when the tag is empty.
pub fn voices_for_language<'a>(voices: &'a [Voice], language: &str) -> Vec<&'a Voice> {
    let wanted = normalize_tag(language);
    if wanted.is_empty() {
        return Vec::new();
    }
    let primary = wanted.split('-').next().unwrap_or("").to_string();
    let mut exact = Vec::new();
    let mut loose = Vec::new();
    for v in voices {
        let tag = normalize_tag(&v.language);
        if tag == wanted {
            exact.push(v);
        } else if tag.split('-').next() == Some(primary.as_str()) {
            loose.push(v);
        }
    }
    exact.extend(loose);
    exact
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().replace('_', "-").to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Speaking,
    Paused,
}

/// Plays a queue of utterances in order. The front of the queue is the one being spoken. The engine calls
/// `finish_current` when an utterance ends, and the synthesizer moves to the next one or goes idle.
#[derive(Debug, Default)]
pub struct Synthesizer {
    queue: VecDeque<Utterance>,
    state: State,
}

impl Default for State {
    fn default() -> Self {
        State::Idle
    }
}

impl Synthesizer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn state(&self) -> State {
        self.state
    }
    /// The utterance being spoken, or waiting while paused.
    pub fn current(&self) -> Option<&Utterance> {
        self.queue.front()
    }
    /// Utterances not yet finished, the current one included.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }
    /// True while speech is running or paused with utterances still queued. Idle is false.
    pub fn is_speaking(&self) -> bool {
        self.state != State::Idle
    }
    /// True only in the paused state.
    pub fn is_paused(&self) -> bool {
        self.state == State::Paused
    }
    /// Adds an utterance to the end of the queue and starts speaking when the synthesizer was idle.
    pub fn speak(&mut self, utterance: Utterance) {
        self.queue.push_back(utterance);
        if self.state == State::Idle {
            self.state = State::Speaking;
        }
    }
    /// Pauses speech. Returns false when nothing was speaking.
    pub fn pause(&mut self) -> bool {
        if self.state == State::Speaking {
            self.state = State::Paused;
            true
        } else {
            false
        }
    }
    /// Resumes paused speech. Returns false when speech was not paused.
    pub fn resume(&mut self) -> bool {
        if self.state == State::Paused {
            self.state = State::Speaking;
            true
        } else {
            false
        }
    }
    /// Drops the whole queue. Returns false when there was nothing to stop.
    pub fn stop(&mut self) -> bool {
        let had_work = self.state != State::Idle || !self.queue.is_empty();
        self.queue.clear();
        self.state = State::Idle;
        had_work
    }
    /// The engine reports that the current utterance ended. Returns it, and the next one becomes current.
    /// Returns `None` when there was nothing to finish.
    pub fn finish_current(&mut self) -> Option<Utterance> {
        let done = self.queue.pop_front()?;
        if self.queue.is_empty() {
            self.state = State::Idle;
        }
        Some(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_clamped_into_their_ranges() {
        let u = Utterance::new("hi").with_rate(3.0).with_pitch(9.0).with_volume(-1.0);
        assert_eq!(u.rate(), RATE_MAX);
        assert_eq!(u.pitch(), PITCH_MAX);
        assert_eq!(u.volume(), VOLUME_MIN);
        let low = Utterance::new("hi").with_rate(-0.2).with_pitch(0.1);
        assert_eq!(low.rate(), RATE_MIN);
        assert_eq!(low.pitch(), PITCH_MIN);
    }

    #[test]
    fn non_numeric_settings_fall_back_to_defaults() {
        let u = Utterance::new("hi").with_rate(f32::NAN).with_pitch(f32::INFINITY).with_post_delay(f32::NAN);
        assert_eq!(u.rate(), RATE_DEFAULT);
        assert_eq!(u.pitch(), PITCH_DEFAULT);
        assert_eq!(u.post_delay(), 0.0);
        assert_eq!(Utterance::new("hi").with_post_delay(-4.0).post_delay(), 0.0);
        assert_eq!(Utterance::new("hi").with_post_delay(0.25).post_delay(), 0.25);
    }

    #[test]
    fn voice_matches_exactly_then_by_primary_subtag() {
        let voices = [Voice::new("us-1", "en-US"), Voice::new("gb-1", "en-GB"), Voice::new("is-1", "is-IS")];
        assert_eq!(select_voice(&voices, "en_gb").map(|v| v.identifier.as_str()), Some("gb-1"));
        assert_eq!(select_voice(&voices, "en-AU").map(|v| v.identifier.as_str()), Some("us-1"));
        assert_eq!(select_voice(&voices, "fr-FR"), None);
        assert_eq!(select_voice(&voices, "  "), None);
    }

    #[test]
    fn queue_speaks_in_order_and_goes_idle_at_the_end() {
        let mut s = Synthesizer::new();
        assert_eq!(s.state(), State::Idle);
        s.speak(Utterance::new("one"));
        s.speak(Utterance::new("two"));
        assert_eq!(s.state(), State::Speaking);
        assert_eq!(s.pending(), 2);
        assert_eq!(s.current().map(|u| u.text.as_str()), Some("one"));
        assert_eq!(s.finish_current().map(|u| u.text), Some("one".into()));
        assert_eq!(s.current().map(|u| u.text.as_str()), Some("two"));
        assert_eq!(s.state(), State::Speaking);
        s.finish_current();
        assert_eq!(s.state(), State::Idle);
        assert_eq!(s.finish_current(), None);
    }

    #[test]
    fn pause_and_resume_only_change_state_from_the_right_one() {
        let mut s = Synthesizer::new();
        assert!(!s.pause());
        assert!(!s.resume());
        s.speak(Utterance::new("long text"));
        assert!(s.pause());
        assert_eq!(s.state(), State::Paused);
        assert!(!s.pause());
        // a paused synthesizer keeps its queue; new text waits behind it
        s.speak(Utterance::new("next"));
        assert_eq!(s.state(), State::Paused);
        assert!(s.resume());
        assert_eq!(s.state(), State::Speaking);
    }

    #[test]
    fn stop_clears_the_queue_and_reports_whether_anything_was_running() {
        let mut s = Synthesizer::new();
        assert!(!s.stop());
        s.speak(Utterance::new("a"));
        s.speak(Utterance::new("b"));
        assert!(s.stop());
        assert_eq!(s.pending(), 0);
        assert_eq!(s.state(), State::Idle);
        assert!(!s.stop());
    }

    #[test]
    fn pre_delay_clamps_bad_values_like_post_delay() {
        assert_eq!(Utterance::new("a").pre_delay(), 0.0);
        assert_eq!(Utterance::new("a").with_pre_delay(0.75).pre_delay(), 0.75);
        assert_eq!(Utterance::new("a").with_pre_delay(-2.0).pre_delay(), 0.0);
        assert_eq!(Utterance::new("a").with_pre_delay(f32::NAN).pre_delay(), 0.0);
        assert_eq!(Utterance::new("a").with_pre_delay(0.3).post_delay(), 0.0);
    }

    #[test]
    fn speaking_and_paused_queries_follow_the_state() {
        let mut s = Synthesizer::new();
        assert!(!s.is_speaking() && !s.is_paused());
        s.speak(Utterance::new("one"));
        assert!(s.is_speaking() && !s.is_paused());
        s.pause();
        assert!(s.is_speaking() && s.is_paused());
        s.finish_current();
        assert!(!s.is_speaking() && !s.is_paused());
    }

    #[test]
    fn voices_for_language_lists_exact_matches_before_primary_matches() {
        let voices = vec![
            Voice::new("a", "en-US"),
            Voice::new("b", "is-IS"),
            Voice::new("c", "en-GB"),
            Voice::new("d", "en_US"),
        ];
        let ids: Vec<&str> = voices_for_language(&voices, "en-us").iter().map(|v| v.identifier.as_str()).collect();
        assert_eq!(ids, vec!["a", "d", "c"]);
        assert!(voices_for_language(&voices, "de").is_empty());
        assert!(voices_for_language(&voices, "  ").is_empty());
    }
}

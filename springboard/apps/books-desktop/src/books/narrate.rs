//! Read Aloud, Look Up and Translate for the reader, as pure models (no drawing, no I/O).
//!
//! Read Aloud queues the sentences of the chapter from the current page on, one speech utterance each. No voice
//! engine is installed on the PC, so nothing is spoken: each sentence is held for the time it takes to read it
//! aloud at the speech rate, and the highlight follows that timing. Look Up and Translate use the language
//! recogniser and the translation language availability; neither has data behind it yet, so their notices say so.

use super::store::Span;
use bifrost_app::controls::language::Language;
use bifrost_app::controls::speech::{State, Synthesizer, Utterance, RATE_DEFAULT};
use bifrost_app::controls::translation::{LanguageAvailability, TranslationError};

/// Shown on the page while a chapter is read aloud and no other notice is up.
pub const READ_ALOUD_NOTE: &str = "Reading aloud: no voice is installed on this PC, so sentences are highlighted at reading pace without sound.";

/// The language translations are made into: the language of the Books window.
pub const TRANSLATE_TARGET: &str = "en";

/// Words per minute at the slowest and the fastest speech rate.
const WPM_SLOW: f64 = 60.0;
const WPM_FAST: f64 = 240.0;
/// A sentence is never held for less than this many milliseconds, so a short one is still seen.
const MIN_MS: f64 = 600.0;

/// Milliseconds the utterance takes to read aloud at its rate (the default rate reads 150 words a minute).
pub fn duration_ms(u: &Utterance) -> f64 {
    let words = u.text.split_whitespace().count().max(1) as f64;
    let wpm = WPM_SLOW + (WPM_FAST - WPM_SLOW) * u.rate() as f64;
    (words * 60_000.0 / wpm).max(MIN_MS)
}

/// The sentences of `text` from char `from` on, each with its span in block `block`. A sentence ends at `.`, `!`,
/// `?` or an ellipsis that is followed by whitespace or the end of the text, so `3.5` does not end one. Whitespace
/// around a sentence is left out, and empty sentences are dropped.
pub fn sentence_spans(text: &str, block: usize, from: usize) -> Vec<(Span, String)> {
    let chars: Vec<char> = text.chars().collect();
    let first = from.min(chars.len());
    let mut start = first;
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for i in first..chars.len() {
        let ends = matches!(chars[i], '.' | '!' | '?' | '\u{2026}') && chars.get(i + 1).map_or(true, |c| c.is_whitespace());
        if ends {
            cuts.push((start, i + 1));
            start = i + 1;
        }
    }
    cuts.push((start, chars.len()));
    cuts.into_iter()
        .filter_map(|(a, b)| {
            let a = (a..b).find(|&k| !chars[k].is_whitespace())?;
            let b = (a..b).rev().find(|&k| !chars[k].is_whitespace())? + 1;
            let sentence: String = chars[a..b].iter().collect();
            Some((Span { block, offset: a, len: b - a }, sentence))
        })
        .collect()
}

/// The block index where the chapter that holds `block` ends (exclusive): the next table of contents start after
/// `block`, or the end of the book. `starts` are the table of contents block indices.
pub fn chapter_end(starts: &[usize], block: usize, blocks: usize) -> usize {
    starts.iter().copied().filter(|&s| s > block).min().unwrap_or(blocks)
}

/// What a tick did to the reading: nothing, the sentence moved on (its span is now spoken), or the chapter ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Same,
    Moved(Span),
    Done,
}

/// A chapter being read aloud. The synthesizer holds the queue; `ends_at` is the time (ms) the current sentence is
/// due to finish, and `left` is the time still owed on it while paused.
#[derive(Debug)]
pub struct Narration {
    sentences: Vec<(Span, f64)>,
    synth: Synthesizer,
    index: usize,
    ends_at: f64,
    left: f64,
}

impl Narration {
    /// Queues every sentence and starts the first one at time `t` (ms). None when there is nothing to read.
    pub fn start(sentences: Vec<(Span, String)>, t: f64) -> Option<Self> {
        let mut synth = Synthesizer::new();
        let mut timed = Vec::with_capacity(sentences.len());
        for (span, text) in sentences {
            let u = Utterance::new(&text).with_rate(RATE_DEFAULT);
            timed.push((span, duration_ms(&u)));
            synth.speak(u);
        }
        let ends_at = t + timed.first()?.1;
        Some(Self { sentences: timed, synth, index: 0, ends_at, left: 0.0 })
    }

    /// The span of the sentence being read (it stays the same while paused).
    pub fn current(&self) -> Option<Span> {
        self.sentences.get(self.index).map(|s| s.0)
    }

    pub fn is_paused(&self) -> bool {
        self.synth.is_paused()
    }

    /// When `tick` must run next (ms). None while paused, since nothing advances then.
    pub fn wake(&self) -> Option<f64> {
        (self.synth.state() == State::Speaking).then_some(self.ends_at)
    }

    /// Moves past every sentence whose time has come at `t` (ms). Returns the sentence now current when it moved.
    pub fn tick(&mut self, t: f64) -> Step {
        let mut moved = false;
        while self.synth.state() == State::Speaking && t >= self.ends_at {
            self.synth.finish_current();
            self.index += 1;
            let Some(&(_, ms)) = self.sentences.get(self.index) else { return Step::Done };
            self.ends_at += ms;
            moved = true;
        }
        match self.current() {
            Some(span) if moved => Step::Moved(span),
            _ => Step::Same,
        }
    }

    /// Pauses at time `t`, keeping the time left on the current sentence. False when it was not speaking.
    pub fn pause(&mut self, t: f64) -> bool {
        if !self.synth.pause() {
            return false;
        }
        self.left = (self.ends_at - t).max(0.0);
        true
    }

    /// Resumes at time `t` with the time that was left. False when it was not paused.
    pub fn resume(&mut self, t: f64) -> bool {
        if !self.synth.resume() {
            return false;
        }
        self.ends_at = t + self.left;
        true
    }
}

/// The name of a language for the person. The variant names are the English language names.
pub fn language_name(language: Language) -> String {
    format!("{language:?}")
}

/// Shortens `s` to `max` chars with an ellipsis when it is longer.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('\u{2026}');
        t
    }
}

/// Find menu: Look Up for `word`. The language is the one the recogniser finds in the sentence it sits in.
pub fn look_up_notice(word: &str, language: Option<Language>) -> String {
    let word = shorten(word.trim(), 40);
    match language {
        Some(l) => format!("Look Up \u{201c}{word}\u{201d} ({}): no dictionary is installed on this PC, so there is no definition.", language_name(l)),
        None => format!("Look Up \u{201c}{word}\u{201d}: no dictionary is installed on this PC, so there is no definition."),
    }
}

/// Find menu: Translate the sentence in `language` into `TRANSLATE_TARGET`. BifrOSt ships no language packs, so no
/// language is listed as supported and the session check reports why the pair cannot run.
pub fn translate_notice(language: Option<Language>) -> String {
    let packs = LanguageAvailability::new::<&str>(&[]);
    let Some(l) = language else {
        return format!("Translate: {}", TranslationError::UnableToIdentifyLanguage.error_description());
    };
    match packs.check_session(l.code(), TRANSLATE_TARGET) {
        Ok(()) => format!("Translate from {}: the language pair is ready.", language_name(l)),
        Err(e) => format!("Translate from {}: {}", language_name(l), e.error_description()),
    }
}

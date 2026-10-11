//! Speech recognition results. A recognizer reports a stretch of audio as a transcription: ranked alternative
//! readings with a confidence each, the time range they cover, and whether the result is final. A partial result is
//! replaced by the next result for the same stretch; a final result is kept. Contextual phrases raise the rank of
//! readings that contain them. These are pure models: the recognizer that produces the readings is not part of this
//! module.

use std::ops::Range;

/// Confidence added to a reading that contains a contextual phrase.
pub const CONTEXT_BOOST: f32 = 0.1;

/// One reading of the audio and the confidence the recognizer gave it. Confidence is clamped into 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Alternative {
    pub text: String,
    confidence: f32,
}

impl Alternative {
    /// Surrounding white space is trimmed from the text.
    pub fn new(text: &str, confidence: f32) -> Self {
        Alternative {
            text: text.trim().into(),
            confidence: clamp_unit(confidence),
        }
    }
    pub fn confidence(&self) -> f32 {
        self.confidence
    }
}

/// A recognizer's result for one stretch of audio. Times are in seconds from the start of the input.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcription {
    /// The audio the result covers.
    pub range: Range<f64>,
    /// A final result is never revised by a later one.
    pub is_final: bool,
    alternatives: Vec<Alternative>,
}

impl Transcription {
    /// Empty readings are dropped. The rest are ranked by confidence, highest first; equal confidences keep their order.
    pub fn new(range: Range<f64>, is_final: bool, alternatives: Vec<Alternative>) -> Self {
        let mut alternatives: Vec<Alternative> = alternatives.into_iter().filter(|a| !a.text.is_empty()).collect();
        rank(&mut alternatives);
        Transcription {
            range,
            is_final,
            alternatives,
        }
    }

    /// The readings, best first.
    pub fn alternatives(&self) -> &[Alternative] {
        &self.alternatives
    }

    /// The highest ranked reading, if there is one.
    pub fn best(&self) -> Option<&Alternative> {
        self.alternatives.first()
    }

    /// Raises every reading that contains one of the contextual phrases as whole words, ignoring case, by
    /// `CONTEXT_BOOST`, then ranks the readings again. A reading gets the boost once, however many phrases it holds.
    pub fn apply_context(&mut self, context: &ContextualStrings) {
        let phrases: Vec<Vec<String>> = context.phrases().map(words).filter(|p| !p.is_empty()).collect();
        for alt in &mut self.alternatives {
            let text = words(&alt.text);
            if phrases.iter().any(|p| contains_run(&text, p)) {
                alt.confidence = clamp_unit(alt.confidence + CONTEXT_BOOST);
            }
        }
        rank(&mut self.alternatives);
    }
}

/// Phrases that should be recognized even when they are not in the system vocabulary, grouped by a tag such as
/// `"contacts"`. A tag is for the app's own bookkeeping; recognition uses the phrases.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextualStrings {
    groups: Vec<(String, Vec<String>)>,
}

impl ContextualStrings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a phrase under a tag. Blank phrases are ignored.
    pub fn add(&mut self, tag: &str, phrase: &str) {
        let phrase = phrase.trim();
        if phrase.is_empty() {
            return;
        }
        match self.groups.iter_mut().find(|(t, _)| t == tag) {
            Some((_, list)) => list.push(phrase.into()),
            None => self.groups.push((tag.into(), vec![phrase.into()])),
        }
    }

    /// Every phrase, in the order it was added.
    pub fn phrases(&self) -> impl Iterator<Item = &str> {
        self.groups.iter().flat_map(|(_, list)| list.iter().map(|p| p.as_str()))
    }
}

/// The results of one recognition session, in order. A partial result replaces the partial result before it; a final
/// result, or anything after a final result, is appended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    results: Vec<Transcription>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, result: Transcription) {
        let replace = self.results.last().map_or(false, |last| !last.is_final);
        if replace {
            self.results.pop();
        }
        self.results.push(result);
    }

    pub fn results(&self) -> &[Transcription] {
        &self.results
    }

    /// The best readings of the final results, joined with spaces.
    pub fn final_text(&self) -> String {
        let parts: Vec<&str> = self
            .results
            .iter()
            .filter(|r| r.is_final)
            .filter_map(|r| r.best())
            .map(|a| a.text.as_str())
            .collect();
        parts.join(" ")
    }

    /// The final text followed by the best reading of the current partial result, if there is one.
    pub fn text(&self) -> String {
        let mut parts: Vec<&str> = self
            .results
            .iter()
            .filter(|r| r.is_final)
            .filter_map(|r| r.best())
            .map(|a| a.text.as_str())
            .collect();
        if let Some(partial) = self.results.last().filter(|r| !r.is_final).and_then(|r| r.best()) {
            parts.push(partial.text.as_str());
        }
        parts.join(" ")
    }
}

fn clamp_unit(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn rank(alternatives: &mut [Alternative]) {
    alternatives.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
}

/// Lower-cased words, split on anything that is not a letter, digit or apostrophe.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn contains_run(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_are_ranked_by_confidence_and_confidence_is_clamped() {
        let t = Transcription::new(
            0.0..1.0,
            false,
            vec![
                Alternative::new("a", -0.2),
                Alternative::new("b", 1.7),
                Alternative::new("c", 0.5),
            ],
        );
        let order: Vec<&str> = t.alternatives().iter().map(|a| a.text.as_str()).collect();
        assert_eq!(order, ["b", "c", "a"]);
        assert_eq!(t.best().map(|a| a.confidence()), Some(1.0));
        assert_eq!(t.alternatives()[2].confidence(), 0.0);
    }

    #[test]
    fn blank_readings_are_dropped_and_leave_no_best() {
        let t = Transcription::new(0.0..0.5, true, vec![Alternative::new("   ", 0.9), Alternative::new("", 0.99)]);
        assert!(t.alternatives().is_empty());
        assert!(t.best().is_none());
    }

    #[test]
    fn a_contextual_phrase_moves_a_close_reading_up_as_a_whole_word() {
        let mut t = Transcription::new(
            0.0..2.0,
            true,
            vec![Alternative::new("open the lampshade", 0.7), Alternative::new("open the lamp", 0.62)],
        );
        let mut ctx = ContextualStrings::new();
        ctx.add("devices", "Lamp");
        t.apply_context(&ctx);
        assert_eq!(t.best().map(|a| a.text.as_str()), Some("open the lamp"));
        // "lampshade" is not the phrase "lamp", so it keeps its own confidence
        assert_eq!(t.alternatives()[1].text, "open the lampshade");
        assert!((t.alternatives()[1].confidence() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn blank_contextual_phrases_are_ignored() {
        let mut ctx = ContextualStrings::new();
        ctx.add("tag", "   ");
        ctx.add("tag", "Bifrost");
        assert_eq!(ctx.phrases().collect::<Vec<_>>(), ["Bifrost"]);
    }

    #[test]
    fn a_session_replaces_the_partial_result_and_keeps_the_final_one() {
        let mut s = Session::new();
        s.push(Transcription::new(0.0..0.5, false, vec![Alternative::new("hel", 0.4)]));
        s.push(Transcription::new(0.0..1.0, false, vec![Alternative::new("hello wor", 0.5)]));
        assert_eq!(s.results().len(), 1);
        s.push(Transcription::new(0.0..1.2, true, vec![Alternative::new("hello world", 0.9)]));
        s.push(Transcription::new(1.2..1.5, false, vec![Alternative::new("again", 0.5)]));
        s.push(Transcription::new(1.2..1.8, false, vec![Alternative::new("again now", 0.6)]));
        assert_eq!(s.results().len(), 3);
        assert_eq!(s.final_text(), "hello world");
        assert_eq!(s.text(), "hello world again now");
    }

    #[test]
    fn a_final_result_is_never_replaced_by_a_later_one() {
        let mut s = Session::new();
        s.push(Transcription::new(0.0..1.0, true, vec![Alternative::new("first", 0.9)]));
        s.push(Transcription::new(1.0..2.0, false, vec![Alternative::new("second", 0.5)]));
        s.push(Transcription::new(1.0..2.0, true, vec![Alternative::new("second", 0.95)]));
        let texts: Vec<&str> = s.results().iter().filter_map(|r| r.best()).map(|a| a.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
        assert!(s.results().iter().all(|r| r.is_final));
        assert_eq!(s.text(), "first second");
    }
}

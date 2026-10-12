//! A text classifier that runs: the reference SDK's custom text model (`predictedLabel(for:)`,
//! `predictedLabels(forTokens:)`, `predictedLabelHypotheses(for:maximumCount:)`), built on [`crate::ml_model`].
//!
//! The text becomes a bag of words over a fixed vocabulary: one input value per vocabulary word, holding how often
//! the word occurs. A single dense layer maps those counts to one score per label, and a softmax turns the scores into
//! probabilities. So a classifier is a [`crate::ml_model::Model`] with a fixed input and output, plus the words and
//! labels that name them.
//!
//! Text is split into words at every character that is not alphanumeric and lowercased, so "Hi, hi!" counts "hi"
//! twice. A word outside the vocabulary is ignored. A text with no vocabulary word carries no evidence, so it has no
//! label (`None`, or an empty hypothesis list), instead of the label of the zero-score tie.
//!
//! Not mirrored: the trained-model file formats of the reference SDK (a model is built in code from its weights), and
//! the word tagger, which labels each word of a text rather than the whole text.

use crate::features::{DictionaryProvider, FeatureProvider, FeatureValue};
use crate::ml_model::{Model, ModelError};
use crate::ml_tensor::Tensor;

/// The input the model reads: one count per vocabulary word.
const INPUT: &str = "wordCounts";
/// The output the model writes: one probability per label.
const OUTPUT: &str = "labelProbabilities";

/// Why a classifier could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassifierError {
    /// The vocabulary or the label list is empty.
    Empty,
    /// A vocabulary entry is empty or holds a character that is not alphanumeric, so no text could produce it.
    BadWord(String),
    /// A vocabulary word or a label appears twice.
    Duplicate(String),
    /// The weights or the bias do not match the vocabulary and label counts.
    Model(ModelError),
}

/// Lowercase words of a text, split at every character that is not alphanumeric. Empty pieces are dropped.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The index of the largest value; a tie goes to the earlier index.
fn best_index(values: &[f64]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, v) in values.iter().enumerate() {
        if best.map_or(true, |b| *v > values[b]) {
            best = Some(i);
        }
    }
    best
}

/// A text classifier: a vocabulary, the labels it predicts, and the model that scores them.
#[derive(Clone, Debug)]
pub struct TextClassifier {
    vocabulary: Vec<String>,
    labels: Vec<String>,
    model: Model,
}

impl TextClassifier {
    /// Builds a classifier from its words, its labels and the trained weights. `weights` holds one row per vocabulary
    /// word and one column per label, so `weights[w * labels.len() + l]` is the weight of word `w` for label `l`.
    /// `bias` holds one value per label.
    pub fn new(vocabulary: &[&str], labels: &[&str], weights: Vec<f32>, bias: Vec<f32>) -> Result<Self, ClassifierError> {
        if vocabulary.is_empty() || labels.is_empty() {
            return Err(ClassifierError::Empty);
        }
        let mut words_seen: Vec<String> = Vec::with_capacity(vocabulary.len());
        for word in vocabulary {
            let lower = word.to_lowercase();
            if lower.is_empty() || !lower.chars().all(char::is_alphanumeric) {
                return Err(ClassifierError::BadWord(word.to_string()));
            }
            if words_seen.contains(&lower) {
                return Err(ClassifierError::Duplicate(lower));
            }
            words_seen.push(lower);
        }
        let mut labels_seen: Vec<String> = Vec::with_capacity(labels.len());
        for label in labels {
            if label.trim().is_empty() {
                return Err(ClassifierError::Empty);
            }
            if labels_seen.iter().any(|l| l.as_str() == *label) {
                return Err(ClassifierError::Duplicate(label.to_string()));
            }
            labels_seen.push(label.to_string());
        }
        let model = Model::new(INPUT, words_seen.len())
            .map_err(ClassifierError::Model)?
            .dense(labels_seen.len(), weights, bias)
            .map_err(ClassifierError::Model)?
            .softmax()
            .with_output(OUTPUT);
        Ok(TextClassifier { vocabulary: words_seen, labels: labels_seen, model })
    }

    /// The vocabulary in lowercase, in the order the weights use.
    pub fn vocabulary(&self) -> &[String] {
        &self.vocabulary
    }

    /// The labels in the order the weights use.
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// The word counts of a text, one per vocabulary word.
    fn counts(&self, text: &str) -> Vec<f32> {
        let mut counts = vec![0.0_f32; self.vocabulary.len()];
        for word in words(text) {
            if let Some(i) = self.vocabulary.iter().position(|w| *w == word) {
                counts[i] += 1.0;
            }
        }
        counts
    }

    /// The probability of every label, in label order. `None` when no vocabulary word occurs in the text.
    fn probabilities(&self, text: &str) -> Option<Vec<f64>> {
        let counts = self.counts(text);
        if counts.iter().all(|c| *c == 0.0) {
            return None;
        }
        let tensor = Tensor::new(&[counts.len()], counts).ok()?;
        let input = DictionaryProvider::new().with(INPUT, FeatureValue::Tensor(tensor));
        let output = self.model.predict(&input).ok()?;
        let values = output.value(OUTPUT).and_then(FeatureValue::as_tensor)?;
        Some(values.values().iter().map(|p| f64::from(*p)).collect())
    }

    /// `predictedLabel(for:)`: the most probable label of the text. A tie goes to the label listed first.
    pub fn predicted_label(&self, text: &str) -> Option<&str> {
        let p = self.probabilities(text)?;
        let best = best_index(&p)?;
        Some(self.labels[best].as_str())
    }

    /// `predictedLabels(forTokens:)`: the label of each token, `None` where the token has no vocabulary word.
    pub fn predicted_labels(&self, tokens: &[&str]) -> Vec<Option<&str>> {
        tokens.iter().map(|t| self.predicted_label(t)).collect()
    }

    /// `predictedLabelHypotheses(for:maximumCount:)`: up to `maximum_count` labels with their probabilities, most
    /// probable first. Equal probabilities keep the listed order. Empty when the text has no vocabulary word.
    pub fn hypotheses(&self, text: &str, maximum_count: usize) -> Vec<(&str, f64)> {
        let Some(p) = self.probabilities(text) else { return Vec::new() };
        let mut order: Vec<usize> = (0..p.len()).collect();
        order.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(b)));
        order.into_iter().take(maximum_count).map(|i| (self.labels[i].as_str(), p[i])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two words, two labels. "hi" scores +2 for greeting and -2 for farewell; "bye" the other way round.
    fn greeter() -> TextClassifier {
        TextClassifier::new(&["hi", "bye"], &["greeting", "farewell"], vec![2.0, -2.0, -2.0, 2.0], vec![0.0, 0.0]).unwrap()
    }

    /// softmax over the two scores (2, -2) gives 1 / (1 + e^-4) for the higher one.
    const TOP: f64 = 0.982_013_79;

    #[test]
    fn words_are_lowercase_runs_of_letters_and_digits() {
        assert_eq!(words("Hi, hi! It's 2 o'clock"), vec!["hi", "hi", "it", "s", "2", "o", "clock"]);
        assert!(words("  ...  ").is_empty());
    }

    #[test]
    fn the_label_with_the_most_matching_words_wins() {
        let c = greeter();
        assert_eq!(c.predicted_label("Hi, hi!"), Some("greeting"));
        assert_eq!(c.predicted_label("bye bye"), Some("farewell"));
    }

    #[test]
    fn probabilities_come_from_the_softmax_of_the_word_counts() {
        let c = greeter();
        // hi twice and bye once: greeting scores 2 * 2 + 1 * -2 = 2, farewell -2 * 2 + 1 * 2 = -2.
        let hyp = c.hypotheses("hi hi bye", 2);
        assert_eq!(hyp[0].0, "greeting");
        assert!((hyp[0].1 - TOP).abs() < 1e-4, "got {}", hyp[0].1);
        assert_eq!(hyp[1].0, "farewell");
        assert!((hyp[0].1 + hyp[1].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn words_outside_the_vocabulary_are_ignored() {
        let c = greeter();
        assert_eq!(c.predicted_label("hello there"), None);
        assert!(c.hypotheses("hello there", 3).is_empty());
        // "hi" alone: the same scores as "hi hi" with one count, so the same top probability as bye above.
        let hyp = c.hypotheses("hi there", 1);
        assert_eq!(hyp[0].0, "greeting");
        assert!((hyp[0].1 - TOP).abs() < 1e-4);
    }

    #[test]
    fn hypotheses_are_limited_and_empty_for_a_zero_count() {
        let c = greeter();
        assert_eq!(c.hypotheses("bye", 1).len(), 1);
        assert_eq!(c.hypotheses("bye", 1)[0].0, "farewell");
        assert!(c.hypotheses("bye", 0).is_empty());
        assert_eq!(c.hypotheses("bye", 9).len(), 2);
    }

    #[test]
    fn a_tie_goes_to_the_label_listed_first() {
        let c = TextClassifier::new(&["hi"], &["a", "b"], vec![0.0, 0.0], vec![0.0, 0.0]).unwrap();
        assert_eq!(c.predicted_label("hi"), Some("a"));
        let hyp = c.hypotheses("hi", 2);
        assert_eq!(hyp[0].0, "a");
        assert!((hyp[0].1 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn tokens_get_one_label_each_or_none() {
        let c = greeter();
        assert_eq!(c.predicted_labels(&["hi", "xyz", "bye"]), vec![Some("greeting"), None, Some("farewell")]);
    }

    #[test]
    fn construction_checks_words_labels_and_weights() {
        assert_eq!(TextClassifier::new(&[], &["a"], vec![], vec![0.0]).unwrap_err(), ClassifierError::Empty);
        assert_eq!(
            TextClassifier::new(&["two words"], &["a"], vec![1.0], vec![0.0]).unwrap_err(),
            ClassifierError::BadWord("two words".into())
        );
        assert_eq!(
            TextClassifier::new(&["Hi", "hi"], &["a"], vec![1.0, 1.0], vec![0.0]).unwrap_err(),
            ClassifierError::Duplicate("hi".into())
        );
        assert_eq!(
            TextClassifier::new(&["hi"], &["a", "a"], vec![1.0, 1.0], vec![0.0, 0.0]).unwrap_err(),
            ClassifierError::Duplicate("a".into())
        );
        // Two words and two labels need four weights; one is missing.
        assert_eq!(
            TextClassifier::new(&["hi", "bye"], &["a", "b"], vec![1.0, 1.0, 1.0], vec![0.0, 0.0]).unwrap_err(),
            ClassifierError::Model(ModelError::LayerSize)
        );
    }

    #[test]
    fn vocabulary_and_labels_are_kept_in_order_and_lowercased() {
        let c = TextClassifier::new(&["Hi", "BYE"], &["greeting", "farewell"], vec![0.0; 4], vec![0.0; 2]).unwrap();
        assert_eq!(c.vocabulary(), ["hi", "bye"]);
        assert_eq!(c.labels(), ["greeting", "farewell"]);
    }
}

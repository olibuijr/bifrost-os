//! Parity sdk-complete-25: headless tests for the analysis runtime, the sentiment preset of the text classifier, the
//! vector-text reader of the embedding, and the shared result log of the sound analyzer. Each test drives the public
//! API and asserts concrete values. Nothing here opens an audio device, a file or the network.

use bifrost_ui::analyzer_input::{AnalyzerInput, AudioFormat};
use bifrost_ui::embedding::{DistanceType, Embedding, EmbeddingError};
use bifrost_ui::features::{DictionaryProvider, FeatureValue};
use bifrost_ui::ml_model::Model;
use bifrost_ui::ml_runtime::{AnalysisRuntime, RuntimeError};
use bifrost_ui::ml_tensor::Tensor;
use bifrost_ui::sound_analyzer::{
    ResultLog, SoundAnalysisError, SoundAnalyzer, SoundClassifier, SoundRequest, SoundResultsObserving,
};
use bifrost_ui::text_classifier::{ClassifierError, TextClassifier};

fn doubler() -> Model {
    // The first input element is doubled; the second feeds nothing.
    Model::new("x", 2)
        .unwrap()
        .dense(2, vec![2.0, 0.0, 0.0, 0.0], vec![0.0, 0.0])
        .unwrap()
        .with_output("y")
}

fn input(values: &[f32]) -> DictionaryProvider {
    let tensor = Tensor::new(&[values.len()], values.to_vec()).unwrap();
    DictionaryProvider::new().with("x", FeatureValue::Tensor(tensor))
}

fn output(provider: &DictionaryProvider) -> Vec<f32> {
    provider.value("y").and_then(FeatureValue::as_tensor).unwrap().values().to_vec()
}

#[test]
fn a_request_runs_the_model_loaded_under_its_name() {
    let mut runtime = AnalysisRuntime::new();
    runtime.load("doubler", doubler()).unwrap();
    let out = runtime.predict("doubler", &input(&[3.0, 5.0])).unwrap();
    assert_eq!(output(&out), vec![6.0, 0.0]);
}

#[test]
fn a_request_for_a_model_that_is_not_loaded_is_refused_with_its_name() {
    let mut runtime = AnalysisRuntime::new();
    let err = runtime.predict("missing", &input(&[1.0, 1.0])).unwrap_err();
    assert_eq!(err, RuntimeError::UnknownModel("missing".into()));
    assert_eq!(err.to_string(), "no model is loaded as `missing`");
}

#[test]
fn a_loaded_name_refuses_a_second_model_until_it_is_unloaded() {
    let mut runtime = AnalysisRuntime::new();
    runtime.load("doubler", doubler()).unwrap();
    assert_eq!(runtime.load("doubler", doubler()), Err(RuntimeError::AlreadyLoaded("doubler".into())));
    assert!(runtime.unload("doubler"));
    assert!(!runtime.unload("doubler"), "a second unload finds nothing");
    assert!(runtime.load("doubler", doubler()).is_ok());
}

#[test]
fn every_request_routed_to_a_model_is_counted_even_when_the_input_is_refused() {
    let mut runtime = AnalysisRuntime::new();
    runtime.load("doubler", doubler()).unwrap();
    runtime.load("idle", doubler()).unwrap();
    runtime.predict("doubler", &input(&[1.0, 1.0])).unwrap();
    assert!(runtime.predict("doubler", &input(&[1.0])).is_err(), "one value is not two");
    assert_eq!(runtime.request_count("doubler"), Some(2));
    assert_eq!(runtime.request_count("idle"), Some(0));
    assert_eq!(runtime.request_count("missing"), None);
}

#[test]
fn loaded_names_are_alphabetical_and_follow_unloading() {
    let mut runtime = AnalysisRuntime::new();
    runtime.load("sentiment", doubler()).unwrap();
    runtime.load("classifier", doubler()).unwrap();
    assert_eq!(runtime.loaded_names(), vec!["classifier", "sentiment"]);
    runtime.unload("sentiment");
    assert!(!runtime.is_loaded("sentiment"));
    assert_eq!(runtime.loaded_names(), vec!["classifier"]);
}

#[test]
fn sentiment_scores_follow_the_positive_and_negative_word_lists() {
    let classifier = TextClassifier::sentiment(&["good", "great"], &["bad", "awful"]).unwrap();
    assert_eq!(classifier.predicted_label("good great, truly good"), Some("positive"));
    assert_eq!(classifier.predicted_label("awful, bad service"), Some("negative"));
}

#[test]
fn sentiment_gives_a_tie_to_the_positive_label_and_no_evidence_no_label() {
    let classifier = TextClassifier::sentiment(&["good"], &["bad"]).unwrap();
    // One positive and one negative word cancel out, so the two labels tie at one half.
    let hypotheses = classifier.hypotheses("good bad", 2);
    assert_eq!(hypotheses.len(), 2);
    assert_eq!(hypotheses[0].0, "positive");
    assert!((hypotheses[0].1 - 0.5).abs() < 1e-6);
    assert_eq!(classifier.predicted_label("good bad"), Some("positive"));
    assert_eq!(classifier.predicted_label("the weather"), None);
    assert!(classifier.hypotheses("the weather", 2).is_empty());
}

#[test]
fn sentiment_refuses_empty_lists_and_a_word_in_both_lists() {
    assert_eq!(TextClassifier::sentiment(&[], &[]).unwrap_err(), ClassifierError::Empty);
    assert_eq!(
        TextClassifier::sentiment(&["fine"], &["fine"]).unwrap_err(),
        ClassifierError::Duplicate("fine".into())
    );
    assert!(TextClassifier::sentiment(&["fine"], &[]).is_ok(), "one list alone is a valid model");
}

#[test]
fn a_vector_text_file_gives_the_dimension_from_its_first_entry() {
    let text = "# two-dimensional vectors\ncat 1 0\n\ndog 0 1\n";
    let e = Embedding::from_vector_text(text).unwrap();
    assert_eq!(e.dimension(), 2);
    assert_eq!(e.vocabulary_size(), 2);
    assert_eq!(e.vector("CAT"), Some(&[1.0f32, 0.0][..]));
    let d = e.distance_between("cat", "dog", DistanceType::Cosine).unwrap();
    assert!((d - 1.0).abs() < 1e-9, "orthogonal vectors are distance one apart");
}

#[test]
fn a_vector_text_entry_of_another_length_is_reported_by_its_line() {
    let text = "cat 1 0\ndog 0 1\nbird 1 1 1\n";
    assert_eq!(Embedding::from_vector_text(text).unwrap_err(), EmbeddingError::BadLine { line: 3 });
}

#[test]
fn a_vector_text_entry_with_a_word_for_a_number_is_reported_by_its_line() {
    let text = "cat 1 0\ndog 0 many\n";
    assert_eq!(Embedding::from_vector_text(text).unwrap_err(), EmbeddingError::BadLine { line: 2 });
    let not_finite = "cat 1 NaN\n";
    assert_eq!(Embedding::from_vector_text(not_finite).unwrap_err(), EmbeddingError::BadLine { line: 1 });
}

#[test]
fn a_term_without_components_is_not_an_entry() {
    assert_eq!(Embedding::from_vector_text("cat\n").unwrap_err(), EmbeddingError::BadLine { line: 1 });
}

#[test]
fn a_vector_text_with_no_entries_has_no_dimension() {
    assert_eq!(Embedding::from_vector_text("# only a comment\n\n").unwrap_err(), EmbeddingError::NoEntries);
}

/// A classifier that returns the same labels for every window.
struct Fixed(Vec<(&'static str, f64)>);

impl SoundClassifier for Fixed {
    fn classify(&mut self, _window: &[f32], _sample_rate: u32) -> Result<Vec<(String, f64)>, SoundAnalysisError> {
        Ok(self.0.iter().map(|(l, c)| (l.to_string(), *c)).collect())
    }
}

/// A classifier that fails on every window.
struct Broken;

impl SoundClassifier for Broken {
    fn classify(&mut self, _window: &[f32], _sample_rate: u32) -> Result<Vec<(String, f64)>, SoundAnalysisError> {
        Err(SoundAnalysisError::OperationFailed)
    }
}

#[test]
fn the_result_log_keeps_each_window_result_in_arrival_order() {
    // Four frames per second and one-second windows: eight samples are two windows, at 0 s and 1 s.
    let mut analyzer = SoundAnalyzer::new(AudioFormat::new(4, 1));
    let log = ResultLog::new();
    let request = SoundRequest::new("classifier").with_window_duration(1.0);
    analyzer.add(request, Box::new(Fixed(vec![("siren", 0.9)])), log.observer());
    analyzer.analyze(&AnalyzerInput::new(0.0, AudioFormat::new(4, 1), &[0.0; 8]));
    let results = log.results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].time_range().start(), 0.0);
    assert_eq!(results[1].time_range().start(), 1.0);
    assert_eq!(results[1].time_range().duration(), 1.0);
    assert_eq!(results[0].highest().map(|c| c.identifier()), Some("siren"));
    assert!(!log.completed());
}

#[test]
fn the_result_log_records_completion_and_stops_later_results() {
    let mut analyzer = SoundAnalyzer::new(AudioFormat::new(4, 1));
    let log = ResultLog::new();
    analyzer.add(SoundRequest::new("c").with_window_duration(1.0), Box::new(Fixed(vec![("a", 1.0)])), log.observer());
    analyzer.complete_analysis();
    analyzer.analyze(&AnalyzerInput::new(0.0, AudioFormat::new(4, 1), &[0.0; 8]));
    assert!(log.completed());
    assert!(log.results().is_empty(), "a completed request ignores later audio");
}

#[test]
fn the_result_log_records_the_classifier_failure_that_stops_a_request() {
    let mut analyzer = SoundAnalyzer::new(AudioFormat::new(4, 1));
    let log = ResultLog::new();
    analyzer.add(SoundRequest::new("c").with_window_duration(1.0), Box::new(Broken), log.observer());
    analyzer.analyze(&AnalyzerInput::new(0.0, AudioFormat::new(4, 1), &[0.0; 8]));
    assert_eq!(log.errors(), vec![SoundAnalysisError::OperationFailed]);
    assert!(log.results().is_empty());
}

#[test]
fn the_result_log_records_a_block_in_the_wrong_format() {
    let mut analyzer = SoundAnalyzer::new(AudioFormat::new(4, 1));
    let log = ResultLog::new();
    analyzer.add(SoundRequest::new("c").with_window_duration(1.0), Box::new(Fixed(vec![("a", 1.0)])), log.observer());
    analyzer.analyze(&AnalyzerInput::new(0.0, AudioFormat::new(8, 1), &[0.0; 8]));
    assert_eq!(log.errors(), vec![SoundAnalysisError::InvalidFormat]);
    assert!(log.results().is_empty());
}

#[test]
fn clones_of_the_result_log_read_the_same_records() {
    let log = ResultLog::new();
    let reader = log.clone();
    let mut observer = log.observer();
    observer.request_did_complete();
    assert!(reader.completed());
}

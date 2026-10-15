//! Parity sdk-rest-05: headless tests of the streaming sound analyzer (bifrost_ui::sound_stream). The classifier is a
//! closure, so no model, device or clock is involved.

use bifrost_ui::sound_classify::{ClassifyRequest, SoundError};
use bifrost_ui::sound_stream::{StreamAnalyzer, StreamFormat};

/// Ten frames per second, so a one-second window is ten samples and a 50 percent overlap hops by five.
const MONO_10HZ: StreamFormat = StreamFormat { sample_rate: 10, channels: 1 };

fn request() -> ClassifyRequest {
    ClassifyRequest::new(1.0, 0.5, vec!["loud".to_string()]).unwrap()
}

/// Labels a window "loud" with confidence 0.9 when its first sample is positive, and "quiet" with 0.4 otherwise.
fn label_by_first_sample(window: &[f32]) -> Vec<(String, f64)> {
    if window[0] > 0.0 {
        vec![("loud".to_string(), 0.9)]
    } else {
        vec![("quiet".to_string(), 0.4)]
    }
}

fn signal(frames: usize) -> Vec<f32> {
    (0..frames).map(|i| if (i / 5) % 2 == 0 { 0.5 } else { -0.5 }).collect()
}

#[test]
fn a_stream_fed_in_pieces_gives_the_same_windows_as_one_buffer() {
    let audio = signal(30);
    let mut whole = StreamAnalyzer::new(MONO_10HZ, &request(), label_by_first_sample).unwrap();
    whole.analyze(&audio, 0).unwrap();
    let whole = whole.complete_analysis();

    let mut pieces = StreamAnalyzer::new(MONO_10HZ, &request(), label_by_first_sample).unwrap();
    for (start, chunk) in audio.chunks(7).enumerate().map(|(i, c)| (i * 7, c)) {
        pieces.analyze(chunk, start as u64).unwrap();
    }
    let pieces = pieces.complete_analysis();

    assert_eq!(whole.len(), 5, "thirty frames with one-second windows and a five-frame hop give five windows");
    assert_eq!(pieces, whole);
}

#[test]
fn the_windows_cover_the_expected_time_spans() {
    let mut stream = StreamAnalyzer::new(MONO_10HZ, &request(), label_by_first_sample).unwrap();
    stream.analyze(&signal(30), 0).unwrap();
    let results = stream.complete_analysis();
    assert_eq!(results[0].time_range, 0.0..1.0);
    assert_eq!(results[1].time_range, 0.5..1.5);
    assert_eq!(results[4].time_range, 2.0..3.0);
}

#[test]
fn a_buffer_reports_only_the_windows_it_completed() {
    let mut stream = StreamAnalyzer::new(MONO_10HZ, &request(), label_by_first_sample).unwrap();
    assert!(stream.analyze(&signal(6), 0).unwrap().is_empty(), "six frames are not yet a full window");
    let done = stream.analyze(&signal(4), 6).unwrap();
    assert_eq!(done.len(), 1, "the tenth frame completes the first window");
    assert_eq!(done[0].time_range, 0.0..1.0);
}

#[test]
fn a_stream_shorter_than_one_window_gives_no_results() {
    let mut stream = StreamAnalyzer::new(MONO_10HZ, &request(), label_by_first_sample).unwrap();
    stream.analyze(&signal(9), 0).unwrap();
    assert!(stream.complete_analysis().is_empty());
}

#[test]
fn stereo_frames_are_averaged_to_mono_before_classification() {
    let stereo = StreamFormat { sample_rate: 10, channels: 2 };
    let confidence_of_first_window = |w: &[f32]| vec![("level".to_string(), w[0] as f64)];
    let mut stream = StreamAnalyzer::new(stereo, &request(), confidence_of_first_window).unwrap();
    let interleaved: Vec<f32> = (0..10).flat_map(|_| [1.0_f32, 0.0]).collect();
    let done = stream.analyze(&interleaved, 0).unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].classifications[0].confidence, 0.5);
}

#[test]
fn a_buffer_that_is_not_whole_frames_is_refused() {
    let stereo = StreamFormat { sample_rate: 10, channels: 2 };
    let mut stream = StreamAnalyzer::new(stereo, &request(), label_by_first_sample).unwrap();
    assert_eq!(stream.analyze(&[0.0; 3], 0), Err(SoundError::InvalidFormat));
}

#[test]
fn a_buffer_that_does_not_follow_on_from_the_last_is_refused() {
    let mut stream = StreamAnalyzer::new(MONO_10HZ, &request(), label_by_first_sample).unwrap();
    stream.analyze(&signal(4), 0).unwrap();
    assert_eq!(stream.analyze(&signal(4), 5), Err(SoundError::OperationFailed));
    assert_eq!(stream.analyze(&signal(4), 3), Err(SoundError::OperationFailed));
}

#[test]
fn a_stream_with_no_sample_rate_or_no_channels_is_refused() {
    let no_rate = StreamFormat { sample_rate: 0, channels: 1 };
    let no_channels = StreamFormat { sample_rate: 10, channels: 0 };
    assert!(matches!(StreamAnalyzer::new(no_rate, &request(), label_by_first_sample), Err(SoundError::InvalidFormat)));
    assert!(matches!(StreamAnalyzer::new(no_channels, &request(), label_by_first_sample), Err(SoundError::InvalidFormat)));
}

#[test]
fn a_classifier_confidence_outside_the_unit_range_fails_the_stream() {
    let broken = |_: &[f32]| vec![("loud".to_string(), 1.5)];
    let mut stream = StreamAnalyzer::new(MONO_10HZ, &request(), broken).unwrap();
    assert_eq!(stream.analyze(&signal(10), 0), Err(SoundError::OperationFailed));
}

//! Parity sdk-complete-07: headless tests for the analyzer input sequences (file blocks and microphone blocks).
//! No test opens a capture device or a file.

use bifrost_ui::analyzer_input::{
    AnalyzerInputConverter, AudioFormat, FileSequence, InputSequence, MicrophoneSequence, ANALYSIS_FORMAT,
};

#[test]
fn a_file_sequence_times_each_block_from_its_start() {
    // 1600 samples at 16 kHz is 0.1 s: two blocks of 800 frames, the second starting 0.05 s later.
    let samples = vec![0.25f32; 1600];
    let mut blocks = FileSequence::new(1.0, ANALYSIS_FORMAT, &samples, 800).unwrap();
    let first = blocks.next_input().unwrap();
    let second = blocks.next_input().unwrap();
    assert_eq!((first.start(), first.frames()), (1.0, 800));
    assert!((second.start() - 1.05).abs() < 1e-9);
    assert_eq!(second.frames(), 800);
    assert_eq!(blocks.next_input(), None);
}

#[test]
fn the_last_file_block_may_be_short() {
    let samples = vec![0.0f32; 1000];
    let mut blocks = FileSequence::new(0.0, ANALYSIS_FORMAT, &samples, 400).unwrap();
    let lengths: Vec<usize> = std::iter::from_fn(|| blocks.next_input()).map(|b| b.frames()).collect();
    assert_eq!(lengths, vec![400, 400, 200]);
}

#[test]
fn a_file_sequence_refuses_zero_frame_blocks_and_drops_a_partial_frame() {
    assert!(FileSequence::new(0.0, ANALYSIS_FORMAT, &[0.0; 10], 0).is_none());
    // Stereo with five samples: two whole frames, the fifth sample is dropped.
    let stereo = AudioFormat::new(16_000, 2);
    let mut blocks = FileSequence::new(0.0, stereo, &[0.1, 0.2, 0.3, 0.4, 0.5], 1).unwrap();
    assert_eq!(blocks.frame_count(), 2);
    assert_eq!(blocks.next_input().unwrap().frames(), 1);
    assert_eq!(blocks.next_input().unwrap().frames(), 1);
    assert_eq!(blocks.next_input(), None);
}

#[test]
fn microphone_blocks_come_out_in_capture_order() {
    let mut mic = MicrophoneSequence::new(4);
    assert!(mic.push(0.0, ANALYSIS_FORMAT, &[0.1; 160]));
    assert!(mic.push(0.1, ANALYSIS_FORMAT, &[0.2; 160]));
    assert_eq!(mic.pending(), 2);
    assert_eq!(mic.next_input().unwrap().start(), 0.0);
    assert_eq!(mic.next_input().unwrap().start(), 0.1);
    assert_eq!(mic.next_input(), None);
}

#[test]
fn a_full_microphone_queue_refuses_blocks_and_counts_them() {
    let mut mic = MicrophoneSequence::new(2);
    assert!(mic.push(0.0, ANALYSIS_FORMAT, &[0.0; 16]));
    assert!(mic.push(0.1, ANALYSIS_FORMAT, &[0.0; 16]));
    assert!(!mic.push(0.2, ANALYSIS_FORMAT, &[0.0; 16]));
    assert_eq!((mic.pending(), mic.refused()), (2, 1));
    mic.next_input();
    assert!(mic.push(0.2, ANALYSIS_FORMAT, &[0.0; 16]), "a taken block frees a place");
}

#[test]
fn a_microphone_block_in_another_format_is_refused() {
    let mut mic = MicrophoneSequence::new(4);
    assert!(mic.push(0.0, ANALYSIS_FORMAT, &[0.0; 16]));
    assert!(!mic.push(0.1, AudioFormat::new(48_000, 2), &[0.0; 16]));
    assert_eq!(mic.refused(), 1);
}

#[test]
fn a_microphone_block_that_goes_back_in_time_or_is_empty_is_refused() {
    let mut mic = MicrophoneSequence::new(4);
    assert!(mic.push(0.5, ANALYSIS_FORMAT, &[0.0; 16]));
    assert!(!mic.push(0.2, ANALYSIS_FORMAT, &[0.0; 16]));
    assert!(!mic.push(0.6, ANALYSIS_FORMAT, &[]));
    assert_eq!(mic.pending(), 1);
}

#[test]
fn a_microphone_block_converts_to_the_analysis_format_and_keeps_its_start() {
    let mut mic = MicrophoneSequence::new(4);
    let stereo_48k = AudioFormat::new(48_000, 2);
    assert!(mic.push(3.25, stereo_48k, &[0.2; 96]));
    let block = mic.next_input().unwrap();
    let out = AnalyzerInputConverter::new().convert(&block);
    assert_eq!(out.format(), ANALYSIS_FORMAT);
    assert_eq!(out.start(), 3.25);
    assert_eq!(out.frames(), 16);
}

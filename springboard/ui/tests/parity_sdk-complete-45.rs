//! Parity sdk-complete-45: the low-pass effect of the audio engine graph and the hit testing of the Notification Center
//! column. Headless: the engine renders to memory and the layout is pure geometry.

use std::collections::HashMap;
use std::f32::consts::PI;

use bifrost_ui::audio_engine::{AudioEngine, EngineError, Format, OUTPUT};
use bifrost_ui::widget::{Controller, DisplayMode, Limits};
use bifrost_ui::widget_column::{block_at, content_height, layout, Block};

const STEREO: Format = Format { sample_rate: 48_000, channels: 2 };

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

/// The smoothing factor of a one-pole low-pass at `cutoff` Hz for the stereo engine rate.
fn alpha(cutoff: f32) -> f32 {
    1.0 - (-2.0 * PI * cutoff / 48_000.0).exp()
}

#[test]
fn a_low_pass_moves_a_step_towards_its_input_by_the_smoothing_factor() {
    let mut engine = AudioEngine::new(STEREO).unwrap();
    let player = engine.attach_player(STEREO, vec![1.0; 8]).unwrap();
    let filter = engine.attach_low_pass(1_000.0).unwrap();
    engine.connect(player, filter, STEREO).unwrap();
    engine.connect(filter, OUTPUT, STEREO).unwrap();
    engine.prepare();
    engine.start().unwrap();
    let out = engine.render(2).unwrap();
    let a = alpha(1_000.0);
    assert!(close(out[0], a), "first left sample is alpha times the step: {}", out[0]);
    assert!(close(out[2], 1.0 - (1.0 - a) * (1.0 - a)), "second left sample keeps approaching the input: {}", out[2]);
}

#[test]
fn a_low_pass_filters_each_channel_on_its_own() {
    let mut engine = AudioEngine::new(STEREO).unwrap();
    let player = engine.attach_player(STEREO, vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]).unwrap();
    let filter = engine.attach_low_pass(1_000.0).unwrap();
    engine.connect(player, filter, STEREO).unwrap();
    engine.connect(filter, OUTPUT, STEREO).unwrap();
    engine.prepare();
    engine.start().unwrap();
    let out = engine.render(4).unwrap();
    assert!(out.chunks(2).all(|frame| frame[1] == 0.0), "a silent right channel stays silent");
    assert!(out[0] > 0.0 && out[2] > out[0], "the left channel rises towards its input");
}

#[test]
fn a_low_pass_cutoff_must_lie_between_zero_and_half_the_sample_rate() {
    let mut engine = AudioEngine::new(STEREO).unwrap();
    assert_eq!(engine.attach_low_pass(0.0).unwrap_err(), EngineError::InvalidValue);
    assert_eq!(engine.attach_low_pass(-5.0).unwrap_err(), EngineError::InvalidValue);
    assert_eq!(engine.attach_low_pass(24_000.0).unwrap_err(), EngineError::InvalidValue);
    assert_eq!(engine.attach_low_pass(f32::NAN).unwrap_err(), EngineError::InvalidValue);
    assert!(engine.attach_low_pass(23_999.0).is_ok());
}

#[test]
fn a_low_pass_takes_one_input_like_a_gain_node() {
    let mut engine = AudioEngine::new(STEREO).unwrap();
    let a = engine.attach_player(STEREO, vec![0.0; 4]).unwrap();
    let b = engine.attach_player(STEREO, vec![0.0; 4]).unwrap();
    let filter = engine.attach_low_pass(500.0).unwrap();
    engine.connect(a, filter, STEREO).unwrap();
    assert_eq!(engine.connect(b, filter, STEREO), Err(EngineError::InputBusy));
}

#[test]
fn a_low_pass_cannot_be_running_while_the_graph_changes() {
    let mut engine = AudioEngine::new(STEREO).unwrap();
    engine.prepare();
    engine.start().unwrap();
    assert_eq!(engine.attach_low_pass(500.0).unwrap_err(), EngineError::Running);
}

fn stacked_column() -> Vec<bifrost_ui::widget_column::Frame> {
    let blocks = vec![Block::Notifications, Block::Widget("is.example.a".into())];
    let limits = Limits::new(342.0, 110.0, 340.0);
    layout(&blocks, &Controller::default(), &HashMap::new(), &limits, 200.0, 8.0)
}

#[test]
fn a_point_inside_a_block_finds_that_block() {
    let frames = stacked_column();
    assert_eq!(block_at(&frames, 0.0).map(|f| &f.block), Some(&Block::Notifications));
    assert_eq!(block_at(&frames, 199.9).map(|f| &f.block), Some(&Block::Notifications));
    assert_eq!(block_at(&frames, 208.0).map(|f| &f.block), Some(&Block::Widget("is.example.a".into())));
    assert_eq!(block_at(&frames, 317.9).map(|f| &f.block), Some(&Block::Widget("is.example.a".into())));
}

#[test]
fn a_point_in_a_gap_or_past_the_column_finds_nothing() {
    let frames = stacked_column();
    assert!(block_at(&frames, 200.0).is_none(), "the bottom edge is outside and the gap is empty");
    assert!(block_at(&frames, 205.0).is_none());
    assert!(block_at(&frames, 318.0).is_none(), "the column ends at its last block");
    assert!(block_at(&frames, -1.0).is_none());
    assert!(block_at(&frames, f32::NAN).is_none());
    assert!(block_at(&[], 0.0).is_none());
}

#[test]
fn the_column_height_runs_from_the_first_block_to_the_last_one_bottom() {
    assert_eq!(content_height(&stacked_column()), 318.0);
    assert_eq!(content_height(&[]), 0.0);
}

#[test]
fn an_expanded_widget_is_taller_in_the_column_and_is_found_at_its_lower_part() {
    let mut modes = HashMap::new();
    modes.insert("is.example.a".to_string(), DisplayMode::Expanded);
    let limits = Limits::new(342.0, 110.0, 340.0);
    let frames = layout(&[Block::Widget("is.example.a".into())], &Controller::default(), &modes, &limits, 0.0, 0.0);
    assert_eq!(content_height(&frames), 340.0);
    assert!(block_at(&frames, 339.0).is_some());
    assert!(block_at(&frames, 340.0).is_none());
}

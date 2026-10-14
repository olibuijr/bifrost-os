//! Parity sdk-complete-22: headless tests for the Now Playing models (held seeking, the playback rate, content kinds),
//! the image kernel base trait and the layer and recurrent cell paths of the neural network model. Nothing here draws
//! a frame, opens a device or runs a graph compiler.

use bifrost_render::gfx::Image;
use bifrost_ui::cnn_layers::{gru_step, lstm_step, Cell, CustomLayer, Feature, Layer, LayerError, Network, RecurrentWeights};
use bifrost_ui::image_kernels::{apply_chain, ImageKernel, Kernel, KernelKind};
use bifrost_ui::media_content::{ContentKind, MediaContent, MediaDuration, MediaType};
use bifrost_ui::media_session::{held_seek_position, MediaPlaybackSnapshot, MediaSession, PlaybackState, HOLD_SEEK_RATE};

fn song(seconds: f64) -> MediaContent {
    MediaContent::new("track-1", "Song", MediaType::Audio, MediaDuration::Finite(seconds)).expect("valid content")
}

#[test]
fn a_held_skip_moves_the_position_at_the_hold_rate() {
    assert_eq!(HOLD_SEEK_RATE, 4);
    assert_eq!(held_seek_position(0, 1_000, true, MediaDuration::Finite(200.0)), 4_000);
}

#[test]
fn a_held_forward_skip_stops_at_the_end_of_finite_content() {
    assert_eq!(held_seek_position(199_000, 5_000, true, MediaDuration::Finite(200.0)), 200_000);
}

#[test]
fn a_held_backward_skip_stops_at_zero() {
    assert_eq!(held_seek_position(3_000, 5_000, false, MediaDuration::Continuous), 0);
}

#[test]
fn a_held_forward_skip_on_live_content_has_no_end() {
    assert_eq!(held_seek_position(0, 60_000, true, MediaDuration::Live), 240_000);
}

#[test]
fn a_held_skip_keeps_the_playback_state_of_the_session() {
    let mut session = MediaSession::new("main").unwrap();
    session.set_content(Some(song(200.0)));
    session.set_playback(PlaybackState::Paused, 10_000);
    session.seek_held(true, 2_000);
    assert_eq!(session.snapshot(), MediaPlaybackSnapshot { state: PlaybackState::Paused, position_ms: 18_000 });
    session.seek_held(false, 60_000);
    assert_eq!(session.snapshot().position_ms, 0, "backward holds stop at zero");
}

#[test]
fn the_playback_rate_is_one_only_while_playing() {
    let playing = MediaPlaybackSnapshot { state: PlaybackState::Playing, position_ms: 0 };
    let paused = MediaPlaybackSnapshot { state: PlaybackState::Paused, position_ms: 0 };
    let stopped = MediaPlaybackSnapshot { state: PlaybackState::Stopped, position_ms: 0 };
    assert_eq!(playing.rate(), 1.0);
    assert_eq!(paused.rate(), 0.0);
    assert_eq!(stopped.rate(), 0.0);
}

#[test]
fn content_starts_generic_and_takes_a_typed_kind() {
    assert_eq!(song(10.0).kind, ContentKind::Generic);
    let film = song(10.0).with_kind(ContentKind::Movie);
    assert_eq!(film.kind, ContentKind::Movie);
    assert_eq!(film.id, "track-1", "the kind changes nothing else");
}

#[test]
fn each_content_kind_has_its_own_wire_name() {
    let names = [
        ContentKind::Generic,
        ContentKind::Music,
        ContentKind::Podcast,
        ContentKind::Movie,
        ContentKind::TvShow,
        ContentKind::Book,
        ContentKind::Radio,
    ]
    .map(ContentKind::name);
    assert_eq!(names, ["generic", "music", "podcast", "movie", "tv_show", "book", "radio"]);
}

struct Invert;

impl ImageKernel for Invert {
    fn kind(&self) -> KernelKind {
        KernelKind::Color
    }
    fn run(&self, img: &Image) -> Image {
        Image { w: img.w, h: img.h, px: img.px.iter().map(|p| p ^ 0x00ff_ffff).collect() }
    }
}

fn pixels() -> Image {
    Image { w: 2, h: 1, px: vec![0xff10_2030, 0xff40_5060] }
}

#[test]
fn a_kernel_defined_by_an_app_runs_through_the_base_trait() {
    // The colour channels flip and the alpha byte keeps 0xff: 0x10 -> 0xef, 0x20 -> 0xdf, 0x30 -> 0xcf.
    let out = Invert.run(&pixels());
    assert_eq!(out.px, vec![0xffef_dfcf, 0xffbf_af9f]);
    assert_eq!(Invert.kind(), KernelKind::Color);
}

#[test]
fn the_built_in_kernels_report_their_class_through_the_trait() {
    let sepia: &dyn ImageKernel = &Kernel::Sepia { intensity: 1.0 };
    let blur: &dyn ImageKernel = &Kernel::GaussianBlur { radius: 2.0 };
    let warp: &dyn ImageKernel = &Kernel::AffineTransform { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };
    assert_eq!(sepia.kind(), KernelKind::Color);
    assert_eq!(blur.kind(), KernelKind::General);
    assert_eq!(warp.kind(), KernelKind::Warp);
}

#[test]
fn a_chain_of_two_inversions_returns_the_input() {
    let input = pixels();
    let twice = apply_chain(&[&Invert, &Invert], &input);
    assert_eq!(twice.px, input.px);
    assert_eq!(apply_chain(&[], &input).px, input.px, "an empty chain copies the image");
}

#[test]
fn a_chain_of_one_kernel_gives_the_same_result_as_running_it() {
    let input = pixels();
    assert_eq!(apply_chain(&[&Invert], &input).px, Invert.run(&input).px);
}

#[test]
fn an_lstm_state_stays_inside_its_bounds_over_a_sequence() {
    // With weights of 0.5 on every input, the hidden state h = o * tanh(c) is bounded by 1 in magnitude.
    let w = RecurrentWeights::new(Cell::Lstm, 2, 1, vec![0.5; 8], vec![0.5; 16], vec![0.0; 8]).unwrap();
    let (mut h, mut c) = (vec![0.0; 2], vec![0.0; 2]);
    for step in 0..6 {
        let x = [if step % 2 == 0 { 1.0 } else { -1.0 }];
        let (next_h, next_c) = lstm_step(&w, &x, &h, &c).unwrap();
        h = next_h;
        c = next_c;
    }
    assert!(h.iter().all(|v| v.is_finite() && v.abs() < 1.0), "{h:?}");
}

#[test]
fn a_gru_step_refuses_a_hidden_state_of_the_wrong_length() {
    let w = RecurrentWeights::new(Cell::Gru, 2, 1, vec![0.0; 6], vec![0.0; 12], vec![0.0; 6]).unwrap();
    assert_eq!(gru_step(&w, &[0.0], &[0.0]).unwrap_err(), LayerError::Length { expected: 2, got: 1 });
}

struct Halve;

impl CustomLayer for Halve {
    fn name(&self) -> &str {
        "halve"
    }
    fn output_dims(&self, input: [usize; 3]) -> Result<[usize; 3], LayerError> {
        Ok(input)
    }
    fn evaluate(&self, input: &Feature) -> Result<Feature, LayerError> {
        Feature::new(input.channels, input.height, input.width, input.data.iter().map(|v| v * 0.5).collect())
    }
}

#[test]
fn a_custom_layer_keeps_the_shape_it_states_and_runs_in_a_network() {
    let mut net = Network::new();
    net.push(Layer::Custom(Box::new(Halve)));
    assert_eq!(net.output_dims([1, 2, 2]).unwrap(), [1, 2, 2]);
    let out = net.forward(Feature::new(1, 2, 2, vec![4.0, 2.0, 1.0, 0.0]).unwrap()).unwrap();
    assert_eq!(out.data, vec![2.0, 1.0, 0.5, 0.0]);
}

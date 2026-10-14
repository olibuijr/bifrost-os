//! Parity sdk-complete-47: headless tests for the linear PCM converter (sample depth, channel layout and sample rate) and
//! the player controller model. Nothing is decoded from a file, no device is opened and no clock is read.

use bifrost_ui::pcm_convert::{convert_bytes, decode, encode, ConvertError, Depth, PcmFormat};
use bifrost_ui::player_view::{Chrome, Hit, PlayerHandle, PlayerViewController, PlayerViewDelegate};
use bifrost_ui::web_extension::{Exchange, ExchangeError, Extension, HostPattern, Kind, Message, Registry};
use bifrost_ui::Rect;

fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn read_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn format(sample_rate: u32, channels: u16, depth: Depth) -> PcmFormat {
    PcmFormat { sample_rate, channels, depth }
}

#[test]
fn a_frame_is_depth_times_channels_bytes() {
    assert_eq!(format(48_000, 2, Depth::S16).frame_bytes(), 4);
    assert_eq!(format(48_000, 6, Depth::F32).frame_bytes(), 24);
    assert_eq!(format(8_000, 1, Depth::U8).frame_bytes(), 1);
}

#[test]
fn unsigned_eight_bit_silence_is_128_and_full_scale_is_near_one() {
    assert_eq!(decode(&[128u8], Depth::U8).unwrap(), vec![0.0]);
    let top = decode(&[255u8], Depth::U8).unwrap()[0];
    assert!((top - 127.0 / 128.0).abs() < 1e-6, "top was {top}");
    assert_eq!(encode(&[0.0], Depth::U8), vec![128u8]);
    assert_eq!(encode(&[2.0], Depth::U8), vec![255u8], "out of range floats are clamped");
}

#[test]
fn signed_sixteen_bit_little_endian_samples_decode_to_floats() {
    let bytes = [0x00, 0x80, 0xff, 0x7f];
    let values = decode(&bytes, Depth::S16).unwrap();
    assert_eq!(values, vec![-1.0, 32767.0 / 32768.0]);
}

#[test]
fn float_samples_round_trip_through_bytes_unchanged() {
    let samples = [0.25, -0.5, 0.0, 1.0];
    assert_eq!(decode(&encode(&samples, Depth::F32), Depth::F32).unwrap(), samples.to_vec());
}

#[test]
fn a_partial_sample_is_refused_as_ragged() {
    assert_eq!(decode(&[0u8, 1, 2], Depth::S16), Err(ConvertError::Ragged));
    assert_eq!(decode(&[0u8; 5], Depth::F32), Err(ConvertError::Ragged));
}

#[test]
fn identical_formats_return_the_same_bytes() {
    let fmt = format(48_000, 1, Depth::S16);
    let bytes: Vec<u8> = (0u8..16).collect();
    assert_eq!(convert_bytes(&bytes, fmt, fmt).unwrap(), bytes);
}

#[test]
fn stereo_to_mono_averages_each_frame() {
    let input = f32_bytes(&[0.5, 0.5, -1.0, 0.0]);
    let out = convert_bytes(&input, format(48_000, 2, Depth::F32), format(48_000, 1, Depth::F32)).unwrap();
    assert_eq!(read_f32(&out), vec![0.5, -0.5]);
}

#[test]
fn mono_to_stereo_repeats_the_sample_on_both_channels() {
    let input = f32_bytes(&[0.25, -0.25]);
    let out = convert_bytes(&input, format(48_000, 1, Depth::F32), format(48_000, 2, Depth::F32)).unwrap();
    assert_eq!(read_f32(&out), vec![0.25, 0.25, -0.25, -0.25]);
}

#[test]
fn halving_the_rate_halves_the_frame_count() {
    let input = f32_bytes(&[0.0, 1.0, 2.0, 3.0]);
    let out = convert_bytes(&input, format(48_000, 1, Depth::F32), format(24_000, 1, Depth::F32)).unwrap();
    assert_eq!(read_f32(&out), vec![0.0, 2.0]);
}

#[test]
fn doubling_the_rate_doubles_the_frame_count_and_interpolates() {
    let input = f32_bytes(&[0.0, 1.0]);
    let out = convert_bytes(&input, format(24_000, 1, Depth::F32), format(48_000, 1, Depth::F32)).unwrap();
    let samples = read_f32(&out);
    assert_eq!(samples.len(), 4);
    assert_eq!(&samples[..2], &[0.0, 0.5]);
}

#[test]
fn depth_change_quantises_to_the_target_depth() {
    let input = f32_bytes(&[0.0, -1.0]);
    let out = convert_bytes(&input, format(48_000, 1, Depth::F32), format(48_000, 1, Depth::S16)).unwrap();
    assert_eq!(out, vec![0u8, 0, 1, 0x80], "0 is silence and -1.0 is -32767 as little-endian bytes");
}

#[test]
fn a_block_that_is_not_whole_frames_is_refused() {
    let input = [0u8; 6];
    let err = convert_bytes(&input, format(48_000, 2, Depth::S16), format(48_000, 1, Depth::S16)).unwrap_err();
    assert_eq!(err, ConvertError::Ragged);
}

#[test]
fn zero_rates_and_zero_channels_are_refused() {
    let ok = format(48_000, 1, Depth::S16);
    assert_eq!(convert_bytes(&[0u8; 2], format(0, 1, Depth::S16), ok), Err(ConvertError::ZeroRate));
    assert_eq!(convert_bytes(&[0u8; 2], ok, format(44_100, 0, Depth::S16)), Err(ConvertError::ZeroChannels));
}

#[test]
fn an_empty_block_converts_to_nothing() {
    let out = convert_bytes(&[], format(48_000, 2, Depth::S16), format(44_100, 1, Depth::U8)).unwrap();
    assert!(out.is_empty());
}

fn reader_registry(enabled: bool) -> Registry {
    let pattern = HostPattern::parse("https://*.example.com").unwrap();
    let extension = Extension::new("com.example.reader", "Reader", Kind::WebExtension, vec![pattern]).unwrap();
    let mut registry = Registry::new();
    registry.install(extension).unwrap();
    registry.set_enabled("com.example.reader", enabled).unwrap();
    registry
}

fn answer(text: &str) -> Message {
    Message::new("answer").unwrap().with("text", text).unwrap()
}

#[test]
fn a_page_request_to_an_allowed_extension_gets_an_id_and_the_next_id_follows() {
    let registry = reader_registry(true);
    let mut exchange = Exchange::new();
    assert_eq!(exchange.request(&registry, "com.example.reader", "https://news.example.com/a"), Ok(0));
    assert_eq!(exchange.request(&registry, "com.example.reader", "https://example.com/b"), Ok(1));
    assert_eq!(exchange.pending_count(), 2);
}

#[test]
fn a_request_is_refused_for_a_disabled_extension_an_unlisted_page_or_an_unknown_extension() {
    let disabled = reader_registry(false);
    let mut exchange = Exchange::new();
    assert_eq!(
        exchange.request(&disabled, "com.example.reader", "https://example.com/"),
        Err(ExchangeError::NotAllowed)
    );
    let enabled = reader_registry(true);
    assert_eq!(
        exchange.request(&enabled, "com.example.reader", "http://example.com/"),
        Err(ExchangeError::NotAllowed),
        "the scheme is not in the permission"
    );
    assert_eq!(
        exchange.request(&enabled, "com.example.other", "https://example.com/"),
        Err(ExchangeError::NotAllowed)
    );
    assert_eq!(exchange.pending_count(), 0);
}

#[test]
fn the_extension_that_got_a_request_answers_it_once() {
    let registry = reader_registry(true);
    let mut exchange = Exchange::new();
    let id = exchange.request(&registry, "com.example.reader", "https://example.com/").unwrap();
    let reply = exchange.reply(&registry, "com.example.reader", id, answer("done")).unwrap();
    assert_eq!(reply.name(), "answer");
    assert_eq!(reply.value("text"), Some("done"));
    assert_eq!(exchange.pending_count(), 0);
    assert_eq!(
        exchange.reply(&registry, "com.example.reader", id, answer("again")),
        Err(ExchangeError::NoSuchRequest)
    );
}

#[test]
fn a_reply_from_another_extension_is_refused_and_the_request_stays_open() {
    let registry = reader_registry(true);
    let mut exchange = Exchange::new();
    let id = exchange.request(&registry, "com.example.reader", "https://example.com/").unwrap();
    assert_eq!(exchange.reply(&registry, "com.example.other", id, answer("x")), Err(ExchangeError::WrongSender));
    assert_eq!(exchange.pending_count(), 1);
    assert!(exchange.reply(&registry, "com.example.reader", id, answer("ok")).is_ok());
}

#[test]
fn a_reply_after_the_extension_was_disabled_is_refused_and_closes_the_request() {
    let mut registry = reader_registry(true);
    let mut exchange = Exchange::new();
    let id = exchange.request(&registry, "com.example.reader", "https://example.com/").unwrap();
    registry.set_enabled("com.example.reader", false).unwrap();
    assert_eq!(
        exchange.reply(&registry, "com.example.reader", id, answer("late")),
        Err(ExchangeError::NotAllowed)
    );
    assert_eq!(exchange.pending_count(), 0);
}

#[test]
fn a_cancelled_request_cannot_be_answered() {
    let registry = reader_registry(true);
    let mut exchange = Exchange::new();
    let id = exchange.request(&registry, "com.example.reader", "https://example.com/").unwrap();
    assert!(exchange.cancel(id));
    assert!(!exchange.cancel(id), "a request is cancelled once");
    assert_eq!(
        exchange.reply(&registry, "com.example.reader", id, answer("x")),
        Err(ExchangeError::NoSuchRequest)
    );
}

#[derive(Default)]
struct StageLog {
    stages: Vec<&'static str>,
}

impl PlayerViewDelegate for StageLog {
    fn will_begin_full_screen(&mut self) {
        self.stages.push("will begin");
    }
    fn did_begin_full_screen(&mut self) {
        self.stages.push("did begin");
    }
    fn will_end_full_screen(&mut self) {
        self.stages.push("will end");
    }
    fn did_end_full_screen(&mut self) {
        self.stages.push("did end");
    }
}

/// A player that is always 100 seconds long and starts paused.
#[derive(Default)]
struct PausedPlayer {
    playing: bool,
}

impl PlayerHandle for PausedPlayer {
    fn is_playing(&self) -> bool {
        self.playing
    }
    fn play(&mut self) {
        self.playing = true;
    }
    fn pause(&mut self) {
        self.playing = false;
    }
    fn current_seconds(&self) -> f64 {
        0.0
    }
    fn duration_seconds(&self) -> f64 {
        100.0
    }
    fn seek_seconds(&mut self, _seconds: f64) {}
}

const BOUNDS: Rect = Rect { x: 0.0, y: 0.0, w: 600.0, h: 400.0 };

fn centre_of(controller: &PlayerViewController, chrome: Chrome) -> (f32, f32) {
    let (_, r) = controller.view.layout(BOUNDS).into_iter().find(|(c, _)| *c == chrome).expect("chrome is laid out");
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn controller_with_toggle() -> PlayerViewController {
    let mut controller = PlayerViewController::new();
    controller.view.shows_full_screen_toggle_button = true;
    controller
}

#[test]
fn a_controller_starts_windowed() {
    assert!(!PlayerViewController::new().is_full_screen());
}

#[test]
fn entering_full_screen_reports_the_will_and_did_stages_in_order() {
    let mut controller = PlayerViewController::new();
    let mut log = StageLog::default();
    assert!(controller.set_full_screen(true, &mut log));
    assert!(controller.is_full_screen());
    assert_eq!(log.stages, vec!["will begin", "did begin"]);
}

#[test]
fn leaving_full_screen_reports_the_will_and_did_end_stages() {
    let mut controller = PlayerViewController::new();
    let mut log = StageLog::default();
    controller.set_full_screen(true, &mut log);
    log.stages.clear();
    assert!(controller.set_full_screen(false, &mut log));
    assert!(!controller.is_full_screen());
    assert_eq!(log.stages, vec!["will end", "did end"]);
}

#[test]
fn asking_for_the_current_state_changes_nothing_and_reports_nothing() {
    let mut controller = PlayerViewController::new();
    let mut log = StageLog::default();
    assert!(!controller.set_full_screen(false, &mut log));
    assert!(log.stages.is_empty());
    controller.set_full_screen(true, &mut log);
    log.stages.clear();
    assert!(!controller.set_full_screen(true, &mut log));
    assert!(log.stages.is_empty());
}

#[test]
fn tapping_the_full_screen_button_toggles_and_reports_each_change() {
    let mut controller = controller_with_toggle();
    let mut player = PausedPlayer::default();
    let mut log = StageLog::default();
    let (x, y) = centre_of(&controller, Chrome::FullScreenToggle);
    assert_eq!(controller.tap(BOUNDS, x, y, &mut player, &mut log), Some(Hit::FullScreenToggle));
    assert!(controller.is_full_screen());
    assert_eq!(log.stages, vec!["will begin", "did begin"]);
    assert_eq!(controller.tap(BOUNDS, x, y, &mut player, &mut log), Some(Hit::FullScreenToggle));
    assert!(!controller.is_full_screen());
    assert_eq!(log.stages, vec!["will begin", "did begin", "will end", "did end"]);
}

#[test]
fn tapping_play_on_the_controller_leaves_full_screen_alone() {
    let mut controller = controller_with_toggle();
    let mut player = PausedPlayer::default();
    let mut log = StageLog::default();
    let (x, y) = centre_of(&controller, Chrome::PlayPause);
    assert_eq!(controller.tap(BOUNDS, x, y, &mut player, &mut log), Some(Hit::PlayPause));
    assert!(player.playing);
    assert!(!controller.is_full_screen());
    assert!(log.stages.is_empty());
}

#[test]
fn a_hidden_full_screen_button_cannot_be_tapped_on_the_controller() {
    let mut controller = PlayerViewController::new();
    let mut player = PausedPlayer::default();
    let mut log = StageLog::default();
    let hit = controller.tap(BOUNDS, 590.0, 380.0, &mut player, &mut log);
    assert_ne!(hit, Some(Hit::FullScreenToggle));
    assert!(!controller.is_full_screen());
    assert!(log.stages.is_empty());
}

//! Parity sdk-os-10: headless tests for the playback, device report, placement and dispatch models. No test starts
//! audio, opens a device or runs a GPU: every model is driven with plain values and a fake clock.

use bifrost_ui::av_asset::{Asset, Pcm};
use bifrost_ui::av_player::{ActionAtItemEnd, ItemStatus, Player, PlayerItem, PlayerStatus, TimeControlStatus};
use bifrost_ui::av_queue_player::QueuePlayer;
use bifrost_ui::compute::{place, Device, Policy};
use bifrost_ui::compute_dispatch::{run_threads, threadgroups_for, validate_threadgroup, DispatchError, Size};
use bifrost_ui::hid_report::{ParseError, ReportDescriptor, ReportKind};

const SECOND_US: i64 = 1_000_000;

fn tone(seconds: f32) -> PlayerItem {
    PlayerItem::new(Asset::from_pcm("test://tone", Pcm::sine(8000, 1, 440.0, seconds)))
}

// ---- av_player: playback of one item -----------------------------------------------------------------------------------

#[test]
fn a_new_player_is_paused_and_has_no_item() {
    let player = Player::new();
    assert!(player.current_item().is_none());
    assert_eq!(player.time_control_status(), TimeControlStatus::Paused);
    assert_eq!(player.status(), PlayerStatus::Unknown);
}

#[test]
fn setting_an_item_does_not_start_playback() {
    let player = Player::with_item(tone(1.0));
    assert_eq!(player.status(), PlayerStatus::ReadyToPlay);
    assert_eq!(player.rate(), 0.0);
    assert_eq!(player.time_control_status(), TimeControlStatus::Paused);
}

#[test]
fn play_starts_a_ready_item_at_the_default_rate_and_pause_stops_it() {
    let mut player = Player::with_item(tone(1.0));
    player.play();
    assert_eq!(player.rate(), 1.0);
    assert_eq!(player.time_control_status(), TimeControlStatus::Playing);
    player.pause();
    assert_eq!(player.time_control_status(), TimeControlStatus::Paused);
}

#[test]
fn advancing_a_playing_player_moves_its_clock_by_the_elapsed_time() {
    let mut player = Player::with_item(tone(2.0));
    player.play();
    player.advance(250_000);
    assert_eq!(player.current_time_us(), 250_000);
}

#[test]
fn an_item_lasts_as_long_as_its_audio() {
    let item = tone(1.0);
    assert_eq!(item.status(), ItemStatus::ReadyToPlay);
    assert_eq!(item.duration_us(), SECOND_US);
}

#[test]
fn a_seek_lands_on_a_time_inside_the_item_and_sets_the_clock() {
    let mut player = Player::with_item(tone(2.0));
    let landed = player.seek_exact(SECOND_US / 2);
    assert!((0..=2 * SECOND_US).contains(&landed));
    assert_eq!(player.current_time_us(), landed);
}

#[test]
fn the_default_action_at_the_end_of_an_item_is_to_pause() {
    assert_eq!(Player::new().action_at_item_end, ActionAtItemEnd::Pause);
}

// ---- av_queue_player: items in order and a looper ----------------------------------------------------------------------

#[test]
fn a_queue_takes_inserts_and_removals_in_item_order() {
    let mut queue = QueuePlayer::new(vec![tone(1.0), tone(2.0)]);
    assert_eq!(queue.items().len(), 2);
    queue.insert(tone(3.0), Some(0));
    assert_eq!(queue.items()[1].duration_us(), 3 * SECOND_US);
    let removed = queue.remove(1).expect("the inserted item");
    assert_eq!(removed.duration_us(), 3 * SECOND_US);
    assert!(queue.remove(9).is_none());
    queue.remove_all_items();
    assert!(queue.items().is_empty());
}

#[test]
fn a_looper_holds_the_same_asset_for_every_pass() {
    let asset = Asset::from_pcm("test://loop", Pcm::sine(8000, 1, 220.0, 1.0));
    let looper = QueuePlayer::looper(asset);
    assert_eq!(looper.items().len(), 2);
    assert!(looper.items().iter().all(|item| item.duration_us() == SECOND_US));
}

// ---- hid_report: reading element values from input reports ------------------------------------------------------------

/// A three-button mouse with a 5-bit constant pad and X and Y axes, in one application collection.
fn mouse() -> Vec<u8> {
    vec![
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03,
        0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81,
        0x25, 0x7f, 0x75, 0x08, 0x95, 0x02, 0x81, 0x06, 0xc0,
    ]
}

#[test]
fn an_input_report_gives_each_button_and_axis_its_value() {
    let descriptor = ReportDescriptor::parse(&mouse()).unwrap();
    // Buttons 1 and 3 pressed, X = -1, Y = 2.
    let payload = [0b0000_0101, 0xff, 0x02];
    let fields = descriptor.fields(ReportKind::Input, 0);
    assert_eq!(fields[0].value(&payload), Some(1));
    assert_eq!(fields[1].value(&payload), Some(0));
    assert_eq!(fields[2].value(&payload), Some(1));
    assert_eq!(fields[4].value(&payload), Some(-1));
    assert_eq!(fields[5].value(&payload), Some(2));
}

#[test]
fn a_value_that_does_not_fit_its_field_is_refused_and_a_constant_field_is_read_only() {
    let descriptor = ReportDescriptor::parse(&mouse()).unwrap();
    let mut payload = [0u8; 3];
    let fields = descriptor.fields(ReportKind::Input, 0);
    assert!(fields[4].set_value(&mut payload, -5));
    assert_eq!(fields[4].value(&payload), Some(-5));
    assert!(!fields[4].set_value(&mut payload, 128));
    assert!(!fields[3].set_value(&mut payload, 1), "the pad is constant");
}

#[test]
fn a_descriptor_with_an_open_collection_is_refused() {
    let open = [0x05, 0x01, 0x09, 0x02, 0xa1, 0x01];
    assert_eq!(ReportDescriptor::parse(&open).err(), Some(ParseError::UnclosedCollection));
}

#[test]
fn an_item_cut_short_is_refused_as_truncated() {
    assert_eq!(ReportDescriptor::parse(&[0x05]).err(), Some(ParseError::Truncated));
}

// ---- compute: which device one operation runs on -----------------------------------------------------------------------

#[test]
fn automatic_placement_prefers_the_gpu_and_falls_back_to_the_cpu() {
    let both = [Device::Cpu, Device::Gpu];
    assert_eq!(place(Policy::Automatic, &both, &both), Some(Device::Gpu));
    assert_eq!(place(Policy::Automatic, &[Device::Cpu], &both), Some(Device::Cpu));
}

#[test]
fn the_cpu_only_policy_ignores_a_gpu_that_is_present() {
    let both = [Device::Cpu, Device::Gpu];
    assert_eq!(place(Policy::CpuOnly, &both, &both), Some(Device::Cpu));
}

#[test]
fn an_operation_is_not_placed_on_a_device_it_cannot_run_on() {
    assert_eq!(place(Policy::Automatic, &[Device::Gpu], &[Device::Cpu]), None);
}

#[test]
fn a_policy_lists_its_devices_in_order_of_preference() {
    assert_eq!(Policy::Automatic.allowed(&[Device::Cpu, Device::Gpu]), vec![Device::Gpu, Device::Cpu]);
    assert_eq!(Policy::default(), Policy::Automatic);
}

// ---- compute_dispatch: grid and threadgroup arithmetic -------------------------------------------------------------------

#[test]
fn a_grid_is_rounded_up_to_whole_threadgroups_on_each_axis() {
    assert_eq!(threadgroups_for(Size::new(10, 1, 1), Size::new(4, 1, 1)), Ok(Size::new(3, 1, 1)));
    assert_eq!(threadgroups_for(Size::new(8, 8, 1), Size::new(8, 8, 1)), Ok(Size::new(1, 1, 1)));
}

#[test]
fn a_threadgroup_with_a_zero_axis_is_refused() {
    assert_eq!(validate_threadgroup(Size::new(0, 1, 1), 1024), Err(DispatchError::ZeroAxis));
}

#[test]
fn a_threadgroup_above_the_device_limit_is_refused() {
    assert_eq!(
        validate_threadgroup(Size::new(32, 32, 2), 1024),
        Err(DispatchError::TooManyThreads { threads: 2048, max: 1024 })
    );
    assert!(validate_threadgroup(Size::new(32, 32, 1), 1024).is_ok());
}

#[test]
fn every_thread_of_the_grid_runs_once_and_the_partial_group_edge_is_skipped() {
    let mut seen = Vec::new();
    let ran = run_threads(Size::new(5, 1, 1), Size::new(4, 1, 1), 1024, |t| seen.push(t));
    assert_eq!(ran, Ok(5));
    assert_eq!(seen.len(), 5);
    assert!(seen.iter().all(|t| t.width < 5));
}

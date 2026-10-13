//! Playback model tests: a fake clock (`advance`) and a sine WAV fixture written by the test.

use bifrost_ui::av_asset::*;
use bifrost_ui::av_player::*;
use bifrost_ui::av_queue_player::QueuePlayer;

const S: i64 = 1_000_000;

fn wav(seconds: f32) -> Vec<u8> {
    Pcm::sine(8000, 1, 440.0, seconds).to_wav()
}

fn asset(seconds: f32) -> Asset {
    Asset::from_wav_bytes("file:///t.wav", &wav(seconds))
}

fn ready(seconds: f32) -> Player {
    Player::with_item(PlayerItem::new(asset(seconds)))
}

#[test]
fn asset_reports_duration_and_tracks() {
    let a = asset(2.0);
    assert_eq!(a.load_status(AssetKey::Duration), LoadStatus::Loaded);
    assert_eq!(a.duration_us(), Some(2 * S));
    let t = a.tracks();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].media_kind, MediaKind::Audio);
}

#[test]
fn bad_bytes_fail_to_load() {
    let a = Asset::from_wav_bytes("x", b"nope");
    assert_eq!(a.load_status(AssetKey::Tracks), LoadStatus::Failed);
    assert_eq!(PlayerItem::new(a).status(), ItemStatus::Failed);
}

#[test]
fn player_status_transitions() {
    let mut p = Player::new();
    assert_eq!(p.status(), PlayerStatus::Unknown);
    p.replace_current_item(Some(PlayerItem::new(asset(1.0))));
    assert_eq!(p.status(), PlayerStatus::ReadyToPlay);
    p.replace_current_item(Some(PlayerItem::new(Asset::from_wav_bytes("x", b"bad"))));
    assert_eq!(p.status(), PlayerStatus::Failed);
}

#[test]
fn nothing_plays_until_play() {
    let mut p = ready(1.0);
    p.advance(S / 2);
    assert_eq!(p.current_time_us(), 0);
    assert_eq!(p.time_control_status(), TimeControlStatus::Paused);
}

#[test]
fn play_pause_and_rate_move_time_control_status() {
    let mut p = ready(4.0);
    p.play();
    assert_eq!(p.time_control_status(), TimeControlStatus::Playing);
    p.advance(S);
    assert_eq!(p.current_time_us(), S);
    p.set_rate(2.0);
    p.advance(S);
    assert_eq!(p.current_time_us(), 3 * S);
    p.pause();
    assert_eq!(p.time_control_status(), TimeControlStatus::Paused);
    p.set_default_rate(0.5);
    p.play();
    assert_eq!(p.rate(), 0.5);
}

#[test]
fn rate_without_item_waits() {
    let mut p = Player::new();
    p.play();
    assert_eq!(p.time_control_status(), TimeControlStatus::WaitingToPlayAtSpecifiedRate);
}

#[test]
fn seek_lands_on_a_sample() {
    let mut p = ready(1.0);
    // 8 kHz: one sample is 125 us. 1_000 us is exactly sample 8.
    assert_eq!(p.seek_exact(1_000), 1_000);
    // 1_050 us is nearer sample 8 (1_000) than sample 9 (1_125).
    assert_eq!(p.seek_exact(1_050), 1_000);
    // Tolerance only after: must not land before the target.
    assert_eq!(p.seek(1_050, 0, 200), 1_125);
    // Seeks clamp to the item.
    assert_eq!(p.seek_exact(10 * S), S);
}

#[test]
fn periodic_observer_fires_on_schedule() {
    let mut p = ready(3.0);
    let id = p.add_periodic_time_observer(S);
    p.play();
    p.advance(S / 2);
    assert!(p.take_events().is_empty());
    p.advance(2 * S);
    let times: Vec<i64> = p.take_events().iter().map(|e| e.time_us).collect();
    assert_eq!(times, vec![S, 2 * S]);
    p.remove_time_observer(id);
    p.advance(S / 4);
    assert!(p.take_events().is_empty());
}

#[test]
fn boundary_observer_fires_when_crossed() {
    let mut p = ready(3.0);
    let id = p.add_boundary_time_observer(&[S + S / 2]);
    p.play();
    p.advance(S);
    assert!(p.take_events().is_empty());
    p.advance(S);
    let ev = p.take_events();
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0].id, ev[0].time_us), (id, S + S / 2));
}

#[test]
fn did_play_to_end_pauses_by_default() {
    let mut p = ready(1.0);
    p.play();
    p.advance(2 * S);
    assert_eq!(p.take_notifications(), vec![Notification::DidPlayToEndTime]);
    assert_eq!(p.current_time_us(), S);
    assert_eq!(p.time_control_status(), TimeControlStatus::Paused);
    // Playing again from the end restarts.
    p.play();
    assert_eq!(p.current_time_us(), 0);
}

#[test]
fn action_none_keeps_rate_at_the_end() {
    let mut p = ready(1.0);
    p.action_at_item_end = ActionAtItemEnd::None;
    p.play();
    p.advance(2 * S);
    assert_eq!(p.rate(), 1.0);
    p.take_notifications();
    p.advance(S);
    assert!(p.take_notifications().is_empty());
}

#[test]
fn item_ranges_cover_the_asset() {
    let item = PlayerItem::new(asset(2.0));
    assert_eq!(item.duration_us(), 2 * S);
    assert_eq!(item.loaded_time_ranges(), vec![(0, 2 * S)]);
    assert_eq!(item.seekable_time_ranges(), vec![(0, 2 * S)]);
}

#[test]
fn queue_player_advances_and_keeps_rate() {
    let mut q = QueuePlayer::new(vec![PlayerItem::new(asset(1.0)), PlayerItem::new(asset(2.0))]);
    q.player.play();
    q.advance(S + 1);
    assert_eq!(q.player.current_item().unwrap().duration_us(), 2 * S);
    assert_eq!(q.player.rate(), 1.0);
    assert!(q.items().is_empty());
    q.advance(3 * S);
    assert_eq!(q.player.time_control_status(), TimeControlStatus::Paused);
}

#[test]
fn queue_insert_and_remove() {
    let mut q = QueuePlayer::new(vec![PlayerItem::new(asset(1.0))]);
    q.insert(PlayerItem::new(asset(2.0)), None);
    q.insert(PlayerItem::new(asset(3.0)), Some(0));
    assert_eq!(q.items().len(), 2);
    assert!(q.remove(0).is_some());
    assert_eq!(q.items().len(), 1);
    q.advance_to_next_item();
    assert_eq!(q.player.current_item().unwrap().duration_us(), 3 * S);
}

#[test]
fn looper_repeats() {
    let mut q = QueuePlayer::looper(asset(1.0));
    q.player.play();
    for _ in 0..5 {
        q.advance(S + 1);
        assert_eq!(q.player.rate(), 1.0);
        assert!(q.player.current_item().is_some());
    }
}

#[test]
fn composition_insert_gives_combined_duration() {
    let a = asset(1.0);
    let b = asset(2.0);
    let mut comp = MutableComposition::new();
    let t = comp.add_track();
    comp.track_mut(t).unwrap().insert_time_range(&a, (0, S), 0).unwrap();
    comp.track_mut(t).unwrap().insert_time_range(&b, (0, S), S).unwrap();
    assert_eq!(comp.duration_us(), 2 * S);
    comp.track_mut(t).unwrap().insert_time_range(&b, (S / 2, S / 2), S / 2).unwrap();
    assert_eq!(comp.duration_us(), 2 * S + S / 2);
    assert_eq!(
        comp.track_mut(t).unwrap().insert_time_range(&b, (0, 5 * S), 0),
        Err(AssetError::OutOfRange)
    );
    let item = PlayerItem::new(comp.as_asset("comp"));
    assert_eq!(item.duration_us(), 2 * S + S / 2);
}

#[test]
fn audio_mix_ramp_is_applied_to_samples() {
    let mut mix = MutableAudioMixInputParameters::new();
    mix.set_volume_ramp(1.0, 0.0, (0, S));
    assert_eq!(mix.volume_at(S / 2), 0.5);
    assert_eq!(mix.volume_at(2 * S), 0.0);
    let pcm = Pcm { sample_rate: 8000, channels: 1, samples: vec![1.0; 8000] };
    let out = mix.apply(&pcm);
    assert_eq!(out.samples[0], 1.0);
    assert!((out.samples[4000] - 0.5).abs() < 1e-3);
    assert!(out.samples[7999] < 0.01);
}

#[test]
fn export_writes_a_trimmed_wav() {
    let a = asset(2.0);
    let bytes = export_wav(&a, "passthrough", Some((S / 2, S)), None).unwrap();
    let out = Pcm::from_wav(&bytes).unwrap();
    assert_eq!(out.duration_us(), S);
    // The first exported sample is the source sample at 0.5 s (frame 4000), within 16-bit rounding.
    assert!((out.samples[0] - a.pcm().unwrap().samples[4000]).abs() < 1e-3);
}

#[test]
fn export_refuses_a_range_past_the_end() {
    assert!(export_wav(&asset(1.0), "passthrough", Some((0, 2 * S)), None).is_err());
    assert!(export_wav(&asset(1.0), "", None, None).is_err());
}

#[test]
fn metadata_groups_are_delivered_when_crossed() {
    let mut item = PlayerItem::new(asset(3.0));
    item.metadata_groups.push(TimedMetadataGroup {
        start_us: S,
        items: vec![MetadataItem { identifier: "title".into(), value: "Chapter".into() }],
    });
    let mut p = Player::with_item(item);
    p.play();
    p.advance(S / 2);
    assert!(p.take_metadata().is_empty());
    p.advance(S);
    assert_eq!(p.take_metadata().len(), 1);
}

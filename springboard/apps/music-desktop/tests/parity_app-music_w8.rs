//! Parity tests for the Music app's playlist folder rename and delete, playback settings and visualizer levels. The
//! models are read through the library's public API; no test opens a window or plays audio.

use bifrost_music::music::equalizer::BANDS;
use bifrost_music::music::folders::Folders;
use bifrost_music::music::playback::{crossfade_gains, Playback, CROSSFADE_MAX_SECS};
use bifrost_music::music::visualizer::{band_levels, smooth};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-parity-app-music-w8").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn renaming_a_folder_keeps_its_playlists_in_it_across_a_reopen() {
    let dir = scratch("rename");
    let file = dir.join("folders.txt");
    let mut folders = Folders::open(file.clone());
    folders.create("Road").expect("the folder is made");
    folders.put("Sunday Mix", Some(0));
    let moved = folders.rename(0, "Long Road");
    assert_eq!(moved, Some(0), "the renamed folder keeps its place in name order");
    let back = Folders::open(file);
    assert_eq!(back.names(), ["Long Road"]);
    assert_eq!(back.folder_of("Sunday Mix"), Some("Long Road"), "the playlist follows its renamed folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renaming_a_folder_moves_it_in_name_order_and_refuses_a_taken_name() {
    let dir = scratch("rename-order");
    let mut folders = Folders::open(dir.join("folders.txt"));
    folders.create("Gym").expect("the folder is made");
    folders.create("Road").expect("the folder is made");
    assert_eq!(folders.rename(1, "Art"), Some(0), "Art sorts before Gym");
    assert_eq!(folders.names(), ["Art", "Gym"]);
    assert_eq!(folders.rename(0, "gym"), None, "another folder already has the name, case aside");
    assert_eq!(folders.rename(0, "///"), None, "an unusable name is refused");
    assert_eq!(folders.rename(9, "Late"), None, "there is no folder 9");
    assert_eq!(folders.names(), ["Art", "Gym"], "a refused rename changes nothing");
    assert_eq!(folders.rename(0, "ART"), Some(0), "a folder can change the case of its own name");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_folder_keeps_its_playlists_and_lists_them_in_no_folder() {
    let dir = scratch("delete");
    let file = dir.join("folders.txt");
    let mut folders = Folders::open(file.clone());
    folders.create("Road").expect("the folder is made");
    folders.create("Gym").expect("the folder is made");
    // name order: Gym is folder 0, Road is folder 1
    folders.put("Sunday Mix", Some(0));
    folders.put("Leg Day", Some(1));
    assert!(folders.delete(0));
    assert_eq!(folders.names(), ["Road"]);
    assert_eq!(folders.folder_of("Sunday Mix"), None, "the playlist of the deleted folder is in no folder now");
    assert_eq!(folders.folder_of("Leg Day"), Some("Road"), "other folders keep their playlists");
    let back = Folders::open(file);
    assert_eq!(back.folder_of("Sunday Mix"), None, "the deletion survives a reopen");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_folder_that_is_not_there_changes_nothing() {
    let dir = scratch("delete-missing");
    let mut folders = Folders::open(dir.join("folders.txt"));
    folders.create("Road").expect("the folder is made");
    assert!(!folders.delete(3));
    assert_eq!(folders.names(), ["Road"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_crossfade_is_clamped_to_the_longest_offered() {
    assert_eq!(Playback::default().crossfade_secs, 0, "crossfade is off by default");
    assert_eq!(Playback::default().with_crossfade(5).crossfade_secs, 5);
    assert_eq!(Playback::default().with_crossfade(30).crossfade_secs, CROSSFADE_MAX_SECS);
    assert_eq!(CROSSFADE_MAX_SECS, 12);
}

#[test]
fn sound_check_off_plays_every_song_at_unity_gain() {
    let off = Playback::default();
    assert_eq!(off.gain(Some(-30.0)), 1.0);
    assert_eq!(off.gain(Some(-5.0)), 1.0);
}

#[test]
fn sound_check_turns_a_loud_song_down_to_the_target() {
    let on = Playback { crossfade_secs: 0, sound_check: true };
    // -10 dB is 6 dB above the -16 dB target: a gain of 10^(-6/20), about 0.5012
    assert!((on.gain(Some(-10.0)) - 0.5012).abs() < 1e-3, "got {}", on.gain(Some(-10.0)));
    assert_eq!(on.gain(Some(-16.0)), 1.0, "a song at the target keeps its level");
}

#[test]
fn sound_check_limits_how_far_a_quiet_song_is_turned_up() {
    let on = Playback { crossfade_secs: 0, sound_check: true };
    // -40 dB would need +24 dB; the change is limited to +9 dB, a gain of about 2.818
    assert!((on.gain(Some(-40.0)) - 2.818).abs() < 1e-3, "got {}", on.gain(Some(-40.0)));
}

#[test]
fn a_song_with_no_loudness_measurement_plays_at_unity_gain() {
    let on = Playback { crossfade_secs: 0, sound_check: true };
    assert_eq!(on.gain(None), 1.0);
    assert_eq!(on.gain(Some(f32::NAN)), 1.0, "a measurement that is not a number is ignored");
}

#[test]
fn the_playback_settings_read_back_what_was_written() {
    let settings = Playback { crossfade_secs: 4, sound_check: true };
    assert_eq!(Playback::parse(&settings.to_text()), settings);
    assert_eq!(settings.to_text(), "crossfade 4\nsound_check 1\n");
}

#[test]
fn a_damaged_settings_file_keeps_the_defaults_for_the_bad_lines() {
    let parsed = Playback::parse("crossfade soon\nsound_check maybe\nunknown 9\ncrossfade 99\n");
    assert_eq!(parsed, Playback { crossfade_secs: CROSSFADE_MAX_SECS, sound_check: false });
}

#[test]
fn the_crossfade_gains_start_on_the_outgoing_song_and_end_on_the_incoming_one() {
    let (out, into) = crossfade_gains(0.0);
    assert!((out - 1.0).abs() < 1e-6 && into.abs() < 1e-6);
    let (out, into) = crossfade_gains(1.0);
    assert!(out.abs() < 1e-6 && (into - 1.0).abs() < 1e-6);
}

#[test]
fn the_crossfade_keeps_the_total_power_level_through_the_overlap() {
    for step in 0..=10 {
        let t = step as f32 / 10.0;
        let (out, into) = crossfade_gains(t);
        assert!((out * out + into * into - 1.0).abs() < 1e-5, "power at {t} is {}", out * out + into * into);
    }
    let (out, into) = crossfade_gains(0.5);
    assert!((out - into).abs() < 1e-6, "the two songs are equally loud at the middle");
}

#[test]
fn a_full_scale_sine_reads_its_amplitude_in_its_own_band_only() {
    // one second at 8000 Hz holds exactly 1000 cycles of a 1000 Hz sine of amplitude 0.5
    let rate = 8000.0;
    let samples: Vec<f32> = (0..8000).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate).sin()).collect();
    let levels = band_levels(&samples, rate, &[250.0, 1000.0, 2000.0]);
    assert!(levels[0] < 0.01, "250 Hz reads {}", levels[0]);
    assert!((levels[1] - 0.5).abs() < 0.01, "1000 Hz reads {}", levels[1]);
    assert!(levels[2] < 0.01, "2000 Hz reads {}", levels[2]);
}

#[test]
fn silence_reads_zero_in_every_band_and_bands_above_half_the_rate_read_zero() {
    let silent = vec![0.0f32; 4096];
    assert!(band_levels(&silent, 44_100.0, &BANDS).iter().all(|&l| l == 0.0));
    let samples = vec![1.0f32; 64];
    let above = band_levels(&samples, 8000.0, &[4000.0, 9000.0]);
    assert!(above.iter().all(|&l| l == 0.0), "at or above half the rate there is no reading");
    assert!(band_levels(&[], 8000.0, &[1000.0])[0] == 0.0, "an empty block reads zero");
}

#[test]
fn a_bar_rises_quickly_and_falls_slowly_toward_its_level() {
    assert!((smooth(0.0, 1.0) - 0.6).abs() < 1e-6, "a rising bar takes 60 percent of the way");
    assert!((smooth(1.0, 0.0) - 0.85).abs() < 1e-6, "a falling bar takes 15 percent of the way");
    let mut shown = 0.0;
    for _ in 0..40 {
        shown = smooth(shown, 0.8);
    }
    assert!((shown - 0.8).abs() < 1e-3, "a steady level is reached");
}

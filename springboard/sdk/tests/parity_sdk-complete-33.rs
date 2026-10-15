//! Parity sdk-complete-33: headless tests of the SDK paths for the screen saver descriptor, its random source and the idle
//! watch's next check time.

use bifrost_app::idle_watch::IdleWatch;
use bifrost_app::screen_saver::{ManifestError, Rect, SaverManifest, SaverRandom};

#[test]
fn the_sdk_reads_a_saver_descriptor_into_a_view() {
    let manifest = SaverManifest::parse("name = Stars\nframe_interval = 0.1\noption.speed = int 0 5 2\n").unwrap();
    assert_eq!(manifest.name(), "Stars");
    let view = manifest.view(Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }, false);
    assert!(view.has_configure_sheet());
    assert!((view.animation_time_interval() - 0.1).abs() < 1e-12);
}

#[test]
fn the_sdk_refuses_a_descriptor_without_a_name() {
    assert_eq!(SaverManifest::parse("option.speed = bool true\n"), Err(ManifestError::MissingName));
}

#[test]
fn the_sdk_random_source_repeats_for_the_same_seed() {
    let mut a = SaverRandom::new(3);
    let mut b = SaverRandom::new(3);
    assert_eq!(a.next_int_between(0, 100), b.next_int_between(0, 100));
}

#[test]
fn an_idle_watch_reports_its_next_check_time() {
    let mut watch = IdleWatch::new(1_000, 0);
    assert_eq!(watch.next_check_at(), Some(1_000));
    watch.input(400);
    assert_eq!(watch.next_check_at(), Some(1_400));
}

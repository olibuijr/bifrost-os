//! Parity sdk-tests-15: headless layout tests for the floating player capsule (`widgets::PlayerLayout`), the transport
//! controls and the progress line of the audio mini player. The tests check the rectangles the capsule hands to hit
//! testing and drawing; no canvas is drawn and no window is opened.

use bifrost_app::widgets::{Player, PlayerLayout, CAPSULE_PT, PLAYER_PT};
use bifrost_ui::Rect;

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn music() -> Player<'static> {
    Player { title: "Song", subtitle: "Artist", playing: true, progress: Some(0.5), skips: Some(("backward.fill", "forward.fill")) }
}

#[test]
fn headless_the_capsule_is_the_player_height_centred_in_its_bounds() {
    let l = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    assert_eq!(PLAYER_PT, 52.0);
    assert_eq!(l.capsule, bounds(10.0, 14.0, 400.0, 52.0));
}

#[test]
fn a_capsule_in_shorter_bounds_takes_their_whole_height() {
    let l = PlayerLayout::new(bounds(0.0, 0.0, 300.0, 30.0), &music());
    assert_eq!(l.capsule, bounds(0.0, 0.0, 300.0, 30.0));
}

#[test]
fn the_capsule_follows_the_vertical_offset_of_its_bounds() {
    let l = PlayerLayout::new(bounds(0.0, 100.0, 300.0, 80.0), &music());
    assert_eq!(l.capsule.y, 114.0);
}

#[test]
fn the_artwork_is_a_square_inset_from_the_capsule_edge() {
    let l = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    assert_eq!(l.artwork, bounds(16.0, 20.0, 40.0, 40.0));
}

#[test]
fn skip_back_play_and_skip_forward_sit_in_a_row_from_the_trailing_edge() {
    let l = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    let back = l.back.expect("skip back is drawn");
    let forward = l.forward.expect("skip forward is drawn");
    assert_eq!((back.x, l.play.x, forward.x), (290.0, 326.0, 362.0));
    assert!(back.right() <= l.play.x && l.play.right() <= forward.x);
    assert!(forward.right() <= l.capsule.right());
}

#[test]
fn the_transport_buttons_are_squares_of_the_capsule_button_size() {
    let l = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    for r in [l.back.unwrap(), l.play, l.forward.unwrap()] {
        assert_eq!((r.w, r.h), (CAPSULE_PT, CAPSULE_PT));
        assert_eq!(r.y, 22.0, "centred on the capsule");
    }
}

#[test]
fn a_player_without_skips_draws_only_play_and_pause() {
    let p = Player { title: "Live", subtitle: "", playing: false, progress: None, skips: None };
    let l = PlayerLayout::new(bounds(0.0, 0.0, 300.0, 52.0), &p);
    assert!(l.back.is_none() && l.forward.is_none());
    assert_eq!(l.play.x, 300.0 - 12.0 - CAPSULE_PT, "play is the trailing button");
}

#[test]
fn the_text_stops_before_the_first_transport_button() {
    let with_skips = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    assert!(with_skips.text.right() < with_skips.back.unwrap().x);
    let bare = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &Player { skips: None, ..music() });
    assert!(bare.text.right() < bare.play.x);
    assert!(bare.text.w > with_skips.text.w, "no skip buttons leave more room for the title");
}

#[test]
fn the_progress_line_runs_under_the_text_only_when_progress_is_shown() {
    let l = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    let track = l.progress.expect("a track with progress has a progress line");
    assert_eq!((track.x, track.w, track.h), (l.text.x, l.text.w, 4.0));
    assert_eq!(track.bottom(), l.text.bottom());
}

#[test]
fn a_live_stream_without_progress_has_no_progress_line() {
    let live = Player { progress: None, ..music() };
    let l = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &live);
    assert!(l.progress.is_none());
}

#[test]
fn the_same_player_in_the_same_bounds_lays_out_the_same_way() {
    let a = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    let b = PlayerLayout::new(bounds(10.0, 0.0, 400.0, 80.0), &music());
    assert_eq!(a, b);
}

#[test]
fn a_wider_capsule_gives_the_text_more_room() {
    let narrow = PlayerLayout::new(bounds(0.0, 0.0, 300.0, 80.0), &music());
    let wide = PlayerLayout::new(bounds(0.0, 0.0, 500.0, 80.0), &music());
    assert!(wide.text.w > narrow.text.w);
    assert_eq!(wide.play.x - narrow.play.x, 200.0, "the transport moves with the trailing edge");
}

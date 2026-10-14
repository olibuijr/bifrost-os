//! Headless tests for the pure models behind the photo library access, fetch options, smart albums, thumbnail caching,
//! the candidates window, the input method session, the player view and the media library change tracker. Each model
//! is driven through its public API with concrete values. Nothing is drawn, no display or device is needed, and no
//! clock is read: every "now" is a fixed number.

use bifrost_ui::candidates::{clamp_opacity, place, Candidates, LocationHint, PanelType};
use bifrost_ui::imsession::InputSession;
use bifrost_ui::media_changes::{LibraryChange, MediaLibrary, Stamp};
use bifrost_ui::photo_access::{Access, Answer, Error, Status};
use bifrost_ui::photo_caching::{changes, preheat_window, ContentMode, DeliveryMode, Key, Manager};
use bifrost_ui::photo_fetch::{count, fetch, FetchOptions, Item, Sort};
use bifrost_ui::photo_media::{kind_for_name, Kind};
use bifrost_ui::photo_smart_albums::{contains, is_screenshot, members, days_left, purge_due, Asset, SmartAlbum};
use bifrost_ui::player_view::{format_remaining, format_time, scrub_time, toggle_play, Chrome, ControlsStyle, Hit, PlayerHandle, PlayerView, VideoPlayer};
use bifrost_ui::Rect;

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---- photo_access: the one-time prompt and limited access ----

#[test]
fn headless_first_allow_grants_the_whole_library() {
    let mut access = Access::new(Status::NotDetermined);
    assert_eq!(access.request(Answer::Allow), Ok(Status::Authorized));
    assert!(access.allows("any-item"));
}

#[test]
fn headless_a_later_request_returns_the_first_answer_unchanged() {
    let mut access = Access::new(Status::NotDetermined);
    assert_eq!(access.request(Answer::Deny), Ok(Status::Denied));
    assert_eq!(access.request(Answer::Allow), Ok(Status::Denied));
    assert!(!access.allows("any-item"));
    assert_eq!(access.chosen(), Vec::<String>::new());
}

#[test]
fn headless_limited_access_keeps_sorted_unique_identifiers() {
    let mut access = Access::new(Status::NotDetermined);
    let answer = Answer::AllowLimited(ids(&["p2", "p1", "p2"]));
    assert_eq!(access.request(answer), Ok(Status::Limited));
    assert_eq!(access.chosen(), ids(&["p1", "p2"]));
    assert!(access.allows("p1"));
    assert!(!access.allows("p3"));
}

#[test]
fn headless_selection_changes_only_in_limited_access() {
    let mut limited = Access::new(Status::Limited);
    assert_eq!(limited.chosen(), Vec::<String>::new());
    limited.select(&ids(&["p9", "p8"])).unwrap();
    assert_eq!(limited.chosen(), ids(&["p8", "p9"]));
    assert!(limited.allows("p9"));
    let mut authorized = Access::new(Status::Authorized);
    assert_eq!(authorized.select(&ids(&["p9"])), Err(Error::NotLimited(Status::Authorized)));
}

#[test]
fn headless_restricted_library_refuses_every_request() {
    let mut access = Access::new(Status::Restricted);
    assert_eq!(access.request(Answer::Allow), Err(Error::Restricted));
    assert_eq!(access.request(Answer::Deny), Err(Error::Restricted));
    assert_eq!(access.status(), Status::Restricted);
    assert!(!access.allows("any-item"));
}

#[test]
fn headless_undecided_and_denied_apps_see_nothing() {
    assert!(!Access::new(Status::NotDetermined).allows("p1"));
    assert!(!Access::new(Status::Denied).allows("p1"));
    assert_eq!(Access::new(Status::Denied).chosen(), Vec::<String>::new());
}

// ---- photo_media: the kind of a file, from its name ----

#[test]
fn headless_file_kind_comes_from_the_extension() {
    assert_eq!(kind_for_name("IMG_1.JPG"), Kind::Image);
    assert_eq!(kind_for_name("clip.mov"), Kind::Video);
    assert_eq!(kind_for_name("song.mp3"), Kind::Audio);
    assert_eq!(kind_for_name("notes.txt"), Kind::Unknown);
    assert!(Kind::Video.is_video());
    assert!(!Kind::Image.is_video());
}

// ---- photo_fetch: fetch options over a library listing ----

fn item(id: &str, kind: Kind, favourite: bool, trashed: bool, created: u64) -> Item {
    Item { id: id.to_string(), kind, favourite, trashed, created }
}

fn sample_library() -> Vec<Item> {
    vec![
        item("a", Kind::Image, true, false, 100),
        item("b", Kind::Video, false, false, 300),
        item("c", Kind::Image, false, true, 200),
        item("d", Kind::Image, true, false, 200),
    ]
}

fn fetched_ids(items: &[Item], opts: &FetchOptions) -> Vec<String> {
    fetch(items, opts).into_iter().map(|it| it.id.clone()).collect()
}

#[test]
fn headless_default_fetch_drops_the_trash_and_sorts_newest_first() {
    let lib = sample_library();
    assert_eq!(fetched_ids(&lib, &FetchOptions::default()), ids(&["b", "d", "a"]));
    assert_eq!(count(&lib, &FetchOptions::default()), 3);
}

#[test]
fn headless_media_kind_and_favourites_filter_the_listing() {
    let lib = sample_library();
    let images = FetchOptions { media: Some(Kind::Image), ..Default::default() };
    assert_eq!(fetched_ids(&lib, &images), ids(&["d", "a"]));
    let favourites = FetchOptions { favourites_only: true, sort: Sort::OldestFirst, ..Default::default() };
    assert_eq!(fetched_ids(&lib, &favourites), ids(&["a", "d"]));
}

#[test]
fn headless_date_range_includes_from_and_excludes_to() {
    let lib = sample_library();
    let inside = FetchOptions { from: Some(200), to: Some(300), ..Default::default() };
    assert_eq!(fetched_ids(&lib, &inside), ids(&["d"]));
    let wider = FetchOptions { from: Some(200), to: Some(301), ..Default::default() };
    assert_eq!(fetched_ids(&lib, &wider), ids(&["b", "d"]));
}

#[test]
fn headless_limit_takes_the_first_items_after_sorting() {
    let lib = sample_library();
    let two = FetchOptions { limit: Some(2), ..Default::default() };
    assert_eq!(fetched_ids(&lib, &two), ids(&["b", "d"]));
    assert_eq!(count(&lib, &two), 2);
    let many = FetchOptions { limit: Some(10), ..Default::default() };
    assert_eq!(count(&lib, &many), 3);
}

#[test]
fn headless_trashed_items_with_equal_times_are_ordered_by_identifier() {
    let lib = sample_library();
    let opts = FetchOptions { include_trashed: true, sort: Sort::OldestFirst, ..Default::default() };
    assert_eq!(fetched_ids(&lib, &opts), ids(&["a", "c", "d", "b"]));
    assert_eq!(count(&[], &FetchOptions::default()), 0);
}

// ---- photo_smart_albums: rules over the library ----

const DAY: u64 = 86_400;
// "Now" is day 100 after the epoch.
const NOW: u64 = 100 * DAY;

fn asset(id: &str, name: &str, favourite: bool, trashed: bool, created: u64) -> Asset {
    Asset { id: id.to_string(), name: name.to_string(), favourite, trashed, created, deleted_at: None }
}

fn album_library() -> Vec<Asset> {
    let mut trashed = asset("c", "screenshot_old.png", true, true, 10 * DAY);
    trashed.deleted_at = Some(80 * DAY);
    vec![
        asset("a", "Screenshot 1.png", false, false, 99 * DAY),
        asset("b", "IMG_2.jpg", true, false, 60 * DAY),
        trashed,
        asset("d", "IMG_3.jpg", false, false, 100 * DAY + 5),
        asset("e", "IMG_4.jpg", false, false, 70 * DAY),
        asset("f", "IMG_5.jpg", false, false, 71 * DAY),
    ]
}

fn album_ids(album: SmartAlbum, assets: &[Asset]) -> Vec<String> {
    members(album, assets, NOW).into_iter().map(|a| a.id.clone()).collect()
}

#[test]
fn headless_screenshots_match_the_name_prefix_and_skip_the_trash() {
    assert!(is_screenshot("SCREENSHOT-1.png"));
    assert!(is_screenshot("screenshot_2.png"));
    assert!(!is_screenshot("my screenshot.png"));
    assert!(!is_screenshot("screen.png"));
    assert_eq!(album_ids(SmartAlbum::Screenshots, &album_library()), ids(&["a"]));
}

#[test]
fn headless_recents_keep_thirty_days_and_exclude_the_trash() {
    let lib = album_library();
    // e is exactly 30 days old, so it is out; f (71 days) is in; c is trashed.
    assert_eq!(album_ids(SmartAlbum::RecentlyAdded, &lib), ids(&["a", "d", "f"]));
    assert!(!contains(SmartAlbum::RecentlyAdded, &lib[4], NOW));
}

#[test]
fn headless_recents_drop_assets_more_than_a_day_in_the_future() {
    let far = vec![asset("x", "IMG_9.jpg", false, false, NOW + DAY + 1)];
    assert!(album_ids(SmartAlbum::RecentlyAdded, &far).is_empty());
    let tomorrow = vec![asset("y", "IMG_9.jpg", false, false, NOW + DAY)];
    assert_eq!(album_ids(SmartAlbum::RecentlyAdded, &tomorrow), ids(&["y"]));
}

#[test]
fn headless_favourites_and_recently_deleted_follow_the_trash_state() {
    let lib = album_library();
    assert_eq!(album_ids(SmartAlbum::Favourites, &lib), ids(&["b"]));
    assert_eq!(album_ids(SmartAlbum::RecentlyDeleted, &lib), ids(&["c"]));
    assert_eq!(SmartAlbum::ALL.len(), 4);
    assert_eq!(SmartAlbum::RecentlyAdded.title(), "Recents");
    assert_eq!(SmartAlbum::RecentlyDeleted.title(), "Recently Deleted");
}

#[test]
fn headless_trash_is_purged_thirty_days_after_deletion() {
    assert_eq!(days_left(80 * DAY, NOW), 10);
    assert_eq!(days_left(NOW, NOW), 30);
    assert_eq!(days_left(NOW + 5 * DAY, NOW), 30, "a deletion time after now counts as just deleted");
    assert_eq!(days_left(0, NOW), 0);
    assert!(purge_due(70 * DAY, NOW), "exactly 30 days is due");
    assert!(!purge_due(71 * DAY, NOW), "29 days is not due");
    assert!(!purge_due(NOW + DAY, NOW));
}

// ---- photo_caching: preparations ahead of need ----

fn key(asset: &str) -> Key {
    Key::new(asset, 96, 96)
}

#[test]
fn headless_starting_a_preparation_twice_counts_once() {
    let mut m = Manager::new();
    assert_eq!(m.start(&[key("a"), key("b")]), 2);
    assert_eq!(m.start(&[key("b"), key("c")]), 1);
    assert_eq!(m.len(), 3);
    let big = Key::new("a", 512, 512);
    assert_eq!(m.start(std::slice::from_ref(&big)), 1, "the same asset at another size is another preparation");
    assert!(m.is_started(&big) && m.is_started(&key("a")));
}

#[test]
fn headless_stop_cancels_listed_keys_and_ignores_the_rest() {
    let mut m = Manager::new();
    m.start(&[key("a"), key("b"), key("c")]);
    assert_eq!(m.stop(&[key("b"), key("zzz")]), 1);
    assert!(!m.is_started(&key("b")));
    assert_eq!(m.len(), 2);
    m.stop_all();
    assert!(m.is_empty());
}

#[test]
fn headless_high_quality_preparations_are_skipped_when_not_allowed() {
    let mut m = Manager::new();
    assert!(m.allows_caching_high_quality_images());
    m.set_allows_caching_high_quality_images(false);
    let mut hq = key("a");
    hq.delivery = DeliveryMode::HighQualityFormat;
    let mut fast = key("a");
    fast.delivery = DeliveryMode::FastFormat;
    assert_eq!(m.start(&[hq.clone(), fast.clone()]), 1);
    assert!(!m.is_started(&hq));
    assert!(m.is_started(&fast));
}

#[test]
fn headless_default_key_is_opportunistic_aspect_fill() {
    assert_eq!(DeliveryMode::default(), DeliveryMode::Opportunistic);
    assert_eq!(key("a").delivery, DeliveryMode::Opportunistic);
    assert_eq!(key("a").content_mode, ContentMode::AspectFill);
}

#[test]
fn headless_preheat_window_adds_rows_on_both_edges_and_clamps() {
    assert_eq!(preheat_window(5..8, 20, 3), 2..11);
    assert_eq!(preheat_window(0..4, 20, 3), 0..7);
    assert_eq!(preheat_window(18..20, 20, 3), 15..20);
    assert_eq!(preheat_window(25..30, 20, 2), 18..20);
    assert!(preheat_window(0..0, 0, 5).is_empty());
}

#[test]
fn headless_scrolling_lists_the_rows_that_entered_and_left() {
    assert_eq!(changes(2..5, 4..7), (vec![5, 6], vec![2, 3]));
    assert_eq!(changes(0..0, 1..3), (vec![1, 2], vec![]));
    assert_eq!(changes(1..3, 1..3), (vec![], vec![]));
    assert_eq!(changes(0..2, 5..6), (vec![5], vec![0, 1]));
}

// ---- candidates: the candidates window of an input method ----

fn words(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("w{i}")).collect()
}

#[test]
fn headless_new_list_clears_selection_and_select_checks_the_range() {
    let mut c = Candidates::new(PanelType::SingleColumnScrolling);
    c.set_items(words(3));
    assert!(c.select(2));
    assert_eq!(c.selected_string(), Some("w2"));
    assert!(!c.select(3));
    assert_eq!(c.selected(), Some(2));
    c.set_items(words(5));
    assert_eq!(c.selected(), None);
}

#[test]
fn headless_stepping_stops_at_both_ends_and_starts_from_the_right_end() {
    let mut c = Candidates::new(PanelType::SingleColumnScrolling);
    c.set_items(words(3));
    c.move_by(1);
    assert_eq!(c.selected(), Some(0));
    c.move_by(-5);
    assert_eq!(c.selected(), Some(0));
    c.move_by(7);
    assert_eq!(c.selected(), Some(2));
    c.clear_selection();
    c.move_by(-1);
    assert_eq!(c.selected(), Some(2));
}

#[test]
fn headless_grid_rows_move_by_the_column_count() {
    let mut c = Candidates::new(PanelType::ScrollingGrid).with_columns(3);
    c.set_items(words(7));
    c.select(1);
    c.move_row(true);
    assert_eq!(c.selected(), Some(4));
    c.move_row(false);
    assert_eq!(c.selected(), Some(1));
    let mut zero = Candidates::new(PanelType::ScrollingGrid).with_columns(0);
    zero.set_items(words(2));
    zero.select(0);
    zero.move_row(true);
    assert_eq!(zero.selected(), Some(1), "zero columns is treated as one");
}

#[test]
fn headless_pages_clamp_to_the_list_and_scrolling_keeps_the_selection_visible() {
    let mut c = Candidates::new(PanelType::SingleColumnScrolling);
    c.set_items(words(7));
    assert_eq!(c.page_range(2, 3), 2..5);
    assert_eq!(c.page_range(9, 3), 7..7);
    assert_eq!(c.page_of(5, 3), 1);
    c.select(4);
    assert_eq!(c.ensure_visible(0, 3), 2);
    c.select(1);
    assert_eq!(c.ensure_visible(3, 3), 1);
    c.clear_selection();
    assert_eq!(c.ensure_visible(3, 3), 3);
}

#[test]
fn headless_window_placement_flips_when_a_side_does_not_fit() {
    let caret = (100.0, 200.0, 1.0, 20.0);
    assert_eq!(place(LocationHint::Below, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 224.0));
    assert_eq!(place(LocationHint::Above, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 116.0));
    assert_eq!(place(LocationHint::Below, (100.0, 560.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 476.0));
    assert_eq!(place(LocationHint::Above, (100.0, 20.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 44.0));
    assert_eq!(place(LocationHint::Right, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (105.0, 170.0));
    assert_eq!(place(LocationHint::Left, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (105.0, 170.0));
    assert_eq!(place(LocationHint::Below, (760.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (650.0, 224.0));
}

#[test]
fn headless_candidate_opacity_is_clamped_and_nan_is_opaque() {
    assert_eq!(clamp_opacity(1.5), 1.0);
    assert_eq!(clamp_opacity(-0.2), 0.0);
    assert_eq!(clamp_opacity(f32::NAN), 1.0);
    let mut c = Candidates::new(PanelType::SingleRowStepping);
    assert_eq!(c.opacity(), 1.0);
    c.set_opacity(0.25);
    assert_eq!(c.opacity(), 0.25);
}

#[test]
fn headless_show_and_hide_remember_the_side_and_the_dismiss_flag() {
    let mut c = Candidates::new(PanelType::SingleColumnScrolling);
    assert!(!c.is_visible());
    assert!(c.dismisses_automatically());
    c.show(LocationHint::Above);
    assert!(c.is_visible());
    assert_eq!(c.location_hint(), LocationHint::Above);
    assert_eq!(c.position((100.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 116.0));
    c.hide();
    assert!(!c.is_visible());
    c.set_dismisses_automatically(false);
    assert!(!c.dismisses_automatically());
}

// ---- imsession: one input session on the provider side ----

#[test]
fn headless_update_then_commit_returns_the_converted_text_once() {
    let mut s = InputSession::new(1, PanelType::SingleColumnScrolling);
    assert!(s.update("nihao", "你好"));
    assert!(s.is_composing());
    assert_eq!(s.original_string(), "nihao");
    assert_eq!(s.commit(), Some("你好".to_string()));
    assert!(!s.is_composing());
    assert_eq!(s.original_string(), "");
    assert_eq!(s.commit(), None);
}

#[test]
fn headless_cancel_returns_the_raw_keystrokes() {
    let mut s = InputSession::new(1, PanelType::SingleColumnScrolling);
    s.update("nihao", "你好");
    assert_eq!(s.cancel(), Some("nihao".to_string()));
    assert!(!s.is_composing());
    assert_eq!(s.cancel(), None);
}

#[test]
fn headless_selecting_a_candidate_replaces_the_converted_text() {
    let mut s = InputSession::new(1, PanelType::ScrollingGrid);
    s.update("nihao", "你好");
    s.set_candidates(vec!["你好".to_string(), "泥好".to_string()]);
    assert_eq!(s.candidates().len(), 2);
    assert!(s.select_candidate(1));
    assert_eq!(s.composed_string(), "泥好");
    assert!(!s.select_candidate(5));
    assert_eq!(s.composed_string(), "泥好");
    assert_eq!(s.commit(), Some("泥好".to_string()));
}

#[test]
fn headless_a_new_candidate_list_clears_the_selection() {
    let mut s = InputSession::new(1, PanelType::SingleRowStepping);
    s.update("ka", "咔");
    s.set_candidates(vec!["咔".to_string()]);
    assert!(s.select_candidate(0));
    s.set_candidates(vec!["卡".to_string()]);
    assert_eq!(s.candidate_window().selected(), None);
    assert_eq!(s.candidates(), &["卡".to_string()][..]);
}

#[test]
fn headless_closing_returns_pending_text_and_refuses_later_updates() {
    let mut s = InputSession::new(2, PanelType::SingleColumnScrolling);
    assert_eq!(s.client(), 2);
    s.update("nihao", "你好");
    assert_eq!(s.close(), Some("你好".to_string()));
    assert!(!s.is_open());
    assert!(!s.update("a", "a"));
    assert!(!s.is_composing());
    assert!(!s.select_candidate(0));
    assert_eq!(s.close(), None);
}

// ---- player_view: the player view as pure state ----

struct FakePlayer {
    playing: bool,
    pos: f64,
}

impl PlayerHandle for FakePlayer {
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
        self.pos
    }
    fn duration_seconds(&self) -> f64 {
        100.0
    }
    fn seek_seconds(&mut self, seconds: f64) {
        self.pos = seconds;
    }
}

const BOUNDS: Rect = Rect { x: 0.0, y: 0.0, w: 600.0, h: 400.0 };

fn styled(style: ControlsStyle) -> PlayerView {
    PlayerView { controls_style: style, ..PlayerView::new() }
}

fn rect_of(view: &PlayerView, chrome: Chrome) -> Rect {
    view.layout(BOUNDS).into_iter().find(|(c, _)| *c == chrome).map(|(_, r)| r).expect("chrome is laid out")
}

#[test]
fn headless_controls_style_maps_to_visible_chrome() {
    assert!(styled(ControlsStyle::None).visible_chrome().is_empty());
    assert_eq!(styled(ControlsStyle::Minimal).visible_chrome(), vec![Chrome::PlayPause]);
    assert_eq!(styled(ControlsStyle::Floating).visible_chrome(), vec![Chrome::PlayPause, Chrome::Scrubber]);
    assert_eq!(styled(ControlsStyle::Inline).visible_chrome().len(), 4);
}

#[test]
fn headless_inline_layout_fills_the_bar_with_the_scrubber() {
    let mut view = styled(ControlsStyle::Inline);
    view.shows_full_screen_toggle_button = true;
    // Bar: y 356, height 44. Buttons 32 wide, labels 52 wide, padding 8. The scrubber takes 600 - 168 - 48 = 384.
    assert_eq!(rect_of(&view, Chrome::PlayPause), Rect { x: 8.0, y: 362.0, w: 32.0, h: 32.0 });
    assert_eq!(rect_of(&view, Chrome::ElapsedLabel), Rect { x: 48.0, y: 362.0, w: 52.0, h: 32.0 });
    assert_eq!(rect_of(&view, Chrome::Scrubber), Rect { x: 108.0, y: 362.0, w: 384.0, h: 32.0 });
    assert_eq!(rect_of(&view, Chrome::RemainingLabel), Rect { x: 500.0, y: 362.0, w: 52.0, h: 32.0 });
    assert_eq!(rect_of(&view, Chrome::FullScreenToggle), Rect { x: 560.0, y: 362.0, w: 32.0, h: 32.0 });
}

#[test]
fn headless_floating_and_minimal_layouts_sit_where_the_style_puts_them() {
    let floating = styled(ControlsStyle::Floating);
    // The bar is 536 wide, 32 in from each side, and 48 above the bottom edge.
    assert_eq!(rect_of(&floating, Chrome::PlayPause), Rect { x: 40.0, y: 330.0, w: 32.0, h: 32.0 });
    assert_eq!(rect_of(&floating, Chrome::Scrubber), Rect { x: 80.0, y: 330.0, w: 480.0, h: 32.0 });
    let minimal = styled(ControlsStyle::Minimal);
    assert_eq!(minimal.layout(BOUNDS), vec![(Chrome::PlayPause, Rect { x: 284.0, y: 184.0, w: 32.0, h: 32.0 })]);
    assert!(styled(ControlsStyle::None).layout(BOUNDS).is_empty());
}

#[test]
fn headless_scrubber_hits_convert_the_x_position_to_seconds() {
    let view = styled(ControlsStyle::Inline);
    let track = rect_of(&view, Chrome::Scrubber);
    match view.hit_test(BOUNDS, track.x + track.w / 2.0, track.y + 1.0, 100.0) {
        Some(Hit::Scrubber(t)) => assert!((t.seconds() - 50.0).abs() < 0.01, "got {}", t.seconds()),
        other => panic!("expected a scrubber hit, got {other:?}"),
    }
    assert_eq!(scrub_time(track, track.x - 50.0, 100.0).seconds(), 0.0);
    assert_eq!(scrub_time(track, track.x + track.w + 50.0, 100.0).seconds(), 100.0);
    assert_eq!(scrub_time(track, track.x, 0.0).seconds(), 0.0, "no duration gives zero");
}

#[test]
fn headless_taps_toggle_play_and_seek_the_player() {
    let mut view = styled(ControlsStyle::Inline);
    let mut player = FakePlayer { playing: false, pos: 0.0 };
    let play = rect_of(&view, Chrome::PlayPause);
    assert_eq!(view.tap(BOUNDS, play.x + 1.0, play.y + 1.0, &mut player), Some(Hit::PlayPause));
    assert!(player.is_playing());
    view.tap(BOUNDS, play.x + 1.0, play.y + 1.0, &mut player);
    assert!(!player.is_playing());
    let track = rect_of(&view, Chrome::Scrubber);
    view.tap(BOUNDS, track.x + track.w / 4.0, track.y + 1.0, &mut player);
    assert!((player.current_seconds() - 25.0).abs() < 0.01);
    toggle_play(&mut player);
    assert!(player.is_playing());
}

#[test]
fn headless_full_screen_toggle_flips_only_when_its_button_is_shown() {
    let mut view = styled(ControlsStyle::Inline);
    let mut player = FakePlayer { playing: false, pos: 0.0 };
    view.toggle_full_screen();
    assert!(!view.is_full_screen());
    view.shows_full_screen_toggle_button = true;
    let toggle = rect_of(&view, Chrome::FullScreenToggle);
    assert_eq!(view.tap(BOUNDS, toggle.x + 1.0, toggle.y + 1.0, &mut player), Some(Hit::FullScreenToggle));
    assert!(view.is_full_screen());
}

#[test]
fn headless_video_area_and_outside_points() {
    let view = styled(ControlsStyle::Inline);
    assert_eq!(view.hit_test(BOUNDS, 10.0, 10.0, 100.0), Some(Hit::Video));
    assert_eq!(view.hit_test(BOUNDS, -1.0, 10.0, 100.0), None);
    assert_eq!(styled(ControlsStyle::None).hit_test(BOUNDS, 300.0, 390.0, 100.0), Some(Hit::Video));
}

#[test]
fn headless_time_labels_use_minutes_and_hours() {
    assert_eq!(format_time(0.0), "0:00");
    assert_eq!(format_time(65.9), "1:05");
    assert_eq!(format_time(3725.0), "1:02:05");
    assert_eq!(format_time(-4.0), "0:00");
    assert_eq!(format_remaining(20.0, 80.0), "-1:00");
}

#[test]
fn headless_video_player_defaults_match_the_reference() {
    let view = PlayerView::new();
    assert_eq!(view.controls_style, ControlsStyle::Inline);
    assert!(!view.shows_full_screen_toggle_button);
    assert!(view.updates_now_playing_info_center);
    let player: VideoPlayer<u8> = VideoPlayer::new(Some(1));
    assert_eq!(player.video_overlay, Some(1));
}

// ---- media_changes: change records for a media library ----

fn stamp(id: &str, revision: u64) -> Stamp {
    Stamp { id: id.to_string(), revision }
}

#[test]
fn headless_media_changes_are_reported_sorted_while_notifications_are_on() {
    let mut library = MediaLibrary::new();
    library.begin_generating();
    let first = library.observe(&[stamp("b", 1), stamp("a", 1), stamp("c", 1)], 10);
    assert_eq!(first, Some(LibraryChange { inserted: ids(&["a", "b", "c"]), ..Default::default() }));
    let second = library.observe(&[stamp("a", 1), stamp("b", 2), stamp("d", 1)], 20);
    assert_eq!(
        second,
        Some(LibraryChange { inserted: ids(&["d"]), removed: ids(&["c"]), changed: ids(&["b"]) })
    );
    assert_eq!(library.last_modified(), Some(20));
}

#[test]
fn headless_an_unchanged_listing_reports_nothing_and_keeps_the_time() {
    let mut library = MediaLibrary::new();
    library.begin_generating();
    library.observe(&[stamp("a", 1)], 10);
    assert_eq!(library.observe(&[stamp("a", 1)], 30), None);
    assert_eq!(library.last_modified(), Some(10));
    assert!(library.is_loaded());
}

#[test]
fn headless_changes_while_notifications_are_off_are_recorded_but_not_reported() {
    let mut library = MediaLibrary::new();
    assert_eq!(library.observe(&[stamp("a", 1), stamp("b", 1)], 100), None);
    assert_eq!(library.last_modified(), Some(100));
    library.end_generating();
    assert!(!library.is_generating());
    assert_eq!(library.observe(&[stamp("a", 1)], 110), None);
    assert_eq!(library.last_modified(), Some(110));
}

#[test]
fn headless_switching_notifications_on_later_does_not_replay_old_changes() {
    let mut library = MediaLibrary::new();
    library.observe(&[stamp("a", 1)], 10);
    library.begin_generating();
    assert_eq!(library.observe(&[stamp("a", 1)], 20), None);
    assert!(library.observe(&[stamp("a", 1), stamp("z", 1)], 30).is_some());
}

#[test]
fn headless_unload_makes_the_next_listing_count_every_item_as_inserted() {
    let mut library = MediaLibrary::new();
    library.begin_generating();
    library.observe(&[stamp("a", 1)], 10);
    library.unload();
    assert!(!library.is_loaded());
    assert_eq!(library.last_modified(), None);
    assert_eq!(
        library.observe(&[stamp("a", 1)], 50),
        Some(LibraryChange { inserted: ids(&["a"]), ..Default::default() })
    );
}

#[test]
fn headless_a_repeated_identifier_takes_the_later_stamp() {
    let mut library = MediaLibrary::new();
    library.begin_generating();
    library.observe(&[stamp("a", 1)], 10);
    assert_eq!(
        library.observe(&[stamp("a", 1), stamp("a", 2)], 20),
        Some(LibraryChange { changed: ids(&["a"]), ..Default::default() })
    );
}

//! Parity sdk-tests-14: tests through the public API of the file sync model, the file provider item model, the widget
//! accessibility text, and the playback and queue player models. Every clock is a value the test passes in; no test
//! reads the wall clock, a device or a display.

use bifrost_ui::av_asset::{Asset, Pcm};
use bifrost_ui::av_player::{ActionAtItemEnd, ItemStatus, Notification, Player, PlayerItem, PlayerStatus, TimeControlStatus};
use bifrost_ui::av_queue_player::QueuePlayer;
use bifrost_ui::file_provider::{ItemCapabilities, ItemError, ItemIdentifier, LocalContent, ProvidedItem};
use bifrost_ui::file_sync::{ExtensionEnablement, SyncController};
use bifrost_ui::widget_ax::{combine, WidgetAccessibility};
use std::path::{Path, PathBuf};

const S: i64 = 1_000_000;

fn wav(seconds: f32) -> Vec<u8> {
    Pcm::sine(8000, 1, 440.0, seconds).to_wav()
}

fn item(seconds: f32) -> PlayerItem {
    PlayerItem::new(Asset::from_wav_bytes("file:///clip.wav", &wav(seconds)))
}

fn ready_player(seconds: f32) -> Player {
    Player::with_item(item(seconds))
}

fn sync_root() -> SyncController {
    let mut c = SyncController::new();
    c.set_directories(vec![PathBuf::from("/Users/u/Sync")]);
    c
}

// --- file_sync: the sync controller -------------------------------------------------------------------------------

#[test]
fn a_managed_directory_covers_the_items_beneath_it_component_wise() {
    let c = sync_root();
    assert!(c.is_managed(Path::new("/Users/u/Sync")));
    assert!(c.is_managed(Path::new("/Users/u/Sync/notes/a.txt")));
    assert!(!c.is_managed(Path::new("/Users/u/Syncs/a.txt")), "a sibling with a longer name is not beneath the folder");
    assert!(!c.is_managed(Path::new("/Users/u/Other")));
}

#[test]
fn observing_needs_a_managed_directory_and_ends_once() {
    let mut c = sync_root();
    assert!(!c.begin_observing(Path::new("/Users/u/Other")), "an unmanaged folder is refused");
    assert!(c.begin_observing(Path::new("/Users/u/Sync/notes")));
    assert!(c.is_observing(Path::new("/Users/u/Sync/notes/deep/file.txt")));
    assert!(!c.is_observing(Path::new("/Users/u/Sync/other")));
    assert!(c.end_observing(Path::new("/Users/u/Sync/notes")));
    assert!(!c.end_observing(Path::new("/Users/u/Sync/notes")), "a second end reports nothing to end");
}

#[test]
fn replacing_the_managed_directories_drops_observations_outside_them() {
    let mut c = sync_root();
    c.begin_observing(Path::new("/Users/u/Sync/notes"));
    c.set_directories(vec![PathBuf::from("/Users/u/Photos")]);
    assert!(!c.is_observing(Path::new("/Users/u/Sync/notes")));
    assert_eq!(c.directories(), vec![Path::new("/Users/u/Photos")]);
}

#[test]
fn a_badge_image_and_an_item_identifier_resolve_to_the_badge() {
    let mut c = sync_root();
    assert!(c.set_badge_image("cloud", "cloud.png", Some("Synced")));
    assert!(c.set_badge_identifier(Path::new("/Users/u/Sync/a.txt"), Some("cloud")));
    assert_eq!(c.badge_identifier(Path::new("/Users/u/Sync/a.txt")), Some("cloud"));
    let badge = c.badge_for(Path::new("/Users/u/Sync/a.txt")).expect("the item shows the registered badge");
    assert_eq!(badge.image, "cloud.png");
    assert_eq!(badge.label.as_deref(), Some("Synced"));
}

#[test]
fn badges_on_unmanaged_items_are_refused_and_clearing_removes_them() {
    let mut c = sync_root();
    c.set_badge_image("cloud", "cloud.png", None);
    assert!(!c.set_badge_identifier(Path::new("/Users/u/Other/a.txt"), Some("cloud")));
    assert!(c.set_badge_identifier(Path::new("/Users/u/Sync/a.txt"), Some("cloud")));
    assert!(c.set_badge_identifier(Path::new("/Users/u/Sync/a.txt"), None));
    assert!(c.badge_for(Path::new("/Users/u/Sync/a.txt")).is_none());
}

#[test]
fn a_badge_needs_a_name_and_an_empty_label_is_no_label() {
    let mut c = sync_root();
    assert!(!c.set_badge_image("", "cloud.png", None));
    assert!(!c.set_badge_image("cloud", "", None));
    assert!(c.set_badge_image("cloud", "cloud.png", Some("")));
    c.set_badge_identifier(Path::new("/Users/u/Sync/a.txt"), Some("cloud"));
    assert_eq!(c.badge_for(Path::new("/Users/u/Sync/a.txt")).unwrap().label, None);
}

#[test]
fn the_selection_and_the_target_folder_are_reported_back_in_order() {
    let mut c = sync_root();
    assert!(c.selected_items().is_empty());
    assert_eq!(c.targeted(), None);
    let picked = vec![PathBuf::from("/Users/u/Sync/b.txt"), PathBuf::from("/Users/u/Sync/a.txt")];
    c.set_selection(picked.clone(), Some(PathBuf::from("/Users/u/Sync")));
    assert_eq!(c.selected_items(), picked.as_slice());
    assert_eq!(c.targeted(), Some(Path::new("/Users/u/Sync")));
    c.set_selection(Vec::new(), None);
    assert!(c.selected_items().is_empty());
    assert_eq!(c.targeted(), None);
}

// --- file_sync: extension enablement -------------------------------------------------------------------------------

#[test]
fn a_file_manager_provider_is_off_until_the_user_enables_it() {
    let mut e = ExtensionEnablement::new();
    assert!(e.declare("com.example.sync", "badges", "Sync Badges"));
    assert!(!e.is_extension_enabled("com.example.sync", "badges"));
    e.set_extension_enabled("com.example.sync", "badges", true);
    assert!(e.is_extension_enabled("com.example.sync", "badges"));
    e.set_extension_enabled("com.example.sync", "badges", false);
    assert!(!e.is_extension_enabled("com.example.sync", "badges"));
}

#[test]
fn the_management_list_holds_every_declared_provider_with_its_choice() {
    let mut e = ExtensionEnablement::new();
    e.declare("b.app", "z", "Zed");
    e.declare("a.app", "y", "Alpha");
    e.set_extension_enabled("b.app", "z", true);
    let list = e.management_list();
    assert_eq!(list.len(), 2);
    assert_eq!(list.iter().filter(|x| x.enabled).count(), 1);
}

#[test]
fn a_provider_with_an_unusable_app_id_or_title_is_not_declared() {
    let mut e = ExtensionEnablement::new();
    assert!(!e.declare("bad app", "x", "Title"));
    assert!(!e.declare("a.app", "Not A Token", "Title"));
    assert!(!e.declare("a.app", "x", "   "));
    assert!(e.management_list().is_empty());
}

#[test]
fn a_choice_made_before_the_declaration_applies_when_declared() {
    let mut e = ExtensionEnablement::new();
    e.set_extension_enabled("a.app", "x", true);
    assert!(e.declare("a.app", "x", "Badges"));
    assert!(e.is_extension_enabled("a.app", "x"));
    assert!(e.management_list()[0].enabled);
}

// --- file_provider: item identifiers, capabilities and local content ----------------------------------------------

#[test]
fn an_item_identifier_is_refused_when_empty_and_the_containers_differ() {
    assert!(ItemIdentifier::new("").is_none());
    assert_eq!(ItemIdentifier::new("doc-7").unwrap().as_str(), "doc-7");
    let roots = [ItemIdentifier::root_container(), ItemIdentifier::working_set(), ItemIdentifier::trash_container()];
    for (i, a) in roots.iter().enumerate() {
        for b in &roots[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn a_new_item_grants_no_capability_until_the_provider_opts_in() {
    let item = ProvidedItem::new(ItemIdentifier::new("doc-1").unwrap(), ItemIdentifier::root_container(), "notes.txt").unwrap();
    assert_eq!(item.capabilities(), ItemCapabilities::default());
    assert!(!item.can(ItemCapabilities::READING));
    let granted = item.with_capabilities(ItemCapabilities::READING.union(ItemCapabilities::RENAMING));
    assert!(granted.can(ItemCapabilities::READING));
    assert!(granted.can(ItemCapabilities::RENAMING));
    assert!(!granted.can(ItemCapabilities::DELETING));
}

#[test]
fn a_filename_that_is_empty_a_dot_name_or_holds_a_separator_is_refused() {
    let id = || ItemIdentifier::new("doc-1").unwrap();
    let root = || ItemIdentifier::root_container();
    assert_eq!(ProvidedItem::new(id(), root(), "").unwrap_err(), ItemError::InvalidFilename);
    assert_eq!(ProvidedItem::new(id(), root(), "..").unwrap_err(), ItemError::InvalidFilename);
    assert_eq!(ProvidedItem::new(id(), root(), "a/b").unwrap_err(), ItemError::ForbiddenCharacter);
}

#[test]
fn renaming_needs_the_renaming_capability_and_a_valid_name() {
    let mut item = ProvidedItem::new(ItemIdentifier::new("doc-1").unwrap(), ItemIdentifier::root_container(), "a.txt").unwrap();
    assert_eq!(item.rename("b.txt"), Err(ItemError::NotAllowed(ItemCapabilities::RENAMING)));
    let mut item = item.with_capabilities(ItemCapabilities::RENAMING);
    assert_eq!(item.rename("a/b"), Err(ItemError::ForbiddenCharacter));
    assert_eq!(item.filename(), "a.txt", "a refused rename keeps the name");
    assert!(item.rename("b.txt").is_ok());
    assert_eq!(item.filename(), "b.txt");
}

#[test]
fn a_placeholder_downloads_once_and_finishes_into_local_content() {
    let mut item = ProvidedItem::new(ItemIdentifier::new("doc-2").unwrap(), ItemIdentifier::root_container(), "big.bin")
        .unwrap()
        .as_placeholder()
        .with_capabilities(ItemCapabilities::READING);
    assert_eq!(item.local_content(), LocalContent::Placeholder);
    assert_eq!(item.finish_download(), Err(ItemError::InvalidState), "only a running download can finish");
    item.request_download().unwrap();
    assert_eq!(item.local_content(), LocalContent::Downloading);
    item.request_download().unwrap();
    assert_eq!(item.local_content(), LocalContent::Downloading, "asking again while the transfer runs changes nothing");
    item.finish_download().unwrap();
    assert_eq!(item.local_content(), LocalContent::Downloaded);
}

#[test]
fn eviction_drops_local_bytes_but_not_while_downloading() {
    let mut item = ProvidedItem::new(ItemIdentifier::new("doc-3").unwrap(), ItemIdentifier::root_container(), "song.wav")
        .unwrap()
        .with_capabilities(ItemCapabilities::EVICTING.union(ItemCapabilities::READING));
    assert_eq!(item.local_content(), LocalContent::Downloaded);
    item.evict().unwrap();
    assert_eq!(item.local_content(), LocalContent::Placeholder);
    item.request_download().unwrap();
    assert_eq!(item.evict(), Err(ItemError::InvalidState));
}

#[test]
fn eviction_needs_the_evicting_capability() {
    let mut item = ProvidedItem::new(ItemIdentifier::new("doc-4").unwrap(), ItemIdentifier::root_container(), "x").unwrap();
    assert_eq!(item.evict(), Err(ItemError::NotAllowed(ItemCapabilities::EVICTING)));
    assert_eq!(item.local_content(), LocalContent::Downloaded);
}

// --- widget_ax: spoken descriptions -------------------------------------------------------------------------------

#[test]
fn a_widget_speaks_its_label_value_and_hint_in_that_order() {
    let w = WidgetAccessibility::new("Weather").with_value("12 C").with_hint("Double tap to open");
    assert_eq!(w.spoken(), "Weather, 12 C, Double tap to open");
}

#[test]
fn blank_widget_parts_are_left_out_and_the_rest_is_trimmed() {
    assert_eq!(WidgetAccessibility::new(" Clock ").with_hint("Double tap to open").spoken(), "Clock, Double tap to open");
    assert_eq!(WidgetAccessibility::default().with_value("Off").spoken(), "Off");
    assert_eq!(WidgetAccessibility::new("   ").spoken(), "");
}

#[test]
fn a_decorative_widget_is_never_spoken() {
    let mut w = WidgetAccessibility::new("Background").with_value("Blue");
    w.decorative = true;
    assert_eq!(w.spoken(), "");
}

#[test]
fn a_widget_made_of_several_parts_is_one_phrase_without_the_silent_parts() {
    let mut pattern = WidgetAccessibility::new("Pattern");
    pattern.decorative = true;
    let parts = vec![
        WidgetAccessibility::new("Battery").with_value("80%"),
        pattern,
        WidgetAccessibility::new("Calendar").with_value("3 events"),
        WidgetAccessibility::default(),
    ];
    assert_eq!(combine(&parts), "Battery, 80%. Calendar, 3 events");
    assert_eq!(combine(&[]), "");
}

// --- av_player: playback state, seeking, observers and end of item -------------------------------------------------

#[test]
fn a_player_without_an_item_is_unknown_and_paused() {
    let p = Player::new();
    assert_eq!(p.status(), PlayerStatus::Unknown);
    assert_eq!(p.time_control_status(), TimeControlStatus::Paused);
    assert_eq!(p.rate(), 0.0);
    assert_eq!(p.current_time_us(), 0);
}

#[test]
fn a_loaded_item_is_ready_and_plays_at_the_default_rate() {
    let mut p = ready_player(2.0);
    assert_eq!(p.current_item().unwrap().status(), ItemStatus::ReadyToPlay);
    assert_eq!(p.status(), PlayerStatus::ReadyToPlay);
    assert_eq!(p.default_rate(), 1.0);
    p.play();
    assert_eq!(p.rate(), 1.0);
    assert_eq!(p.time_control_status(), TimeControlStatus::Playing);
    p.pause();
    assert_eq!(p.time_control_status(), TimeControlStatus::Paused);
}

#[test]
fn the_clock_moves_only_while_the_player_plays() {
    let mut p = ready_player(2.0);
    p.advance(S / 2);
    assert_eq!(p.current_time_us(), 0, "a paused player does not move");
    p.play();
    p.advance(S / 2);
    assert_eq!(p.current_time_us(), S / 2);
    p.set_rate(2.0);
    p.advance(S / 4);
    assert_eq!(p.current_time_us(), S / 2 + S / 2, "rate two moves the clock twice as far");
}

#[test]
fn seeking_clamps_to_the_item_and_lands_on_a_sample_boundary() {
    let mut p = ready_player(2.0);
    assert_eq!(p.seek_exact(S), S, "one second is a sample boundary at 8 kHz");
    assert_eq!(p.current_time_us(), S);
    assert_eq!(p.seek_exact(10 * S), 2 * S, "a seek past the end lands on the end");
    assert_eq!(p.seek_exact(-S), 0, "a seek before the start lands on the start");
}

#[test]
fn a_player_with_no_item_seeks_to_zero() {
    let mut p = Player::new();
    assert_eq!(p.seek_exact(S), 0);
}

#[test]
fn a_boundary_observer_reports_when_its_time_is_crossed_and_can_be_removed() {
    let mut p = ready_player(2.0);
    let id = p.add_boundary_time_observer(&[S / 2]);
    p.play();
    p.advance(S);
    let events = p.take_events();
    assert!(events.iter().any(|e| e.id == id && e.time_us == S / 2), "the boundary is reported at its time");
    p.remove_time_observer(id);
    p.seek_exact(0);
    p.advance(S);
    assert!(p.take_events().iter().all(|e| e.id != id), "a removed observer stays silent");
}

#[test]
fn a_player_posts_did_play_to_end_once_the_item_finishes() {
    let mut p = ready_player(1.0);
    p.play();
    p.advance(2 * S);
    assert_eq!(p.take_notifications(), vec![Notification::DidPlayToEndTime]);
    assert!(p.take_notifications().is_empty(), "taking notifications clears them");
}

#[test]
fn playing_from_the_end_restarts_the_item() {
    let mut p = ready_player(1.0);
    p.play();
    p.advance(2 * S);
    p.set_rate(1.0);
    assert_eq!(p.current_time_us(), 0);
}

#[test]
fn a_player_with_no_item_pauses_at_the_end_by_default() {
    let p = Player::new();
    assert_eq!(p.action_at_item_end, ActionAtItemEnd::Pause);
}

// --- av_queue_player: queue order, removal and looping -------------------------------------------------------------

#[test]
fn a_queue_plays_its_first_item_and_keeps_the_rest_in_order() {
    let q = QueuePlayer::new(vec![item(1.0), item(2.0), item(3.0)]);
    assert_eq!(q.player.status(), PlayerStatus::ReadyToPlay);
    assert_eq!(q.items().len(), 2);
    assert_eq!(q.items()[0].duration_us(), 2 * S);
    assert_eq!(q.items()[1].duration_us(), 3 * S);
    assert_eq!(q.player.action_at_item_end, ActionAtItemEnd::Advance);
}

#[test]
fn an_empty_queue_has_no_current_item() {
    let q = QueuePlayer::new(Vec::new());
    assert!(q.player.current_item().is_none());
    assert!(q.items().is_empty());
}

#[test]
fn the_queue_advances_to_the_next_item_when_the_current_one_ends() {
    let mut q = QueuePlayer::new(vec![item(1.0), item(2.0)]);
    q.player.play();
    let notes = q.advance(2 * S);
    assert_eq!(notes, vec![Notification::DidPlayToEndTime]);
    assert!(q.items().is_empty(), "the second item is now current");
    assert_eq!(q.player.current_item().unwrap().duration_us(), 2 * S);
    assert_eq!(q.player.rate(), 1.0, "the queue keeps the rate across items");
}

#[test]
fn an_item_inserted_after_a_position_plays_after_that_item() {
    let mut q = QueuePlayer::new(vec![item(1.0), item(2.0)]);
    q.insert(item(3.0), Some(0));
    assert_eq!(q.items().len(), 2);
    assert_eq!(q.items()[0].duration_us(), 2 * S, "the item after position zero is queued behind the first pending item");
    assert_eq!(q.items()[1].duration_us(), 3 * S);
    q.insert(item(4.0), None);
    assert_eq!(q.items()[0].duration_us(), 4 * S, "no position puts the item at the front");
}

#[test]
fn removing_a_queued_item_returns_it_and_an_out_of_range_index_removes_nothing() {
    let mut q = QueuePlayer::new(vec![item(1.0), item(2.0), item(3.0)]);
    assert_eq!(q.remove(0).unwrap().duration_us(), 2 * S);
    assert!(q.remove(9).is_none());
    assert_eq!(q.items().len(), 1);
}

#[test]
fn removing_every_item_leaves_no_current_item() {
    let mut q = QueuePlayer::new(vec![item(1.0), item(2.0)]);
    q.remove_all_items();
    assert!(q.items().is_empty());
    assert_eq!(q.player.status(), PlayerStatus::Unknown);
}

#[test]
fn a_looper_keeps_its_asset_queued_so_playback_never_runs_out() {
    let asset = Asset::from_wav_bytes("file:///loop.wav", &wav(1.0));
    let mut q = QueuePlayer::looper(asset);
    assert_eq!(q.items().len(), 1);
    q.player.play();
    q.advance(2 * S);
    assert_eq!(q.items().len(), 1, "the looped asset is queued again after each item");
    assert_eq!(q.player.current_item().unwrap().duration_us(), S);
}

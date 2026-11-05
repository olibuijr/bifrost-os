use super::library::Scanned;
use super::*;
use springboard::media::audiofile::Tags;
use std::path::Path;

fn scanned(title: &str, artist: &str, added: u64) -> Scanned {
    Scanned {
        path: PathBuf::from(format!("/m/{title}.mp3")),
        tags: Tags { title: Some(title.into()), artist: Some(artist.into()), album: Some("Album".into()), album_artist: None, track: None, year: None, cover: None, lyrics: None, chapters: Vec::new(), replaygain_db: None },
        duration_ms: Some(200_000.0),
        has_cover: false,
        added,
    }
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-app").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A Music over three songs (Alpha and Charlie by Abba, Bravo by Blur; ids 0 to 2 in title order, Alpha the newest
/// file). The service is not connected, so commands to it are dropped. Everything it keeps is under `dir`.
fn music(dir: &Path) -> Music {
    let mut m = Music::new(800, 600, vec![dir.join("Music")], LoadOptions::all(), Client::for_root(dir)).unwrap();
    m.enable_library_store(dir.join("data"));
    let files = vec![scanned("Alpha", "Abba", 300), scanned("Bravo", "Blur", 200), scanned("Charlie", "Abba", 100)];
    m.apply(Done::Scanned(files, Vec::new()));
    m
}

fn song_path(m: &Music, id: usize) -> PathBuf {
    m.library.songs[id].path.clone()
}

fn test_ui() -> Ui {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), 1.0)
}

#[test]
fn an_empty_library_names_the_section_and_says_how_to_add_music() {
    assert_eq!(library_empty_text(Tab::Albums, Scope::All, false, "").0, "No Albums");
    assert_eq!(library_empty_text(Tab::Artists, Scope::All, false, "").0, "No Artists");
    assert_eq!(library_empty_text(Tab::Songs, Scope::All, false, "").0, "No Songs");
    assert_eq!(library_empty_text(Tab::Songs, Scope::RecentlyAdded, false, "").0, "Nothing Added Yet");
    for (tab, scope) in [(Tab::Artists, Scope::All), (Tab::Songs, Scope::Favourites), (Tab::Songs, Scope::RecentlyPlayed)] {
        let hint = library_empty_text(tab, scope, false, "").1;
        assert!(hint.contains("Import") && hint.contains("Music folder"), "{hint}");
    }
    assert_eq!(library_empty_text(Tab::Songs, Scope::All, true, "").1, "", "no hint while the folder is read");
}

#[test]
fn a_search_with_no_match_reads_as_no_results_not_an_empty_library() {
    assert_eq!(library_empty_text(Tab::Songs, Scope::All, false, "zzq"), ("No Results", ""));
    assert_eq!(library_empty_text(Tab::Songs, Scope::Favourites, false, "zzq").0, "No Results");
    assert_eq!(library_empty_text(Tab::Albums, Scope::All, false, "zzq").0, "No Albums", "the search is for the Songs list");
    assert_eq!(library_empty_text(Tab::Songs, Scope::All, true, "zzq").0, "Looking for music\u{2026}", "a scan still comes first");
}

#[test]
fn the_sidebar_puts_radio_above_the_library() {
    let dir = folder("sidebar-order");
    let m = music(&dir);
    let rows: Vec<String> = m
        .sidebar()
        .items
        .iter()
        .map(|i| match i {
            bifrost_app::controls::sidebar::SidebarItem::Row { label, .. } => label.clone(),
            bifrost_app::controls::sidebar::SidebarItem::Header(label) | bifrost_app::controls::sidebar::SidebarItem::Section { label, .. } => format!("[{label}]"),
        })
        .collect();
    assert_eq!(rows[0], "Radio");
    assert_eq!(rows[1], "[Library]");
    assert_eq!(rows.iter().filter(|r| r.as_str() == "Radio").count(), 1, "one Radio row");
    assert!(rows.iter().position(|r| r == "Artists") < rows.iter().position(|r| r == "[Playlists]"));
}

#[test]
fn the_mini_bar_says_live_and_the_time_on_air_for_a_station_and_elapsed_of_total_for_a_song() {
    assert_eq!(mini_status(true, 83_000.0, None), "LIVE \u{00b7} 1:23");
    assert_eq!(mini_status(false, 83_000.0, Some(225_000.0)), "1:23 / 3:45");
    assert_eq!(mini_status(false, 300_000.0, Some(225_000.0)), "3:45 / 3:45", "never past the end");
    assert_eq!(mini_status(false, 5_000.0, None), "0:05", "a song of unknown length shows the elapsed time");
}

#[test]
fn the_mini_bar_has_a_volume_slider_left_of_the_route_button_when_it_is_wide_enough() {
    let ui = test_ui();
    let wide = Mini::new(&ui, 900, 600);
    let track = mini_volume_track(&ui, &wide).expect("a wide bar has a volume slider");
    let route_x = wide.play.0 - ui.pt(MINI_ROUTE_GAP_PT);
    assert!(track.x + track.w < route_x && track.x > wide.text_x(&ui), "{track:?}");
    assert!(wide.bar.contains(track.x, track.y), "the slider sits in the bar");
    let right = mini_status_right(&ui, &wide).expect("a wide bar shows the time");
    assert!(right <= track.x && mini_text_w(&ui, 900, 600) + wide.text_x(&ui) + ui.pt(MINI_STATUS_MIN_PT) <= right, "the title stops before the time and the slider");
    let middle = Mini::new(&ui, 500, 600);
    if mini_volume_track(&ui, &middle).is_some() && mini_status_right(&ui, &middle).is_none() {
        assert!(mini_text_w(&ui, 500, 600) >= ui.pt(MINI_TITLE_MIN_PT), "without the time the title keeps its room");
    }
    let narrow = Mini::new(&ui, 400, 600);
    assert_eq!(mini_volume_track(&ui, &narrow), None, "a narrow bar keeps only its buttons");
}

#[test]
fn dragging_the_mini_bar_volume_sets_the_volume() {
    let dir = folder("mini-volume");
    let mut m = music(&dir);
    let ui = test_ui();
    let mini = Mini::new(&ui, m.w, m.h);
    let Some(track) = mini_volume_track(&ui, &mini) else { return };
    m.vol_drag = Some(track);
    m.touch(&ui, Event::Move(track.x + track.w * 0.25, track.y), 0.0);
    assert!((m.volume - 0.25).abs() < 0.02, "{}", m.volume);
    m.touch(&ui, Event::Up(track.x + track.w * 0.25, track.y), 0.0);
    assert_eq!(m.vol_drag, None);
}

#[test]
fn a_command_the_service_cannot_take_leaves_its_cause_on_screen() {
    let dir = folder("service-down");
    let mut m = music(&dir);
    assert!(m.playback_problem().is_none());
    m.report("tune in the station", Err("the audio service is not reachable (no socket)".into()));
    assert_eq!(m.playback_problem().as_deref(), Some("the audio service is not reachable (no socket)"));
    m.report("tune in the station", Ok(()));
    assert!(m.playback_problem().is_none(), "a command that was sent clears it");
}

#[test]
fn the_same_failure_again_is_not_a_repaint() {
    let dir = folder("same-failure");
    let mut m = music(&dir);
    let cause = || Err("the audio service is not reachable (no socket)".to_string());
    m.report("tune in the station", cause());
    assert!(m.dirty, "the first failure is shown");
    m.dirty = false;
    m.report("tune in the station", cause());
    assert!(!m.dirty, "the same message again changes nothing on screen");
    m.report("tune in the station", Err("the audio service is not reachable (refused)".into()));
    assert!(m.dirty, "another cause is shown");
}

#[test]
fn a_failed_tune_in_waits_for_its_retry_and_the_persons_next_choice_ends_it() {
    let dir = folder("retry-ends");
    let mut m = music(&dir);
    // armed a second ago: due at 100 ms on the app clock, before the usual 250 ms wake
    m.retry.arm(retry::Wanted::Station(99), -900.0);
    assert_eq!(m.next_wake(10.0), Some(100.0), "the app wakes for the retry");
    m.retry_due(99.0);
    assert_eq!(m.retry.pending(), Some(retry::Wanted::Station(99)), "not yet due");
    // due, but the station is no longer in the list: forgotten, not sent
    m.retry_due(100.0);
    assert_eq!(m.retry.pending(), None);
    // pausing, skipping or any other transport choice ends a pending retry
    m.retry.arm(retry::Wanted::Station(99), 0.0);
    m.control(nowplaying::Target::Next);
    assert_eq!(m.retry.pending(), None, "a transport choice ends the retries");
    // choosing a song does too
    m.retry.arm(retry::Wanted::Station(99), 0.0);
    m.push_queue(0.0, true);
    assert_eq!(m.retry.pending(), None);
}

#[test]
fn tuning_in_without_the_service_reports_the_unreachable_socket() {
    let dir = folder("tune-without-service");
    let mut m = music(&dir);
    let tracks = vec![Track::live(4, "http://example.test/live".into())];
    let sent = m.player.play_queue(tracks, 0, false, 1);
    let cause = sent.expect_err("nothing listens on the service socket");
    assert!(cause.contains("audio service"), "{cause}");
}

#[test]
fn the_recent_and_favourite_lists_follow_the_listening_data() {
    let dir = folder("scopes");
    let mut m = music(&dir);
    m.set_scope(Scope::Favourites);
    assert!(m.view.is_empty());
    let charlie = song_path(&m, 2);
    m.stats.toggle_favourite(&charlie);
    m.stats_changed();
    assert_eq!(m.view, [2], "a new favourite shows at once");
    m.set_scope(Scope::RecentlyAdded);
    assert_eq!(m.view, [0, 1, 2], "newest file first");
    m.set_scope(Scope::RecentlyPlayed);
    assert!(m.view.is_empty());
    let alpha = song_path(&m, 0);
    m.stats.record_start(&alpha, 50);
    m.stats_changed();
    assert_eq!(m.view, [0]);
    // the Songs row of the source list leaves the scope and its order
    m.activate(Target::Seg(2), 0.0);
    assert_eq!((m.scope, m.sort, m.view.len()), (Scope::All, SortKey::Title, 3));
    assert_eq!(m.title(), "Songs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn menu_commands_point_the_window_and_act_on_nothing_without_a_song() {
    let dir = folder("menus");
    let mut m = music(&dir);
    let state = m.menu_state();
    assert!(!state.has_track && !state.has_song);
    assert_eq!(state.place, Place::Songs);
    assert!(m.run_command(Command::Rate(3)));
    assert!(m.stats.is_empty(), "no song is playing, so no rating is set");
    m.run_command(Command::Show(Place::Favourites));
    assert_eq!((m.scope, m.menu_state().place), (Scope::Favourites, Place::Favourites));
    m.run_command(Command::Show(Place::Playlists));
    assert_eq!((m.tab, m.scope), (Tab::Playlists, Scope::All));
    assert_eq!(m.menu_state().place, Place::Playlists);
    m.run_command(Command::SortBy(SortKey::Rating));
    assert_eq!((m.tab, m.sort), (Tab::Songs, SortKey::Rating), "sorting shows the songs");
    m.run_command(Command::Show(Place::History));
    assert_eq!((m.page(), m.menu_state().place), (Page::History, Place::History));
    assert!(!m.run_command(Command::Import), "the window owns the open panel");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_crossfade_choice_is_kept_in_the_menu_state_and_the_library_store() {
    let dir = folder("crossfade");
    let mut m = music(&dir);
    assert_eq!(m.menu_state().crossfade_secs, 0, "gapless until chosen");
    assert!(m.run_command(Command::Crossfade(6)));
    assert_eq!(m.menu_state().crossfade_secs, 6);
    assert!(m.run_command(Command::Crossfade(99)));
    assert_eq!(m.menu_state().crossfade_secs, 12, "cut to the longest crossfade");
    m.run_command(Command::Crossfade(4));
    let saved = std::fs::read_to_string(dir.join("data").join("playback.txt")).expect("the choice is written");
    assert!(saved.contains("crossfade 4"), "{saved}");
    // a new session over the same store starts with the saved choice
    let again = music(&dir);
    assert_eq!(again.menu_state().crossfade_secs, 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_smart_playlist_opens_its_rules_and_lists_the_songs_they_pick() {
    let dir = folder("smart");
    let mut m = music(&dir);
    m.activate(Target::Seg(4), 0.0);
    assert_eq!(m.playlist_rows(), 3, "New Playlist, New Smart Playlist and New Playlist Folder");
    m.activate(Target::Row(1), 0.0);
    assert_eq!(m.page(), Page::SmartEdit(0));
    assert_eq!(m.playlist_rows(), 4);
    m.sync_smart();
    assert_eq!(m.smart_view, [0, 1, 2], "a rule without a value keeps every song");
    // Artist contains "abba"
    m.activate(Target::RuleValue(0), 0.0);
    assert!(m.wants_keyboard());
    m.type_text("abba");
    m.type_text("\n");
    assert!(!m.wants_keyboard());
    m.sync_smart();
    assert_eq!(m.smart_view, [0, 2]);
    // a second rule on the title, matching any
    m.activate(Target::RuleAdd, 0.0);
    m.activate(Target::MatchToggle, 0.0);
    m.activate(Target::RuleValue(1), 0.0);
    m.type_text("bravo");
    m.activate(Target::Content, 0.0);
    m.sync_smart();
    assert_eq!(m.smart_view, [0, 1, 2], "tapping elsewhere keeps the typed value");
    // back to the list: the playlist is there, opens, renames and deletes
    m.activate(Target::Back, 0.0);
    m.activate(Target::Row(3), 0.0);
    assert_eq!(m.page(), Page::Smart(0));
    assert_eq!(m.title(), "Smart Playlist");
    m.activate(Target::Rename, 0.0);
    assert_eq!(m.renaming.as_deref(), Some("Smart Playlist"));
    m.renaming = Some("Mix".into());
    m.type_text("\n");
    assert_eq!(m.smart.lists()[0].name, "Mix");
    assert_eq!(m.page(), Page::Smart(0));
    m.activate(Target::DeletePlaylist, 0.0);
    assert!(m.smart.lists().is_empty());
    assert_eq!(m.page(), Page::Root);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cmd_a_selects_the_name_being_renamed_so_typing_replaces_it() {
    let dir = folder("rename-all");
    let mut m = music(&dir);
    m.activate(Target::Seg(4), 0.0);
    m.activate(Target::Row(1), 0.0);
    m.activate(Target::Back, 0.0);
    m.activate(Target::Row(3), 0.0);
    assert_eq!(m.page(), Page::Smart(0));
    assert!(!m.select_all_name(), "no name is being typed yet");
    m.activate(Target::Rename, 0.0);
    assert!(m.select_all_name());
    m.type_text("Mix");
    assert_eq!(m.renaming.as_deref(), Some("Mix"), "the typed text replaces the selected name");
    m.type_text("\n");
    assert_eq!(m.smart.lists()[0].name, "Mix");
    // Backspace on a selected name clears it all
    m.activate(Target::Rename, 0.0);
    assert!(m.select_all_name());
    m.backspace();
    assert_eq!(m.renaming.as_deref(), Some(""));
    // a new rename starts unselected: typing appends
    m.cancel(0.0);
    m.activate(Target::Rename, 0.0);
    m.type_text("!");
    assert_eq!(m.renaming.as_deref(), Some("Mix!"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_smart_playlist_follows_ratings_as_they_change() {
    let dir = folder("smart-live");
    let mut m = music(&dir);
    m.activate(Target::Seg(4), 0.0);
    m.activate(Target::Row(1), 0.0);
    // the starting rule is on the artist; Rating is three fields on (Album, Year, Rating)
    for _ in 0..3 {
        m.activate(Target::RuleField(0), 0.0);
    }
    assert_eq!(m.smart.lists()[0].rules[0].field, smart::Field::Rating);
    m.activate(Target::RuleOp(0), 0.0);
    m.activate(Target::RuleOp(0), 0.0);
    assert_eq!(m.smart.lists()[0].rules[0].op, smart::Op::Greater);
    m.activate(Target::RuleValue(0), 0.0);
    m.rule_edit = Some((0, "3".to_string()));
    m.type_text("\n");
    m.sync_smart();
    assert!(m.smart_view.is_empty(), "nothing is rated above 3 yet");
    let bravo = song_path(&m, 1);
    m.stats.set_rating(&bravo, 5);
    m.stats_changed();
    m.sync_smart();
    assert_eq!(m.smart_view, [1]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_song_menu_steps_the_rating_and_flips_the_favourite() {
    let dir = folder("song-menu");
    let mut m = music(&dir);
    let bravo = song_path(&m, 1);
    m.open_menu(1, None);
    let items = m.menu.as_ref().map(|menu| menu.items.clone()).unwrap();
    assert_eq!(items[..4].to_vec(), vec![MenuItem::PlayNext, MenuItem::PlayLast, MenuItem::Rate, MenuItem::ToggleFavourite]);
    m.run_menu(MenuItem::Rate, 1);
    m.run_menu(MenuItem::Rate, 1);
    assert_eq!(m.stats.rating(&bravo), 2);
    m.run_menu(MenuItem::ToggleFavourite, 1);
    assert!(m.stats.is_favourite(&bravo));
    // the data is kept: a second Music over the same folder reads it back
    let again = music(&dir);
    assert_eq!((again.stats.rating(&bravo), again.stats.is_favourite(&bravo)), (2, true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_song_that_plays_is_logged_in_the_history_and_counted_after_thirty_seconds() {
    let dir = folder("plays");
    let mut m = music(&dir);
    let alpha = song_path(&m, 0);
    m.snap.playing = true;
    m.snap.track = Some(Track::file(0, alpha.clone()));
    m.snap.duration_ms = Some(200_000.0);
    m.snap.position_ms = 1_000.0;
    m.track_plays();
    assert_eq!(m.history_ids(), [0]);
    assert_eq!(m.stats.plays(&alpha), 0);
    m.snap.position_ms = 31_000.0;
    m.track_plays();
    assert_eq!(m.stats.plays(&alpha), 1);
    m.snap.playing = false;
    m.snap.position_ms = 120_000.0;
    m.track_plays();
    assert_eq!(m.stats.plays(&alpha), 1, "a paused song counts for nothing");
    assert_eq!(m.current_song(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_an_up_next_row_moves_that_song() {
    let dir = folder("drag");
    let mut m = music(&dir);
    let paths: Vec<PathBuf> = (0..3).map(|id| song_path(&m, id)).collect();
    m.queue = UpNext::new(paths.clone(), 0);
    m.queue_move(1, 0);
    assert_eq!(m.queue.items().to_vec(), vec![paths[0].clone(), paths[2].clone(), paths[1].clone()]);
    m.queue_move(0, 0);
    assert_eq!(m.queue.items().to_vec(), vec![paths[0].clone(), paths[2].clone(), paths[1].clone()], "dropped where it was");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_route_button_asks_for_the_devices_and_the_chosen_one_is_handed_over() {
    let dir = folder("outputs");
    let mut m = music(&dir);
    assert!(!m.take_output_request());
    m.activate(Target::Route, 0.0);
    assert!(m.take_output_request());
    assert!(!m.take_output_request(), "asked for once");
    m.show_outputs(vec![("speaker".into(), "Speakers".into()), ("headphones".into(), "Headphones".into())], Some("speaker".into()));
    assert!(m.overlay());
    m.activate(Target::MenuItem(1), 0.0);
    assert_eq!(m.take_output_choice().as_deref(), Some("headphones"));
    assert_eq!(m.take_output_choice(), None);
    assert!(!m.overlay());
    m.show_outputs(Vec::new(), None);
    assert!(!m.overlay(), "no devices, no menu");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_copies_songs_into_the_first_music_folder_and_scans_again() {
    let dir = folder("import");
    let mut m = music(&dir);
    assert!(m.import_song("riff.mid", &mut &b"MThd"[..]));
    assert!(dir.join("Music/Imported/riff.mid").exists());
    assert!(!m.import_song("notes.txt", &mut &b"x"[..]));
    assert!(!m.is_scanning());
    m.imports_done(0);
    assert!(!m.is_scanning(), "nothing copied, nothing to scan");
    m.imports_done(1);
    assert!(m.is_scanning());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn get_info_edits_a_song_and_the_scan_lays_the_edit_over_its_tag() {
    let dir = folder("get-info");
    let mut m = music(&dir);
    let bravo = song_path(&m, 1);
    m.open_info(1);
    assert_eq!(m.page(), Page::Info);
    assert_eq!(m.title(), "Get Info");
    m.activate(Target::InfoField(0), 0.0);
    assert!(m.wants_keyboard());
    assert_eq!(m.info_edit.as_ref().map(|(f, t)| (*f, t.as_str())), Some((Field::Title, "Bravo")));
    m.info_edit = Some((Field::Title, "Bravo Live".into()));
    m.type_text("\n");
    assert!(!m.wants_keyboard());
    assert_eq!(m.info.value(&bravo, Field::Title), Some("Bravo Live"));
    // the scan that follows reads the tags again and lays the edit over them
    m.apply(Done::Scanned(vec![scanned("Bravo", "Blur", 200)], Vec::new()));
    assert!(m.library.songs.iter().any(|s| s.path == bravo && s.title == "Bravo Live"));
    m.activate(Target::InfoReset, 0.0);
    assert_eq!(m.info.value(&bravo, Field::Title), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_playlist_moves_into_a_folder_from_its_page_and_the_tab_nests_it() {
    let dir = folder("folders");
    let mut m = music(&dir);
    m.activate(Target::Seg(4), 0.0);
    m.run_command(Command::NewFolder);
    assert_eq!(m.tab, Tab::Playlists);
    assert_eq!(m.folders.names(), ["Folder"]);
    m.activate(Target::Row(0), 0.0);
    assert_eq!(m.page(), Page::Playlist(0));
    m.open_folder_menu();
    m.activate(Target::MenuItem(1), 0.0);
    assert_eq!(m.folders.folder_of("Playlist"), Some("Folder"));
    m.activate(Target::Back, 0.0);
    let rows = m.tab_rows();
    assert_eq!(rows[3], TabRow::Folder(0));
    assert_eq!(rows[4], TabRow::Playlist { index: 0, in_folder: true });
    m.activate(Target::Row(3), 0.0);
    assert_eq!(m.tab_rows().len(), 4, "the folder is hidden and its playlist with it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lyrics_are_read_beside_the_playing_song_and_dropped_with_it() {
    let dir = folder("lyrics");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.lrc"), "[00:01.00]one\n[00:05.00]two\n").unwrap();
    let mut m = music(&dir);
    m.snap.track = Some(Track::file(0, dir.join("x.mp3")));
    m.sync_lyrics();
    assert_eq!(m.lyrics.as_ref().map(|l| l.lines().len()), Some(2));
    assert!(m.menu_state().has_lyrics);
    m.run_command(Command::Lyrics);
    assert!(m.lyrics_on && m.menu_state().lyrics_shown);
    m.snap.track = Some(Track::file(0, dir.join("y.mp3")));
    m.sync_lyrics();
    assert!(m.lyrics.is_none() && !m.lyrics_on, "a song without lyrics shows none");
    m.snap.track = Some(Track::live(7, "http://stream".into()));
    m.sync_lyrics();
    assert!(m.lyrics.is_none(), "a station has none");
    let _ = std::fs::remove_dir_all(&dir);
}

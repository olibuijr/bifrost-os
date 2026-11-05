/// E2E13: a shortcut run whose intent call panics still lowers the in-flight count, so the shell stops polling.
#[test]
fn a_panicking_shortcut_run_is_no_longer_counted() {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let before = super::shortcut_runs().0;
    assert!(super::spawn_shortcut_run(move || {
        rx.recv().ok();
        panic!("the intent call panicked");
    }));
    assert!(super::shortcut_runs().0 > before, "counted while it is out");
    tx.send(()).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while super::shortcut_runs().0 > before && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(super::shortcut_runs().0, before, "the panic unwound through the guard");
    // a run that finishes normally is uncounted the same way
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    assert!(super::spawn_shortcut_run(move || tx.send(()).unwrap()));
    rx.recv().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while super::shortcut_runs().0 > before && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(super::shortcut_runs().0, before);
}

use super::super::tests::{settle, shell, swipe, unlock};
use super::super::{AppId, Effect, Event, Screen, Shell, WifiAction};
use super::*;
use crate::gfx::Surface;

fn ui() -> Ui {
    super::super::tests::ui()
}

/// A finger moving from y0 to y1 at x in ten steps `step_ms` apart.
fn drag(sh: &mut Shell, x: f32, y0: f32, y1: f32, t: f64, step_ms: f64) -> f64 {
    sh.event(Event::Down(x, y0), t);
    for i in 1..=10 {
        sh.event(Event::Move(x, y0 + (y1 - y0) * i as f32 / 10.0), t + step_ms * i as f64);
    }
    let end = t + step_ms * 10.0 + 200.0;
    sh.event(Event::Up(x, y1), end);
    end
}

fn tap(sh: &mut Shell, x: f32, y: f32, t: f64) -> Vec<Effect> {
    let mut fx = sh.event(Event::Down(x, y), t);
    fx.extend(sh.event(Event::Up(x, y), t + 30.0));
    fx
}

fn open(sh: &mut Shell, t: f64) -> f64 {
    let end = drag(sh, 600.0, 800.0, 800.0 + sh.ui.pt(60.0), t, 20.0);
    assert_eq!(sh.quicksearch.describe(), "opening");
    settle(sh, end);
    assert!(sh.quicksearch.is_shown());
    end + 1000.0
}

fn type_text(sh: &mut Shell, text: &str, t: f64) {
    sh.quicksearch.key(&sh.ui, sh.h, Out::Text(text.into()), t);
}

#[test]
fn the_golden_gate_field_hides_the_caret_until_typing() {
    let ui = ui();
    let mut q = QuickSearch::new(&ui, 1200.0, ui.s);
    q.field_mut().focused = true;
    assert!(q.field_mut().draws_caret(0.0), "the tablet field keeps its caret over the placeholder");
    q.set_mac(&ui, true);
    let f = q.field_mut();
    f.focused = true;
    assert!(!f.draws_caret(0.0), "the empty Golden Gate field shows only \"Search\"");
    f.set_text(&ui, "ab");
    assert!(f.draws_caret(0.0), "the caret appears once there is a query");
}

#[test]
fn pull_decides_direction_once_past_the_slop() {
    let p = PullDown::new(100.0, 100.0);
    assert_eq!(p.moved(103.0, 105.0, 10.0), Pull::Undecided);
    assert_eq!(p.moved(100.0, 112.0, 10.0), Pull::Down);
    assert_eq!(p.moved(108.0, 112.0, 10.0), Pull::Down, "45 degrees still counts as a pull");
    assert_eq!(p.moved(114.0, 112.0, 10.0), Pull::Page, "more sideways than down pages");
    assert_eq!(p.moved(100.0, 88.0, 10.0), Pull::Page, "upwards is never a pull");
    assert_eq!(p.moved(-20.0, 100.0, 10.0), Pull::Page);
}

/// E2E9 1: "terminal" finds the built-in Terminal (named "Term" in the Dock) as an Application, and it is the
/// Top Hit, so Return raises it instead of opening the word in Dictionary.
#[test]
fn the_built_in_terminal_is_found_by_its_common_name() {
    let g = results("terminal", &[]);
    assert_eq!(g[0], Group { title: "Top Hit", hits: vec![Hit::App(AppId::Terminal)] });
    assert!(results("term", &[]).iter().any(|g| g.hits.contains(&Hit::App(AppId::Terminal))), "its own name still finds it");
    assert!(results("shell", &[]).iter().any(|g| g.hits.contains(&Hit::App(AppId::Terminal))), "a whole alias finds it");
}

/// E2E10: an alias is found only when the query starts one of its words, never by initials or scattered letters:
/// "wi" is Wi-Fi, not the browser's "web internet", and "se" does not list Term through "shell".
#[test]
fn an_app_alias_matches_only_as_a_word_prefix() {
    assert!(starts_a_word("terminal shell command line", "sh"));
    assert!(starts_a_word("terminal shell command line", "command l"), "running on into the next word");
    assert!(starts_a_word("font book", "font b"));
    assert!(!starts_a_word("web internet", "wi"), "initials do not count");
    assert!(!starts_a_word("terminal shell command line", "se"), "scattered letters do not count");
    assert!(!starts_a_word("web internet", ""));
    assert_eq!(results("wi", &[])[0].hits, vec![Hit::Settings("Wi-Fi")]);
    assert!(!results("se", &[]).iter().any(|g| g.hits.contains(&Hit::App(AppId::Terminal))), "Term is not an \"se\" match");
}

#[test]
fn results_rank_a_top_hit_then_each_source_and_the_terminal_last() {
    let titles = vec!["Severance".to_string(), "Sex Education".to_string(), "Seinfeld".to_string(), "Secret Invasion".to_string()];
    let g = results("se", &titles);
    assert_eq!(g[0], Group { title: "Top Hit", hits: vec![Hit::App(AppId::Settings)] });
    assert_eq!(g[1].title, "Applications");
    assert!(g[1].hits.is_empty() || g[1].hits != vec![Hit::App(AppId::Settings)], "the top hit is not listed twice");
    let deildu = g.iter().find(|g| g.title == "Deildu").unwrap();
    assert_eq!(deildu.hits.len(), PER_GROUP, "each source stops at three");
    assert_eq!(deildu.hits[0], Hit::Deildu("Seinfeld".into()), "shorter prefix matches first");
    assert_eq!(g.last().unwrap(), &Group { title: "Search in Apps", hits: vec![Hit::Terminal("se".into())] });
    assert!(results("", &titles).is_empty());
    assert!(results("  ", &titles).is_empty());
    assert_eq!(results("wi", &[])[0].hits, vec![Hit::Settings("Wi-Fi")]);
    assert_eq!(results("tok", &[])[0].hits, vec![Hit::City("Tokyo")]);
    assert_eq!(results("francisco", &[])[0].hits, vec![Hit::City("San Francisco")]);
    let none = results("zzzz", &[]);
    assert_eq!(none.len(), 1, "only the terminal action: {none:?}");
    assert_eq!(none[0].hits, vec![Hit::Terminal("zzzz".into())]);
}

#[test]
fn settings_intents_are_found_as_shortcuts_and_open_their_pane() {
    let groups = results("open keyboard", &[]);
    let wanted = Hit::SettingsIntent { id: "open-keyboard", title: "Open Keyboard" };
    assert!(groups.iter().flat_map(|g| g.hits.iter()).any(|h| *h == wanted), "{groups:?}");
    assert!(super::super::settings_intents::page_for("open-keyboard").is_some());
    assert!(super::super::settings_intents::page_for("no-such-intent").is_none());
}

#[test]
fn wifi_action_intents_are_found_and_run_their_effect_without_a_window() {
    use super::super::settings_actions::SettingsIntent;
    let groups = results("turn wi-fi off", &[]);
    let wanted = Hit::SettingsAction { intent: SettingsIntent::WifiPower(false), title: "Turn Wi-Fi Off".into() };
    assert!(groups.iter().flat_map(|g| g.hits.iter()).any(|h| *h == wanted), "{groups:?}");
    let scan = Hit::SettingsAction { intent: SettingsIntent::WifiScan, title: "Scan for Wi-Fi Networks".into() };
    assert!(results("scan for wi-fi", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == scan));
    set_wallpaper_catalogue(vec![("kirkjufell_dusk".into(), "Kirkjufell Dusk".into())]);
    let picture = Hit::SettingsAction { intent: SettingsIntent::SetWallpaper("kirkjufell_dusk".into()), title: "Set Wallpaper to Kirkjufell Dusk".into() };
    assert!(results("kirkjufell", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == picture));
    set_wallpaper_catalogue(Vec::new());
    let lock = Hit::SettingsAction { intent: SettingsIntent::LockScreen, title: "Lock Screen".into() };
    assert!(results("lock screen", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == lock));
    let next = Hit::MediaTransport { command: bifrost_ui::media_session::RemoteCommand::Next, title: "Next Track".into() };
    assert!(results("next track", &[]).iter().flat_map(|g| g.hits.iter()).any(|h| *h == next));
}

#[test]
fn search_scope_hides_switched_off_kinds_and_excluded_folders() {
    use super::super::search_scope::{Category, Scope};
    let mut scope = Scope::default();
    scope.set_allowed(Category::Settings, false);
    scope.exclude(Path::new("/home/u/Private"));
    let groups = vec![Group {
        title: "Top Hit",
        hits: vec![
            Hit::SettingsIntent { id: "open-keyboard", title: "Open Keyboard" },
            Hit::File { path: PathBuf::from("/home/u/Private/a.txt"), name: "a.txt".into(), modified: 0 },
            Hit::File { path: PathBuf::from("/home/u/Work/b.txt"), name: "b.txt".into(), modified: 0 },
        ],
    }, Group { title: "Settings", hits: vec![Hit::SettingsIntent { id: "open-sound", title: "Open Sound" }] }, Group { title: "Search in Apps", hits: vec![Hit::Terminal("x".into())] }];
    let kept = scoped(groups, &scope);
    assert_eq!(kept.len(), 2, "the Settings group empties and goes: {kept:?}");
    assert_eq!(kept[0].hits, vec![Hit::File { path: PathBuf::from("/home/u/Work/b.txt"), name: "b.txt".into(), modified: 0 }]);
    assert_eq!(kept[1].title, "Search in Apps", "the action is never hidden");
}

/// A one-piece torrent named `name`.
fn torrent(name: &str) -> Vec<u8> {
    let mut b = format!("d4:infod6:lengthi1e4:name{}:{name}12:piece lengthi16384e6:pieces20:", name.len()).into_bytes();
    b.extend([7u8; 20]);
    b.extend(b"ee");
    b
}

#[test]
fn cached_titles_come_from_the_torrents_on_disk() {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-cache"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(cached_titles(&dir).is_empty(), "no cache yet");
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(dir.join("cache/1.torrent"), torrent("The.Movie.2024.1080p.WEB-DL.x264")).unwrap();
    std::fs::write(dir.join("cache/2.torrent"), torrent("Another_Show_S01E02_720p")).unwrap();
    std::fs::write(dir.join("cache/3.torrent"), torrent("The.Movie.2024.720p")).unwrap();
    std::fs::write(dir.join("cache/4.torrent"), b"not a torrent").unwrap();
    std::fs::write(dir.join("cache/5.img"), b"art").unwrap();
    let titles = cached_titles(&dir);
    assert_eq!(titles, ["Another Show S01E02", "The Movie 2024"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recents_persist_newest_first_and_stop_at_six() {
    let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-recents/quicksearch.ini"));
    let _ = std::fs::remove_file(&path);
    let ui = ui();
    let mut sp = QuickSearch::new(&ui, 1200.0, ui.s);
    sp.load(&path);
    assert!(sp.recents().is_empty());
    for app in [AppId::Settings, AppId::Files, AppId::Settings, AppId::Terminal, AppId::Deildu, AppId::Files] {
        sp.note_launch(app);
    }
    assert_eq!(sp.recents(), [AppId::Files, AppId::Deildu, AppId::Terminal, AppId::Settings]);
    let mut again = QuickSearch::new(&ui, 1200.0, ui.s);
    again.load(&path);
    assert_eq!(again.recents(), [AppId::Files, AppId::Deildu, AppId::Terminal, AppId::Settings]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "recents=files,deildu,terminal,settings\n");
    std::fs::write(&path, "recents=files,bogus,settings\n").unwrap();
    again.load(&path);
    assert_eq!(again.recents(), [AppId::Files, AppId::Settings], "unknown names are skipped");
}

#[test]
fn launches_are_kept_and_predict_the_next_app() {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-launches"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("quicksearch.ini");
    let ui = ui();
    let mut sp = QuickSearch::new(&ui, 1200.0, ui.s);
    sp.load(&path);
    assert!(sp.predicted_apps(1_000_000, 0, 3).is_empty(), "no history, no prediction");
    for app in [AppId::Files, AppId::Files, AppId::Terminal] {
        sp.note_launch(app);
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 60;
    // the last app used (Terminal) is already open, so Files is the prediction
    assert_eq!(sp.predicted_apps(now, 0, 3), [AppId::Files]);
    let mut again = QuickSearch::new(&ui, 1200.0, ui.s);
    again.load(&path);
    assert_eq!(again.launches.count("files"), 2, "the history survives a restart");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_vertical_pull_opens_quicksearch_and_a_page_swipe_does_not() {
    let mut sh = shell();
    unlock(&mut sh);
    // sideways: the pager's, nothing opens
    drag(&mut sh, 900.0, 800.0, 800.0, 1000.0, 20.0);
    sh.event(Event::Down(900.0, 800.0), 2000.0);
    sh.event(Event::Move(700.0, 830.0), 2050.0);
    assert_eq!(sh.quicksearch.describe(), "hidden");
    sh.event(Event::Up(600.0, 840.0), 2100.0);
    settle(&mut sh, 2100.0);
    assert!(!sh.quicksearch.active());
    // a short slow pull springs back
    sh.event(Event::Down(600.0, 800.0), 3000.0);
    sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(20.0)), 3300.0);
    assert_eq!(sh.quicksearch.describe(), "dragging");
    sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(30.0)), 3600.0);
    sh.event(Event::Up(600.0, 800.0 + sh.ui.pt(30.0)), 3900.0);
    assert_eq!(sh.quicksearch.describe(), "closing");
    settle(&mut sh, 3900.0);
    assert!(!sh.quicksearch.active());
    // a pull past 40 pt opens, even when released slowly
    sh.event(Event::Down(600.0, 800.0), 5000.0);
    sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(25.0)), 5300.0);
    sh.event(Event::Move(600.0, 800.0 + sh.ui.pt(50.0)), 5600.0);
    sh.event(Event::Up(600.0, 800.0 + sh.ui.pt(50.0)), 5900.0);
    assert_eq!(sh.quicksearch.describe(), "opening");
    assert!(sh.needs_frame());
    settle(&mut sh, 5900.0);
    assert!(sh.quicksearch.is_shown());
    assert!(sh.describe().contains("quicksearch=shown"));
    assert_eq!(sh.screen, Screen::Home);
    // the sideways drag paged to the App Library, the page after the last home page
    assert_eq!(sh.pager.page(), 1);
}

#[test]
fn pulls_from_the_status_bar_the_home_zone_or_the_lock_screen_do_nothing() {
    let mut sh = shell();
    drag(&mut sh, 600.0, 800.0, 900.0, 0.0, 20.0);
    assert!(!sh.quicksearch.active(), "locked");
    unlock(&mut sh);
    drag(&mut sh, 300.0, 5.0, 300.0, 2000.0, 20.0);
    assert!(!sh.quicksearch.active(), "from the status bar");
    swipe(&mut sh, 600.0, 1915.0, 1500.0, 3000.0);
    settle(&mut sh, 3200.0);
    assert!(!sh.quicksearch.active(), "the home zone is the go-home swipe's");
    sh.open(AppId::Files, 4000.0);
    settle(&mut sh, 4000.0);
    drag(&mut sh, 600.0, 800.0, 900.0, 5000.0, 20.0);
    assert!(!sh.quicksearch.active(), "not inside an app");
}

#[test]
fn typing_lists_results_and_rows_hit_test_in_order() {
    let mut sh = shell();
    unlock(&mut sh);
    let t = open(&mut sh, 1000.0);
    assert!(sh.quicksearch.rows().is_empty(), "empty query lists nothing");
    type_text(&mut sh, "fil", t);
    assert_eq!(sh.quicksearch.query(), "fil");
    let rows = sh.quicksearch.rows();
    assert_eq!(rows[0], (0, 0, Hit::App(AppId::Files)));
    assert_eq!(rows.last().unwrap().2, Hit::Terminal("fil".into()));
    assert_eq!(sh.quicksearch.top_hit(), Some(Hit::App(AppId::Files)));
    let rects = sh.quicksearch.row_rects(&sh.ui, sh.w);
    assert_eq!(rects.len(), rows.len());
    for (i, (at, (x, y, w, h))) in rects.iter().enumerate() {
        assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, x + w / 2.0, y + h / 2.0), Target::Row(at.0, at.1));
        if i > 0 {
            assert!(*y >= rects[i - 1].1 .1 + rects[i - 1].1 .3, "rows run top to bottom");
        }
    }
    let (_, _, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
    let last = rects.last().unwrap().1;
    assert!(last.1 + last.3 <= sh.ui.pt(CARD_TOP_PT) + ch, "the card covers every row");
    assert!(sh.ui.pt(CARD_TOP_PT) + ch < sh.quicksearch.keyboard_top(&sh.ui, sh.h), "and stops above the keyboard");
    let kb = sh.quicksearch.keyboard_top(&sh.ui, sh.h) + 10.0;
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, kb), Target::Keyboard);
    let (fx, fy, fw, fh) = sh.quicksearch.field_rect(&sh.ui, sh.w);
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, fx + fw / 2.0, fy + fh / 2.0), Target::Field);
    let (kx, ky) = sh.quicksearch.clear_centre(&sh.ui, sh.w);
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, kx, ky), Target::Clear);
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, 1300.0), Target::Outside);
    // a long list is cut to what fits above the keyboard
    let titles: Vec<String> = (0..30).map(|i| format!("Clone Wars {i}")).collect();
    sh.quicksearch.set_deildu_titles(titles);
    type_text(&mut sh, "n", t + 10.0);
    let rects = sh.quicksearch.row_rects(&sh.ui, sh.w);
    let last = rects.last().unwrap().1;
    assert!(last.1 + last.3 < sh.quicksearch.keyboard_top(&sh.ui, sh.h));
    // clear empties the query and brings the suggestions back
    sh.quicksearch.note_launch(AppId::Files);
    tap(&mut sh, kx, ky, t + 100.0);
    assert_eq!(sh.quicksearch.query(), "");
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, kx, ky), Target::Field, "no clear button without text");
    let (sx, sy, sw, shh) = sh.quicksearch.suggestion_rect(&sh.ui, sh.w, 0);
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, sx + sw / 2.0, sy + shh / 2.0), Target::Suggestion(0));
}

#[test]
fn tapping_a_row_or_return_opens_the_hit_and_records_it() {
    let mut sh = shell();
    unlock(&mut sh);
    let t = open(&mut sh, 1000.0);
    type_text(&mut sh, "fil", t);
    let (_, (x, y, w, h)) = sh.quicksearch.row_rects(&sh.ui, sh.w)[0];
    sh.event(Event::Down(x + w / 2.0, y + h / 2.0), t + 100.0);
    assert!(sh.event(Event::Up(x + w / 2.0, y + 400.0), t + 130.0).is_empty(), "slid off the row");
    assert!(sh.quicksearch.is_shown());
    tap(&mut sh, x + w / 2.0, y + h / 2.0, t + 200.0);
    assert!(!sh.quicksearch.active());
    settle(&mut sh, t + 300.0);
    assert_eq!(sh.screen, Screen::App(AppId::Files));
    assert_eq!(sh.quicksearch.recents(), [AppId::Files]);
    swipe(&mut sh, 600.0, 1915.0, 1500.0, t + 2000.0);
    settle(&mut sh, t + 2200.0);
    let t = open(&mut sh, t + 4000.0);
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, 1300.0), Target::Outside);
    type_text(&mut sh, "wi", t);
    let (kx, ky) = sh.quicksearch.keyboard.centre("return").unwrap();
    let top = sh.quicksearch.keyboard_top(&sh.ui, sh.h);
    let fx = tap(&mut sh, kx, top + ky, t + 100.0);
    assert_eq!(fx, vec![Effect::Wifi(WifiAction::Scan)], "return opens the top hit: Settings › Wi-Fi");
    settle(&mut sh, t + 200.0);
    assert_eq!(sh.screen, Screen::App(AppId::Settings));
    assert_eq!(sh.settings.as_ref().unwrap().page(), crate::shell::pages::PageId::Wifi);
    assert_eq!(sh.quicksearch.recents(), [AppId::Settings, AppId::Files]);
}

#[test]
fn keyboard_types_into_the_field_and_hide_keeps_the_card() {
    let mut sh = shell();
    unlock(&mut sh);
    let t = open(&mut sh, 1000.0);
    let top = sh.quicksearch.keyboard_top(&sh.ui, sh.h);
    for (i, k) in ["s", "e"].iter().enumerate() {
        let (x, y) = sh.quicksearch.keyboard.centre(k).unwrap();
        tap(&mut sh, x, top + y, t + 100.0 * i as f64);
    }
    assert_eq!(sh.quicksearch.query(), "se");
    assert_eq!(sh.quicksearch.top_hit(), Some(Hit::App(AppId::Settings)));
    let (x, y) = sh.quicksearch.keyboard.centre("hide").unwrap();
    tap(&mut sh, x, top + y, t + 500.0);
    assert!(sh.quicksearch.is_shown() && !sh.quicksearch.keyboard.shown());
    assert!(sh.needs_frame(), "the keyboard slides out");
    settle(&mut sh, t + 500.0);
    assert_eq!(sh.quicksearch.hit(&sh.ui, sh.w, sh.h, 600.0, top + 10.0), Target::Outside, "no keyboard to hit");
    let (fx, fy, fw, fh) = sh.quicksearch.field_rect(&sh.ui, sh.w);
    tap(&mut sh, fx + fw / 2.0, fy + fh / 2.0, t + 2000.0);
    assert!(sh.quicksearch.keyboard.shown(), "tapping the field brings it back");
    assert!(sh.next_wake(t + 2001.0).is_some(), "the caret blinks");
}

#[test]
fn tap_outside_swipe_up_power_and_launch_dismiss() {
    let mut sh = shell();
    unlock(&mut sh);
    let t = open(&mut sh, 1000.0);
    tap(&mut sh, 600.0, 1300.0, t);
    assert_eq!(sh.quicksearch.describe(), "closing");
    settle(&mut sh, t + 30.0);
    assert!(!sh.quicksearch.active());
    let t = open(&mut sh, t + 2000.0);
    drag(&mut sh, 600.0, 1300.0, 700.0, t, 15.0);
    assert_eq!(sh.quicksearch.describe(), "closing");
    settle(&mut sh, t + 400.0);
    assert!(!sh.quicksearch.active());
    // a short tug snaps back
    let t = open(&mut sh, t + 3000.0);
    sh.event(Event::Down(600.0, 1300.0), t);
    sh.event(Event::Move(600.0, 1280.0), t + 200.0);
    assert!(sh.quicksearch.progress(t + 200.0) < 1.0);
    sh.event(Event::Up(600.0, 1280.0), t + 500.0);
    settle(&mut sh, t + 500.0);
    assert!(sh.quicksearch.is_shown());
    // the go-home swipe from the indicator dismisses too
    swipe(&mut sh, 600.0, 1915.0, 1400.0, t + 1000.0);
    assert_eq!(sh.quicksearch.describe(), "closing");
    settle(&mut sh, t + 1200.0);
    assert!(!sh.quicksearch.active() && sh.screen == Screen::Home);
    let t = open(&mut sh, t + 3000.0);
    assert_eq!(sh.event(Event::Power, t), vec![Effect::ScreenOff]);
    assert!(!sh.quicksearch.active() && sh.is_locked());
}

#[test]
fn golden_gate_panel_is_centred_near_the_top_with_no_suggestions() {
    let mut sh = shell();
    unlock(&mut sh);
    sh.quicksearch.note_launch(AppId::Deildu);
    sh.desk = Some(crate::shell::desk::Desk::new());
    open(&mut sh, 2000.0);
    let (x, y, w, h) = sh.quicksearch.card(&sh.ui, sh.w);
    let k = sh.quicksearch.k(&sh.ui);
    assert!((1.0..=MAC_K_MAX).contains(&k));
    assert_eq!(w, sh.ui.pt(520.0 * k), "520 pt, grown with the screen so the card is 0.37 of its width");
    assert!(w >= sh.ui.pt(520.0) && (w >= sh.w as f32 * 0.37 - 1.0 || k == MAC_K_MAX));
    assert!((x - (sh.w as f32 - w) / 2.0).abs() < 1.0);
    assert_eq!(y, sh.ui.pt(139.0 * k));
    assert_eq!(h, sh.ui.pt(sh.quicksearch.m().field * k), "the empty Golden Gate field shows no Assistant Suggestions, only the field");
}

#[test]
fn golden_gate_card_grows_with_a_wide_screen_and_the_bifrost_does_not() {
    let mut sh = shell();
    unlock(&mut sh);
    sh.desk = Some(crate::shell::desk::Desk::new());
    open(&mut sh, 2000.0);
    sh.quicksearch.resize(sh.ui.pt(520.0) * 4.0);
    assert_eq!(sh.quicksearch.k(&sh.ui), MAC_K_MAX.min(0.37 * sh.quicksearch.screen_w / sh.ui.pt(520.0)));
    assert!(sh.quicksearch.k(&sh.ui) > 1.0, "a screen four card widths wide enlarges the card");
    sh.quicksearch.resize(sh.ui.pt(520.0));
    assert_eq!(sh.quicksearch.k(&sh.ui), 1.0, "a narrow screen keeps the point size");
    sh.quicksearch.mac = false;
    sh.quicksearch.resize(sh.ui.pt(520.0) * 4.0);
    assert_eq!(sh.quicksearch.k(&sh.ui), 1.0, "the bifrost layout never scales");
}

#[test]
fn golden_gate_results_have_chips_two_line_rows_and_filter_by_kind() {
    let mut sh = shell();
    unlock(&mut sh);
    sh.desk = Some(crate::shell::desk::Desk::new());
    let t = open(&mut sh, 2000.0);
    assert!(sh.quicksearch.chip_rects(&sh.ui, sh.w).is_empty(), "no chips before anything is typed");
    type_text(&mut sh, "se", t);
    let qs = &sh.quicksearch;
    assert!(!qs.chips.is_empty(), "one chip per kind of result");
    assert!(qs.chips.iter().all(|(title, _)| !NOT_A_KIND.contains(title)), "the best match and the actions are not kinds");
    assert!(qs.sections.iter().all(|sec| sec.subs.len() == sec.rows.len()));
    assert!(qs.sections[0].subs[0].is_some(), "the top row has a second line");
    // the rows start under the chips, as drawn, and are the Golden Gate row height tall
    let chips = qs.chip_rects(&sh.ui, sh.w);
    assert_eq!(chips.len(), qs.chips.len());
    let rows = qs.row_rects(&sh.ui, sh.w);
    assert!(rows[0].1 .1 >= chips[0].1 + chips[0].3, "the first row is under the chips");
    assert_eq!(rows[0].1 .3, qs.px(&sh.ui, qs.m().row));
    let (cx, cy, cw, ch) = chips[0];
    assert_eq!(qs.hit(&sh.ui, sh.w, sh.h, cx + cw / 2.0, cy + ch / 2.0), Target::Chip(0));
    let title = qs.chips[0].0;
    let (x, y) = (cx + cw / 2.0, cy + ch / 2.0);
    // a tap on the chip narrows the list to that kind
    sh.quicksearch.down(&sh.ui, sh.w, sh.h, (x, y), t + 10.0, 0.0);
    assert_eq!(sh.quicksearch.up(&sh.ui, sh.w, sh.h, x, y, t + 40.0), None);
    assert_eq!(sh.quicksearch.chip, Some(title));
    assert_eq!(sh.quicksearch.sections.len(), 1, "one kind of result");
    // the same chip again shows every kind
    sh.quicksearch.down(&sh.ui, sh.w, sh.h, (x, y), t + 100.0, 0.0);
    sh.quicksearch.up(&sh.ui, sh.w, sh.h, x, y, t + 130.0);
    assert_eq!(sh.quicksearch.chip, None);
    assert!(sh.quicksearch.sections.len() >= 1);
}

#[test]
fn golden_gate_card_is_drawn_with_chips_and_symbols_without_panicking() {
    let mut sh = shell();
    unlock(&mut sh);
    sh.desk = Some(crate::shell::desk::Desk::new());
    let t = open(&mut sh, 2000.0);
    type_text(&mut sh, "wi", t);
    let (w, h) = (sh.w as usize, sh.h as usize);
    let mut px = vec![0u32; w * h];
    let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
    sh.quicksearch.draw(&mut s, &sh.ui, sh.w, sh.h, t + 5000.0);
    let (cx, cy, cw, _) = sh.quicksearch.card(&sh.ui, sh.w);
    assert!(px.iter().any(|&p| p >> 24 != 0), "something was drawn over {cx} {cy} {cw}");
}

#[test]
fn suggestions_show_the_recents_and_open_them() {
    let mut sh = shell();
    unlock(&mut sh);
    sh.quicksearch.note_launch(AppId::Deildu);
    let t = open(&mut sh, 2000.0);
    assert_eq!(sh.quicksearch.recents(), [AppId::Deildu]);
    let (x, y, w, h) = sh.quicksearch.suggestion_rect(&sh.ui, sh.w, 0);
    assert!((x - (sh.quicksearch.card_x(&sh.ui, sh.w) + sh.ui.pt(SUGGEST_INSET_PT))).abs() < 1.0);
    let (_, _, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
    assert_eq!(ch, sh.ui.pt(FIELD_PT + HEADER_PT + SUGGEST_PT));
    sh.deildu_transport = crate::deildu::http::tests::Fake::new(vec![]);
    sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-deildu").into();
    tap(&mut sh, x + w / 2.0, y + h / 2.0, t);
    settle(&mut sh, t + 100.0);
    assert_eq!(sh.screen, Screen::App(AppId::Deildu));
    let _ = std::fs::remove_dir_all(&sh.deildu_dir);
}

#[test]
fn deildu_city_and_terminal_hits_open_their_apps() {
    let mut sh = shell();
    unlock(&mut sh);
    sh.term_cmd = &["/bin/sh"];
    sh.deildu_transport = crate::deildu::http::tests::Fake::new(vec![]);
    sh.deildu_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-quicksearch-hits").into();
    let _ = std::fs::remove_dir_all(&sh.deildu_dir);
    let t = open(&mut sh, 1000.0);
    assert!(sh.quicksearch_act(Hit::City("Tokyo"), t).is_empty());
    settle(&mut sh, t);
    sh.go_home(t + 1000.0);
    settle(&mut sh, t + 1000.0);
    sh.quicksearch_act(Hit::Deildu("Severance".into()), t + 2000.0);
    settle(&mut sh, t + 2000.0);
    assert_eq!(sh.screen, Screen::App(AppId::Deildu));
    assert!(sh.deildu.as_ref().unwrap().describe().contains("deildu_screen=library"));
    sh.go_home(t + 3000.0);
    settle(&mut sh, t + 3000.0);
    sh.quicksearch_act(Hit::Terminal("ls -la".into()), t + 4000.0);
    settle(&mut sh, t + 4000.0);
    assert_eq!(sh.screen, Screen::App(AppId::Terminal));
    assert!(sh.term.is_some() && sh.keyboard.shown());
    assert_eq!(sh.quicksearch.recents(), [AppId::Terminal, AppId::Deildu]);
    let _ = std::fs::remove_dir_all(&sh.deildu_dir);
}

#[test]
fn draws_every_phase() {
    let mut sh = shell();
    unlock(&mut sh);
    let mut buf = vec![0u32; 1200 * 1920];
    let mut draw = |sh: &mut Shell, t: f64| {
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        buf.clone()
    };
    sh.event(Event::Down(600.0, 800.0), 1000.0);
    sh.event(Event::Move(600.0, 850.0), 1050.0);
    let mid = draw(&mut sh, 1050.0);
    assert!(sh.quicksearch.active());
    sh.event(Event::Move(600.0, 900.0), 1100.0);
    sh.event(Event::Up(600.0, 900.0), 1120.0);
    draw(&mut sh, 1200.0);
    settle(&mut sh, 1120.0);
    sh.quicksearch.note_launch(AppId::Files);
    let shown = draw(&mut sh, 3000.0);
    let (cx, cy, _, _) = sh.quicksearch.card(&sh.ui, sh.w);
    let at = |buf: &[u32], x: f32, y: f32| buf[y as usize * 1200 + x as usize];
    assert_ne!(at(&shown, cx + 30.0, cy + 30.0), at(&mid, cx + 30.0, cy + 30.0), "the card settles into place");
    let kb_y = sh.quicksearch.keyboard_top(&sh.ui, sh.h) + 5.0;
    assert_ne!(at(&shown, 600.0, kb_y), at(&mid, 600.0, kb_y), "the keyboard is up");
    type_text(&mut sh, "se", 3000.0);
    sh.event(Event::Down(cx + 200.0, cy + sh.ui.pt(FIELD_PT + HEADER_PT + 10.0)), 3100.0);
    let pressed = draw(&mut sh, 3100.0);
    assert_ne!(pressed, shown);
    sh.event(Event::Up(cx + 200.0, cy + 2000.0), 3130.0);
    tap(&mut sh, 600.0, 1300.0, 3200.0);
    draw(&mut sh, 3300.0);
    settle(&mut sh, 3300.0);
    assert!(draw(&mut sh, 5000.0) != shown);
}
#[test]
fn contact_results_match_names_phone_email_and_fold_icelandic_without_duplicates() {
    use super::super::contactstore::Contact;
    let people = vec![
        Contact { id: 7, name: "Þór Árnason".into(), phone: "5551234".into(), email: "thor@example.org".into(), notes: "private-only-value".into(), ..Contact::default() },
        Contact { id: 8, name: "Ægir".into(), phone: "5559876".into(), ..Contact::default() },
    ];
    for (query, id) in [("thor", 7), ("5551234", 7), ("example.org", 7), ("aegir", 8)] {
        let groups = results_with_contacts(query, &[], &people);
        assert_eq!(groups.first().unwrap().title, "Top Hit");
        assert!(matches!(groups[0].hits[0], Hit::Contact { id: got, .. } if got == id));
        assert_eq!(groups.iter().flat_map(|g| &g.hits).filter(|hit| matches!(hit, Hit::Contact { id: got, .. } if *got == id)).count(), 1);
    }
    assert!(results_with_contacts("", &[], &people).is_empty());
    assert!(!results_with_contacts("private-only-value", &[], &people).iter().flat_map(|g| &g.hits).any(|hit| matches!(hit, Hit::Contact { .. })));
}

/// Strict sections match only at word starts, which cost Contacts the search by any part of a phone number: a query
/// of digits alone (spaces, `+` and `-` aside) finds a contact whose number holds those digits anywhere.
#[test]
fn a_contact_is_found_by_any_run_of_digits_of_its_phone_number() {
    use super::super::contactstore::Contact;
    let people = vec![Contact { id: 11, name: "Ada King".into(), phone: "+354 555-1234".into(), ..Contact::default() }];
    let found = |q: &str| results_with_contacts(q, &[], &people).iter().flat_map(|g| &g.hits).any(|hit| matches!(hit, Hit::Contact { id: 11, .. }));
    assert!(found("1234"), "a run inside the number");
    assert!(found("5551"), "a run over the punctuation");
    assert!(found("555 12"), "spaces in the query are ignored");
    assert_eq!(phone_query("+354-555"), Some("354555".into()), "+ and - are ignored too");
    assert!(found("3545551234"));
    assert!(!found("4321"), "the digits must be contiguous");
    assert!(!found("ing"), "letters still need a word start");
    assert_eq!(phone_query("12 3"), Some("123".into()));
    assert_eq!(phone_query("+-"), None);
    assert_eq!(phone_query("a1"), None);
}

#[test]
fn a_contact_is_found_by_company_but_not_by_address_or_notes() {
    use super::super::contactstore::Contact;
    let people = vec![Contact {
        id: 9,
        name: "Ada King".into(),
        company: "Analytical Engines".into(),
        address: "12 Bryn Road".into(),
        notes: "private-only-value".into(),
        ..Contact::default()
    }];
    let found = |q: &str| results_with_contacts(q, &[], &people).iter().flat_map(|g| &g.hits).any(|hit| matches!(hit, Hit::Contact { id: 9, .. }));
    assert!(found("analytical"));
    assert!(!found("bryn road"));
    assert!(!found("private-only-value"));
}

#[test]
fn quicksearch_refreshes_saved_contacts_and_opens_the_selected_person() {
    use super::super::contactstore::{self, Contact};
    let mut sh = shell();
    unlock(&mut sh);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-quicksearch-contacts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("appdata/is.olibuijr.contacts/Contacts/contacts.tsv");
    let person = Contact { id: 42, name: "Þór".into(), phone: "5551234".into(), ..Contact::default() };
    contactstore::save(&path, &[person]).unwrap();
    sh.data_dir = dir.clone();
    let t = open(&mut sh, 1000.0);
    type_text(&mut sh, "thor", t);
    let mut image = crate::gfx::Image::new(sh.w as usize, sh.h as usize);
    sh.draw(&mut Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 }, t);
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/target/quicksearch-contacts.png"), super::super::screenshot::encode(&image)).unwrap();
    // contacts are read on a worker (`refresh_sources`): let the read land, as the frames would
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !matches!(sh.quicksearch.top_hit(), Some(Hit::Contact { .. })) && std::time::Instant::now() < end {
        let h = sh.h as i32;
        sh.quicksearch.tick(&sh.ui, h, t);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let hit = sh.quicksearch.top_hit().unwrap();
    assert!(matches!(hit, Hit::Contact { id: 42, .. }));
    sh.quicksearch_act(hit, t + 50.0);
    sh.go_home(t + 50.0);
    settle(&mut sh, t + 50.0);
    sh.go_home(t + 1000.0);
    settle(&mut sh, t + 1000.0);
    let t = open(&mut sh, t + 2000.0);
    type_text(&mut sh, "5551234", t);
    assert!(matches!(sh.quicksearch.top_hit(), Some(Hit::Contact { id: 42, .. })));
    let _ = std::fs::remove_dir_all(dir);
}

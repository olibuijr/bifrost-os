use super::*;
use crate::shell::store::api::parse_catalog;

const SUM: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn a_description_that_does_not_fit_ends_in_an_ellipsis_at_a_word() {
    let ui = crate::shell::tests::ui();
    let text = "A fast, private web browser powered by Servo, with its own BifrOSt profile";
    let w = ui.measure("A fast, private web browser", 17.0, Weight::Regular) + 4.0;
    let lines = wrap_clamped(&ui, text, 17.0, Weight::Regular, w, 2);
    assert_eq!(lines.len(), 2);
    assert!(lines[1].ends_with('\u{2026}'), "{lines:?}");
    assert!(ui.measure(&lines[1], 17.0, Weight::Regular) <= w);
    let kept = lines[1].trim_end_matches('\u{2026}');
    assert!(text.contains(kept) && !kept.ends_with(' '), "cut at a word boundary: {kept:?}");
    let all = wrap_clamped(&ui, text, 17.0, Weight::Regular, w, 4);
    assert!(!all.last().unwrap().ends_with('\u{2026}'), "text that fits is untouched");
    assert_eq!(all.join(" "), text);
}

#[test]
fn a_package_for_another_sdk_protocol_shows_needs_update_not_get_or_open() {
    let mk = |p: u32, v: &str, kind: &str| StoreApp { id: "a".into(), version: v.into(), sdk_protocol: p, kind: kind.into(), ..Default::default() };
    assert_eq!(button_for(&mk(1, "1.0.0", "app"), None, 2), Btn::NeedsUpdate, "no GET for an app that cannot run");
    assert_eq!(button_for(&mk(1, "1.0.0", "app"), Some("1.0.0"), 2), Btn::NeedsUpdate, "no OPEN for an installed one either");
    assert_eq!(button_for(&mk(2, "1.1.0", "app"), Some("1.0.0"), 2), Btn::Update, "a compatible newer version is an update");
    assert_eq!(button_for(&mk(1, "1.1.0", "app"), Some("1.0.0"), 2), Btn::NeedsUpdate, "a newer incompatible one is no update");
    assert_eq!(button_for(&mk(2, "1.0.0", "app"), None, 2), Btn::Get);
    assert_eq!(button_for(&mk(0, "1.0.0", "app"), Some("1.0.0"), 2), Btn::Open, "unknown protocol: assume it runs");
}

#[test]
fn a_failed_icon_is_retried_after_a_minute_not_never() {
    assert!(icon_due(None, 0.0));
    assert!(!icon_due(Some(1_000.0), 1_000.0 + ICON_RETRY_MS - 1.0));
    assert!(icon_due(Some(1_000.0), 1_000.0 + ICON_RETRY_MS));
}

/// E2E10 finding 10 and E2E11 9: Discover's Essential apps drew F and R tiles for Files and Remote, first for
/// good, then for seconds while a download ran although the icon was on disk.
#[test]
fn an_icon_comes_from_the_install_then_the_cache_and_the_network_only_when_neither_has_it() {
    let png: &[u8] = include_bytes!("../../../apps/files/icon.png");
    let dir = std::env::temp_dir().join(format!("apps-store-icons-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cache = icon_cache_path(&dir, "is.o/files");
    assert_eq!(cache, dir.join("icon-cache").join("is.o_files.png"), "an id cannot leave the cache folder");
    let broken = dir.join("icon.png");
    std::fs::write(&broken, b"not a png").unwrap();
    assert!(local_icon(Some(broken.as_path()), &cache).is_none(), "a damaged install and no cache: fetch it");
    assert!(fetch_icon(|| Some(png.to_vec()), &cache).is_some(), "the catalogue's icon is used");
    assert_eq!(std::fs::read(&cache).unwrap(), png, "the downloaded icon is kept");
    assert_eq!(local_icon(Some(broken.as_path()), &cache).map(|(_, c)| c), Some(true), "a damaged installed icon falls through to the cache");
    assert_eq!(local_icon(None, &cache).map(|(_, c)| c), Some(true), "the cached icon is shown without the network");
    assert!(fetch_icon(|| Some(b"junk".to_vec()), &cache).is_none(), "an undecodable download is no icon");
    assert_eq!(std::fs::read(&cache).unwrap(), png, "and does not replace the cached one");
    assert!(fetch_icon(|| None, &cache).is_none(), "a failed download is no icon");
    let good = dir.join("installed.png");
    std::fs::write(&good, png).unwrap();
    std::fs::remove_file(&cache).unwrap();
    assert_eq!(local_icon(Some(good.as_path()), &cache).map(|(_, c)| c), Some(false), "an installed icon needs no cache");
    let _ = std::fs::remove_dir_all(&dir);
}

/// E2E11 9 and E2E12: an icon already on disk is decoded on a worker, not on the tick (the UI thread), and is in
/// place as soon as the worker answers, before any download; an app with no icon anywhere waits for the retry
/// instead of being asked for again every tick.
#[test]
fn a_cached_icon_is_decoded_off_the_tick_and_painted_without_a_download() {
    let png: &[u8] = include_bytes!("../../../apps/files/icon.png");
    let mut s = store();
    s.pool = Pool::new(1).ok();
    let dir = std::env::temp_dir().join(format!("apps-store-first-tick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.root = dir.clone();
    s.installed.clear();
    let cache = icon_cache_path(&dir, "is.o.calc");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, png).unwrap();
    s.fetching = true; // no catalogue refresh in this test
    s.maintain(0.0);
    assert!(s.icons.get("is.o.calc").is_none(), "the tick does not read or decode the PNG");
    assert!(s.icon_pending.contains("is.o.calc") && s.icon_pending.contains("is.o.chess"), "the worker does");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !s.icon_pending.is_empty() && std::time::Instant::now() < deadline {
        s.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(s.icons.get("is.o.calc"), Some(Some(_))), "the cached icon is painted once the worker answers");
    assert!(!s.icon_pending.contains("is.o.calc"), "and nothing is downloaded for it (no catalogue URL)");
    assert!(s.icons.get("is.o.chess").is_none() && !s.icon_pending.contains("is.o.chess"));
    assert_eq!(s.icon_failed.get("is.o.chess"), Some(&0.0), "no icon on disk and no URL: retried later");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_workers_the_tick_reads_no_icon_and_waits_for_the_retry() {
    let mut s = store();
    s.fetching = true;
    s.maintain(0.0);
    assert!(s.icons.get("is.o.calc").is_none() && !s.icon_pending.contains("is.o.calc"));
    assert_eq!(s.icon_failed.get("is.o.calc"), Some(&0.0));
}

/// A cached icon is painted first and then fetched from the catalogue once more, so an icon the catalogue
/// changed shows; it is revalidated once per session, and a failed revalidation keeps the cached icon.
#[test]
fn a_cached_icon_is_revalidated_once_after_it_is_painted() {
    let png: &[u8] = include_bytes!("../../../apps/files/icon.png");
    let img = crate::cg::imageio::decode(png).unwrap();
    let mut s = store(); // no workers: the revalidation is decided but not sent
    let cache = PathBuf::from("/nonexistent/icon.png");
    s.icon_pending.insert("is.o.calc".into());
    s.local_icon_done("is.o.calc".into(), Some(img.clone()), true, "/store/calc.png".into(), cache.clone());
    assert!(matches!(s.icons.get("is.o.calc"), Some(Some(_))), "the cached copy is painted first");
    assert!(s.icon_revalidated.contains("is.o.calc"), "then fetched again from the catalogue");
    assert!(!s.icon_pending.contains("is.o.calc"));
    // an installed icon, or a cached one with no catalogue URL, is not fetched
    s.local_icon_done("is.o.chess".into(), Some(img.clone()), false, "/store/chess.png".into(), cache.clone());
    s.local_icon_done("is.o.notes".into(), Some(img), true, String::new(), cache);
    assert!(!s.icon_revalidated.contains("is.o.chess") && !s.icon_revalidated.contains("is.o.notes"));
    // the revalidation failing keeps the painted icon and schedules no retry (which would wake the store)
    s.icon_pending.insert("is.o.calc".into());
    s.pool = Pool::new(1).ok();
    s.pool.as_ref().unwrap().spawn(|| Done::Icon { id: "is.o.calc".into(), image: None });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while s.icon_pending.contains("is.o.calc") && std::time::Instant::now() < deadline {
        s.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(s.icons.get("is.o.calc"), Some(Some(_))));
    assert!(!s.icon_failed.contains_key("is.o.calc"));
}

#[test]
fn a_failed_icon_wakes_the_store_when_its_retry_is_due() {
    let mut s = store();
    assert_eq!(s.next_wake(0.0), None);
    s.icon_failed.insert("is.o.calc".into(), 1_000.0);
    assert_eq!(s.next_wake(2_000.0), Some(1_000.0 + ICON_RETRY_MS));
}

#[test]
fn an_icon_whose_retry_is_in_flight_does_not_wake_the_store() {
    let mut s = store();
    s.icon_failed.insert("is.o.calc".into(), 1_000.0);
    s.icon_pending.insert("is.o.calc".into());
    assert_eq!(s.next_wake(1_000.0 + ICON_RETRY_MS + 5_000.0), None, "the pending request's reply wakes it instead");
    s.icon_failed.insert("is.o.chess".into(), 3_000.0);
    assert_eq!(s.next_wake(4_000.0), Some(3_000.0 + ICON_RETRY_MS), "another failed icon still schedules its retry");
}

fn store() -> AppStore {
    let ui = crate::shell::tests::ui();
    let mut s = AppStore::new(&ui, 1000, 1400, std::env::temp_dir().join(format!("apps-store-ui-{}", std::process::id())));
    s.pool = None;
    s.catalog = parse_catalog(&format!(
        r#"{{"apps":[{{"id":"is.o.calc","name":"Calc","version":"1.1.0","sha256":"{SUM}","package":"/p","size":2000000}},{{"id":"is.o.chess","name":"Chess","version":"1.0.0","sha256":"{SUM}","package":"/q","category":"Games"}}]}}"#
    ))
    .unwrap();
    s
}

fn local(id: &str, v: &str) -> InstalledApp {
    InstalledApp { id: id.into(), name: id.into(), version: v.into(), ..Default::default() }
}

#[test]
fn a_new_window_starts_on_the_front_page() {
    let mut s = store();
    s.tab = Tab::Apps;
    s.category = Some(0);
    s.confirm_remove = Some("is.o.calc".into());
    s.page = Some(Page { id: "is.o.calc".into(), back: Tab::Apps, desc: Vec::new(), list_scroll: 0.0, whatsnew: Vec::new(), history: false, hist: Vec::new() });
    s.show_front_page();
    assert_eq!(s.tab, Tab::Today);
    assert!(s.page.is_none() && s.category.is_none() && s.confirm_remove.is_none());
}

#[test]
fn update_rows_show_two_note_lines_until_more_is_tapped() {
    let mut s = store();
    s.catalog[0].changelog = vec![api::Release { version: "1.1.0".into(), date: "2026-10-05".into(), notes: vec!["a".into(), "b".into(), "c".into()] }];
    s.installed = vec![local("is.o.calc", "1.0.0")];
    s.tab = Tab::Updates;
    let find = |s: &AppStore| s.rows().into_iter().find_map(|r| if let Row::Update(i, n, m) = r { Some((i, n, m)) } else { None });
    assert_eq!(find(&s), Some((0, 2, true)));
    s.expanded.insert("is.o.calc".into());
    assert_eq!(find(&s), Some((0, 3, false)));
    assert!(Row::Update(0, 3, false).height() > Row::Update(0, 2, true).height() && Row::Update(0, 0, false).height() == ROW);
}

fn with_background_packages() -> AppStore {
    let mut s = store();
    for (id, kind) in [("is.o.update", "service"), ("is.o.audio", "service"), ("is.o.kb", "system"), ("is.o.files", "system-app")] {
        s.catalog.push(StoreApp { id: id.into(), name: id.into(), version: "2.0.0".into(), min_shell: "0.0.0".into(), kind: kind.into(), category: "System".into(), ..Default::default() });
    }
    s
}

fn listed(s: &AppStore) -> Vec<String> {
    s.rows().into_iter().filter_map(|r| if let Row::App(i) | Row::Update(i, ..) = r { Some(s.catalog[i].id.clone()) } else { None }).collect()
}

#[test]
fn services_are_hidden_from_every_tab_and_the_badge() {
    let mut s = with_background_packages();
    s.installed = vec![InstalledApp { kind: "service".into(), ..local("is.o.update", "1.0.0") }, local("is.o.audio", "1.0.0")];
    for tab in [Tab::Today, Tab::Games, Tab::Apps, Tab::Updates, Tab::Search, Tab::Categories, Tab::Shelf(Shelf::Develop), Tab::Shelf(Shelf::Play)] {
        s.tab = tab;
        let ids = listed(&s);
        assert!(!ids.iter().any(|i| i == "is.o.update" || i == "is.o.audio"), "{tab:?} lists a service");
    }
    assert_eq!(s.update_count(), 0);
    let plan = api::system_plan(&s.catalog, &[], "9.9.9", &[]);
    assert!(plan.iter().any(|a| a.id == "is.o.update") && plan.iter().any(|a| a.id == "is.o.audio"));
    assert!(api::updates(&s.catalog, &s.installed).is_empty());
}

#[test]
fn background_packages_never_offer_get() {
    let mut s = with_background_packages();
    for a in s.catalog.clone().iter().filter(|a| a.is_preinstalled()) {
        assert_ne!(s.button(a), Btn::Get, "{}", a.id);
    }
    let files = s.catalog.iter().find(|a| a.id == "is.o.files").unwrap().clone();
    s.installed.push(InstalledApp { kind: "system-app".into(), ..local("is.o.files", "2.0.0") });
    assert_eq!(s.button(&files), Btn::Open);
    s.tab = Tab::Apps;
    assert!(listed(&s).contains(&"is.o.kb".to_string()));
}

#[test]
fn button_follows_install_state() {
    let mut s = store();
    let calc = s.catalog[0].clone();
    assert_eq!(s.button(&calc), Btn::Get);
    s.installed.push(local("is.o.calc", "1.0.0"));
    assert_eq!(s.button(&calc), Btn::Update);
    s.installed[0].version = "1.1.0".into();
    assert_eq!(s.button(&calc), Btn::Open);
    s.jobs.insert(calc.id.clone(), Job { done: Arc::new(AtomicU64::new(500_000)), size: 2_000_000, cancel: Arc::new(AtomicBool::new(false)) });
    assert_eq!(s.button(&calc), Btn::Progress(0.25));
}

#[test]
fn tabs_split_apps_and_games_and_updates_list_only_newer() {
    let mut s = store();
    s.tab = Tab::Games;
    assert!(s.rows().iter().any(|r| matches!(r, Row::App(1))) && !s.rows().iter().any(|r| matches!(r, Row::App(0))));
    s.tab = Tab::Updates;
    assert!(s.rows().iter().any(|r| matches!(r, Row::Note(n) if n.contains("up to date"))));
    s.installed.push(local("is.o.calc", "1.0.0"));
    assert_eq!(s.update_list(), vec![0]);
    assert!(s.rows().contains(&Row::Header("Available Updates".into(), true)));
}

#[test]
fn open_hands_the_installed_app_to_the_host() {
    let mut s = store();
    s.installed.push(local("is.o.calc", "1.1.0"));
    s.press_button("is.o.calc");
    assert_eq!(s.take_launch().map(|a| a.id), Some("is.o.calc".to_string()));
    assert!(s.take_launch().is_none());
}

#[test]
fn icon_is_opaque_and_sizes_read_well() {
    assert!(icon(64).px.iter().all(|p| p >> 24 == 255));
    assert_eq!((size_text(0), size_text(2_500_000), size_text(900)), ("-".to_string(), "2.5 MB".to_string(), "900 bytes".to_string()));
    assert_eq!(row_detail("Productivity", "1.7.0", 2_500_000), "Productivity \u{b7} v1.7.0 \u{b7} 2.5 MB", "every row names the version the store has");
    assert_eq!(crate::bare::timezone::civil_from_days(0), (1970, 1, 1));
}

fn with_system() -> AppStore {
    let mut s = store();
    s.catalog.push(StoreApp { id: "is.o.kb".into(), name: "Keyboard".into(), version: "1.1.0".into(), kind: "system".into(), category: "System".into(), size: 11_000_000, ..Default::default() });
    s
}

#[test]
fn apps_tab_lists_system_components_in_their_own_section() {
    let mut s = with_system();
    s.tab = Tab::Apps;
    let rows = s.rows();
    let at = rows.iter().position(|r| *r == Row::Header("System".into(), false)).expect("System header");
    assert_eq!(rows[at + 1..], [Row::App(2)]);
    assert!(!rows[..at].contains(&Row::App(2)), "not among Top Apps");
    s.tab = Tab::Games;
    assert!(!s.rows().contains(&Row::App(2)));
    s.tab = Tab::Today;
    assert!(!s.rows().contains(&Row::App(2)) && !s.rows().iter().any(|r| matches!(r, Row::Cards(c) if c.iter().any(|k| k.apps().contains(&2)))));
    s.tab = Tab::Search;
    assert!(!s.rows().contains(&Row::App(2)));
}

#[test]
fn system_components_have_no_get_or_open_only_update_and_count_in_the_badge() {
    let mut s = with_system();
    let kb = s.catalog[2].clone();
    assert_eq!(s.button(&kb), Btn::None, "not installed yet: preinstalled in the background, no GET");
    s.installed.push(InstalledApp { kind: "system".into(), ..local("is.o.kb", "1.1.0") });
    assert_eq!(s.button(&kb), Btn::None, "current: no OPEN");
    s.press_button("is.o.kb");
    assert!(s.take_launch().is_none(), "a system component never opens");
    assert_eq!(s.update_count(), 0);
    s.installed[0].version = "1.0.0".into();
    assert_eq!(s.button(&kb), Btn::Update);
    assert_eq!((s.update_count(), s.update_list()), (1, vec![2]));
    s.installed.push(local("is.o.calc", "1.0.0"));
    assert_eq!(s.update_count(), 2, "apps and system components both count");
    assert_eq!(category_label(&kb), "System Component");
    assert_eq!(category_label(&s.catalog[0]), "Utilities");
}

#[test]
fn purchased_lists_installed_apps_by_purchase_day_and_hides_services() {
    let mut s = store();
    s.installed = vec![
        InstalledApp { purchased: 120, ..local("is.o.chess", "1.0.0") },
        InstalledApp { purchased: 100, ..local("is.o.calc", "1.1.0") },
        InstalledApp { kind: "service".into(), purchased: 90, ..local("is.o.update", "1.0.0") },
        InstalledApp { purchased: 9_999_999, ..local("is.o.later", "1.0.0") },
    ];
    assert_eq!(s.purchased(200), vec![0, 1], "the service and the purchase after today are left out");
    s.tab = Tab::Updates;
    let rows = s.rows();
    let at = rows.iter().position(|r| *r == Row::Header("Purchased".into(), false)).expect("Purchased header");
    assert_eq!(rows[at + 1..at + 3], [Row::App(1), Row::App(0)], "chess (bought first) then calc, both in the catalogue");
    assert_eq!(rows[at + 3], Row::Restore, "Restore Purchases closes the tab");
}

#[test]
fn a_paid_app_opens_the_purchase_sheet_and_installs_nothing_yet() {
    let mut s = store();
    s.catalog[1].price = Some(bifrost_ui::purchase::Price::new(990, 0, "ISK"));
    let chess = s.catalog[1].clone();
    s.press_button(&chess.id);
    assert!(s.jobs.is_empty(), "no download before the customer confirms");
    let open = s.sheet.as_ref().expect("the sheet is up");
    assert_eq!((open.id.as_str(), open.purchase.price.display()), ("is.o.chess", "990 ISK".to_string()));
    assert_eq!(s.button(&chess), Btn::Get);
}

#[test]
fn rating_an_app_keeps_the_review_and_reopens_it_for_editing() {
    let ui = crate::shell::tests::ui();
    let mut s = store();
    let _ = std::fs::remove_file(s.root.join(REVIEWS_FILE));
    s.reviews = bifrost_core::app_review::ReviewBook::new();
    let sidebar = s.sidebar as f32;
    let to_window = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0 + sidebar, r.1 + r.3 / 2.0);
    s.activate(&ui, Target::Rate("is.o.chess".into()));
    assert_eq!(s.review.as_ref().map(|r| r.id.as_str()), Some("is.o.chess"));
    let (x, y) = to_window(review_sheet::rating_rects(&ui, s.w, s.h)[3]);
    s.touch(&ui, Event::Up(x, y), 0.0);
    let (x, y) = to_window(review_sheet::body_rect(&ui, s.w, s.h));
    s.touch(&ui, Event::Up(x, y), 0.0);
    assert!(s.take_keyboard_request(), "a tap on a field raises the keyboard");
    s.key(Out::Text("Fun to play".into()));
    let (x, y) = to_window(review_sheet::buttons(&ui, s.w, s.h).0);
    s.touch(&ui, Event::Up(x, y), 0.0);
    assert!(s.review.is_none(), "Send closes the sheet");
    let kept = s.reviews.get("is.o.chess").expect("the review is in the book");
    assert_eq!((kept.stars, kept.body.as_str()), (4, "Fun to play"));
    // the book is on disk, so a new store opens the sheet with the review to edit
    let again = AppStore::new(&ui, 1000, 1400, s.root.clone());
    assert_eq!(again.reviews.get("is.o.chess"), s.reviews.get("is.o.chess"));
    s.activate(&ui, Target::Rate("is.o.chess".into()));
    assert_eq!(s.review.as_ref().map(|r| (r.stars(), r.body().to_string())), Some((4, "Fun to play".to_string())));
    let (x, y) = to_window(review_sheet::buttons(&ui, s.w, s.h).1);
    s.touch(&ui, Event::Up(x, y), 0.0);
    assert!(s.review.is_none(), "Cancel closes it and keeps the earlier review");
    assert_eq!(s.reviews.get("is.o.chess").map(|r| r.stars), Some(4));
    s.activate(&ui, Target::Rate("is.o.nothing".into()));
    assert!(s.review.is_none(), "an app the catalogue does not list has no sheet");
}

#[test]
fn tapping_buy_records_the_purchase_and_cancel_records_nothing() {
    let ui = crate::shell::tests::ui();
    let mut s = store();
    s.catalog[1].price = Some(bifrost_ui::purchase::Price::new(990, 0, "ISK"));
    let chess = s.catalog[1].clone();
    // the pane sits right of the sidebar on desktop, so window x = sidebar + pane x
    let sidebar = s.sidebar as f32;
    let to_window = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0 + sidebar, r.1 + r.3 / 2.0);
    s.press_button(&chess.id);
    let (cancel_x, cancel_y) = to_window(sheet::sheet_buttons(&ui, s.w, s.h).1);
    s.touch(&ui, Event::Up(cancel_x, cancel_y), 0.0);
    assert!(s.sheet.is_none());
    assert!(!s.bought.is_entitled(&chess.id, today_days()), "a cancelled sheet records nothing");
    s.press_button(&chess.id);
    let (buy_x, buy_y) = to_window(sheet::sheet_buttons(&ui, s.w, s.h).0);
    s.touch(&ui, Event::Up(buy_x, buy_y), 0.0);
    assert!(s.sheet.is_none(), "Buy closes the sheet");
    assert!(s.bought.is_entitled(&chess.id, today_days()), "the purchase is in the ledger");
    assert!(s.jobs.is_empty(), "the test store has no worker pool, so the install is not started");
}

#[test]
fn an_install_request_goes_through_the_coordinator() {
    let mut s = store();
    let calc = s.catalog[0].clone();
    s.start(&calc.id);
    assert!(s.jobs.is_empty(), "no worker pool: the install is not started");
    assert!(s.coord.jobs().is_empty(), "and the coordinator does not keep the failed job");
    let phases: Vec<coordination::Phase> = s
        .coord
        .drain_events()
        .into_iter()
        .filter_map(|e| match e {
            coordination::Event::Phase { phase, .. } => Some(phase),
            _ => None,
        })
        .collect();
    assert_eq!(phases, vec![coordination::Phase::Queued, coordination::Phase::Downloading, coordination::Phase::Failed]);
}

#[test]
fn a_second_install_waits_while_one_downloads() {
    let mut s = store();
    let (calc, chess) = (s.catalog[0].id.clone(), s.catalog[1].id.clone());
    s.coord.request(&calc, coordination::current_uid(), 100).unwrap();
    s.coord.request(&chess, coordination::current_uid(), 100).unwrap();
    s.jobs.insert(calc.clone(), Job { done: Arc::new(AtomicU64::new(0)), size: 100, cancel: Arc::new(AtomicBool::new(false)) });
    s.jobs.insert(chess.clone(), Job { done: Arc::new(AtomicU64::new(0)), size: 100, cancel: Arc::new(AtomicBool::new(false)) });
    assert_eq!(s.coord.start_next().as_deref(), Some(calc.as_str()));
    assert_eq!(s.coord.start_next(), None, "the second download waits its turn");
    // the download counter feeds the coordinator, and the job is still shown with its ring
    s.jobs[&calc].done.store(60, Ordering::Relaxed);
    s.sync_coordinator();
    assert_eq!(s.coord.job(&calc).map(|j| j.done), Some(60));
    assert!(s.jobs.contains_key(&chess), "the waiting install keeps its progress ring");
}

#[test]
fn restore_purchases_reports_the_count_on_the_updates_tab() {
    let ui = crate::shell::tests::ui();
    let mut s = store();
    s.tab = Tab::Updates;
    assert!(s.rows().contains(&Row::Restore));
    s.activate(&ui, Target::Restore);
    assert_eq!(s.restored.as_deref(), Some("No purchases to restore."));
    assert!(s.rows().iter().any(|r| *r == Row::Note("No purchases to restore.".into())));
}

#[test]
fn a_search_matches_every_word_across_name_summary_category_and_id() {
    let mut calc = StoreApp { id: "is.o.calc".into(), name: "Calc".into(), summary: "Adds numbers quickly".into(), category: "Utilities".into(), ..Default::default() };
    assert!(matches_query(&calc, ""), "an empty query matches everything");
    assert!(matches_query(&calc, "CALC") && matches_query(&calc, "numbers utilities"));
    assert!(matches_query(&calc, "o.calc"), "the id is searched too");
    assert!(!matches_query(&calc, "calc chess"), "every word must match");
    calc.summary.clear();
    assert!(!matches_query(&calc, "numbers"));
}

#[test]
fn typing_filters_the_search_tab_and_says_when_nothing_matches() {
    let mut s = store();
    s.tab = Tab::Search;
    assert!(!s.rows().contains(&Row::Search), "on the desktop the query field is the sidebar's");
    s.desktop = false;
    assert!(s.rows().contains(&Row::Search), "the tablet's query field is the first row under the title");
    s.desktop = true;
    s.key(Out::Text("chess".into()));
    assert_eq!(s.query, "chess");
    assert_eq!(listed(&s), vec!["is.o.chess".to_string()]);
    s.key(Out::Backspace);
    s.key(Out::Backspace);
    s.key(Out::Backspace);
    s.key(Out::Backspace);
    s.key(Out::Backspace);
    assert_eq!(listed(&s).len(), 2, "an empty query lists every app again");
    s.key(Out::Text("zzz".into()));
    assert!(s.rows().iter().any(|r| matches!(r, Row::Note(n) if n.starts_with("No results"))));
    s.key(Out::Enter);
    s.key(Out::Named("Up"));
    assert_eq!(s.query, "zzz", "keys other than text and backspace are ignored");
}

#[test]
fn typing_goes_nowhere_off_the_search_tab_or_on_a_product_page() {
    let mut s = store();
    s.tab = Tab::Apps;
    s.key(Out::Text("x".into()));
    assert!(s.query.is_empty(), "the Apps tab has no query field");
    s.tab = Tab::Search;
    s.page = Some(Page { id: "is.o.calc".into(), back: Tab::Search, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
    s.key(Out::Text("x".into()));
    assert!(s.query.is_empty(), "a product page is open");
}

#[test]
fn tapping_the_query_field_asks_for_the_keyboard_once() {
    let ui = crate::shell::tests::ui();
    let mut s = store();
    s.desktop = false;
    s.tab = Tab::Search;
    let mut field_row_y = None;
    s.each_row(&ui, |r, ry, _| {
        if *r == Row::Search {
            field_row_y = Some(ry);
        }
    });
    assert!(field_row_y.is_some(), "the Search row is laid out");
    s.activate(&ui, Target::SearchField);
    assert!(s.take_keyboard_request());
    assert!(!s.take_keyboard_request(), "the request is read once");
}

/// A catalogue with one app per shelf category and one with a category no shelf names.
fn shelved() -> AppStore {
    let mut s = store();
    s.catalog = parse_catalog(&format!(
            r#"{{"apps":[{{"id":"is.o.draw","name":"Draw","version":"1.0.0","sha256":"{SUM}","package":"/a","category":"Graphics & Design"}},
                {{"id":"is.o.words","name":"Words","version":"1.0.0","sha256":"{SUM}","package":"/b","category":"Productivity"}},
                {{"id":"is.o.chess","name":"Chess","version":"1.0.0","sha256":"{SUM}","package":"/c","category":"Games"}},
                {{"id":"is.o.calc","name":"Calc","version":"1.0.0","sha256":"{SUM}","package":"/d","category":"Utilities"}},
            {{"id":"is.o.maps","name":"Maps","version":"1.0.0","sha256":"{SUM}","package":"/e","category":"Travel"}}]}}"#
    ))
    .unwrap();
    s
}

#[test]
fn each_shelf_lists_the_apps_of_its_categories_and_categories_lists_them_all() {
    let mut s = shelved();
    for (shelf, want) in [(Shelf::Create, "is.o.draw"), (Shelf::Work, "is.o.words"), (Shelf::Play, "is.o.chess"), (Shelf::Develop, "is.o.calc")] {
        s.tab = Tab::Shelf(shelf);
        assert_eq!(listed(&s), vec![want.to_string()], "{shelf:?}");
        assert_eq!(s.rows()[0], Row::Title(shelf.title()));
    }
    s.tab = Tab::Categories;
    let mut all = listed(&s);
    all.sort();
    assert_eq!(all, ["is.o.calc", "is.o.chess", "is.o.draw", "is.o.maps", "is.o.words"], "a category no shelf names is still listed");
    assert!(s.rows().contains(&Row::Header("Travel".into(), false)));
    s.catalog.retain(|a| a.id != "is.o.draw");
    s.tab = Tab::Shelf(Shelf::Create);
    assert!(s.rows().iter().any(|r| matches!(r, Row::Note(n) if n.contains("create"))), "an empty shelf says so");
}

#[test]
fn the_desktop_sidebar_has_a_search_field_the_sections_and_the_account_chip() {
    let ui = crate::shell::tests::ui();
    let mut s = shelved();
    let names: Vec<&str> = SIDEBAR.iter().map(|(_, n)| *n).collect();
    assert_eq!(names, ["Discover", "Create", "Work", "Play", "Develop", "Categories", "Updates"]);
    let (fx, fy, fw, fh) = s.side_search_rect(&ui);
    assert!(fy + fh <= s.side_row_rect(&ui, 0).1, "the search field sits above the sections");
    assert_eq!(s.side_hit(&ui, fx + fw / 2.0, fy + fh / 2.0), Some(SideHit::Search));
    let (rx, ry, rw, rh) = s.side_row_rect(&ui, 3);
    assert_eq!(s.side_hit(&ui, rx + rw / 2.0, ry + rh / 2.0), Some(SideHit::Tab(Tab::Shelf(Shelf::Play))));
    let (ax, ay, aw, ah) = s.side_account_rect(&ui);
    assert!(ay > s.side_row_rect(&ui, SIDEBAR.len() - 1).1, "the account chip is at the foot");
    assert_eq!(s.side_hit(&ui, ax + aw / 2.0, ay + ah / 2.0), Some(SideHit::Account));
    // a click on the field opens the results and raises the keyboard; typing filters them
    s.touch(&ui, Event::Up(fx + fw / 2.0, fy + fh / 2.0), 0.0);
    assert_eq!(s.tab, Tab::Search);
    assert!(s.take_keyboard_request() && s.text_focused());
    s.key(Out::Text("draw".into()));
    assert_eq!(listed(&s), vec!["is.o.draw".to_string()]);
    s.touch(&ui, Event::Up(rx + rw / 2.0, ry + rh / 2.0), 0.0);
    assert_eq!(s.tab, Tab::Shelf(Shelf::Play));
    assert_eq!(tab_name(Tab::Today, true), "Discover");
    assert_eq!(tab_name(Tab::Today, false), "Today");
    assert_eq!(tab_name(Tab::Categories, false), "Categories");
}

#[test]
fn the_full_size_desktop_window_reports_its_toolbar_controls_and_starts_its_pane_under_the_band() {
    let ui = crate::shell::tests::ui();
    let mut s = store();
    let pt = ui.pt(1.0);
    // Today starts at the top edge as the reference's Discover does (its large title sits under the empty band);
    // a product page and the other tabs start under the unified toolbar band
    assert_eq!(s.tab, Tab::Today);
    assert_eq!(s.top(&ui), 0.0);
    s.tab = Tab::Apps;
    assert_eq!(s.top(&ui), ui.pt(DESKTOP_BAR_PT));
    s.tab = Tab::Today;
    s.page = Some(Page { id: "is.o.chess".into(), back: Tab::Today, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
    assert_eq!(s.top(&ui), ui.pt(DESKTOP_BAR_PT));
    // the back control in that band takes the press
    assert_eq!(s.hit(&ui, ui.pt(20.0), ui.pt(DESKTOP_BAR_PT / 2.0)), Some(Target::Back));
    // the controls the shell's drag zone leaves to the app are where the window draws them
    assert!(toolbar_controls(s.sidebar + s.w, pt, 0.0).iter().all(|n| n.label != "Back"), "no back control while it is not shown");
    let c = toolbar_controls(s.sidebar + s.w, pt, 90.0);
    let (fx, fy, fw, fh) = s.side_search_rect(&ui);
    let search = c.iter().find(|n| n.label == "Search").expect("the search field");
    assert!((search.bounds.x * pt - fx).abs() < 1.0 && (search.bounds.y * pt - fy).abs() < 1.0, "search at {fx},{fy}");
    assert!((search.bounds.w * pt - fw).abs() < 1.0 && (search.bounds.h * pt - fh).abs() < 1.0);
    let back = c.iter().find(|n| n.label == "Back").expect("the back control");
    assert!((back.bounds.x * pt - s.sidebar as f32).abs() < 1.0, "the back control starts at the pane's left edge");
    assert!(back.bounds.y < DESKTOP_BAR_PT && back.bounds.h >= DESKTOP_BAR_PT);
    assert_eq!(back.bounds.w, 90.0, "the hole covers the back control only");
}

#[test]
fn the_account_chip_shows_the_local_names_initials() {
    assert_eq!(initials("Ada Lovelace"), "AL");
    assert_eq!(initials("owner"), "O");
    assert_eq!(initials("Ólafur Búi Ólafsson"), "ÓB");
    assert_eq!(initials(""), "");
}

#[test]
fn remove_takes_two_taps_and_only_for_an_app_the_customer_chose() {
    let mut confirm = None;
    assert!(!remove_step(&mut confirm, "is.o.calc"), "the first tap only asks");
    assert_eq!(confirm.as_deref(), Some("is.o.calc"));
    assert!(remove_step(&mut confirm, "is.o.calc"), "the second tap removes");
    assert!(confirm.is_none());

    let mut s = with_system();
    s.installed = vec![local("is.o.calc", "1.0.0"), InstalledApp { kind: "system".into(), ..local("is.o.kb", "1.0.0") }];
    assert!(s.can_remove("is.o.calc"));
    assert!(!s.can_remove("is.o.kb"), "a system component is never removed here");
    assert!(!s.can_remove("is.o.chess"), "an app that is not installed has nothing to remove");
    s.jobs.insert("is.o.calc".into(), Job { done: Arc::new(AtomicU64::new(0)), size: 1, cancel: Arc::new(AtomicBool::new(false)) });
    assert!(!s.can_remove("is.o.calc"), "not while it is being downloaded");
}

#[test]
fn the_product_page_reserves_a_strip_only_for_apps_with_screenshots() {
    let ui = crate::shell::tests::ui();
    let mut s = store();
    s.catalog[0].screenshots = vec!["/s/1.png".into()];
    s.page = Some(Page { id: "is.o.calc".into(), back: Tab::Today, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
    assert_eq!(s.shots_pt(), SHOTS_PT);
    s.page = Some(Page { id: "is.o.chess".into(), back: Tab::Today, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
    assert_eq!(s.shots_pt(), 0.0);
    assert!(s.page_height(&ui) > 0.0);
}

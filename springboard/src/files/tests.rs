//! Behaviour tests for the Files app against a temp directory tree: the
//! stack, listings through the worker pool, the context menu and its
//! alerts, the picker, previews and the video hand-off.

use super::fs::{self, Glyph, Location, Sort};
use super::menu::{Action, Subject};
use super::pages::{self, Body, LocationsPage, Page, Target, CONTENT_TOP_PT, NAV_MID_PT, ROW_PT};
use super::{Files, Request, LONG_PRESS_MS};
use crate::gfx::Surface;
use crate::shell::keyboard::Out;
use crate::shell::ui::Ui;
use crate::shell::Event;
use std::path::{Path, PathBuf};

fn mono() -> &'static [u8] {
    Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice())
}

/// A temp tree: home/{Notes.txt, Photos/, Clips/video.mp4, data.bin, pic.png} and a second root.
fn tree(tag: &str) -> (PathBuf, Vec<Location>) {
    let dir = fs::tests::temp(tag);
    let home = dir.join("home");
    std::fs::create_dir_all(home.join("Photos")).unwrap();
    std::fs::create_dir_all(home.join("Clips")).unwrap();
    std::fs::write(home.join("Notes.txt"), "hello\nworld\n").unwrap();
    std::fs::write(home.join("data.bin"), [0u8, 1, 2, 3]).unwrap();
    std::fs::write(home.join("pic.png"), crate::cg::imageio::tests::png_bytes(4, 4, [255, 0, 0, 255])).unwrap();
    std::fs::write(home.join("Clips/video.mp4"), b"not really").unwrap();
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let locations = vec![Location { name: "On My PC", path: home.clone(), glyph: Glyph::Computer }, Location { name: "Other", path: other, glyph: Glyph::Root }];
    (home, locations)
}

fn app(tag: &str) -> (Files, Ui, PathBuf) {
    let ui = crate::shell::tests::ui();
    let (home, locations) = tree(tag);
    let f = Files::with_locations(&ui, 1200, 1920, mono(), locations).unwrap();
    (f, ui, home)
}

/// Pumps finished jobs until `done` holds (or 5 s pass).
fn pump_until(f: &mut Files, ui: &Ui, done: impl Fn(&Files) -> bool) {
    let start = std::time::Instant::now();
    while !done(f) && start.elapsed().as_secs() < 5 {
        if crate::bare::sys::wait(&f.fds(), 50).contains(&true) {
            f.pump(ui);
        }
    }
    assert!(done(f), "timed out waiting for a job");
}

fn settle(f: &mut Files, ui: &Ui, t: f64) {
    f.tick(ui, t + 1000.0);
}

fn tap(f: &mut Files, ui: &Ui, x: f32, y: f32, t: f64) -> Vec<Request> {
    let mut r = f.touch(ui, Event::Down(x, y), t);
    r.extend(f.touch(ui, Event::Up(x, y), t + 30.0));
    r
}

fn loaded(f: &Files) -> bool {
    matches!(f.nav.top(), Page::Dir(d) if !d.loading)
}

fn entries(f: &Files) -> Vec<String> {
    match f.nav.top() {
        Page::Dir(d) => d.shown.iter().map(|&i| d.entries[i].name.clone()).collect(),
        _ => vec![],
    }
}

/// Opens "On My PC" from the locations list and waits for the listing.
fn open_home(f: &mut Files, ui: &Ui, t: f64) {
    tap(f, ui, 600.0, LocationsPage::row_top(ui, 0) + 10.0, t);
    settle(f, ui, t);
    pump_until(f, ui, loaded);
}

/// Centre of list row `k` (into `shown`).
fn row(ui: &Ui, k: usize) -> (f32, f32) {
    (600.0, ui.pt(CONTENT_TOP_PT) + (k as f32 + 0.5) * ui.pt(ROW_PT))
}

fn index_of(f: &Files, name: &str) -> usize {
    entries(f).iter().position(|n| n == name).unwrap_or_else(|| panic!("no entry {name} in {:?}", entries(f)))
}

fn long_press(f: &mut Files, ui: &Ui, x: f32, y: f32, t: f64) {
    f.touch(ui, Event::Down(x, y), t);
    f.tick(ui, t + LONG_PRESS_MS + 10.0);
    f.touch(ui, Event::Up(x, y), t + LONG_PRESS_MS + 20.0);
}

fn menu_tap(f: &mut Files, ui: &Ui, action: Action, t: f64) -> Vec<Request> {
    let bottom = 1920.0 - ui.pt(28.0);
    let m = f.menu.as_ref().expect("menu open");
    let (_, (x, y, w, h)) = m.rows(ui, 1200, bottom).into_iter().find(|(a, _)| *a == action).unwrap_or_else(|| panic!("no {action:?}"));
    tap(f, ui, x + w / 2.0, y + h / 2.0, t)
}

fn alert_button(f: &mut Files, ui: &Ui, i: usize, t: f64) {
    let bottom = if f.keyboard.shown() { f.kb_top(ui) } else { 1920.0 - ui.pt(28.0) };
    let (x, y, w, h) = f.alert.as_ref().expect("alert open").button_rect(ui, 1200, bottom, i);
    tap(f, ui, x + w / 2.0, y + h / 2.0, t);
}

fn draw(f: &mut Files, ui: &Ui, t: f64) -> Vec<u32> {
    let mut buf = vec![0u32; 1200 * 1920];
    f.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, t);
    buf
}

#[test]
fn locations_push_a_listing_and_back_pops_it() {
    let (mut f, ui, home) = app("nav");
    assert!(home.is_dir(), "the home folder is created");
    assert!(matches!(f.nav.top(), Page::Locations(_)));
    assert_eq!(f.fds().len(), 1);
    tap(&mut f, &ui, 600.0, LocationsPage::row_top(&ui, 0) + 10.0, 1000.0);
    assert!(f.nav.animating() && f.animating());
    assert!(matches!(f.nav.top(), Page::Dir(d) if d.loading));
    draw(&mut f, &ui, 1100.0);
    settle(&mut f, &ui, 1000.0);
    pump_until(&mut f, &ui, loaded);
    assert_eq!(entries(&f), ["Clips", "data.bin", "Notes.txt", "Photos", "pic.png"], "sorted by name, folders mixed");
    assert!(f.describe().contains("files_depth=2"));
    // into Photos and back out
    let (x, y) = row(&ui, index_of(&f, "Photos"));
    tap(&mut f, &ui, x, y, 3000.0);
    settle(&mut f, &ui, 3000.0);
    pump_until(&mut f, &ui, loaded);
    assert!(matches!(f.nav.top(), Page::Dir(d) if d.path == home.join("Photos") && d.crumbs.len() == 2));
    assert!(entries(&f).is_empty());
    let buf = draw(&mut f, &ui, 4000.0);
    assert!(buf.iter().any(|&p| p != crate::shell::ui::BLACK));
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 5000.0);
    assert_eq!(f.nav.direction(), Some(super::nav::Dir::Pop));
    draw(&mut f, &ui, 5100.0);
    settle(&mut f, &ui, 5000.0);
    assert_eq!(f.nav.depth(), 2);
    // crumbs: the root crumb pops to the root listing
    tap(&mut f, &ui, x, y, 6000.0);
    settle(&mut f, &ui, 6000.0);
    pump_until(&mut f, &ui, loaded);
    tap(&mut f, &ui, ui.pt(pages::MARGIN_PT) + 5.0, ui.pt(pages::CRUMB_TOP_PT + pages::CRUMB_H_PT / 2.0), 7000.0);
    settle(&mut f, &ui, 7000.0);
    assert_eq!(f.nav.depth(), 2);
    assert!(matches!(f.nav.top(), Page::Dir(d) if d.path == home));
}

#[test]
fn sort_view_toggle_and_search() {
    let (mut f, ui, _) = app("sort");
    open_home(&mut f, &ui, 0.0);
    let cy = ui.pt(pages::CRUMB_TOP_PT + pages::CRUMB_H_PT / 2.0);
    tap(&mut f, &ui, pages::sort_x(&ui, 1200, 2) + 5.0, cy, 1000.0);
    assert_eq!(f.sort, Sort::Size);
    assert_eq!(entries(&f).last().map(String::as_str), Some("Photos"), "folders last by size");
    assert_eq!(entries(&f)[0], "pic.png");
    tap(&mut f, &ui, pages::sort_x(&ui, 1200, 0) + 5.0, cy, 1100.0);
    assert_eq!(entries(&f)[0], "Clips");
    assert!(!f.grid);
    tap(&mut f, &ui, pages::nav_right_x(&ui, 1200, 0) + 10.0, ui.pt(NAV_MID_PT), 2000.0);
    assert!(f.grid);
    draw(&mut f, &ui, 2100.0);
    // search
    let (sx, sy, _, sh) = pages::search_rect(&ui, 1200, false);
    tap(&mut f, &ui, sx + 10.0, sy + sh / 2.0, 3000.0);
    assert!(f.keyboard.shown());
    assert!(matches!(f.nav.top(), Page::Dir(d) if d.searching && d.query.focused));
    assert!(f.next_wake(3000.0).is_some(), "the caret blinks");
    f.key(&ui, Out::Text("p".into()), 3100.0);
    assert_eq!(entries(&f), ["Clips", "Photos", "pic.png"], "case-insensitive substring");
    f.key(&ui, Out::Backspace, 3200.0);
    assert_eq!(entries(&f).len(), 5);
    f.key(&ui, Out::Text("not".into()), 3300.0);
    assert_eq!(entries(&f), ["Notes.txt"]);
    draw(&mut f, &ui, 3400.0);
    f.key(&ui, Out::Enter, 3500.0);
    assert!(!f.keyboard.shown(), "return dismisses the keyboard, keeping the results");
    assert_eq!(entries(&f), ["Notes.txt"]);
    let (sx, sy, sw, sh) = pages::search_rect(&ui, 1200, true);
    tap(&mut f, &ui, sx + sw + 20.0, sy + sh / 2.0, 4000.0);
    assert_eq!(entries(&f).len(), 5, "Cancel clears the search");
    assert!(matches!(f.nav.top(), Page::Dir(d) if !d.searching));
}

#[test]
fn long_press_opens_the_menu_and_rename_delete_new_folder_work() {
    let (mut f, ui, home) = app("menu");
    open_home(&mut f, &ui, 0.0);
    let (x, y) = row(&ui, index_of(&f, "Notes.txt"));
    f.touch(&ui, Event::Down(x, y), 1000.0);
    assert_eq!(f.next_wake(1000.0), Some(1000.0 + LONG_PRESS_MS));
    f.tick(&ui, 1000.0 + LONG_PRESS_MS - 1.0);
    assert!(f.menu.is_none(), "not yet");
    f.touch(&ui, Event::Move(x + 3.0, y - 2.0), 1200.0);
    f.tick(&ui, 1000.0 + LONG_PRESS_MS + 1.0);
    let m = f.menu.as_ref().expect("menu opens after the long press");
    assert_eq!(m.subject, Subject::Entry(index_of(&f, "Notes.txt")));
    assert!(f.touch(&ui, Event::Up(x, y), 1600.0).is_empty());
    assert!(f.menu.is_some(), "the lift after a long press does not activate the row");
    draw(&mut f, &ui, 1700.0);
    // a tap outside dismisses
    tap(&mut f, &ui, 1100.0, 1800.0, 2000.0);
    assert!(f.menu.is_none());
    // a drag is not a long press
    f.touch(&ui, Event::Down(x, y), 2500.0);
    f.touch(&ui, Event::Move(x, y - 100.0), 2600.0);
    f.tick(&ui, 2500.0 + LONG_PRESS_MS + 1.0);
    assert!(f.menu.is_none());
    f.touch(&ui, Event::Up(x, y - 100.0), 3100.0);
    settle(&mut f, &ui, 3100.0);

    // rename through the alert's field
    long_press(&mut f, &ui, x, y, 4000.0);
    menu_tap(&mut f, &ui, Action::Rename, 4600.0);
    assert!(f.menu.is_none() && f.alert.is_some() && f.keyboard.shown());
    assert!(f.next_wake(4600.0).is_some());
    draw(&mut f, &ui, 4700.0);
    for _ in 0..9 {
        f.key(&ui, Out::Backspace, 4700.0);
    }
    f.key(&ui, Out::Text("Memo.txt".into()), 4800.0);
    alert_button(&mut f, &ui, 1, 5000.0);
    assert!(f.alert.is_none() && !f.keyboard.shown());
    pump_until(&mut f, &ui, |f| entries(f).contains(&"Memo.txt".to_string()));
    assert!(home.join("Memo.txt").exists() && !home.join("Notes.txt").exists());

    // delete asks first; Cancel keeps the file
    let (x, y) = row(&ui, index_of(&f, "Memo.txt"));
    long_press(&mut f, &ui, x, y, 6000.0);
    menu_tap(&mut f, &ui, Action::Delete, 6600.0);
    assert!(matches!(f.alert.as_ref().map(|a| &a.purpose), Some(super::alert::Purpose::Delete(_))));
    draw(&mut f, &ui, 6700.0);
    alert_button(&mut f, &ui, 0, 6800.0);
    assert!(f.alert.is_none() && home.join("Memo.txt").exists());
    long_press(&mut f, &ui, x, y, 7000.0);
    menu_tap(&mut f, &ui, Action::Delete, 7600.0);
    alert_button(&mut f, &ui, 1, 7800.0);
    pump_until(&mut f, &ui, |f| !entries(f).contains(&"Memo.txt".to_string()));
    assert!(!home.join("Memo.txt").exists());

    // new folder from the nav bar
    tap(&mut f, &ui, pages::nav_right_x(&ui, 1200, 1) + 10.0, ui.pt(NAV_MID_PT), 8000.0);
    assert!(matches!(f.alert.as_ref().map(|a| &a.purpose), Some(super::alert::Purpose::NewFolder(_))));
    alert_button(&mut f, &ui, 1, 8100.0);
    assert!(f.alert.is_some(), "an empty name cannot be created");
    f.key(&ui, Out::Text("Docs".into()), 8200.0);
    f.key(&ui, Out::Enter, 8300.0);
    assert!(f.alert.is_none());
    pump_until(&mut f, &ui, |f| entries(f).contains(&"Docs".to_string()));
    assert!(home.join("Docs").is_dir());

    // background long press: New Folder, and Paste once something was copied
    let blank_y = ui.pt(CONTENT_TOP_PT) + 5.5 * ui.pt(ROW_PT) + 200.0;
    long_press(&mut f, &ui, 600.0, blank_y, 9000.0);
    let m = f.menu.as_ref().expect("background menu");
    assert_eq!((m.subject, m.items.clone()), (Subject::Background, vec![Action::NewFolder]));
    tap(&mut f, &ui, 1100.0, 1800.0, 9600.0);
    let (x, y) = row(&ui, index_of(&f, "pic.png"));
    long_press(&mut f, &ui, x, y, 10_000.0);
    menu_tap(&mut f, &ui, Action::Copy, 10_600.0);
    assert_eq!(f.clipboard, Some(home.join("pic.png")));
    long_press(&mut f, &ui, 600.0, blank_y, 11_000.0);
    assert!(f.menu.as_ref().unwrap().items.contains(&Action::Paste));
    menu_tap(&mut f, &ui, Action::Paste, 11_600.0);
    pump_until(&mut f, &ui, |f| entries(f).contains(&"pic 2.png".to_string()));
    assert!(home.join("pic 2.png").exists());
}

#[test]
fn share_opens_the_terminal_and_info_shows_a_sheet() {
    let (mut f, ui, home) = app("share");
    open_home(&mut f, &ui, 0.0);
    let (x, y) = row(&ui, index_of(&f, "data.bin"));
    long_press(&mut f, &ui, x, y, 1000.0);
    assert!(menu_tap(&mut f, &ui, Action::Share, 1600.0).is_empty());
    let m = f.menu.as_ref().expect("share list replaces the menu");
    assert!(m.sharing && m.items == [Action::SendNearby, Action::OpenInTerminal]);
    draw(&mut f, &ui, 1700.0);
    assert_eq!(menu_tap(&mut f, &ui, Action::OpenInTerminal, 2000.0), vec![Request::OpenTerminal(home.clone())], "a file opens its folder");
    assert!(f.menu.is_none());
    let (x, y) = row(&ui, index_of(&f, "Clips"));
    long_press(&mut f, &ui, x, y, 3000.0);
    menu_tap(&mut f, &ui, Action::Share, 3600.0);
    assert_eq!(menu_tap(&mut f, &ui, Action::OpenInTerminal, 4000.0), vec![Request::OpenTerminal(home.join("Clips"))]);
    // Get Info pushes the sheet at once
    let (x, y) = row(&ui, index_of(&f, "data.bin"));
    long_press(&mut f, &ui, x, y, 5000.0);
    menu_tap(&mut f, &ui, Action::GetInfo, 5600.0);
    assert!(matches!(f.nav.top(), Page::Preview(p) if matches!(p.body, Body::Info(ref rows) if rows.len() == 5)));
    assert!(f.describe().contains("files_page=preview:"));
    draw(&mut f, &ui, 5700.0);
}

#[test]
fn previews_text_images_info_and_hand_videos_to_the_shell() {
    let (mut f, ui, home) = app("preview");
    open_home(&mut f, &ui, 0.0);
    let (x, y) = row(&ui, index_of(&f, "Notes.txt"));
    tap(&mut f, &ui, x, y, 1000.0);
    assert!(matches!(f.nav.top(), Page::Preview(p) if matches!(p.body, Body::Loading)));
    pump_until(&mut f, &ui, |f| matches!(f.nav.top(), Page::Preview(p) if !matches!(p.body, Body::Loading)));
    assert!(matches!(f.nav.top(), Page::Preview(p) if matches!(p.body, Body::Text(ref t) if t.len() == 2)));
    settle(&mut f, &ui, 1000.0);
    let buf = draw(&mut f, &ui, 2000.0);
    assert!(buf.contains(&crate::shell::ui::WHITE));
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 3000.0);
    settle(&mut f, &ui, 3000.0);
    // image
    let (x, y) = row(&ui, index_of(&f, "pic.png"));
    tap(&mut f, &ui, x, y, 4000.0);
    pump_until(&mut f, &ui, |f| matches!(f.nav.top(), Page::Preview(p) if !matches!(p.body, Body::Loading)));
    assert!(matches!(f.nav.top(), Page::Preview(p) if matches!(p.body, Body::Image(ref i) if i.w == 4)));
    settle(&mut f, &ui, 4000.0);
    let buf = draw(&mut f, &ui, 5000.0);
    assert!(buf.contains(&crate::gfx::pack(255, 0, 0, 255)), "the red pixels show");
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 6000.0);
    settle(&mut f, &ui, 6000.0);
    // a binary file of unknown kind gets the info sheet
    let (x, y) = row(&ui, index_of(&f, "data.bin"));
    tap(&mut f, &ui, x, y, 7000.0);
    pump_until(&mut f, &ui, |f| matches!(f.nav.top(), Page::Preview(p) if !matches!(p.body, Body::Loading)));
    assert!(matches!(f.nav.top(), Page::Preview(p) if matches!(p.body, Body::Info(_)) && p.kind == fs::Kind::Other));
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 8000.0);
    settle(&mut f, &ui, 8000.0);
    // a video is the shell's job
    let (x, y) = row(&ui, index_of(&f, "Clips"));
    tap(&mut f, &ui, x, y, 9000.0);
    settle(&mut f, &ui, 9000.0);
    pump_until(&mut f, &ui, loaded);
    let (x, y) = row(&ui, 0);
    assert_eq!(tap(&mut f, &ui, x, y, 10_000.0), vec![Request::Play(home.join("Clips/video.mp4"))]);
    assert_eq!(f.nav.depth(), 3, "nothing was pushed");
    f.play(&ui, &home.join("Clips/video.mp4"));
    assert!(f.is_playing() && f.describe().contains("files_overlay=playing"));
    let buf = draw(&mut f, &ui, 10_100.0);
    assert!(buf.contains(&crate::shell::ui::WHITE), "the Stop button");
    let (sx, sy, sw, sh) = f.stop_rect(&ui);
    tap(&mut f, &ui, sx + sw / 2.0, sy + sh / 2.0, 11_000.0);
    assert!(!f.is_playing());
}

#[test]
fn move_to_picks_a_destination() {
    let (mut f, ui, home) = app("move");
    open_home(&mut f, &ui, 0.0);
    let (x, y) = row(&ui, index_of(&f, "Notes.txt"));
    long_press(&mut f, &ui, x, y, 1000.0);
    menu_tap(&mut f, &ui, Action::MoveTo, 1600.0);
    assert!(f.picker.is_some() && f.describe().contains("files_overlay=picker"));
    draw(&mut f, &ui, 1700.0);
    // the picker starts at the locations; Move is disabled there, Cancel closes
    assert_eq!(f.hit(&ui, 40.0, ui.pt(NAV_MID_PT)), Some(Target::Cancel));
    tap(&mut f, &ui, 600.0, LocationsPage::row_top(&ui, 1) + 10.0, 2000.0);
    settle(&mut f, &ui, 2000.0);
    pump_until(&mut f, &ui, |f| matches!(f.picker.as_ref().unwrap().nav.top(), Page::Dir(d) if !d.loading));
    assert_eq!(f.hit(&ui, 1150.0, ui.pt(NAV_MID_PT)), Some(Target::Confirm));
    draw(&mut f, &ui, 2100.0);
    tap(&mut f, &ui, 1150.0, ui.pt(NAV_MID_PT), 3000.0);
    assert!(f.picker.is_none());
    pump_until(&mut f, &ui, |f| !entries(f).contains(&"Notes.txt".to_string()));
    assert!(home.parent().unwrap().join("other/Notes.txt").exists());
    // Cancel
    let (x, y) = row(&ui, index_of(&f, "pic.png"));
    long_press(&mut f, &ui, x, y, 4000.0);
    menu_tap(&mut f, &ui, Action::MoveTo, 4600.0);
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 5000.0);
    assert!(f.picker.is_none() && home.join("pic.png").exists());
}

#[test]
fn an_unreadable_folder_shows_an_error_and_a_failed_op_an_alert() {
    let (mut f, ui, home) = app("errors");
    let missing = home.join("gone");
    let root = Location { name: "Gone", path: missing.clone(), glyph: Glyph::Root };
    f.push_dir(&ui, &root, &missing, 0.0);
    settle(&mut f, &ui, 0.0);
    pump_until(&mut f, &ui, |f| matches!(f.nav.top(), Page::Dir(d) if !d.loading));
    assert!(matches!(f.nav.top(), Page::Dir(d) if d.error.is_some() && d.footer.is_none()));
    draw(&mut f, &ui, 100.0);
    f.spawn_op("rename", move || fs::rename(Path::new("/nonexistent/x"), "y").map(|_| ()));
    pump_until(&mut f, &ui, |f| f.alert.is_some());
    assert!(matches!(f.alert.as_ref().map(|a| &a.purpose), Some(super::alert::Purpose::Error)));
    draw(&mut f, &ui, 200.0);
    alert_button(&mut f, &ui, 0, 300.0);
    assert!(f.alert.is_none());
}

#[test]
fn listings_arrive_in_order_and_stale_ones_are_dropped() {
    let (mut f, ui, home) = app("stale");
    open_home(&mut f, &ui, 0.0);
    // a reload keeps the old entries until the new ones land
    f.refresh_top();
    assert!(matches!(f.nav.top(), Page::Dir(d) if d.loading && d.entries.len() == 5));
    std::fs::write(home.join("late.txt"), "").unwrap();
    f.refresh_top();
    pump_until(&mut f, &ui, |f| entries(f).len() == 6);
    assert!(entries(&f).contains(&"late.txt".to_string()));
    f.set_wall(1_759_537_320, 0);
    assert_eq!(f.now, 1_759_537_320);
}

#[test]
fn bulk_ops_carry_on_after_a_failure_and_report_the_first() {
    let dir = fs::tests::temp("each");
    let (a, missing) = (dir.join("a"), dir.join("missing"));
    std::fs::write(&a, "x").unwrap();
    let r = super::each(&[missing, a.clone()], |p| std::fs::remove_file(p));
    assert!(r.is_err() && !a.exists(), "the second item was still processed");
}

#[test]
fn select_mode_marks_items_and_bulk_delete_goes_through_one_alert() {
    let (mut f, ui, home) = app("select");
    open_home(&mut f, &ui, 1000.0);
    f.activate(&ui, Target::Select, 5000.0);
    assert!(f.selecting);
    let i = index_of(&f, "Notes.txt");
    f.activate(&ui, Target::Entry(i), 5100.0);
    assert_eq!(f.selected, vec![home.join("Notes.txt")]);
    f.activate(&ui, Target::BulkDelete, 5200.0);
    assert!(matches!(f.alert.as_ref().map(|a| &a.purpose), Some(super::alert::Purpose::DeleteMany(p)) if p.len() == 1));
    f.activate(&ui, Target::Select, 5300.0);
    assert!(!f.selecting && f.selected.is_empty());
}

#[test]
fn pdf_and_epub_files_are_handed_to_books_only_when_the_shell_asks() {
    let (mut f, ui, home) = app("books");
    std::fs::write(home.join("novel.epub"), b"x").unwrap();
    std::fs::write(home.join("paper.pdf"), b"%PDF-").unwrap();
    open_home(&mut f, &ui, 0.0);
    let (x, y) = row(&ui, index_of(&f, "novel.epub"));
    // without the hand-off Files previews the file itself
    tap(&mut f, &ui, x, y, 1000.0);
    assert!(matches!(f.nav.top(), Page::Preview(_)));
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 2000.0);
    settle(&mut f, &ui, 2000.0);
    f.set_handoff(true);
    let (x, y) = row(&ui, index_of(&f, "novel.epub"));
    assert_eq!(tap(&mut f, &ui, x, y, 3000.0), vec![Request::OpenBook(home.join("novel.epub"))]);
    let (x, y) = row(&ui, index_of(&f, "paper.pdf"));
    assert_eq!(tap(&mut f, &ui, x, y, 4000.0), vec![Request::OpenBook(home.join("paper.pdf"))]);
}

#[test]
fn a_program_is_handed_to_the_shell_to_assess_only_when_the_shell_asks() {
    use std::os::unix::fs::PermissionsExt;
    let (mut f, ui, home) = app("program");
    let tool = home.join("tool.run");
    std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    open_home(&mut f, &ui, 0.0);
    let (x, y) = row(&ui, index_of(&f, "tool.run"));
    // without the hand-off Files previews the file itself
    tap(&mut f, &ui, x, y, 1000.0);
    assert!(matches!(f.nav.top(), Page::Preview(_)));
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 2000.0);
    settle(&mut f, &ui, 2000.0);
    f.set_handoff(true);
    let (x, y) = row(&ui, index_of(&f, "tool.run"));
    let requests = tap(&mut f, &ui, x, y, 3000.0);
    assert!(matches!(requests.as_slice(), [Request::Launch(e)] if e.path == tool), "{requests:?}");
}

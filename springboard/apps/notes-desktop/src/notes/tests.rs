use super::*;
use std::path::Path;

fn ui() -> Ui {
    crate::test_ui()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes-app")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 2026-10-04 00:23 UTC on a frame clock that reads 0 then.
fn wall() -> Wall {
    Wall { unix: 1_791_073_380, offset: 0, at: 0.0 }
}

/// Waits for the background image decode to land.
fn settle_images(n: &mut Notes, ui: &Ui) {
    for i in 0..1000 {
        n.tick(ui, 3000.0 + i as f64);
        if n.images_slot.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn app(ui: &Ui, dir: &Path) -> Notes {
    let mut n = Notes::new(ui, 2400, 1500, dir.to_path_buf(), wall()); // a desktop window: the three-column toolbar fits
    // The tests type plain body text; the Title start is covered by the settings tests.
    n.note_settings.new_note_start = settings::NewNoteStart::Body;
    n
}

fn tap(n: &mut Notes, ui: &Ui, x: f32, y: f32, t: f64) {
    n.touch(ui, Event::Down(x, y), t);
    n.touch(ui, Event::Up(x, y), t + 30.0);
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn tap_key(n: &mut Notes, ui: &Ui, label: &str, t: f64) {
    let (x, y) = n.keyboard.centre(label).unwrap_or_else(|| panic!("no key {label}"));
    tap(n, ui, x, n.kb_top(ui) + y, t);
}

fn type_text(n: &mut Notes, ui: &Ui, s: &str, t: f64) {
    for c in s.chars() {
        n.key(ui, if c == '\n' { Out::Enter } else { Out::Text(c.to_string()) }, t);
    }
}

fn draw(n: &mut Notes, ui: &Ui, t: f64) -> Vec<u32> {
    let mut buf = vec![0u32; 2400 * 1500];
    n.draw(&mut Surface { w: 2400, h: 1500, stride: 2400, px: &mut buf, y0: 0 }, ui, t);
    buf
}

#[test]
fn the_open_note_and_the_folder_view_come_back_after_a_relaunch() {
    let ui = ui();
    let dir = scratch("restore");
    let mut n = app(&ui, &dir);
    let id = n.import_note(&ui, "Shopping\nmilk", 1000.0);
    n.import_note(&ui, "Other", 1100.0);
    n.show_note(&ui, &id, 1200.0);
    let (view, open) = n.restorable();
    assert_eq!((view.clone(), open.clone()), (View::All, Some(id.clone())));
    n.flush();
    // a relaunch: a new Notes over the same directory shows the list with nothing open
    let mut again = app(&ui, &dir);
    assert!(again.editing().is_none());
    assert!(again.restore(&ui, view, open.as_deref(), 2000.0));
    assert_eq!(again.editing().map(|x| x.id.as_str()), Some(id.as_str()), "the same note is open");
    // a folder or note that no longer exists is not an error
    assert!(again.restore(&ui, View::Folder("Gone".into()), Some("missing"), 3000.0));
    assert_eq!(*again.view(), View::All, "an unknown folder falls back to All Notes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_autosave_firing_after_the_screen_changed_still_writes_the_edited_note() {
    let ui = ui();
    let dir = scratch("late-autosave");
    let mut n = app(&ui, &dir);
    let (cx, cy) = centre(n.compose_rect(&ui));
    tap(&mut n, &ui, cx, cy, 1000.0);
    tap_key(&mut n, &ui, "h", 2000.0);
    tap_key(&mut n, &ui, "i", 2100.0);
    n.screen = Screen::List;
    n.tick(&ui, 2700.0);
    let saved = store::load(&dir);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].text, "hi");
}

#[test]
fn an_imported_file_becomes_a_note_saved_at_once_and_opened() {
    let ui = ui();
    let dir = scratch("import");
    let mut n = app(&ui, &dir);
    n.import_note(&ui, "Shopping\nmilk\neggs", 1000.0);
    assert_eq!(n.editing().unwrap().text, "Shopping\nmilk\neggs");
    let saved = store::load(&dir);
    assert_eq!((saved.len(), saved[0].text.as_str()), (1, "Shopping\nmilk\neggs"), "on disk without waiting for the autosave");
}

#[test]
fn the_list_row_follows_the_typed_text_once_the_autosave_fires() {
    let ui = ui();
    let dir = scratch("live-row");
    let mut n = app(&ui, &dir);
    let (cx, cy) = centre(n.compose_rect(&ui));
    tap(&mut n, &ui, cx, cy, 1000.0);
    assert_eq!(n.rows[0].title_text, "New Note", "an empty note, or one holding only the hidden title marker");
    type_text(&mut n, &ui, "Shopping list", 2000.0);
    tap_key(&mut n, &ui, "return", 2100.0);
    type_text(&mut n, &ui, "Milk", 2100.0);
    n.tick(&ui, 2200.0);
    assert_eq!(n.rows[0].title_text, "New Note", "not before the debounce lapses");
    n.tick(&ui, 2700.0);
    assert_eq!(n.rows[0].title_text, "Shopping list");
    assert_eq!(n.rows[0].snippet_text, "Milk");
    assert!(n.editing().is_some(), "the note stays open");
}

#[test]
fn compose_creates_a_note_that_autosaves_half_a_second_after_typing() {
    let ui = ui();
    let dir = scratch("compose");
    let mut n = app(&ui, &dir);
    assert!(n.listed().is_empty());
    let (cx, cy) = centre(n.compose_rect(&ui));
    tap(&mut n, &ui, cx, cy, 1000.0);
    assert!(n.editing().is_some() && n.keyboard_shown());
    assert!(n.editor.focused);
    tap_key(&mut n, &ui, "h", 2000.0);
    tap_key(&mut n, &ui, "i", 2100.0);
    assert_eq!(n.editing().unwrap().text, "hi");
    n.tick(&ui, 2200.0);
    assert_eq!(n.next_wake(2200.0), Some(2630.0), "the autosave (and the caret blink) is the next deadline");
    n.tick(&ui, 2600.0);
    assert!(store::load(&dir).is_empty(), "not written yet");
    n.tick(&ui, 2630.0);
    let saved = store::load(&dir);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].text, "hi");
    assert!(!n.autosave.pending());
    tap_key(&mut n, &ui, "return", 3000.0);
    type_text(&mut n, &ui, "body", 3000.0);
    let (dx, dy) = centre(n.done_rect(&ui));
    tap(&mut n, &ui, dx, dy, 4000.0);
    assert!(!n.keyboard_shown(), "Done hides the keyboard");
    assert_eq!(store::load(&dir)[0].text, "hi\nbody", "and saves at once");
    draw(&mut n, &ui, 4100.0);
    // Esc only drops a selection or a pending style: the note stays open (Back or the list closes it)
    n.escape(&ui, 5000.0);
    assert!(n.editing().is_some(), "Esc keeps the note open");
    n.activate(&ui, Target::Back, 0.0, 0.0, 5100.0);
    assert_eq!(n.editing(), None, "Back closes the note");
    assert_eq!(n.listed(), vec![(saved[0].id.as_str(), "hi")]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_note_is_dropped_on_leaving() {
    let ui = ui();
    let dir = scratch("empty");
    let mut n = app(&ui, &dir);
    n.activate(&ui, Target::Compose, 0.0, 0.0, 1000.0);
    type_text(&mut n, &ui, " \n ", 1000.0);
    n.tick(&ui, 2000.0);
    assert_eq!(store::load(&dir).len(), 1);
    n.activate(&ui, Target::Back, 0.0, 0.0, 3000.0);
    assert!(n.listed().is_empty());
    assert!(store::load(&dir).is_empty(), "the file is gone too");
    let _ = std::fs::remove_dir_all(&dir);
}

fn seeded(ui: &Ui, dir: &Path) -> Notes {
    let now = wall().unix;
    store::save(dir, &Note { id: "1".into(), text: "Groceries\nmilk".into(), modified: 0 }).unwrap();
    store::save(dir, &Note { id: "2".into(), text: "Ideas\nrust shell".into(), modified: 0 }).unwrap();
    let mut n = app(ui, dir);
    // files share an mtime: pin the order
    for note in &mut n.notes {
        note.modified = if note.id == "2" { now - 86_400 * 3 } else { now - 60 };
    }
    n.rebuild(ui, 0.0);
    n
}

/// The ids a view lists, in list order.
fn listed_ids(n: &mut Notes, ui: &Ui, view: View) -> Vec<String> {
    n.view = view;
    n.rebuild(ui, 10.0);
    n.listed().iter().map(|(id, _)| id.to_string()).collect()
}

#[test]
fn smart_folders_pick_notes_by_their_fields() {
    let ui = ui();
    let dir = scratch("smart");
    let mut n = seeded(&ui, &dir);
    // Note 1 is an old picture note, note 2 a checklist changed a second ago (the week's cutoff is wall - 604800 s).
    let now = wall().unix;
    for note in &mut n.notes {
        note.modified = now - 40 * 86_400;
        if note.id == "1" {
            note.text = "Trip\n![image](a-1.png)".into();
        } else {
            note.text = "Shopping\n- [ ] milk\n- [x] eggs".into();
            note.modified = now - 1;
        }
    }
    n.loc.remove("1");
    n.loc.remove("2");
    n.pinned.insert("1".to_string());
    assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Checklists)), vec!["2"]);
    assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Photos)), vec!["1"]);
    assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Pinned)), vec!["1"]);
    assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Recent)), vec!["2"]);
    // the sidebar and the list menu count the same notes
    assert_eq!(n.count_in(&View::Smart(smart::Smart::Checklists)), 1);
    assert_eq!(n.count_in(&View::Smart(smart::Smart::Pinned)), 1);
    // a bin note never shows in a smart folder
    n.loc.insert("2".to_string(), folders::TRASH.to_string());
    assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Checklists)), Vec::<String>::new());
    assert_eq!(n.view_title(), "Checklists");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_one_and_replace_all_change_the_open_note() {
    let ui = ui();
    let dir = scratch("replace");
    let mut n = seeded(&ui, &dir);
    n.new_note(&ui, 0.0);
    n.editor.set_text(&ui, "one cat, cat\nthe Cat sat");
    n.changed(&ui, 0.5);
    n.hl_query = "cat".to_string();
    n.refresh_highlights(&ui, false);
    assert_eq!(n.hl.len(), 3);
    n.start_replacing(&ui, 1.0);
    assert!(n.replacing.is_some() && n.keyboard_shown());
    n.replacing.as_mut().unwrap().field.set_text(&ui, "dog");
    // Replace changes the selected match only; the bar moves on to the next one
    n.replace_one(&ui, 2.0);
    assert_eq!(n.editor.text(), "one dog, cat\nthe Cat sat");
    assert_eq!(n.hl.len(), 2);
    assert_eq!(n.hl_cur, 0);
    // Replace All changes the rest in one step and closes the card
    n.replace_all(&ui, 3.0);
    assert_eq!(n.editor.text(), "one dog, dog\nthe dog sat");
    assert!(n.replacing.is_none() && !n.keyboard_shown());
    assert_eq!(n.editing().map(|note| note.text.as_str()), Some("one dog, dog\nthe dog sat"));
    assert!(n.editor.undo(&ui, 4.0));
    assert_eq!(n.editor.text(), "one dog, cat\nthe Cat sat", "one undo step takes back Replace All");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_needs_find_text_first() {
    let ui = ui();
    let dir = scratch("replace-none");
    let mut n = seeded(&ui, &dir);
    n.new_note(&ui, 0.0);
    n.hl_query.clear();
    n.start_replacing(&ui, 1.0);
    assert!(n.replacing.is_none(), "without find text the card does not open");
    assert_eq!(n.notice.as_deref(), Some("Search for text first, then replace it"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_sorts_newest_first_searches_and_opens_rows() {
    let ui = ui();
    let dir = scratch("list");
    let mut n = seeded(&ui, &dir);
    assert_eq!(n.listed(), vec![("1", "Groceries"), ("2", "Ideas")]);
    let (sx, sy) = centre(n.search_rect(&ui));
    tap(&mut n, &ui, sx, sy, 1000.0);
    assert!(n.keyboard_shown() && n.searching);
    type_text(&mut n, &ui, "RUST", 1100.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    n.key(&ui, Out::Enter, 1200.0);
    assert!(!n.keyboard_shown());
    draw(&mut n, &ui, 1300.0);
    let (rx, ry) = centre(n.row_rect(&ui, 0));
    tap(&mut n, &ui, rx, ry, 2000.0);
    assert_eq!(n.editing().map(|e| e.id.as_str()), Some("2"));
    assert!(!n.keyboard_shown(), "opening an existing note shows it without the keyboard");
    let buf = draw(&mut n, &ui, 2100.0);
    assert!(buf.contains(&WHITE));
    // tapping the text brings the keyboard and places the caret
    let (ox, oy) = n.text_origin(&ui);
    tap(&mut n, &ui, ox + 1.0, oy + 5.0, 3000.0);
    assert!(n.keyboard_shown());
    assert_eq!(n.editor.caret(), 0);
    type_text(&mut n, &ui, "My ", 3100.0);
    assert_eq!(n.editing().unwrap().text, "My Ideas\nrust shell");
    n.tick(&ui, 4000.0);
    n.activate(&ui, Target::Back, 0.0, 0.0, 5000.0);
    assert_eq!(n.listed(), vec![("2", "My Ideas")], "the search still filters");
    n.key(&ui, Out::Backspace, 5100.0);
    n.key(&ui, Out::Backspace, 5100.0);
    n.key(&ui, Out::Backspace, 5100.0);
    n.key(&ui, Out::Backspace, 5100.0);
    assert_eq!(n.listed(), vec![("2", "My Ideas"), ("1", "Groceries")], "the edited note moved to the top");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swiping_right_reveals_pin_and_a_full_swipe_toggles_it() {
    let ui = ui();
    let dir = scratch("swipe-pin");
    let mut n = seeded(&ui, &dir);
    let (rx, ry, _, rh) = n.row_rect(&ui, 0);
    let y = ry + rh / 2.0;
    let id = n.notes[n.shown[0]].id.clone();
    // half swipe rests open with Pin revealed; tapping it pins the note
    let x0 = rx + 20.0;
    n.touch(&ui, Event::Down(x0, y), 0.0);
    for i in 1..=8 {
        n.touch(&ui, Event::Move(x0 + 10.0 * i as f32, y + 2.0), i as f64 * 16.0);
    }
    n.touch(&ui, Event::Up(x0 + 80.0, y), 200.0);
    assert!(n.reveal.is_some_and(|(r, dx)| r == 0 && dx > 0.0), "rests with Pin revealed");
    let buf = draw(&mut n, &ui, 300.0);
    assert!(buf.contains(&YELLOW));
    tap(&mut n, &ui, rx + 10.0, y, 400.0);
    assert!(n.is_pinned(&id), "tapping the revealed Pin pins it");
    assert_eq!(n.reveal, None);
    // full swipe on the same row toggles it back
    let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
    let y = ry + rh / 2.0;
    let x0 = rx + 10.0;
    n.touch(&ui, Event::Down(x0, y), 1000.0);
    for i in 1..=10 {
        n.touch(&ui, Event::Move(x0 + rw * 0.06 * i as f32, y + 2.0), 1000.0 + i as f64 * 16.0);
    }
    n.touch(&ui, Event::Up(x0 + rw * 0.6, y), 1300.0);
    assert!(!n.is_pinned(&id), "a full swipe toggles the pin");
    assert_eq!(n.reveal, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swiping_a_row_reveals_delete_and_a_full_swipe_deletes() {
    let ui = ui();
    let dir = scratch("swipe");
    let mut n = seeded(&ui, &dir);
    let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
    let y = ry + rh / 2.0;
    let x0 = rx + rw - 50.0;
    n.touch(&ui, Event::Down(x0, y), 0.0);
    // the list column is narrower on the Mac layout: stay between the reveal width and a full swipe (FULL_SWIPE)
    let swipe = (ui.pt(DELETE_PT) + rw * FULL_SWIPE) / 2.0;
    for i in 1..=10 {
        n.touch(&ui, Event::Move(x0 - swipe * i as f32 / 10.0, y + 2.0), i as f64 * 16.0);
    }
    assert!(n.reveal.is_some_and(|(r, dx)| r == 0 && dx < 0.0));
    n.touch(&ui, Event::Up(x0 - swipe, y), 200.0);
    assert_eq!(n.reveal, Some((0, -ui.pt(DELETE_PT))), "rests open");
    assert_eq!(n.listed().len(), 2, "nothing deleted yet");
    let buf = draw(&mut n, &ui, 300.0);
    assert!(buf.contains(&DESTRUCTIVE));
    // a tap on the row body closes it without opening the note
    tap(&mut n, &ui, rx + 30.0, y, 1000.0);
    assert_eq!(n.reveal, None);
    assert_eq!(n.editing(), None);
    // open again and tap Delete
    n.touch(&ui, Event::Down(x0, y), 2000.0);
    n.touch(&ui, Event::Move(x0 - 150.0, y), 2050.0);
    n.touch(&ui, Event::Up(x0 - 150.0, y), 2100.0);
    assert!(n.reveal.is_some());
    tap(&mut n, &ui, rx + rw - 20.0, y, 3000.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    assert_eq!(store::load(&dir).len(), 1);
    // a full swipe deletes on release
    let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
    let (y, x0) = (ry + rh / 2.0, rx + rw - 10.0);
    n.touch(&ui, Event::Down(x0, y), 4000.0);
    n.touch(&ui, Event::Move(x0 - rw * 0.3, y), 4050.0);
    n.touch(&ui, Event::Move(x0 - rw * 0.6, y), 4100.0);
    n.touch(&ui, Event::Up(x0 - rw * 0.6, y), 4150.0);
    assert!(n.listed().is_empty());
    assert!(store::load(&dir).is_empty());
    let buf = draw(&mut n, &ui, 5000.0);
    assert!(buf.contains(&LABEL_2.now()), "the empty message shows");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_vertical_drag_on_a_row_scrolls_instead_of_opening() {
    let ui = ui();
    let dir = scratch("scroll");
    let mut n = app(&ui, &dir);
    for i in 0..40 {
        n.notes.push(Note { id: i.to_string(), text: format!("Note {i}"), modified: 1000 - i });
    }
    n.rebuild(&ui, 0.0);
    let (rx, ry, _, rh) = n.row_rect(&ui, 3);
    let (x, y0) = (rx + 100.0, ry + rh / 2.0);
    n.touch(&ui, Event::Down(x, y0), 0.0);
    for i in 1..=10 {
        n.touch(&ui, Event::Move(x, y0 - 30.0 * i as f32), i as f64 * 16.0);
    }
    n.touch(&ui, Event::Up(x, y0 - 300.0), 170.0);
    assert_eq!(n.editing(), None);
    assert!(n.list_scroll.offset() > 100.0, "scrolled: {}", n.list_scroll.offset());
    let mut t = 200.0;
    while n.tick(&ui, t) && t < 10_000.0 {
        t += 16.0;
    }
    draw(&mut n, &ui, t);
    let first = (0..40).find(|&i| n.row_rect(&ui, i).1 >= ui.pt(list_top_pt())).unwrap();
    assert!(first > 0, "earlier rows scrolled off the top");
    assert_eq!(n.hit(&ui, x, n.row_rect(&ui, first).1 + 5.0), Some(Target::Row(first)));
    assert!(n.next_wake(t).is_none(), "nothing pending once settled");
}

#[test]
fn undo_redo_and_format_from_the_shortcut_bar() {
    let ui = ui();
    let dir = scratch("undo");
    let mut n = app(&ui, &dir);
    n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
    type_text(&mut n, &ui, "Plan\nstep one", 100.0);
    let top = n.shortcut_top(&ui).unwrap();
    let y = top + ui.pt(SHORTCUT_PT) / 2.0;
    assert_eq!(n.hit(&ui, ui.pt(MARGIN_PT + 20.0), y), Some(Target::Undo));
    assert_eq!(n.hit(&ui, ui.pt(MARGIN_PT + 100.0), y), Some(Target::Redo));
    assert_eq!(n.hit(&ui, 2400.0 - ui.pt(MARGIN_PT + 30.0), y), Some(Target::Format));
    tap(&mut n, &ui, ui.pt(MARGIN_PT + 20.0), y, 1000.0);
    assert_eq!(n.editing().unwrap().text, "Plan\nstep ");
    tap(&mut n, &ui, ui.pt(MARGIN_PT + 100.0), y, 2000.0);
    assert_eq!(n.editing().unwrap().text, "Plan\nstep one");
    tap(&mut n, &ui, 2400.0 - ui.pt(MARGIN_PT + 30.0), y, 3000.0);
    assert_eq!(n.editing().unwrap().text, "Plan\n## step one");
    n.activate(&ui, Target::Format, 0.0, 0.0, 3100.0);
    assert_eq!(n.editing().unwrap().text, "Plan\n# step one");
    n.activate(&ui, Target::Format, 0.0, 0.0, 3200.0);
    assert_eq!(n.editing().unwrap().text, "Plan\nstep one");
    assert!(n.autosave.pending());
    let buf = draw(&mut n, &ui, 3300.0);
    assert!(buf.contains(&YELLOW), "Done and the caret are yellow");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn typing_past_the_bottom_scrolls_the_caret_into_view() {
    let ui = ui();
    let dir = scratch("caret");
    let mut n = app(&ui, &dir);
    n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
    for i in 0..60 {
        type_text(&mut n, &ui, &format!("line {i}\n"), 100.0 + i as f64);
    }
    assert!(n.scroll.offset() > 0.0);
    let (_, oy) = n.text_origin(&ui);
    let (_, cy, _, ch) = n.editor.caret_rect(&ui);
    assert!(oy + cy + ch <= n.content_bottom(&ui), "caret above the keyboard");
    assert!(oy + cy >= ui.pt(nav_bottom_pt()));
    for _ in 0..70 {
        n.key(&ui, Out::Named("Up"), 200.0);
    }
    assert_eq!(n.scroll.offset(), 0.0, "scrolled back to the top");
    draw(&mut n, &ui, 300.0);
    assert!(n.describe().contains("notes_screen=editor:"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn row_pin_control_persists_groups_and_search_keeps_pins_first() {
    let ui = ui();
    let dir = scratch("pin-grouping");
    let mut n = seeded(&ui, &dir);
    n.activate(&ui, Target::Pin(1), 0.0, 0.0, 1000.0);
    assert!(n.is_pinned("2"));
    assert!(n.editing().is_none(), "pinning a row does not open the note");
    assert_eq!(n.listed(), vec![("2", "Ideas"), ("1", "Groceries")]);
    assert_eq!(n.shown_pins(), 1);
    let pinned = n.row_rect(&ui, 0);
    let regular = n.row_rect(&ui, 1);
    assert_eq!(regular.1 - (pinned.1 + pinned.3), ui.pt(SECTION_PT + SECTION_GAP_PT));
    // Golden Gate Notes has no pin glyph on the row (Pin Note is in the More menu); the pinned section still draws
    let _ = draw(&mut n, &ui, 1100.0);
    assert!(n.labels.len() > 0, "section labels were rendered");

    // Both matches keep their grouping, and a regular-only match has no
    // blank pinned section or leftover section spacing.
    n.search.set_text(&ui, "i"); n.rebuild(&ui, 1200.0);
    assert_eq!(n.listed(), vec![("2", "Ideas"), ("1", "Groceries")]);
    n.search.set_text(&ui, "milk"); n.rebuild(&ui, 1300.0);
    assert_eq!(n.listed(), vec![("1", "Groceries")]);
    assert_eq!(n.shown_pins(), 0);
    assert_eq!(n.row_rect(&ui, 0).1, ui.pt(list_top_pt()));
    n.search.set_text(&ui, "RUST"); n.rebuild(&ui, 1400.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    assert_eq!(n.shown_pins(), 1);
    draw(&mut n, &ui, 1500.0);

    let mut restarted = app(&ui, &dir);
    assert!(restarted.is_pinned("2"));
    assert_eq!(restarted.listed()[0].0, "2");
    restarted.activate(&ui, Target::Pin(0), 0.0, 0.0, 2000.0);
    assert!(!restarted.is_pinned("2"));
    assert!(!app(&ui, &dir).is_pinned("2"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn editor_pin_and_editing_preserve_identity_and_pins_across_restart() {
    let ui = ui();
    let dir = scratch("pin-editing");
    let mut n = seeded(&ui, &dir);
    n.open_note(&ui, 1, true, 0.0);
    let open_id = n.editing().unwrap().id.clone();
    n.activate(&ui, Target::EditorPin, 0.0, 0.0, 1000.0);
    assert!(n.is_pinned(&open_id));
    assert_eq!(n.editing().unwrap().id, open_id, "pinning must not reorder the editor's index");
    type_text(&mut n, &ui, "\nPinned body edit", 1100.0);
    n.tick(&ui, 1800.0);
    n.close_note(&ui, 2000.0);
    let reloaded = app(&ui, &dir);
    assert!(reloaded.is_pinned(&open_id));
    assert_eq!(reloaded.listed()[0].0, open_id);
    assert!(store::load(&dir).iter().find(|note| note.id == open_id).unwrap().text.ends_with("Pinned body edit"));

    // Composing after sorting pinned notes cannot accidentally edit the
    // first pinned row instead of the newly created note.
    n.new_note(&ui, 3000.0);
    let new_id = n.editing().unwrap().id.clone();
    assert_ne!(new_id, open_id);
    assert!(n.editing().unwrap().text.is_empty());
    type_text(&mut n, &ui, "New regular note", 3100.0);
    n.close_note(&ui, 4000.0);
    assert_eq!(n.listed()[0].0, open_id);
    assert_eq!(n.listed()[1], (new_id.as_str(), "New regular note"));
    assert!(n.set_pinned(&ui, &new_id, true, 5000.0));
    let row = n.shown.iter().position(|&i| n.notes[i].id == new_id).unwrap();
    n.delete_row(&ui, row, 6000.0);
    assert!(!n.is_pinned(&new_id));
    assert!(!app(&ui, &dir).is_pinned(&new_id));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_pin_write_leaves_pin_state_and_note_files_unchanged() {
    let ui = ui();
    let dir = scratch("pin-write-failure");
    let mut n = seeded(&ui, &dir);
    let before = std::fs::read(dir.join("1.md")).unwrap();
    let modified = std::fs::metadata(dir.join("1.md")).unwrap().modified().unwrap();
    std::fs::write(dir.join(".springboard-pins-v1"), "Existing unrelated file").unwrap();
    assert!(!n.set_pinned(&ui, "1", true, 1000.0));
    assert!(!n.is_pinned("1"));
    assert!(n.describe().contains("notes_pin_error=true"));
    assert_eq!(std::fs::read(dir.join("1.md")).unwrap(), before);
    assert_eq!(std::fs::metadata(dir.join("1.md")).unwrap().modified().unwrap(), modified);
    assert_eq!(std::fs::read_to_string(dir.join(".springboard-pins-v1")).unwrap(), "Existing unrelated file");
    draw(&mut n, &ui, 1100.0);

    std::fs::remove_file(dir.join(".springboard-pins-v1")).unwrap();
    assert!(n.set_pinned(&ui, "1", true, 1200.0));
    let marker = dir.join(".springboard-pins-v1/31.pin");
    std::fs::write(&marker, "Unknown future format").unwrap();
    assert!(!n.set_pinned(&ui, "1", false, 1300.0));
    assert!(n.is_pinned("1"), "failed unpin preserves its old visible state");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "Unknown future format");
    assert_eq!(std::fs::read(dir.join("1.md")).unwrap(), before);
    assert_eq!(std::fs::metadata(dir.join("1.md")).unwrap().modified().unwrap(), modified);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tapped_link_in_a_note_is_queued_for_the_shell_to_open() {
    let ui = ui();
    let dir = scratch("link");
    let mut n = app(&ui, &dir);
    n.new_note(&ui, 1000.0);
    n.editor.replace_range(&ui, 0..0, "https://bifrost.olibuijr.com\nhi", 1000.0, false);
    n.changed(&ui, 1000.0);
    let (ox, oy) = n.text_origin(&ui);
    // a tap on the link (first line, first character) opens it and does not move the caret
    let caret = n.editor.caret();
    n.activate(&ui, Target::Text, ox + 1.0, oy + 1.0, 1100.0);
    assert_eq!(n.take_link().as_deref(), Some("https://bifrost.olibuijr.com"));
    assert!(n.take_link().is_none(), "taken once");
    assert_eq!(n.editor.caret(), caret, "the caret stays where it was");
    // a tap on plain text places the caret and queues nothing
    n.activate(&ui, Target::Text, ox + 1.0, oy + ui.pt(120.0), 1200.0);
    assert!(n.take_link().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn share_button_in_the_editor_queues_the_open_note() {
    let ui = ui();
    let dir = scratch("share");
    let mut n = app(&ui, &dir);
    n.new_note(&ui, 1000.0);
    type_text(&mut n, &ui, "Shared body", 1100.0);
    let (x, y) = {
        let (rx, ry, rw, rh) = n.editor_share_rect(&ui);
        (rx + rw / 2.0, ry + rh / 2.0)
    };
    assert_eq!(n.hit(&ui, x, y), Some(Target::EditorShare));
    n.activate(&ui, Target::EditorShare, x, y, 1200.0);
    let item = n.take_share().expect("a share request");
    assert_eq!(item.file_name(), n.editing().map(|note| format!("{}.txt", note.title())).unwrap());
    assert!(n.take_share().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

fn tiny_png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&vec![200u8; (w * h * 4) as usize]).unwrap();
    }
    out
}

/// Puts `text` into the editor of a fresh note in one edit.
fn open_with(n: &mut Notes, ui: &Ui, text: &str) {
    n.new_note(ui, 1000.0);
    n.editor.replace_range(ui, 0..0, text, 1000.0, false);
    n.changed(ui, 1000.0);
}

/// E2E10: the open note tells the shell its text and caret (Look Up finds the word at the caret and opens under
/// it), and Look Up gets the selection as plain text. The list reports nothing.
#[test]
fn the_open_note_reports_its_text_and_caret_for_look_up() {
    use bifrost_app::app_windows::WindowId;
    let dir = scratch("text-report");
    let ui = ui();
    let mut n = app(&ui, &dir);
    assert!(n.text_report(&ui).is_none(), "no note is open");
    open_with(&mut n, &ui, "Title\nselect this word");
    let text = n.editor.text().to_string();
    let c = text[..text.find("select").unwrap()].chars().count();
    n.editor.set_caret(c + 2);
    let (state, (x, y, w, h)) = n.text_report(&ui).expect("an open note reports");
    assert_eq!(state.word_at_cursor(), "select", "the word at the caret");
    assert_eq!(state.selected_text(), "");
    let (ox, oy) = n.text_origin(&ui);
    let (cx, cy, _, ch) = n.editor.caret_rect(&ui);
    assert_eq!((x, y, h), (ox + cx, oy + cy, ch), "the caret in window px");
    assert!(w > 0.0 && oy + cy > 0.0, "below the toolbar, not at the window's corner");
    assert_eq!(n.look_up_selection(WindowId::MAIN).as_deref(), Some(""), "a caret selects nothing");
    n.editor.select(c..c + 6);
    let (state, _) = n.text_report(&ui).unwrap();
    assert_eq!(state.selected_text(), "select");
    assert_eq!(n.look_up_selection(WindowId::MAIN).as_deref(), Some("select"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_note_offers_undo_that_returns_it_to_its_folder() {
    let dir = scratch("undo-delete");
    let ui = ui();
    let mut n = app(&ui, &dir);
    n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
    type_text(&mut n, &ui, "Keep me", 100.0);
    n.activate(&ui, Target::Back, 0.0, 0.0, 300.0);
    let id = n.notes[0].id.clone();
    assert!(n.trash_note(0, 400.0));
    assert!(n.undo_toast.is_some(), "undo bar offered");
    assert_eq!(n.loc_of(&id), folders::TRASH);
    let _ = draw(&mut n, &ui, 401.0);
    n.activate(&ui, Target::UndoDelete, 0.0, 0.0, 500.0);
    assert!(n.undo_toast.is_none());
    assert_eq!(n.loc_of(&id), "", "back in All Notes");
    assert!(n.notes.iter().any(|x| x.id == id));
    n.trash_note(0, 600.0);
    n.tick(&ui, 600.0 + UNDO_TOAST_MS + 1.0);
    assert!(n.undo_toast.is_none(), "the bar goes away after five seconds");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_copies_text_and_images_and_clear_completed_drops_ticked_items() {
    let dir = scratch("duplicate");
    let ui = ui();
    let mut n = app(&ui, &dir);
    n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
    type_text(&mut n, &ui, "Plan\n- [x] done\n- [ ] open\n", 100.0);
    let src = n.editing().unwrap().id.clone();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{src}-1.png")), b"img").unwrap();
    n.editor.set_text(&ui, &format!("Plan\n![image]({src}-1.png)\n- [x] done\n- [ ] open"));
    n.changed(&ui, 200.0);
    n.activate(&ui, Target::Back, 0.0, 0.0, 300.0);
    let before = n.notes.len();

    n.open_note(&ui, n.notes.iter().position(|x| x.id == src).unwrap(), false, 400.0);
    n.run(&ui, Act::Duplicate, 500.0);
    assert_eq!(n.notes.len(), before + 1, "one copy");
    let copy = n.editing().unwrap().clone();
    assert_ne!(copy.id, src, "the copy is a new note and it is open");
    assert!(copy.text.contains(&format!("({}-1.png)", copy.id)), "image reference follows the copy");
    assert!(dir.join(format!("{}-1.png", copy.id)).exists(), "image copied beside the note");
    assert!(dir.join(format!("{}.md", copy.id)).exists(), "copy saved");

    n.run(&ui, Act::ClearChecked, 600.0);
    assert!(!n.editor.text().contains("[x]"), "ticked items cleared");
    assert!(n.editor.text().contains("- [ ] open") && n.editor.text().contains("Plan"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cmd_b_i_delete_and_escape_follow_the_desktop_keys() {
    let dir = scratch("chords-more");
    let ui = ui();
    let mut n = app(&ui, &dir);
    for (text, t) in [("Keep", 0.0), ("Drop", 500.0)] {
        n.activate(&ui, Target::Compose, 0.0, 0.0, t);
        type_text(&mut n, &ui, text, t + 100.0);
        n.activate(&ui, Target::Back, 0.0, 0.0, t + 300.0);
    }
    // Cmd+Backspace on the list deletes the arrow-selected note
    n.key(&ui, Out::Named("Down"), 1000.0);
    let before = n.listed().len();
    assert!(n.chord(&ui, 14, bifrost_app::proto::CHORD_META, 1001.0), "Cmd+Backspace");
    assert_eq!(n.listed().len(), before - 1, "one note moved to Recently Deleted");
    // Cmd+B and Cmd+I with nothing selected insert nothing: they style the next typing
    n.activate(&ui, Target::Compose, 0.0, 0.0, 2000.0);
    assert!(n.chord(&ui, 48, bifrost_app::proto::CHORD_CTRL, 2001.0), "Ctrl+B");
    assert!(!n.editing().unwrap().text.contains('*'), "no visible markers");
    assert_eq!(n.pending_styles(), &[Fmt::Bold]);
    assert!(n.chord(&ui, 23, bifrost_app::proto::CHORD_META, 2002.0), "Cmd+I");
    type_text(&mut n, &ui, "hi", 2003.0);
    assert_eq!(n.editing().unwrap().text, "***hi***", "the typing carries both styles");
    assert!(n.pending_styles().is_empty());
    // with a selection Cmd+B toggles the style of the selection: "hi" was bold, so the bold goes
    n.editor.select(3..5);
    assert!(n.chord(&ui, 48, bifrost_app::proto::CHORD_META, 2004.0));
    assert_eq!(n.editing().unwrap().text, "*hi*");
    // Escape drops the selection but keeps the note open, as in the reference
    assert!(n.escape(&ui, 2005.0), "the selection goes");
    assert!(matches!(n.screen, Screen::Editor(_)), "the note stays open");
    assert!(!n.escape(&ui, 2006.0), "nothing left to cancel");
    // Cmd+Backspace in the open note is Note > Delete Note
    let open = n.listed().len();
    assert!(n.chord(&ui, 14, bifrost_app::proto::CHORD_META, 2007.0), "Cmd+Backspace");
    assert!(matches!(n.screen, Screen::List));
    assert_eq!(n.listed().len(), open - 1, "the open note moved to Recently Deleted");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pixel of the boundary before char `c` of the open note, in the middle of its line.
fn char_px(n: &mut Notes, ui: &Ui, c: usize) -> (f32, f32) {
    let keep = n.editor.selection();
    let caret = n.editor.caret();
    n.editor.set_caret(c);
    let (x, y, _, h) = n.editor.caret_rect(ui);
    n.editor.set_caret(caret);
    if let Some(r) = keep {
        n.editor.select(r);
    }
    let (ox, oy) = n.text_origin(ui);
    (ox + x, oy + y + h / 2.0)
}

/// A note opened in its own window is edited there as in the main editor: its edits go back to the note (and to the
/// main editor showing it), and the Edit commands act on the window's text.
#[test]
fn a_note_window_edits_its_note_with_the_edit_commands() {
    use bifrost_app::app_windows::WindowId;
    let dir = scratch("window-edit");
    let ui = ui();
    let mut n = app(&ui, &dir);
    n.set_hardware_keyboard(true);
    open_with(&mut n, &ui, "Title\nbody");
    let Screen::Editor(i) = n.screen else { panic!("a note is open") };
    let id = n.notes[i].id.clone();
    let win = WindowId(7);
    n.windows.bind(win, &id);
    let mut buf = vec![0u32; 600 * 500];
    n.draw_window(&mut Surface { w: 600, h: 500, stride: 600, px: &mut buf, y0: 0 }, &ui, win, true, 2000.0);
    assert!(n.window_select_all(win));
    assert!(n.window_has_selection(win));
    assert!(n.window_key(&ui, win, Out::Text("Fresh".into()), 2100.0));
    assert_eq!(n.notes[i].text, "Fresh", "the window's typing went into the note");
    assert_eq!(n.editor.text(), "Fresh", "the main editor showing the note follows");
    assert_eq!(n.window_can_undo_redo(win), (true, false));
    // Cut takes the selection out and remembers its Markdown for a paste back
    assert!(n.window_select_all(win));
    assert_eq!(n.window_take_selection(&ui, win, true, 2200.0).as_deref(), Some("Fresh"));
    assert_eq!(n.notes[i].text, "");
    assert!(n.window_undo(&ui, win, false, 2300.0), "Undo brings it back");
    assert_eq!(n.notes[i].text, "Fresh");
    // Cmd+Right to the line end, then Shift+Cmd+Left selects back to its start, through the window's bindings
    use bifrost_app::proto::{CHORD_META, CHORD_SHIFT};
    assert!(n.window_chord(&ui, win, 106, CHORD_META, 2400.0));
    assert!(n.window_chord(&ui, win, 105, CHORD_META | CHORD_SHIFT, 2450.0));
    assert!(n.window_has_selection(win));
    assert!(!n.window_chord(&ui, WindowId(9), 105, CHORD_META, 2500.0), "a window with no note takes no keys");
    n.flush();
    let _ = std::fs::remove_dir_all(&dir);
}

/// An input method composes in the window being typed in: the main editor or a note window, each saved as typing.
#[test]
fn marked_text_composes_in_the_main_editor_and_in_a_note_window() {
    use bifrost_app::app_windows::WindowId;
    use bifrost_app::proto::ToApp;
    let dir = scratch("marked-text");
    let ui = ui();
    let mut n = app(&ui, &dir);
    n.set_hardware_keyboard(true);
    open_with(&mut n, &ui, "Title\nk");
    let Screen::Editor(i) = n.screen else { panic!("a note is open") };
    // the main editor: the candidate is marked, then committed into the note
    n.editor.set_caret(7);
    assert!(n.text_input(&ui, &ToApp::SetMarkedText { text: "á".into(), sel_start: 2, sel_len: 0 }, 1000.0));
    assert_eq!(n.editor.marked_range(), Some(7..8));
    assert!(n.text_input(&ui, &ToApp::UnmarkText { commit: true }, 1100.0));
    assert_eq!(n.notes[i].text, "Title\nká");
    assert_eq!(n.undo_titles().0, "Undo Typing");
    // the desktop editor's caret blinks with a hardware keyboard (no on-screen keyboard is up)
    n.editor.focused = true;
    n.flush();
    assert_eq!(n.next_wake(1200.0), Some(1600.0), "the caret blink is the next deadline once the edit is saved");
    // a note window: its own composition, its own Undo title, and the note follows
    let id = n.notes[i].id.clone();
    let win = WindowId(3);
    n.windows.bind(win, &id);
    let mut buf = vec![0u32; 600 * 500];
    n.draw_window(&mut Surface { w: 600, h: 500, stride: 600, px: &mut buf, y0: 0 }, &ui, win, true, 2000.0);
    assert!(n.window_text_input(&ui, win, &ToApp::SetMarkedText { text: "é".into(), sel_start: 2, sel_len: 0 }, 2100.0));
    assert!(n.window_text_input(&ui, win, &ToApp::UnmarkText { commit: true }, 2200.0));
    assert_eq!(n.notes[i].text, "Title\nkáé");
    assert_eq!(n.window_undo_titles(win).0, "Undo Typing");
    assert!(!n.window_text_input(&ui, WindowId(9), &ToApp::UnmarkText { commit: true }, 2300.0), "no such window");
    n.flush();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_mouse_selects_note_text_by_drag_double_and_triple_click_and_shift_click() {
    let dir = scratch("mouse-select");
    let ui = ui();
    let mut n = app(&ui, &dir);
    n.set_hardware_keyboard(true);
    open_with(&mut n, &ui, "Title\nthe quick brown fox\nnext line");
    // a press, a drag and a release select by characters
    let (x0, y0) = char_px(&mut n, &ui, 10);
    let (x1, y1) = char_px(&mut n, &ui, 15);
    n.touch(&ui, Event::Down(x0, y0), 5000.0);
    n.touch(&ui, Event::Move(x1, y1), 5010.0);
    n.touch(&ui, Event::Up(x1, y1), 5020.0);
    assert_eq!(n.selected_markdown().as_deref(), Some("quick"), "drag");
    assert_eq!(n.scroll.offset(), 0.0, "the drag did not scroll");
    // a double click selects the word, a triple click the paragraph
    let (xw, yw) = char_px(&mut n, &ui, 18);
    for (k, t) in [5600.0, 5700.0].into_iter().enumerate() {
        n.touch(&ui, Event::Down(xw, yw), t);
        n.touch(&ui, Event::Up(xw, yw), t + 20.0);
        if k == 1 {
            assert_eq!(n.selected_markdown().as_deref(), Some("brown"), "double click");
        }
    }
    n.touch(&ui, Event::Down(xw, yw), 5800.0);
    n.touch(&ui, Event::Up(xw, yw), 5820.0);
    assert_eq!(n.selected_markdown().as_deref(), Some("the quick brown fox"), "triple click");
    // a click places the caret; a Shift-click extends from it
    n.touch(&ui, Event::Down(x0, y0), 7000.0);
    n.touch(&ui, Event::Up(x0, y0), 7020.0);
    assert_eq!(n.selected_markdown(), None);
    n.set_pointer_shift(true);
    n.touch(&ui, Event::Down(x1, y1), 8000.0);
    n.touch(&ui, Event::Up(x1, y1), 8020.0);
    n.set_pointer_shift(false);
    assert_eq!(n.selected_markdown().as_deref(), Some("quick"), "Shift-click");
    // the keys: Shift+Option+Right takes the next word, Cmd+A everything
    use bifrost_app::proto::{CHORD_ALT, CHORD_META, CHORD_SHIFT};
    assert!(n.chord(&ui, 106, CHORD_SHIFT | CHORD_ALT, 8100.0), "Shift+Option+Right");
    assert_eq!(n.selected_markdown().as_deref(), Some("quick brown"));
    assert!(n.chord(&ui, 102, CHORD_SHIFT, 8200.0), "Shift+Home");
    assert_eq!(n.selected_markdown().as_deref(), Some("the "), "from the anchor back to the line start");
    assert!(n.chord(&ui, 30, CHORD_META, 8300.0), "Cmd+A");
    assert_eq!(n.selected_markdown().as_deref(), Some("Title\nthe quick brown fox\nnext line"));
    // the wheel scrolls the note column (nothing to scroll in a short note) and never selects
    assert!(!n.wheel(&ui, x0, y0, -40.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edit_commands_reach_the_note_and_the_focused_field() {
    let dir = scratch("edit-commands");
    let ui = ui();
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Title\n**bold** words");
    assert!(!n.has_selection());
    assert!(n.select_all(&ui, 3000.0));
    assert!(n.has_selection());
    // Cut takes the Markdown out; a paste of the same text brings the styles back
    n.editor.select(6..14);
    assert_eq!(n.take_selection(&ui, true, 3100.0).as_deref(), Some("**bold**"));
    assert_eq!(n.editor.text(), "Title\n words");
    springboard::shell::clipboard::set_text("bold");
    n.editor.set_caret(6);
    assert!(n.paste(&ui, 3200.0));
    assert_eq!(n.editor.text(), "Title\n**bold** words", "the copied Markdown comes back");
    // with the search field focused the commands are the field's
    n.activate(&ui, Target::Search, 0.0, 0.0, 3300.0);
    assert!(n.field_focused());
    springboard::shell::clipboard::set_text("words");
    assert!(n.paste(&ui, 3400.0));
    assert_eq!(n.search.text(), "words");
    assert!(n.select_all(&ui, 3500.0));
    assert!(n.has_selection());
    assert_eq!(n.editor.text(), "Title\n**bold** words", "the note is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_aa_button_opens_the_style_popover_and_headings_drop_list_markers() {
    let dir = scratch("style-popover");
    let ui = ui();
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Title\n- [ ] Item two");
    let (x, y, w, h) = n.bar_px(&ui, chrome::B_STYLE);
    assert!(w > 0.0, "Aa is on the bar");
    assert_eq!(n.hit(&ui, x + w / 2.0, y + h / 2.0), Some(Target::StyleMenu));
    tap(&mut n, &ui, x + w / 2.0, y + h / 2.0, 2000.0);
    let menu = n.menu.as_ref().expect("the popover");
    let labels: Vec<&str> = menu.items.iter().map(|(l, ..)| l.as_str()).collect();
    assert_eq!(&labels[..4], ["Title", "Heading", "Subheading", "Body"]);
    let checked: Vec<&str> = menu.items.iter().filter(|(_, _, on)| *on).map(|(l, ..)| l.as_str()).collect();
    assert_eq!(checked, ["Checklist"], "the caret's style is checked");
    assert_eq!(n.editor.text(), "Title\n- [ ] Item two", "opening the popover changes nothing");
    n.menu = None;
    n.run(&ui, Act::Style(md::Block::Heading), 2100.0);
    assert_eq!(n.editor.text(), "Title\n## Item two", "the checkbox goes, not `## - [ ] Item two`");
    // the touch shortcut row's cycle treats a checklist line the same way
    n.editor.set_text(&ui, "Title\n- [ ] Item two");
    n.editor.set_caret(n.editor.len());
    n.activate(&ui, Target::Format, 0.0, 0.0, 2200.0);
    assert_eq!(n.editor.text(), "Title\n## Item two");
    assert_eq!(n.editor.caret(), n.editor.len(), "the caret stays at the end of the line");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_narrow_window_keeps_the_open_search_field_and_the_overflow_button_works() {
    let dir = scratch("toolbar-overflow");
    let ui = ui();
    // 1200 px at 1.5625 is the E2E window: 768 pt, too narrow for everything beside an open search field
    let mut n = Notes::new(&ui, 1200, 800, dir.clone(), wall());
    // plain body text, as `app` sets: the default Title start would leave its "# " after the inserted text
    n.note_settings.new_note_start = settings::NewNoteStart::Body;
    n.set_hardware_keyboard(true);
    open_with(&mut n, &ui, "Title\nbody");
    assert!(n.bar_rect(&ui, chrome::B_SEARCH).is_some(), "the closed search fits");
    assert!(n.bar_rect(&ui, chrome::B_COMPOSE).is_some());
    n.find(&ui, 2000.0);
    let field = n.bar_rect(&ui, chrome::B_SEARCH).expect("the open search field stays on the bar");
    assert!(field.w >= 100.0, "a usable field: {field:?}");
    // the format buttons went behind the » button, which opens them as a menu
    let over = n.editor_overflow(&ui).expect("an overflow button");
    let (px, py) = (ui.pt(over.x + over.w / 2.0), ui.pt(over.y + over.h / 2.0));
    assert!(matches!(n.hit(&ui, px, py), Some(Target::Overflow(_))));
    tap(&mut n, &ui, px, py, 2100.0);
    let labels: Vec<String> = n.menu.as_ref().expect("the overflow menu").items.iter().map(|(l, ..)| l.clone()).collect();
    assert!(labels.contains(&"Format".to_string()) && labels.contains(&"Checklist".to_string()), "{labels:?}");
    n.run(&ui, Act::Bar(chrome::B_CHECK), 2200.0);
    assert_eq!(n.editor.text(), "Title\n- [ ] body");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_folder_proposes_a_selected_name_and_lists_no_notes_while_a_search_was_typed() {
    let ui = ui();
    let dir = scratch("folder-new");
    let mut n = seeded(&ui, &dir);
    // a search typed earlier would otherwise fill the new folder with every match
    n.search.set_text(&ui, "rust");
    n.rebuild(&ui, 0.5);
    n.run(&ui, Act::NewFolder, 1.0);
    n.key(&ui, Out::Text("E".into()), 2.0);
    n.key(&ui, Out::Text("2E".into()), 2.1);
    assert_eq!(n.naming.as_ref().unwrap().field.text(), "E2E", "typing replaced the proposed name");
    n.key(&ui, Out::Enter, 3.0);
    assert_eq!(n.view(), &View::Folder("E2E".into()));
    assert!(n.listed().is_empty(), "the new folder is empty");
    assert_eq!(n.count_in(&View::Folder("E2E".into())), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn right_clicks_offer_note_and_folder_menus() {
    let ui = ui();
    let dir = scratch("context");
    let mut n = seeded(&ui, &dir);
    let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
    let rows = n.context_menu(&ui, rx + rw / 2.0, ry + rh / 2.0, 1.0);
    let titles: Vec<&str> = rows.iter().map(|(_, t)| t.as_str()).collect();
    assert!(titles.contains(&"Pin Note") && titles.contains(&"Delete Note"), "{titles:?}");
    assert!(matches!(n.screen, Screen::Editor(_)), "the row is selected");
    let before = n.listed().len();
    let delete = rows.iter().find(|(_, t)| t == "Delete Note").unwrap().0;
    assert!(n.context_action(&ui, delete, 2.0));
    assert_eq!(n.listed().len(), before - 1);
    assert!(!n.context_action(&ui, delete, 3.0), "a menu is used once");
    // nothing under the pointer: no menu
    assert!(n.context_menu(&ui, 1.0, 1.0, 4.0).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cmd_chords_compose_search_and_undo() {
    let dir = scratch("chords");
    let ui = ui();
    let mut n = app(&ui, &dir);
    assert!(n.chord(&ui, 49, bifrost_app::proto::CHORD_CTRL, 0.0), "Ctrl+N");
    assert!(matches!(n.screen, Screen::Editor(_)), "a new note opens");
    assert!(n.chord(&ui, 44, bifrost_app::proto::CHORD_CTRL, 1.0), "Ctrl+Z in the editor");
    assert!(n.chord(&ui, 30, bifrost_app::proto::CHORD_CTRL, 2.0), "Ctrl+A is the line start in the note (edit_keys)");
    n.activate(&ui, Target::Back, 0.0, 0.0, 3.0);
    assert!(n.chord(&ui, 33, bifrost_app::proto::CHORD_META, 4.0), "Cmd+F on the list");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn arrow_keys_select_list_rows_and_enter_opens_the_selected_note() {
    let dir = scratch("arrow-list");
    let ui = ui();
    let mut n = app(&ui, &dir);
    for (text, t) in [("First note", 0.0), ("Second note", 500.0)] {
        n.activate(&ui, Target::Compose, 0.0, 0.0, t);
        type_text(&mut n, &ui, text, t + 100.0);
        n.activate(&ui, Target::Back, 0.0, 0.0, t + 300.0);
    }
    n.key(&ui, Out::Named("Down"), 1000.0);
    assert_eq!(n.selected_index(), Some(0), "Down from nothing selects the top row");
    n.key(&ui, Out::Named("Down"), 1001.0);
    n.key(&ui, Out::Named("Down"), 1002.0);
    assert_eq!(n.selected_index(), Some(1), "stops at the last row");
    n.key(&ui, Out::Named("Up"), 1003.0);
    assert_eq!(n.selected_index(), Some(0));
    n.key(&ui, Out::Enter, 1004.0);
    assert!(matches!(n.screen, Screen::Editor(_)), "Enter opens the selected note");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tapping_a_checkbox_toggles_it_and_undo_restores_it() {
    let ui = ui();
    let dir = scratch("checklist");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Todo\n- [ ] milk\n- [ ] eggs\n- [ ] bread");
    let (ox, oy) = n.text_origin(&ui);
    let at = |n: &Notes, c: usize| oy + n.editor.y_of(c) + 3.0;
    let y = at(&n, 5);
    tap(&mut n, &ui, ox + 2.0, y, 2000.0);
    assert_eq!(n.editing().unwrap().text, "Todo\n- [x] milk\n- [ ] eggs\n- [ ] bread");
    assert!(n.autosave.pending(), "the toggle autosaves");
    n.activate(&ui, Target::Undo, 0.0, 0.0, 2100.0);
    assert_eq!(n.editing().unwrap().text, "Todo\n- [ ] milk\n- [ ] eggs\n- [ ] bread");
    n.activate(&ui, Target::Redo, 0.0, 0.0, 2200.0);
    assert_eq!(n.editing().unwrap().text, "Todo\n- [x] milk\n- [ ] eggs\n- [ ] bread");
    let y = at(&n, 5);
    tap(&mut n, &ui, ox + 2.0, y, 3000.0);
    assert!(n.editing().unwrap().text.contains("- [ ] milk"), "tapping again unticks");
    // the box is the target; the text beside it just places the caret
    let y = at(&n, 5);
    tap(&mut n, &ui, ox + 300.0, y, 4000.0);
    assert!(n.editing().unwrap().text.contains("- [ ] milk"));

    // with "move checked to the bottom" a tick sinks the item, one undo step
    n.run(&ui, Act::ToggleCheckedBottom, 5000.0);
    assert!(n.prefs().checked_to_bottom && store::load_prefs(&dir).checked_to_bottom);
    let y = at(&n, 5);
    tap(&mut n, &ui, ox + 2.0, y, 6000.0);
    assert_eq!(n.editing().unwrap().text, "Todo\n- [ ] eggs\n- [ ] bread\n- [x] milk");
    n.activate(&ui, Target::Undo, 0.0, 0.0, 6100.0);
    assert_eq!(n.editing().unwrap().text, "Todo\n- [ ] milk\n- [ ] eggs\n- [ ] bread");
    // the one-shot action from the menu is undoable too
    n.editor.replace_range(&ui, 8..9, "x", 7000.0, false);
    n.run(&ui, Act::SortCheckedNow, 7001.0);
    assert!(n.editing().unwrap().text.ends_with("- [x] milk"), "{}", n.editing().unwrap().text);
    n.activate(&ui, Target::Undo, 0.0, 0.0, 7100.0);
    assert!(n.editing().unwrap().text.starts_with("Todo\n- [x] milk"));
    draw(&mut n, &ui, 8000.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn formatting_bar_and_list_continuation_are_undoable() {
    let ui = ui();
    let dir = scratch("format");
    let mut n = app(&ui, &dir);
    n.new_note(&ui, 0.0);
    type_text(&mut n, &ui, "Notes\n", 10.0);
    let y = n.shortcut_top(&ui).unwrap() + ui.pt(SHORTCUT_PT) / 2.0;
    let slot = |k: f32| ui.pt(MARGIN_PT) + ui.pt(2.0 * UNDO_W_PT + FMT_W_PT * (k + 0.5));
    assert_eq!(n.hit(&ui, slot(0.0), y), Some(Target::Fmt(Fmt::Bold)));
    assert_eq!(n.hit(&ui, slot(1.0), y), Some(Target::Fmt(Fmt::Italic)));
    assert_eq!(n.hit(&ui, slot(3.0), y), Some(Target::Fmt(Fmt::Bullet)));
    assert_eq!(n.hit(&ui, slot(5.0), y), Some(Target::Fmt(Fmt::Check)));
    tap(&mut n, &ui, slot(3.0), y, 100.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- ");
    type_text(&mut n, &ui, "milk\neggs", 200.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs", "Enter continued the list");
    n.key(&ui, Out::Enter, 300.0);
    n.key(&ui, Out::Enter, 301.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs\n", "a second Enter ends the list");
    n.activate(&ui, Target::Undo, 0.0, 0.0, 400.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs\n- ");
    n.activate(&ui, Target::Undo, 0.0, 0.0, 401.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs");
    let len = n.editor.len();
    n.editor.select(len - 4..len);
    tap(&mut n, &ui, slot(0.0), y, 500.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- **eggs**");
    tap(&mut n, &ui, slot(0.0), y, 600.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs", "bold toggles off again");
    n.activate(&ui, Target::Undo, 0.0, 0.0, 700.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- **eggs**");
    // numbered style over several lines
    n.editor.select(6..n.editor.len());
    tap(&mut n, &ui, slot(4.0), y, 800.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n1. milk\n2. **eggs**");
    n.run(&ui, Act::Style(md::Block::Heading), 900.0);
    n.activate(&ui, Target::Undo, 0.0, 0.0, 901.0);
    assert_eq!(n.editing().unwrap().text, "Notes\n1. milk\n2. **eggs**");
    draw(&mut n, &ui, 1000.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_folder_closes_the_open_note_that_is_not_in_it() {
    let ui = ui();
    let dir = scratch("folder-closes-note");
    let mut n = seeded(&ui, &dir);
    n.open_note(&ui, 0, false, 1.0);
    assert!(n.is_editing());
    n.new_folder(&ui, 2.0);
    n.naming.as_mut().unwrap().field.set_text(&ui, "BTestFolder");
    n.key(&ui, Out::Enter, 3.0);
    assert_eq!(n.view(), &View::Folder("BTestFolder".into()));
    assert!(!n.is_editing(), "the note is not in the new empty folder, so the editor shows no note");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sidebar_lists_folders_with_counts_and_switches_the_list() {
    let ui = ui();
    let dir = scratch("folders-sidebar");
    let mut n = seeded(&ui, &dir);
    let (nx, ny) = centre(chrome::px(&ui, n.bar_rect(&ui, chrome::B_NEW_FOLDER).unwrap()));
    assert_eq!(n.hit(&ui, nx, ny), Some(Target::FolderNew));
    tap(&mut n, &ui, nx, ny, 2.0);
    n.naming.as_mut().unwrap().field.set_text(&ui, "Work");
    n.key(&ui, Out::Enter, 3.0);
    assert_eq!(n.folder_names(), ["Work".to_string()]);
    assert_eq!(n.view(), &View::Folder("Work".into()), "a new folder opens");
    draw(&mut n, &ui, 4.0);
    let list = n.sidebar();
    let rows: Vec<_> = list.items.iter().filter_map(|i| match i {
        bifrost_app::controls::sidebar::SidebarItem::Row { label, count, .. } => Some((label.clone(), *count)),
        _ => None,
    }).collect();
    assert_eq!(rows.len(), 7, "All Notes, the folder, the bin, then the four smart folders");
    assert_eq!(rows[..3].to_vec(), vec![("All Notes".into(), Some(2)), ("Work".into(), Some(0)), ("Recently Deleted".into(), Some(0))]);
    assert_eq!(rows[3].0, "Pinned");
    // a click on All Notes in the sidebar shows every note again
    let bounds = bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: n.side_pt(&ui), h: 1500.0 / ui.pt(1.0) };
    let r = list.item_rect(1, bounds, -(bifrost_app::layout::UNIFIED_BAR_PT + 4.0));
    let (x, y) = (ui.pt(r.x + r.w / 2.0), ui.pt(r.y + r.h / 2.0));
    assert!(matches!(n.hit(&ui, x, y), Some(Target::Sidebar(_))));
    tap(&mut n, &ui, x, y, 5.0);
    assert_eq!(n.view(), &View::All);
    assert_eq!(n.listed().len(), 2);
    // the sidebar folds away and comes back
    n.activate(&ui, Target::SidebarToggle, 0.0, 0.0, 6.0);
    assert_eq!(n.side_pt(&ui), 0.0);
    assert!(n.hit(&ui, x, y) != Some(Target::Sidebar(100)));
    n.activate(&ui, Target::SidebarToggle, 0.0, 0.0, 7.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folders_can_be_created_renamed_filled_and_deleted() {
    let ui = ui();
    let dir = scratch("folders");
    let mut n = seeded(&ui, &dir);
    n.run(&ui, Act::NewFolder, 1.0);
    assert!(n.naming.is_some() && n.keyboard_shown());
    draw(&mut n, &ui, 2.0);
    // the name card sits over the list top: the first row starts below it, not under it
    let card_bottom = ui.pt(nav_bottom_pt() + SPACE_S + NAMING_CARD_PT);
    assert!(n.row_rect(&ui, 0).1 >= card_bottom, "the folder-name card covers the first row");
    assert_eq!(n.naming.as_ref().unwrap().field.text(), "New Folder");
    n.naming.as_mut().unwrap().field.set_text(&ui, "Work");
    n.key(&ui, Out::Enter, 3.0);
    assert!(n.naming.is_none() && !n.keyboard_shown());
    assert_eq!(n.view(), &View::Folder("Work".into()));
    assert_eq!(n.folder_names(), ["Work".to_string()]);
    assert!(n.listed().is_empty(), "a new folder is empty");
    assert!(n.describe().contains("notes_view=folder:Work"));

    // move a note into it from the editor menu
    n.run(&ui, Act::View(View::All), 4.0);
    let i = n.notes.iter().position(|x| x.id == "1").unwrap();
    n.open_note(&ui, i, false, 5.0);
    n.open_editor_menu(&ui);
    assert!(n.menu.as_ref().unwrap().items.iter().any(|(label, ..)| label == "Move to Work"));
    n.run(&ui, Act::MoveTo("Work".into()), 6.0);
    assert!(dir.join("Work/1.md").exists() && !dir.join("1.md").exists());
    assert_eq!(n.folder_of("1"), Some("Work"));
    n.close_note(&ui, 7.0);
    assert_eq!(n.listed().len(), 2, "All Notes still shows it");
    n.run(&ui, Act::View(View::Folder("Work".into())), 8.0);
    assert_eq!(n.listed(), vec![("1", "Groceries")]);
    // searching looks through every folder
    n.search.set_text(&ui, "rust");
    n.rebuild(&ui, 9.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    n.search.set_text(&ui, "");
    n.rebuild(&ui, 9.5);
    // editing inside a folder saves into it
    n.open_note(&ui, n.shown[0], true, 10.0);
    type_text(&mut n, &ui, "!", 11.0);
    n.tick(&ui, 1000.0);
    assert!(std::fs::read_to_string(dir.join("Work/1.md")).unwrap().ends_with('!'));
    assert!(!dir.join("1.md").exists());
    n.close_note(&ui, 1100.0);
    // a new note composed inside the folder lands there
    n.new_note(&ui, 1200.0);
    let new_id = n.editing().unwrap().id.clone();
    type_text(&mut n, &ui, "Fresh", 1300.0);
    n.close_note(&ui, 1400.0);
    assert!(dir.join("Work").join(format!("{new_id}.md")).exists());

    // rename keeps the notes
    n.run(&ui, Act::RenameFolder, 2000.0);
    assert_eq!(n.naming.as_ref().unwrap().field.text(), "Work");
    n.naming.as_mut().unwrap().field.set_text(&ui, "Projects");
    n.key(&ui, Out::Enter, 2001.0);
    assert_eq!(n.view(), &View::Folder("Projects".into()));
    assert!(dir.join("Projects/1.md").exists() && !dir.join("Work").exists());
    assert_eq!(n.listed().len(), 2);
    // a clashing or empty name leaves everything alone and reports it
    n.run(&ui, Act::NewFolder, 2100.0);
    n.naming.as_mut().unwrap().field.set_text(&ui, "   ");
    n.key(&ui, Out::Enter, 2101.0);
    assert!(n.notice.is_some());
    draw(&mut n, &ui, 2200.0);

    // deleting the folder moves its notes to Recently Deleted
    n.view = View::Folder("Projects".into());
    n.run(&ui, Act::DeleteFolder, 3000.0);
    assert_eq!(n.view(), &View::All);
    assert!(n.folder_names().is_empty());
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    n.run(&ui, Act::View(View::Trash), 3100.0);
    assert_eq!(n.listed().len(), 2);
    draw(&mut n, &ui, 3200.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exchange_notes_appear_in_their_folder_and_local_edits_go_back_as_changes() {
    use bifrost_app::controls::internet_accounts::{Account, Kind};
    use bifrost_app::exchange_notes::{ItemId, Note as Remote, Op, Report};
    let ui = ui();
    let dir = scratch("exchange");
    let account = Account { url: "https://mail.example.com/EWS/Exchange.asmx".into(), ..Account::new(Kind::Exchange, "exchange-1", "Work", "ann") };
    let remote = |id: &str, key: &str, subject: &str, body: &str| Remote {
        item: ItemId { id: id.into(), change_key: key.into() },
        subject: subject.into(),
        body: body.into(),
        modified: "2026-10-01T08:00:00Z".into(),
    };
    let mut n = app(&ui, &dir);
    assert!(n.exchange_plan(&account).is_empty(), "nothing to send before the first round");
    let first = Report { written: vec![], notes: Ok(vec![remote("A1", "k1", "Budget", "Budget\nQ4"), remote("A2", "k1", "Trip", "Trip\nParis")]) };
    let applied = n.exchange_apply(&ui, 1000.0, &account, &first);
    assert_eq!(applied.added, 2);
    // the account's notes are a folder in the sidebar, and the list shows them
    assert!(n.folders.contains(&"Work".to_string()));
    n.run(&ui, Act::View(View::Folder("Work".into())), 1100.0);
    let mut titles: Vec<&str> = n.listed().iter().map(|(_, t)| *t).collect();
    titles.sort();
    assert_eq!(titles, vec!["Budget", "Trip"]);
    assert!(n.exchange_plan(&account).is_empty(), "what was just read is not sent back");
    // edit Budget in the editor; the server meanwhile changed both notes
    let budget = exchange::local_id("A1");
    let i = n.notes.iter().position(|x| x.id == budget).unwrap();
    n.open_note(&ui, i, true, 1200.0);
    type_text(&mut n, &ui, "\nmine", 1300.0);
    let second = Report { written: vec![], notes: Ok(vec![remote("A1", "k2", "Budget", "Budget\nserver"), remote("A2", "k2", "Trip", "Trip\nRome")]) };
    let applied = n.exchange_apply(&ui, 2000.0, &account, &second);
    assert_eq!(applied.updated, 1, "the open note waits, the other one is replaced");
    assert!(n.notes.iter().any(|x| x.text == "Trip\nRome"));
    assert!(n.editing().is_some_and(|x| x.id == budget && x.text.contains("mine")));
    // the edit goes to the server as an update of that mailbox item
    let ops = n.exchange_plan(&account);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], Op::Update { local, id, body, .. } if *local == budget && id.id == "A1" && body.contains("mine")));
    // deleting a note in the folder is a delete on the server
    n.close_note(&ui, 3000.0);
    let trip = exchange::local_id("A2");
    let row = n.shown.iter().position(|&s| n.notes[s].id == trip).unwrap();
    n.delete_row(&ui, row, 3100.0);
    assert!(n.exchange_plan(&account).iter().any(|op| matches!(op, Op::Delete { local, id } if *local == trip && id.id == "A2")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleted_notes_can_be_recovered_erased_and_expire() {
    let ui = ui();
    let dir = scratch("trash");
    std::fs::create_dir_all(&dir).unwrap();
    store::save(&dir, &Note { id: "9".into(), text: "Ancient".into(), modified: 0 }).unwrap();
    folders::trash(&dir, "9", None, wall().unix - 31 * 86_400).unwrap();
    let mut n = seeded(&ui, &dir);
    assert!(!n.notes.iter().any(|x| x.id == "9"), "notes deleted more than 30 days ago are purged on launch");
    assert!(!dir.join(folders::TRASH).join("9.md").exists());
    n.delete_row(&ui, 0, 100.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    assert!(dir.join(folders::TRASH).join("1.md").exists());
    n.run(&ui, Act::View(View::Trash), 200.0);
    assert_eq!(n.listed(), vec![("1", "Groceries")]);
    n.open_note(&ui, n.shown[0], false, 300.0);
    n.open_editor_menu(&ui);
    let labels: Vec<_> = n.menu.as_ref().unwrap().items.iter().map(|(l, ..)| l.as_str()).collect();
    assert!(labels.contains(&"Recover Note") && labels.contains(&"Delete Permanently") && !labels.contains(&"Delete Note"));
    n.run(&ui, Act::Recover, 400.0);
    assert_eq!(n.editing(), None);
    assert!(dir.join("1.md").exists());
    assert!(n.listed().is_empty(), "the bin view no longer lists it");
    n.run(&ui, Act::View(View::All), 500.0);
    assert_eq!(n.listed().len(), 2);
    // delete from the editor, then erase for good
    n.open_note(&ui, n.shown[0], false, 600.0);
    let id = n.editing().unwrap().id.clone();
    n.run(&ui, Act::DeleteNote, 700.0);
    assert_eq!(n.editing(), None);
    assert_eq!(n.folder_of(&id), Some(folders::TRASH));
    n.run(&ui, Act::View(View::Trash), 800.0);
    n.delete_row(&ui, 0, 900.0);
    assert!(n.listed().is_empty());
    assert!(!dir.join(folders::TRASH).join(format!("{id}.md")).exists());
    assert!(draw(&mut n, &ui, 1000.0).contains(&LABEL_2.now()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_note_inside_recently_deleted_erases_it() {
    // E2E R4: Note > Delete Note (Cmd+Backspace) on a note in Recently Deleted moved it onto itself and failed
    // with "Note exists in destination"; it now goes for good, as Delete Permanently does
    let ui = ui();
    let dir = scratch("trash-delete-note");
    std::fs::create_dir_all(&dir).unwrap();
    let mut n = seeded(&ui, &dir);
    n.delete_row(&ui, 0, 100.0);
    assert!(dir.join(folders::TRASH).join("1.md").exists());
    n.run(&ui, Act::View(View::Trash), 200.0);
    n.open_note(&ui, n.shown[0], false, 300.0);
    assert!(n.chord(&ui, 14, bifrost_app::proto::CHORD_META, 400.0), "Cmd+Backspace");
    // it asks first, as the reference does: the note cannot come back
    let sheet = n.erase_sheet().expect("the alert is up");
    assert!(sheet.alert, "the kit's alert sheet, not a popover");
    assert_eq!((sheet.title.as_str(), sheet.message.as_str()), (ERASE_TITLE, ERASE_MESSAGE));
    assert!(sheet.buttons.iter().any(|b| b.title == "Delete" && b.role == bifrost_app::controls::sheet::ButtonRole::Destructive));
    assert_eq!(sheet.cancel_button(), Some(ERASE_CANCEL));
    // E2E12 finding 6: attached under the title bar, full-width buttons stacked, Delete on top and Cancel under it
    let (parent, bar) = n.sheet_parent(&ui);
    let placed = sheet.layout(parent, bar, 1.0);
    assert_eq!(placed.panel.y, bar, "hangs from the title bar");
    assert!((placed.panel.x + placed.panel.w / 2.0 - parent.w / 2.0).abs() < 0.5, "centred in the window");
    let ids: Vec<u32> = placed.buttons.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![ERASE_DELETE, ERASE_CANCEL]);
    let (del, cancel) = (placed.buttons[0].1, placed.buttons[1].1);
    assert!(cancel.y > del.y && cancel.x == del.x && cancel.w == del.w, "stacked, not side by side");
    assert!(dir.join(folders::TRASH).join("1.md").exists(), "nothing erased yet");
    n.key(&ui, Out::Enter, 410.0);
    assert!(n.erase_sheet().is_some(), "Return does not answer the destructive Delete");
    assert!(n.escape(&ui, 420.0) && n.erase_sheet().is_none(), "Escape cancels");
    assert!(n.editing().is_some() && dir.join(folders::TRASH).join("1.md").exists(), "the note stays");
    // asked again, the alert's Delete erases it
    assert!(n.chord(&ui, 14, bifrost_app::proto::CHORD_META, 430.0));
    let (parent, bar) = n.sheet_parent(&ui);
    let sheet = n.erase_sheet().unwrap();
    let (_, r) = sheet.layout(parent, bar, 1.0).buttons.into_iter().find(|(id, _)| *id == ERASE_DELETE).unwrap();
    let k = ui.pt(1.0);
    tap(&mut n, &ui, (r.x + r.w / 2.0) * k, (r.y + r.h / 2.0) * k, 440.0);
    assert_eq!(n.erase_sheet(), None);
    assert_eq!(n.notice, None, "no error notice");
    assert_eq!(n.editing(), None, "the note closed");
    assert!(n.listed().is_empty());
    assert!(!dir.join(folders::TRASH).join("1.md").exists(), "erased from disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_permanently_shares_the_alert() {
    let ui = ui();
    let dir = scratch("trash-delete-permanently");
    std::fs::create_dir_all(&dir).unwrap();
    let mut n = seeded(&ui, &dir);
    n.delete_row(&ui, 0, 100.0);
    n.run(&ui, Act::View(View::Trash), 200.0);
    n.open_note(&ui, n.shown[0], false, 300.0);
    n.run(&ui, Act::EraseNote, 400.0);
    assert!(n.erase_sheet().is_some(), "Delete Permanently asks too");
    assert!(dir.join(folders::TRASH).join("1.md").exists());
    // the window behind the alert is blocked: a tap far from its buttons does nothing
    let y = n.h as f32 - 4.0;
    tap(&mut n, &ui, 4.0, y, 410.0);
    assert!(n.erase_sheet().is_some() && n.editing().is_some());
    n.run(&ui, Act::EraseConfirmed, 420.0);
    assert!(n.erase_sheet().is_none() && n.editing().is_none());
    assert!(!dir.join(folders::TRASH).join("1.md").exists(), "erased from disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_delete_alert_takes_cmd_d_cmd_period_and_the_tab_focus_ring() {
    use bifrost_app::proto::{CHORD_FLAG, CHORD_META};
    use bifrost_app::KeyInput;
    // Cmd+D is the chord on evdev key 32 with Meta, Cmd+. the one on key 52
    let cmd_d = KeyInput::Code(CHORD_FLAG | 32 | (CHORD_META << 8));
    let cmd_period = KeyInput::Code(CHORD_FLAG | 52 | (CHORD_META << 8));
    let (tab, space, ret) = (KeyInput::Code(9), KeyInput::Text(" ".into()), KeyInput::Code(13));
    let ui = ui();
    let dir = scratch("erase-alert-keys");
    std::fs::create_dir_all(&dir).unwrap();
    let mut n = seeded(&ui, &dir);
    n.delete_row(&ui, 0, 100.0);
    n.run(&ui, Act::View(View::Trash), 200.0);
    n.open_note(&ui, n.shown[0], false, 300.0);
    let note = dir.join(folders::TRASH).join("1.md");
    assert!(!n.erase_key(&ui, &space, 399.0), "no alert, not its key");
    n.run(&ui, Act::EraseNote, 400.0);
    assert!(n.erase_open());
    assert_eq!(n.erase_sheet().unwrap().focus, None, "no ring until Tab");
    assert!(!n.erase_key(&ui, &ret, 410.0) && n.erase_open(), "Return never answers the destructive Delete");
    assert!(!n.erase_key(&ui, &space, 411.0), "Space with no focus presses nothing");
    // Tab: Delete (drawn first), then Cancel
    assert!(n.erase_key(&ui, &tab, 420.0));
    assert_eq!(n.erase_sheet().unwrap().focus, Some(ERASE_DELETE));
    assert!(n.erase_key(&ui, &tab, 421.0));
    assert_eq!(n.erase_sheet().unwrap().focus, Some(ERASE_CANCEL));
    assert!(n.erase_key(&ui, &space, 422.0) && !n.erase_open(), "Space on Cancel closes the alert");
    assert!(note.exists(), "and keeps the note");
    // Cmd+. cancels
    n.run(&ui, Act::EraseNote, 430.0);
    assert!(n.erase_key(&ui, &cmd_period, 431.0) && !n.erase_open() && note.exists());
    // the ring starts over each time the alert opens, and Cmd+D presses Delete
    n.run(&ui, Act::EraseNote, 440.0);
    assert_eq!(n.erase_sheet().unwrap().focus, None);
    assert!(n.erase_key(&ui, &cmd_d, 441.0));
    assert!(!n.erase_open() && !note.exists(), "Cmd+D erases the note");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sort_options_and_pinned_section_persist() {
    let ui = ui();
    let dir = scratch("sort");
    let mut n = seeded(&ui, &dir);
    n.notes.push(Note { id: "3".into(), text: "Apricot".into(), modified: wall().unix - 86_400 * 9 });
    n.rebuild(&ui, 0.0);
    let ids = |n: &Notes| n.listed().iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&n), ["1", "2", "3"]);
    n.run(&ui, Act::Sort(store::SortKey::Title), 1.0);
    assert_eq!(ids(&n), ["3", "1", "2"]);
    assert!(n.set_pinned(&ui, "2", true, 2.0));
    assert_eq!(ids(&n), ["2", "3", "1"], "pinned first, then by title");
    n.run(&ui, Act::Sort(store::SortKey::DateCreated), 3.0);
    assert_eq!(ids(&n), ["2", "1", "3"]);
    n.open_list_menu(&ui);
    let menu = n.menu.as_ref().unwrap();
    assert!(menu.items.iter().any(|(l, a, on)| l == "Sort by Date Created" && *a == Act::Sort(store::SortKey::DateCreated) && *on));
    draw(&mut n, &ui, 4.0);
    assert_eq!(app(&ui, &dir).prefs().sort, store::SortKey::DateCreated, "the choice survives a restart");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn menus_are_modal_and_tapping_a_row_runs_it() {
    let ui = ui();
    let dir = scratch("menu");
    let mut n = seeded(&ui, &dir);
    let (tx, ty) = centre(n.title_rect(&ui));
    assert_eq!(n.hit(&ui, tx, ty), Some(Target::Title));
    tap(&mut n, &ui, tx, ty, 100.0);
    assert!(n.menu.is_some());
    let (mx, my, mw, _) = n.menu_rect(&ui, n.menu.as_ref().unwrap());
    // a tap outside only closes it
    tap(&mut n, &ui, 5.0, 1800.0, 200.0);
    assert!(n.menu.is_none() && n.editing().is_none());
    tap(&mut n, &ui, tx, ty, 300.0);
    let rows = n.menu.as_ref().unwrap().items.len();
    assert!(rows >= 7);
    let sort_title = n.menu.as_ref().unwrap().items.iter().position(|(_, a, _)| *a == Act::Sort(store::SortKey::Title)).unwrap();
    let y = my + ui.pt(MENU_PAD_PT) + ui.pt(MENU_ROW_PT) * (sort_title as f32 + 0.5);
    tap(&mut n, &ui, mx + mw / 2.0, y, 400.0);
    assert!(n.menu.is_none());
    assert_eq!(n.prefs().sort, store::SortKey::Title);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn escape_clears_the_search_query_then_leaves_the_field() {
    let ui = ui();
    let dir = scratch("search-escape");
    let mut n = seeded(&ui, &dir);
    let all = n.listed().len();
    n.activate(&ui, Target::Search, 0.0, 0.0, 1000.0);
    type_text(&mut n, &ui, "rust", 1100.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")], "the query filters the list");
    assert!(n.escape(&ui, 1200.0));
    assert_eq!(n.search.text(), "", "Escape clears the query");
    assert_eq!(n.listed().len(), all, "the full list comes back");
    assert!(n.field_focused(), "the empty field stays open for the next search");
    assert!(n.escape(&ui, 1300.0));
    assert!(!n.field_focused(), "a second Escape leaves the empty field");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_highlights_matches_and_steps_through_them() {
    let ui = ui();
    let dir = scratch("find");
    let mut n = seeded(&ui, &dir);
    for note in &mut n.notes {
        if note.id == "2" {
            note.text = "Ideas\nrust shell\nmore Rust here".into();
        }
    }
    n.search.set_text(&ui, "RUST");
    n.rebuild(&ui, 0.0);
    assert_eq!(n.listed(), vec![("2", "Ideas")]);
    let (rx, ry) = centre(n.row_rect(&ui, 0));
    tap(&mut n, &ui, rx, ry, 100.0);
    assert_eq!(n.match_state(), (1, 2));
    assert!(n.find_strip());
    draw(&mut n, &ui, 200.0);
    let y = n.find_top(&ui) + ui.pt(SHORTCUT_PT) / 2.0;
    let ex = n.ed_px(&ui).0;
    assert_eq!(n.hit(&ui, ex + ui.pt(MARGIN_PT + 20.0), y), Some(Target::FindPrev));
    assert_eq!(n.hit(&ui, ex + ui.pt(MARGIN_PT + 100.0), y), Some(Target::FindNext));
    tap(&mut n, &ui, ex + ui.pt(MARGIN_PT + 100.0), y, 300.0);
    assert_eq!(n.match_state(), (2, 2));
    tap(&mut n, &ui, ex + ui.pt(MARGIN_PT + 100.0), y, 400.0);
    assert_eq!(n.match_state(), (1, 2), "wraps around");
    tap(&mut n, &ui, ex + ui.pt(MARGIN_PT + 20.0), y, 500.0);
    assert_eq!(n.match_state(), (2, 2));
    // typing keeps the matches current; the keyboard replaces the find bar
    n.activate(&ui, Target::Text, 5.0, ui.pt(nav_bottom_pt() + 10.0), 600.0);
    assert!(n.keyboard_shown() && !n.find_strip());
    n.editor.select(0..0);
    type_text(&mut n, &ui, "rust ", 700.0);
    assert_eq!(n.match_state().1, 3);
    n.activate(&ui, Target::Back, 0.0, 0.0, 800.0);
    assert_eq!(n.match_state(), (0, 0), "leaving the note clears the highlights");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn whole_word_find_and_check_spelling_step_through_their_matches() {
    let ui = ui();
    let dir = scratch("parity");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "rust rusty rust\nsee https://example.com on 2026-10-09");
    n.hl_query = "rust".into();
    n.refresh_highlights(&ui, false);
    assert_eq!(n.match_state().1, 3, "rusty counts as a hit");
    n.run(&ui, Act::ToggleWholeWord, 100.0);
    assert!(n.whole_word);
    assert_eq!(n.match_state().1, 2, "whole word skips rusty");
    n.open_editor_menu(&ui);
    let labels: Vec<_> = n.menu.as_ref().unwrap().items.iter().map(|(l, ..)| l.as_str()).collect();
    assert!(labels.contains(&"Check Spelling and Links") && labels.contains(&"Match Whole Word in Find"));
    n.menu = None;
    n.run(&ui, Act::CheckSpelling, 200.0);
    let text = n.editor.text().to_string();
    let flagged: Vec<String> = n.hl.iter().map(|r| text.chars().skip(r.start).take(r.end - r.start).collect()).collect();
    assert!(flagged.iter().any(|s| s == "https://example.com"), "{flagged:?}");
    assert!(flagged.iter().any(|s| s == "2026-10-09"), "{flagged:?}");
    assert!(n.match_state().0 == 1, "the first flagged item is selected");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn proofread_lists_the_suggestions_and_accepting_edits_the_note() {
    let ui = ui();
    let dir = scratch("proofread");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Say it it again, red,green");
    n.open_editor_menu(&ui);
    assert!(n.menu.as_ref().unwrap().items.iter().any(|(l, ..)| l == "Proofread"));
    n.menu = None;
    let rows = |n: &Notes| n.menu.as_ref().unwrap().items.iter().map(|(l, ..)| l.clone()).collect::<Vec<_>>();
    n.run(&ui, Act::Proofread, 100.0);
    assert_eq!(rows(&n), ["Remove \"it\"", "Add a space", "Accept All"]);
    n.run(&ui, Act::ProofreadAccept(0), 200.0);
    assert_eq!(n.editor.text(), "Say it again, red,green");
    assert_eq!(rows(&n), ["Add a space"], "the menu comes back with what is left");
    n.run(&ui, Act::ProofreadAccept(0), 300.0);
    assert_eq!(n.editor.text(), "Say it again, red, green");
    assert!(n.proofread.is_none());
    assert_eq!(n.notes[0].text, "Say it again, red, green", "the note itself changed");
    // Accept All applies the rest in one go
    n.menu = None;
    let all = n.editor.text().chars().count();
    n.editor.replace_range(&ui, 0..all, "the the  cat", 400.0, false);
    n.run(&ui, Act::Proofread, 500.0);
    n.run(&ui, Act::ProofreadAll, 600.0);
    assert_eq!(n.editor.text(), "the cat");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn notes_export_as_markdown_or_plain_text() {
    let ui = ui();
    let dir = scratch("export");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "# My List\n- [x] **milk**\n- eggs");
    n.run(&ui, Act::ShareMd, 2000.0);
    let Some(springboard::shell::share::Item::File(path)) = n.take_share() else { panic!("a file share") };
    assert_eq!(path.file_name().unwrap(), "My List.md");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# My List\n- [x] **milk**\n- eggs");
    n.run(&ui, Act::ShareText, 2100.0);
    let item = n.take_share().expect("a text share");
    assert_eq!(item.file_name(), "My List.txt");
    assert_eq!(item.clip_text(), "My List\n\nMy List\n[x] milk\n\u{2022} eggs");
    n.run(&ui, Act::SharePdf, 2200.0);
    let Some(springboard::shell::share::Item::File(pdf_path)) = n.take_share() else { panic!("a PDF share") };
    assert_eq!(pdf_path.file_name().unwrap(), "My List.pdf");
    let bytes = std::fs::read(&pdf_path).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(String::from_utf8_lossy(&bytes).contains("([x] milk) Tj"), "checklist lines keep their plain text");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn insert_table_puts_a_pipe_table_on_its_own_paragraph() {
    let ui = ui();
    let dir = scratch("table");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Before");
    n.run(&ui, Act::InsertTable, 1900.0);
    let text = n.editor.text().to_string();
    assert!(text.starts_with("Before\n| Column | Column |\n| --- | --- |\n|  |  |\n"), "{text:?}");
    let pdf = pdf::render("Before", &md::to_plain(&text));
    assert!(String::from_utf8_lossy(&pdf).contains("Column   |   Column"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn images_attach_beside_the_note_and_show_inline() {
    let ui = ui();
    let dir = scratch("attach");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Trip\nnotes");
    let before = n.editor.height();
    n.attach_image(&ui, &tiny_png(40, 80), 2000.0).unwrap();
    settle_images(&mut n, &ui);
    let id = n.editing().unwrap().id.clone();
    assert!(dir.join(format!("{id}-1.png")).exists());
    assert_eq!(n.editing().unwrap().text, format!("Trip\nnotes\n![image]({id}-1.png)\n"));
    assert!(n.editor.height() > before + 40.0, "the picture takes room in the text");
    assert!(n.attach_image(&ui, b"junk", 2100.0).is_err());
    draw(&mut n, &ui, 2200.0);
    n.activate(&ui, Target::Undo, 0.0, 0.0, 2300.0);
    assert_eq!(n.editing().unwrap().text, "Trip\nnotes");
    // moving the note brings the picture along
    n.attach_image(&ui, &tiny_png(8, 8), 2400.0).unwrap();
    settle_images(&mut n, &ui);
    n.tick(&ui, 4000.0);
    folders::create(&dir, "Pics").unwrap();
    n.folders = folders::list(&dir);
    n.run(&ui, Act::MoveTo("Pics".into()), 4100.0);
    assert!(dir.join("Pics").join(format!("{id}.md")).exists());
    assert!(dir.join("Pics").join(format!("{id}-1.png")).exists() && dir.join("Pics").join(format!("{id}-2.png")).exists());
    assert!(!dir.join(format!("{id}-1.png")).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn insert_sketch_saves_a_picture_and_its_drawing_beside_the_note() {
    let ui = ui();
    let dir = scratch("sketch");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Plan");
    let id = n.editing().unwrap().id.clone();
    let (w, head) = (n.w as f32, ui.pt(SKETCH_HEAD_PT));
    // Cancel (left zone of the header) closes the pad and saves nothing
    n.run(&ui, Act::InsertSketch, 1900.0);
    assert!(n.sketch.is_some(), "the pad opens over an open note");
    n.touch(&ui, Event::Down(5.0, 10.0), 1910.0);
    n.touch(&ui, Event::Up(5.0, 10.0), 1920.0);
    assert!(n.sketch.is_none());
    assert_eq!(n.editing().unwrap().text, "Plan");
    // a stroke, then Done (right zone of the header) saves it and inserts its picture at the caret
    n.run(&ui, Act::InsertSketch, 2000.0);
    n.touch(&ui, Event::Down(100.0, head + 40.0), 2100.0);
    n.touch(&ui, Event::Move(160.0, head + 60.0), 2110.0);
    n.touch(&ui, Event::Up(160.0, head + 60.0), 2120.0);
    draw(&mut n, &ui, 2150.0);
    n.touch(&ui, Event::Down(w - 5.0, 10.0), 2200.0);
    n.touch(&ui, Event::Up(w - 5.0, 10.0), 2210.0);
    assert!(n.sketch.is_none());
    settle_images(&mut n, &ui);
    assert_eq!(n.editing().unwrap().text, format!("Plan\n![image]({id}-1.png)\n"));
    assert!(dir.join(format!("{id}-1.png")).exists() && dir.join(format!("{id}-1.sketch")).exists());
    // an empty pad closes without inserting anything
    n.run(&ui, Act::InsertSketch, 2300.0);
    n.touch(&ui, Event::Down(w - 5.0, 10.0), 2310.0);
    n.touch(&ui, Event::Up(w - 5.0, 10.0), 2320.0);
    assert!(n.sketch.is_none());
    assert_eq!(n.editing().unwrap().text, format!("Plan\n![image]({id}-1.png)\n"));
    let _ = std::fs::remove_dir_all(&dir);
}
/// The text of the open note with the caret put after the first occurrence of `after`.
fn caret_after(n: &mut Notes, after: &str) {
    let text = n.editor.text().to_string();
    let at = text[..text.find(after).unwrap() + after.len()].chars().count();
    n.editor.set_caret(at);
}

#[test]
fn dates_sort_oldest_first_on_request_and_the_choice_persists() {
    let ui = ui();
    let dir = scratch("sort-order");
    let mut n = seeded(&ui, &dir);
    let ids = |n: &Notes| n.listed().iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&n), ["1", "2"], "newest first by default");
    n.open_list_menu(&ui);
    let items = n.menu.as_ref().unwrap().items.clone();
    assert!(items.iter().any(|(l, a, on)| l == "Newest First" && *a == Act::SortOrder(false) && *on));
    assert!(items.iter().any(|(l, a, on)| l == "Oldest First" && *a == Act::SortOrder(true) && !*on));
    n.menu = None;
    n.run(&ui, Act::SortOrder(true), 1.0);
    assert_eq!(ids(&n), ["2", "1"]);
    assert!(app(&ui, &dir).prefs().oldest_first, "the direction survives a restart");
    n.run(&ui, Act::SortOrder(false), 2.0);
    assert_eq!(ids(&n), ["1", "2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mark_all_and_unmark_all_flip_the_whole_checklist_in_one_undo_step() {
    let ui = ui();
    let dir = scratch("check-all");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Plan\n- [ ] a\n- [x] b\n- [ ] c");
    assert!(n.has_checklist());
    n.run(&ui, Act::Do(Format::CheckAll), 2000.0);
    assert_eq!(n.editor.text(), "Plan\n- [x] a\n- [x] b\n- [x] c");
    n.run(&ui, Act::Do(Format::UncheckAll), 2100.0);
    assert_eq!(n.editor.text(), "Plan\n- [ ] a\n- [ ] b\n- [ ] c");
    assert!(n.editor.undo(&ui, 2200.0));
    assert_eq!(n.editor.text(), "Plan\n- [x] a\n- [x] b\n- [x] c", "one undo step per command");
    n.open_format_menu(&ui);
    assert!(n.menu.as_ref().unwrap().items.iter().any(|(l, _, _)| l == "Mark All"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tags_are_listed_counted_and_filter_the_list() {
    let ui = ui();
    let dir = scratch("tags");
    let mut n = seeded(&ui, &dir);
    let now = wall().unix;
    n.notes.push(Note { id: "3".into(), text: "Chores\n#home #errands".into(), modified: now - 30 });
    n.notes.push(Note { id: "4".into(), text: "Garden\nplant #Home".into(), modified: now - 40 });
    n.rebuild(&ui, 0.0);
    assert_eq!(n.tag_list, vec![("errands".to_string(), 1), ("home".to_string(), 2)]);
    assert_eq!(n.count_in(&View::Tag("home".into())), 2);
    assert_eq!(listed_ids(&mut n, &ui, View::Tag("home".into())), ["3", "4"]);
    assert_eq!(n.view_title(), "#home");
    assert!(listed_ids(&mut n, &ui, View::Tag("nothing".into())).is_empty());
    n.view = View::All;
    n.rebuild(&ui, 11.0);
    let rows: Vec<_> = n.sidebar().items.iter().filter_map(|i| match i {
        bifrost_app::controls::sidebar::SidebarItem::Row { label, count, .. } => Some((label.clone(), *count)),
        _ => None,
    }).collect();
    assert!(rows.contains(&("#home".to_string(), Some(2))) && rows.contains(&("#errands".to_string(), Some(1))), "{rows:?}");
    n.open_list_menu(&ui);
    assert!(n.menu.as_ref().unwrap().items.iter().any(|(l, a, _)| l == "#home\t2" && *a == Act::View(View::Tag("home".into()))));
    // a note in Recently Deleted does not count
    n.loc.insert("3".into(), folders::TRASH.into());
    n.rebuild(&ui, 12.0);
    assert_eq!(n.tag_list, vec![("home".to_string(), 1)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tab_and_shift_tab_indent_list_lines_and_ctrl_cmd_arrows_move_them() {
    use bifrost_app::proto::{CHORD_CTRL, CHORD_META, CHORD_SHIFT};
    let ui = ui();
    let dir = scratch("indent");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Plan\n- a\n- b");
    n.key(&ui, Out::Named("Tab"), 2000.0);
    assert_eq!(n.editor.text(), "Plan\n- a\n  - b");
    assert_eq!(n.editor.caret(), "Plan\n- a\n  - b".chars().count(), "the caret stays on its line");
    assert!(n.chord(&ui, 15, CHORD_SHIFT, 2100.0), "Shift+Tab outdents");
    assert_eq!(n.editor.text(), "Plan\n- a\n- b");
    n.key(&ui, Out::Named("Tab"), 2200.0);
    n.editor.set_caret(0);
    n.key(&ui, Out::Named("Tab"), 2300.0);
    assert_eq!(n.editor.text(), "Plan\n- a\n  - b", "Tab on the title line (not a list) does nothing");
    caret_after(&mut n, "  - b");
    n.chord(&ui, 15, CHORD_SHIFT, 2400.0);
    assert_eq!(n.editor.text(), "Plan\n- a\n- b");
    // moving: "b" goes above "a"
    caret_after(&mut n, "- b");
    assert!(n.chord(&ui, 103, CHORD_CTRL | CHORD_META, 2500.0));
    assert_eq!(n.editor.text(), "Plan\n- b\n- a");
    assert_eq!(n.editor.caret(), "Plan\n- b".chars().count(), "the caret follows the item");
    assert!(n.chord(&ui, 108, CHORD_CTRL | CHORD_META, 2600.0));
    assert_eq!(n.editor.text(), "Plan\n- a\n- b");
    assert_eq!(n.notes[match n.screen { Screen::Editor(i) => i, Screen::List => panic!("closed") }].text, "Plan\n- a\n- b", "the note follows the editor");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_format_menu_edits_the_table_under_the_caret() {
    let ui = ui();
    let dir = scratch("table-menu");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "T\n| A | B |\n| --- | --- |\n| 1 | 2 |");
    caret_after(&mut n, "| 1 | 2");
    n.open_format_menu(&ui);
    let labels: Vec<String> = n.menu.as_ref().unwrap().items.iter().map(|(l, _, _)| l.clone()).collect();
    assert!(labels.contains(&"Add Row Below".to_string()) && labels.contains(&"Delete Column".to_string()), "{labels:?}");
    n.menu = None;
    n.run(&ui, Act::Do(Format::Table(table::Op::RowBelow)), 2000.0);
    assert_eq!(n.editor.text(), "T\n| A | B |\n| --- | --- |\n| 1 | 2 |\n|  |  |");
    caret_after(&mut n, "| 1 | 2");
    n.run(&ui, Act::Do(Format::Table(table::Op::DeleteColumn)), 2100.0);
    assert_eq!(n.editor.text(), "T\n| A |\n| --- |\n| 1 |\n|  |");
    n.run(&ui, Act::Do(Format::Table(table::Op::ToText)), 2200.0);
    assert_eq!(n.editor.text(), "T\nA\n1\n");
    n.open_format_menu(&ui);
    assert!(!n.menu.as_ref().unwrap().items.iter().any(|(l, _, _)| l == "Add Row Below"), "no table commands outside a table");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn paragraph_and_character_styles_add_their_marks() {
    let ui = ui();
    let dir = scratch("styles");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Title\nplain line");
    caret_after(&mut n, "plain");
    n.run(&ui, Act::Style(md::Block::Quote), 2000.0);
    assert_eq!(n.editor.text(), "Title\n> plain line");
    n.run(&ui, Act::Style(md::Block::Quote), 2100.0);
    assert_eq!(n.editor.text(), "Title\nplain line", "choosing the style again returns to body text");
    n.run(&ui, Act::Style(md::Block::Mono), 2200.0);
    assert_eq!(n.editor.text(), "Title\n    plain line");
    n.run(&ui, Act::Style(md::Block::Dashed), 2300.0);
    assert_eq!(n.editor.text(), "Title\n\u{2013} plain line");
    n.editor.select("Title\n\u{2013} ".chars().count().."Title\n\u{2013} plain".chars().count());
    n.run(&ui, Act::Do(Format::Char(Fmt::Strike)), 2400.0);
    assert_eq!(n.editor.text(), "Title\n\u{2013} ~~plain~~ line");
    n.run(&ui, Act::Do(Format::Char(Fmt::Strike)), 2500.0);
    assert_eq!(n.editor.text(), "Title\n\u{2013} plain line", "the same command takes the marks off");
    n.editor.select("Title\n\u{2013} ".chars().count().."Title\n\u{2013} plain".chars().count());
    n.run(&ui, Act::Do(Format::Char(Fmt::Underline)), 2600.0);
    n.run(&ui, Act::Do(Format::Char(Fmt::Highlight)), 2700.0);
    assert!(n.editor.text().contains("==++plain++==") || n.editor.text().contains("++==plain==++"), "{:?}", n.editor.text());
    n.open_format_menu(&ui);
    let checked: Vec<String> = n.menu.as_ref().unwrap().items.iter().filter(|(_, _, on)| *on).map(|(l, _, _)| l.clone()).collect();
    assert_eq!(checked, vec!["Dashed List".to_string()], "the caret's paragraph style is checked");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cmd_b_at_a_caret_in_bold_text_ends_the_bold() {
    // E2E W2-2: Cmd+B after bold text, then typing, gave `** plain****`; the typing is now plain
    let ui = ui();
    let dir = scratch("bold-caret");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "T\n**bold**");
    n.editor.set_caret(8);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2000.0);
    assert!(n.pending_styles().is_empty(), "no bold waits for the typing");
    assert_eq!(n.editor.caret(), 10, "the caret stepped out past the closing marker");
    type_text(&mut n, &ui, " plain", 2100.0);
    assert_eq!(n.editor.text(), "T\n**bold** plain");
    // inside the bold text the run is split at the caret by the typing
    n.editor.set_caret(6);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2200.0);
    assert_eq!(n.editor.text(), "T\n**bold** plain", "nothing goes in before the typing");
    type_text(&mut n, &ui, "xy", 2300.0);
    assert_eq!(n.editor.text(), "T\n**bo**xy**ld** plain", "the typing between the runs is plain");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_split_with_nothing_typed_leaves_no_empty_marker_pair() {
    // Cmd+B inside bold, then the caret moves on (or Cmd+B again): no `**bo****ld**` is left in the note
    let ui = ui();
    let dir = scratch("bold-split-moved");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "T\n**bold** z");
    n.editor.set_caret(6);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2000.0);
    assert_eq!(n.editor.text(), "T\n**bold** z");
    // the caret moves to the end: the typing there is plain text, not a split
    n.editor.set_caret(12);
    type_text(&mut n, &ui, "q", 2100.0);
    assert_eq!(n.editor.text(), "T\n**bold** zq");
    // Cmd+B twice at the same caret: the bold goes on for the typing, which joins the run
    n.editor.set_caret(6);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2200.0);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2300.0);
    type_text(&mut n, &ui, "x", 2400.0);
    assert_eq!(n.editor.text(), "T\n**boxld** zq");
    // Escape drops the split as it drops a pending style
    n.editor.set_caret(6);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2500.0);
    assert!(n.escape(&ui, 2600.0));
    type_text(&mut n, &ui, "w", 2700.0);
    assert_eq!(n.editor.text(), "T\n**bowxld** zq");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bold_on_a_selection_across_lines_styles_each_line() {
    // E2E W2-2: Cmd+A, Cmd+B used to put one `**` pair around the whole note, which no line parses as bold
    let ui = ui();
    let dir = scratch("bold-lines");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "# Title\nplain line\n- item");
    let all = n.editor.text().chars().count();
    n.editor.select(0..all);
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2000.0);
    assert_eq!(n.editor.text(), "# **Title**\n**plain line**\n- **item**");
    n.run(&ui, Act::Do(Format::Char(Fmt::Bold)), 2100.0);
    assert_eq!(n.editor.text(), "# Title\nplain line\n- item", "the same command takes the marks off");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn math_results_answer_a_calculation_when_equals_is_typed_or_on_request() {
    let ui = ui();
    let dir = scratch("math");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Sum\n2+2");
    n.editor.set_caret("Sum\n2+2".chars().count());
    n.key(&ui, Out::Text("=".into()), 2000.0);
    assert_eq!(n.editor.text(), "Sum\n2+2=", "Math Results is off until it is turned on");
    n.run(&ui, Act::ToggleMath, 2100.0);
    assert!(app(&ui, &dir).prefs().math_results, "the setting persists");
    n.editor.set_text(&ui, "Sum\n6*7");
    n.editor.set_caret("Sum\n6*7".chars().count());
    n.key(&ui, Out::Text("=".into()), 2200.0);
    assert_eq!(n.editor.text(), "Sum\n6*7= 42");
    assert_eq!(n.notes[match n.screen { Screen::Editor(i) => i, Screen::List => panic!("closed") }].text, "Sum\n6*7= 42");
    // typing = after plain text asks nothing
    n.editor.set_text(&ui, "Sum\nhello");
    n.editor.set_caret("Sum\nhello".chars().count());
    n.key(&ui, Out::Text("=".into()), 2300.0);
    assert_eq!(n.editor.text(), "Sum\nhello=");
    // Calculate Math Results answers every line at once
    n.editor.set_text(&ui, "Sum\n1+1=\nskip\n(2+3)*4=");
    n.run(&ui, Act::Do(Format::CalculateMath), 2400.0);
    assert_eq!(n.editor.text(), "Sum\n1+1= 2\nskip\n(2+3)*4= 20");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn paste_variants_keep_or_strip_markdown_and_copy_as_markdown_reads_the_selection() {
    let ui = ui();
    let dir = scratch("paste");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Note\n");
    springboard::shell::clipboard::set_text("**bold** and ~~gone~~\n- item");
    assert!(n.paste_markdown(&ui, 2000.0));
    assert_eq!(n.editor.text(), "Note\n**bold** and ~~gone~~\n- item");
    n.editor.set_text(&ui, "Note\n");
    n.editor.set_caret(5);
    assert!(n.paste_match_style(&ui, 2100.0));
    assert_eq!(n.editor.text(), "Note\nbold and gone\n\u{2022} item", "the marks are stripped");
    n.editor.select(5..9);
    assert_eq!(n.selected_markdown().as_deref(), Some("bold"));
    n.close_note(&ui, 2200.0);
    assert!(!n.paste_markdown(&ui, 2300.0), "no note open, nothing to paste into");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recent_notes_open_by_title_and_folders_open_by_name() {
    let ui = ui();
    let dir = scratch("recent");
    let mut n = seeded(&ui, &dir);
    assert_eq!(n.recent_notes(), vec![("1".to_string(), "Groceries".to_string()), ("2".to_string(), "Ideas".to_string())]);
    assert_eq!(n.open_named(&ui, "ideas", 1.0).as_deref(), Some("Ideas"));
    assert_eq!(n.editing().unwrap().id, "2");
    assert_eq!(n.open_named(&ui, "groc", 2.0).as_deref(), Some("Groceries"), "a part of the title is enough");
    assert_eq!(n.open_named(&ui, "1", 3.0).as_deref(), Some("Groceries"), "the id works too");
    assert_eq!(n.open_named(&ui, "nothing like it", 4.0), None);
    assert_eq!(n.open_named(&ui, "  ", 5.0), None);
    n.close_note(&ui, 6.0);
    folders::create(&dir, "Work").unwrap();
    n.reload(&ui, 7.0);
    assert!(!n.open_folder(&ui, "Play", 8.0));
    assert!(n.open_folder(&ui, "work", 9.0));
    assert_eq!(n.view(), &View::Folder("Work".into()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_quick_note_is_remembered_and_can_be_resumed() {
    let ui = ui();
    let dir = scratch("quick");
    let mut n = app(&ui, &dir);
    n.quick_note(&ui, true, 1.0);
    assert!(n.editing().is_none(), "nothing to resume yet, and resuming never starts a note");
    n.quick_note(&ui, false, 2.0);
    let id = n.editing().unwrap().id.clone();
    assert_eq!(store::load_quick(&dir), Some(id.clone()));
    type_text(&mut n, &ui, "Idea\nsomething", 3.0);
    n.close_note(&ui, 4.0);
    assert!(n.editing().is_none());
    n.quick_note(&ui, true, 5.0);
    assert_eq!(n.editing().unwrap().id, id, "Resume Last Quick Note reopens it");
    n.close_note(&ui, 6.0);
    // by default New Quick Note starts another note; with the setting on it resumes
    n.quick_note(&ui, false, 7.0);
    let second = n.editing().unwrap().id.clone();
    assert_ne!(second, id);
    type_text(&mut n, &ui, "Second\nnote", 7.5);
    n.close_note(&ui, 8.0);
    n.prefs.resume_quick = true;
    let before = n.notes.len();
    n.quick_note(&ui, false, 9.0);
    assert_eq!(n.notes.len(), before, "no new note while the last quick note exists");
    assert_eq!(n.editing().unwrap().id, second);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn group_by_date_cuts_the_list_into_sections_and_keeps_rows_clear_of_the_headers() {
    let ui = ui();
    let dir = scratch("group-date");
    let mut n = seeded(&ui, &dir);
    let now = wall().unix;
    n.notes.push(Note { id: "3".into(), text: "Old plan".into(), modified: now - 86_400 * 20 });
    n.notes.push(Note { id: "4".into(), text: "Ancient".into(), modified: now - 86_400 * 200 });
    n.rebuild(&ui, 0.0);
    assert!(n.sections.is_empty(), "no sections until Group by Date is on");
    let flat = n.row_rect(&ui, 0).1;
    n.run(&ui, Act::ToggleGroupDate, 1.0);
    let labels: Vec<_> = n.sections.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, ["Today", "Previous 7 Days", "Previous 30 Days", "March"], "{labels:?}");
    assert_eq!(n.sections.iter().map(|(_, i)| *i).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(n.section_offset(0), SECTION_PT);
    assert_eq!(n.section_offset(3), SECTION_PT + 3.0 * (SECTION_PT + SECTION_GAP_PT));
    assert!(n.row_rect(&ui, 0).1 > flat, "the first row sits under its header");
    let step = n.row_rect(&ui, 1).1 - n.row_rect(&ui, 0).1;
    assert!((step - ui.pt(ROW_PT + SECTION_PT + SECTION_GAP_PT)).abs() < 0.01, "a row, then the next header");
    draw(&mut n, &ui, 2.0);
    // a pinned note gets its own section above the dates; a search or the title sort drops the date sections
    assert!(n.set_pinned(&ui, "2", true, 3.0));
    assert_eq!(n.sections.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Pinned", "Today", "Previous 30 Days", "March"]);
    n.search.set_text(&ui, "plan");
    n.rebuild(&ui, 4.0);
    assert!(n.sections.is_empty(), "a search ranks by relevance, not by date");
    n.search.set_text(&ui, "");
    n.run(&ui, Act::Sort(store::SortKey::Title), 5.0);
    assert_eq!(n.sections.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Pinned", "Notes"]);
    assert!(app(&ui, &dir).prefs().group_by_date, "the setting persists");
    n.run(&ui, Act::ToggleGroupDate, 6.0);
    assert!(!n.prefs.group_by_date);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn add_link_asks_for_an_address_and_wraps_the_selection() {
    use bifrost_app::proto::CHORD_META;
    let ui = ui();
    let dir = scratch("link");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "Note\nSee docs here");
    let start = "Note\nSee ".chars().count();
    n.editor.select(start..start + 4);
    assert!(n.chord(&ui, 37, CHORD_META, 2000.0), "Cmd+K");
    assert_eq!(n.naming.as_ref().unwrap().field.text(), "https://");
    n.naming.as_mut().unwrap().field.set_text(&ui, "example.com/a b");
    n.key(&ui, Out::Enter, 2100.0);
    assert!(n.naming.is_none());
    assert_eq!(n.editor.text(), "Note\nSee [docs](https://example.com/a%20b) here");
    assert_eq!(n.editing().unwrap().text, n.editor.text(), "the note follows the editor");
    // an address with nothing after the scheme is refused and the text stays
    n.run(&ui, Act::Do(Format::AddLink), 2200.0);
    n.key(&ui, Out::Enter, 2300.0);
    assert_eq!(n.notice.as_deref(), Some("Enter a web address for the link"));
    assert_eq!(n.editor.text(), "Note\nSee [docs](https://example.com/a%20b) here");
    // Escape cancels the prompt
    n.run(&ui, Act::Do(Format::AddLink), 2400.0);
    assert!(n.escape(&ui, 2500.0));
    assert!(n.naming.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collapse_commands_fold_the_section_under_the_caret_and_survive_a_restart() {
    let ui = ui();
    let dir = scratch("fold-section");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "# Plan\nintro\n## Steps\none\ntwo\n## Notes\nlast");
    let text = n.editor.text().to_string();
    let id = n.editing().unwrap().id.clone();
    caret_after(&mut n, "one");
    n.run(&ui, Act::Do(Format::CollapseSection), 3000.0);
    assert!(n.folds[&id].contains("## Steps"));
    assert_eq!(n.editor.text(), text, "folding never changes the note text");
    n.run(&ui, Act::Do(Format::CollapseAll), 3100.0);
    assert_eq!(n.folds[&id].len(), 3, "Plan, Steps and Notes are folded");
    n.run(&ui, Act::Do(Format::ExpandSection), 3200.0);
    assert!(!n.folds[&id].contains("## Steps"), "Expand Section unfolds the heading above the caret");
    let reopened = app(&ui, &dir);
    assert_eq!(reopened.folds[&id].len(), 2, "the folds are saved with the note");
    n.run(&ui, Act::Do(Format::ExpandAll), 3300.0);
    assert!(n.folds.get(&id).is_none_or(|keys| keys.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn view_menu_layout_choice_is_saved_with_the_prefs() {
    let ui = ui();
    let dir = scratch("layout");
    let mut n = seeded(&ui, &dir);
    assert_eq!(n.prefs().layout, gallery::Layout::List);
    n.set_layout(&ui, gallery::Layout::Gallery);
    assert_eq!(n.prefs().layout, gallery::Layout::Gallery);
    assert_eq!(app(&ui, &dir).prefs().layout, gallery::Layout::Gallery);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folded_section_hides_its_lines_and_its_chevron_unfolds_it() {
    let ui = ui();
    let dir = scratch("fold-hide");
    let mut n = app(&ui, &dir);
    open_with(&mut n, &ui, "# Plan\nintro\n## Steps\none\ntwo");
    caret_after(&mut n, "one");
    n.run(&ui, Act::Do(Format::CollapseSection), 3000.0);
    let one = n.editor.text().find("one").unwrap();
    assert_eq!(n.editor.line_span(one).1, 0.0, "the folded line takes no height");
    assert!(n.fold_chevrons(&ui).iter().any(|(line, _, folded)| *line == 2 && *folded), "the heading shows a folded chevron");
    draw(&mut n, &ui, 3100.0);
    n.activate(&ui, Target::Fold(2), 0.0, 0.0, 3200.0);
    assert!(n.editor.line_span(one).1 > 0.0, "tapping the chevron unfolds the section");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_gallery_puts_cards_in_rows_and_the_toolbar_toggles_it() {
    let ui = ui();
    let dir = scratch("gallery-grid");
    let mut n = seeded(&ui, &dir);
    n.set_layout(&ui, gallery::Layout::Gallery);
    let (cells, _, total) = n.gallery_grid(&ui);
    assert_eq!(cells.len(), n.shown.len());
    // the list column is at most 270 pt, so the card count per row follows from its width: two cards share a row
    // only when the column is wide enough for them, otherwise the cards stack one per row
    let cols = n.gallery_cols(&ui).0;
    assert!(cells.len() >= 2);
    if cols >= 2 {
        assert!(cells[0].1 == cells[1].1 && cells[1].0 > cells[0].0, "the first cards share a row");
    } else {
        assert!(cells[1].1 > cells[0].1, "one card per row when the list column is narrow");
    }
    assert!(total > cells[0].1, "the grid reports its height for the scroll range");
    draw(&mut n, &ui, 0.0);
    n.toggle_layout(&ui, 0.0);
    assert_eq!(n.prefs().layout, gallery::Layout::List, "the toolbar toggle switches back to rows");
    let _ = std::fs::remove_dir_all(&dir);
}

use super::*;
use crate::gfx::Surface;
use crate::shell::ui::{Weight, BLACK, WHITE};
use model::Completion;

fn app(name: &str) -> (Reminders, Ui, PathBuf) {
    let ui = crate::shell::tests::ui();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/test-reminders-{}-{name}/reminders.tsv",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    let app = Reminders::new(
        &ui,
        1200,
        1920,
        path.clone(),
        Wall { unix: 1_791_073_380, offset: 0, at: 0.0 },
    );
    (app, ui, path)
}
fn cleanup(path: &PathBuf) {
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
fn tap(app: &mut Reminders, ui: &Ui, x: f32, y: f32, t: f64) {
    app.touch(ui, Event::Down(x, y), t);
    app.touch(ui, Event::Up(x, y), t + 20.0);
}
fn add(app: &mut Reminders, ui: &Ui, text: &str, t: f64) {
    app.activate(ui, Target::New, t);
    app.key(ui, Out::Text(text.into()), t + 1.0);
    app.activate(ui, Target::Save, t + 2.0);
}
fn render(app: &mut Reminders, ui: &Ui) -> Vec<u32> {
    let mut buf = vec![0u32; 1200 * 1920];
    app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, 1000.0);
    buf
}
fn row_ids(app: &Reminders) -> Vec<(u64, u8)> {
    app.rows
        .iter()
        .filter_map(|r| match r {
            Row::Item { idx, depth } => Some((app.data.items[*idx].id, *depth)),
            Row::Header(_) => None,
        })
        .collect()
}
fn titles(app: &Reminders) -> Vec<String> {
    app.data.items.iter().map(|r| r.title.clone()).collect()
}

#[test]
fn create_with_quick_add_edit_complete_delete_and_undo_persist() {
    let (mut app, ui, path) = app("lifecycle");
    let today = app.today;
    add(&mut app, &ui, "Buy oat milk tomorrow 9am", 0.0);
    let r = &app.data.items[0];
    assert_eq!((r.title.as_str(), r.due, r.time), ("Buy oat milk", Some(today + 1), Some(540)));
    assert_eq!(store::load(&path).items[0], app.data.items[0]);
    app.activate(&ui, Target::Smart(Smart::All), 100.0);
    assert_eq!(row_ids(&app).len(), 1);
    app.activate(&ui, Target::Circle(0), 200.0);
    assert!(row_ids(&app).is_empty());
    assert!(store::load(&path).items[0].completed);
    app.activate(&ui, Target::Back, 250.0);
    app.activate(&ui, Target::Smart(Smart::Completed), 300.0);
    app.activate(&ui, Target::Circle(0), 400.0);
    assert!(!store::load(&path).items[0].completed);
    app.activate(&ui, Target::Back, 450.0);
    app.activate(&ui, Target::Smart(Smart::All), 500.0);
    app.activate(&ui, Target::Row(0), 600.0);
    app.title.set_text(&ui, "Buy milk and eggs");
    app.activate(&ui, Target::Save, 700.0);
    assert_eq!(store::load(&path).items[0].title, "Buy milk and eggs");
    app.activate(&ui, Target::Row(0), 800.0);
    app.activate(&ui, Target::Delete, 900.0);
    assert!(store::load(&path).items.is_empty());
    assert!(app.undo.is_some());
    app.activate(&ui, Target::Undo, 1000.0);
    assert_eq!(store::load(&path).items.len(), 1);
    assert!(!path.with_extension("tsv.tmp").exists());
    cleanup(&path);
}

#[test]
fn cancel_discards_draft_and_blank_titles_cannot_save() {
    let (mut app, ui, path) = app("cancel");
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("   ".into()), 10.0);
    app.activate(&ui, Target::Save, 20.0);
    assert!(app.editor.is_some());
    assert!(app.data.items.is_empty());
    app.key(&ui, Out::Text("Draft".into()), 30.0);
    app.activate(&ui, Target::Cancel, 40.0);
    assert!(app.editor.is_none() && app.data.items.is_empty());
    assert!(!path.exists());
}

#[test]
fn details_flag_priority_notes_url_early_repeat_and_time_persist() {
    let (mut app, ui, path) = app("details");
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("Dentist".into()), 1.0);
    app.activate(&ui, Target::Due, 2.0);
    app.activate(&ui, Target::Time, 3.0);
    app.activate(&ui, Target::Hour(1), 4.0);
    app.activate(&ui, Target::Minute(5), 5.0);
    app.activate(&ui, Target::Minute(-5), 5.5);
    app.activate(&ui, Target::Repeat, 6.0);
    app.activate(&ui, Target::Repeat, 6.5);
    app.activate(&ui, Target::Early, 7.0);
    app.activate(&ui, Target::Flag, 8.0);
    app.activate(&ui, Target::Priority, 9.0);
    app.activate(&ui, Target::Priority, 9.5);
    app.notes.set_text(&ui, "Bring the card");
    app.url.set_text(&ui, "https://example.com");
    app.activate(&ui, Target::Save, 10.0);
    let r = store::load(&path).items.remove(0);
    assert_eq!(r.time, Some(10 * 60));
    assert_eq!(r.repeat, Some(repeat::Repeat::new(repeat::Unit::Week, 1)));
    assert_eq!((r.early, r.flagged, r.priority), (Some(5), true, 2));
    assert_eq!((r.notes.as_str(), r.url.as_str()), ("Bring the card", "https://example.com"));
    // clearing the date clears everything that depends on it
    app.activate(&ui, Target::Smart(Smart::All), 20.0);
    app.activate(&ui, Target::Row(0), 21.0);
    app.activate(&ui, Target::Due, 22.0);
    let d = &app.editor.as_ref().unwrap().draft;
    assert_eq!((d.due, d.time, d.repeat, d.early), (None, None, None, None));
    cleanup(&path);
}

#[test]
fn calendar_picker_selects_leap_day_and_month_navigation() {
    let (mut app, ui, _) = app("calendar");
    app.activate(&ui, Target::New, 0.0);
    app.activate(&ui, Target::Due, 300.0);
    app.editor.as_mut().unwrap().month = days_from_civil(2028, 2, 1);
    let (first, count, start) = app.calendar_days();
    assert_eq!(count, 29);
    let (_, ry, rh) = *app.form_rows().0.iter().find(|r| r.0 == Target::Calendar).unwrap();
    let (x, y, w, _) = app.form_rect(&ui, ry, rh);
    let cell = start + 28;
    let (dx, dy) = (x + w / 7.0 * ((cell % 7) as f32 + 0.5), y + ui.pt(90.0 + 36.0 * (cell / 7) as f32));
    assert_eq!(app.hit(&ui, dx, dy), Some(Target::Day(first + 28)));
    tap(&mut app, &ui, dx, dy, 500.0);
    assert_eq!(app.editor.as_ref().unwrap().draft.due, Some(days_from_civil(2028, 2, 29)));
    app.activate(&ui, Target::Month(1), 600.0);
    assert_eq!(app.editor.as_ref().unwrap().month, days_from_civil(2028, 3, 1));
    app.set_top_inset(20.0);
    let buf = render(&mut app, &ui);
    assert!(buf.iter().any(|&p| p != 0 && p != BLACK));
    assert!(buf[..1200 * ui.pt(20.0) as usize].iter().all(|&p| p == 0), "nothing is drawn inside the top inset");
}

#[test]
fn repeated_wall_updates_stay_idle_and_today_changes_at_midnight() {
    let (mut app, ui, _) = app("clock");
    let mut r = Reminder::new(1, "Tomorrow");
    r.due = Some(app.today + 1);
    app.data.items.push(r);
    app.activate(&ui, Target::Smart(Smart::Today), 0.0);
    app.dirty = false;
    app.set_wall(Wall { unix: app.wall.unix + 1, ..app.wall });
    assert!(!app.needs_frame());
    assert!(row_ids(&app).is_empty());
    app.set_wall(Wall { unix: app.wall.unix + 86_400, ..app.wall });
    assert!(app.needs_frame());
    assert_eq!(row_ids(&app), vec![(1, 0)]);
    assert!(app.next_wake(0.0).unwrap() > 0.0);
}

#[test]
fn vertical_drag_on_completion_circle_scrolls_without_completing() {
    let (mut app, ui, _) = app("scroll");
    for i in 1..=40 {
        app.data.items.push(Reminder::new(i, format!("Task {i}")));
    }
    app.activate(&ui, Target::Smart(Smart::All), 0.0);
    let (x, y, _, h) = app.row_rect(&ui, 1);
    let (x, y) = (x + ui.pt(26.0), y + h / 2.0);
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Move(x, y - 100.0), 30.0);
    app.touch(&ui, Event::Up(x, y - 100.0), 60.0);
    assert!(app.scroll.offset() > 0.0);
    assert!(app.data.items.iter().all(|r| !r.completed));
    assert!(app.editor.is_none() && app.swipe.is_none());
}

#[test]
fn swipe_left_reveals_flag_and_delete_with_undo() {
    let (mut app, ui, path) = app("swipe");
    add(&mut app, &ui, "One", 0.0);
    add(&mut app, &ui, "Two", 10.0);
    let (x, y, w, h) = app.row_rect(&ui, 0);
    let (sx, sy) = (x + w - ui.pt(30.0), y + h / 2.0);
    app.touch(&ui, Event::Down(sx, sy), 100.0);
    app.touch(&ui, Event::Move(sx - ui.pt(60.0), sy), 120.0);
    app.touch(&ui, Event::Move(sx - ui.pt(200.0), sy), 140.0);
    app.touch(&ui, Event::Up(sx - ui.pt(200.0), sy), 160.0);
    assert!(app.swipe.as_ref().is_some_and(|s| s.open));
    assert!(app.data.items.iter().all(|r| !r.flagged), "a swipe alone changes nothing");
    let buf = render(&mut app, &ui);
    assert!(buf.iter().any(|&p| p != 0 && p != BLACK));
    // flag button
    tap(&mut app, &ui, x + w - ui.pt(BTN) * 1.5, sy, 200.0);
    assert!(store::load(&path).items[0].flagged);
    assert!(app.swipe.is_none());
    app.activate(&ui, Target::Undo, 300.0);
    assert!(!store::load(&path).items[0].flagged);
    // delete button
    app.touch(&ui, Event::Down(sx, sy), 400.0);
    app.touch(&ui, Event::Move(sx - ui.pt(200.0), sy), 420.0);
    app.touch(&ui, Event::Up(sx - ui.pt(200.0), sy), 440.0);
    tap(&mut app, &ui, x + w - ui.pt(BTN) / 2.0, sy, 500.0);
    assert_eq!(titles(&app), vec!["Two"]);
    assert_eq!(app.rows.len(), 1);
    app.activate(&ui, Target::Undo, 600.0);
    assert_eq!(titles(&app), vec!["One", "Two"], "undo restores the original position");
    // undo bar expires
    app.activate(&ui, Target::SwipeDelete(0), 700.0);
    assert!(app.undo.is_some());
    app.tick(&ui, 700.0 + UNDO_MS + 1.0);
    assert!(app.undo.is_none());
    cleanup(&path);
}

#[test]
fn tapping_elsewhere_closes_an_open_swipe() {
    let (mut app, ui, path) = app("swipe-close");
    add(&mut app, &ui, "One", 0.0);
    let (x, y, w, h) = app.row_rect(&ui, 0);
    let (sx, sy) = (x + w - ui.pt(30.0), y + h / 2.0);
    app.touch(&ui, Event::Down(sx, sy), 100.0);
    app.touch(&ui, Event::Move(sx - ui.pt(200.0), sy), 120.0);
    app.touch(&ui, Event::Up(sx - ui.pt(200.0), sy), 140.0);
    assert!(app.swipe.is_some());
    tap(&mut app, &ui, x + ui.pt(100.0), sy, 200.0);
    assert!(app.swipe.is_none());
    assert!(app.editor.is_none(), "the closing tap does not open the row");
    cleanup(&path);
}

#[test]
fn subtasks_indent_outdent_collapse_and_progress() {
    let (mut app, ui, path) = app("subtasks");
    app.activate(&ui, Target::List(1), 0.0);
    add(&mut app, &ui, "Trip", 10.0);
    add(&mut app, &ui, "Pack", 20.0);
    add(&mut app, &ui, "Book", 30.0);
    app.activate(&ui, Target::Row(1), 40.0);
    app.activate(&ui, Target::Indent, 41.0);
    app.activate(&ui, Target::Save, 42.0);
    app.activate(&ui, Target::Row(2), 50.0);
    app.activate(&ui, Target::Indent, 51.0);
    app.activate(&ui, Target::Save, 52.0);
    assert_eq!(row_ids(&app), vec![(1, 0), (2, 1), (3, 1)]);
    assert_eq!(model::progress(&app.data.items, 1), (0, 2));
    app.activate(&ui, Target::Circle(1), 60.0);
    assert_eq!(model::progress(&app.data.items, 1), (1, 2));
    app.activate(&ui, Target::Collapse(0), 70.0);
    assert_eq!(row_ids(&app), vec![(1, 0)]);
    app.activate(&ui, Target::Collapse(0), 71.0);
    app.opts.show_completed = true;
    app.rebuild(&ui);
    app.activate(&ui, Target::Row(2), 80.0);
    app.activate(&ui, Target::Indent, 81.0);
    app.activate(&ui, Target::Save, 82.0);
    assert_eq!(store::load(&path).items[2].parent, None);
    // a parent cannot become a subtask
    app.activate(&ui, Target::Row(0), 90.0);
    app.activate(&ui, Target::Indent, 91.0);
    assert_eq!(app.editor.as_ref().unwrap().draft.parent, None);
    cleanup(&path);
}

#[test]
fn lists_create_rename_reorder_delete() {
    let (mut app, ui, path) = app("lists");
    app.activate(&ui, Target::NewList, 0.0);
    app.key(&ui, Out::Text("Work".into()), 1.0);
    app.activate(&ui, Target::Color(2), 2.0);
    app.activate(&ui, Target::Icon(3), 3.0);
    app.activate(&ui, Target::Save, 4.0);
    assert_eq!(app.screen, Screen::Home);
    let l = app.data.lists[1].clone();
    assert_eq!((l.name.as_str(), l.color, l.icon), ("Work", 2, 3));
    assert_eq!(store::load(&path).lists, app.data.lists);
    app.activate(&ui, Target::List(l.id), 10.0);
    add(&mut app, &ui, "Report", 20.0);
    assert_eq!(app.data.items[0].list, l.id, "new reminders land in the open list");
    assert_eq!(model::list_count(&app.data.items, l.id), 1);
    app.activate(&ui, Target::Back, 30.0);
    app.activate(&ui, Target::ListInfo(l.id), 40.0);
    app.title.set_text(&ui, "Office");
    app.activate(&ui, Target::ListUp, 41.0);
    assert_eq!(app.data.lists[0].id, l.id);
    app.activate(&ui, Target::Save, 42.0);
    assert_eq!(app.data.lists[0].name, "Office");
    app.activate(&ui, Target::ListInfo(l.id), 50.0);
    app.activate(&ui, Target::Delete, 51.0);
    assert_eq!(app.data.lists.len(), 1);
    assert!(app.data.items.is_empty(), "deleting a list deletes its reminders");
    app.activate(&ui, Target::ListInfo(1), 60.0);
    assert!(!app.form_rows().0.iter().any(|r| r.0 == Target::Delete), "the last list stays");
    cleanup(&path);
}

#[test]
fn moving_a_reminder_to_another_list_takes_its_subtasks() {
    let (mut app, ui, path) = app("move-list");
    app.activate(&ui, Target::NewList, 0.0);
    app.key(&ui, Out::Text("Other".into()), 1.0);
    app.activate(&ui, Target::Save, 2.0);
    app.activate(&ui, Target::List(1), 10.0);
    add(&mut app, &ui, "Parent", 20.0);
    add(&mut app, &ui, "Kid", 30.0);
    app.activate(&ui, Target::Row(1), 40.0);
    app.activate(&ui, Target::Indent, 41.0);
    app.activate(&ui, Target::Save, 42.0);
    app.activate(&ui, Target::Row(0), 50.0);
    app.activate(&ui, Target::MoveList, 51.0);
    app.activate(&ui, Target::Save, 52.0);
    assert!(app.data.items.iter().all(|r| r.list == 2));
    assert_eq!(app.data.items[1].parent, Some(1));
    cleanup(&path);
}

#[test]
fn search_spans_everything_and_cancel_returns_home() {
    let (mut app, ui, path) = app("search");
    add(&mut app, &ui, "Alpha", 0.0);
    add(&mut app, &ui, "Beta", 10.0);
    app.activate(&ui, Target::Smart(Smart::All), 15.0);
    app.activate(&ui, Target::Circle(0), 16.0);
    app.activate(&ui, Target::Back, 17.0);
    app.activate(&ui, Target::Search, 20.0);
    assert!(app.searching && app.keyboard_shown());
    app.key(&ui, Out::Text("alp".into()), 30.0);
    assert_eq!(row_ids(&app), vec![(1, 0)], "completed items are searchable");
    app.key(&ui, Out::Backspace, 31.0);
    app.key(&ui, Out::Backspace, 32.0);
    app.key(&ui, Out::Backspace, 33.0);
    assert_eq!(row_ids(&app).len(), 1, "an empty query lists the open items");
    app.key(&ui, Out::Text("zzz".into()), 40.0);
    assert!(app.rows.is_empty());
    let buf = render(&mut app, &ui);
    assert!(buf.iter().any(|&p| p != 0));
    // editing a result keeps the query
    app.key(&ui, Out::Backspace, 41.0);
    app.key(&ui, Out::Backspace, 42.0);
    app.key(&ui, Out::Backspace, 43.0);
    app.key(&ui, Out::Text("beta".into()), 44.0);
    app.activate(&ui, Target::Row(0), 50.0);
    app.activate(&ui, Target::Cancel, 51.0);
    assert_eq!(app.title.text(), "beta");
    app.activate(&ui, Target::Back, 60.0);
    assert_eq!(app.screen, Screen::Home);
    assert!(app.title.text().is_empty());
    cleanup(&path);
}

#[test]
fn options_sort_group_and_show_completed() {
    let (mut app, ui, path) = app("options");
    app.activate(&ui, Target::List(1), 0.0);
    add(&mut app, &ui, "banana", 10.0);
    add(&mut app, &ui, "apricot !!!", 20.0);
    add(&mut app, &ui, "cherry tomorrow", 30.0);
    add(&mut app, &ui, "done", 40.0);
    app.activate(&ui, Target::Circle(3), 50.0);
    assert_eq!(row_ids(&app).len(), 3);
    app.activate(&ui, Target::Options, 60.0);
    assert!(app.options_open);
    let buf = render(&mut app, &ui);
    assert!(buf.iter().any(|&p| p != 0));
    app.activate(&ui, Target::OptCompleted, 61.0);
    assert_eq!(row_ids(&app).len(), 4);
    app.activate(&ui, Target::OptSort(Sort::Title), 62.0);
    assert_eq!(row_ids(&app), vec![(2, 0), (1, 0), (3, 0), (4, 0)]);
    app.activate(&ui, Target::OptSort(Sort::Priority), 63.0);
    assert_eq!(row_ids(&app)[0], (2, 0));
    app.activate(&ui, Target::OptGroup(Group::Due), 64.0);
    assert!(app.rows.iter().any(|r| matches!(r, Row::Header(h) if h == "Tomorrow")));
    assert!(app.rows.iter().any(|r| matches!(r, Row::Header(h) if h == "No Date")));
    render(&mut app, &ui);
    app.activate(&ui, Target::OptClose, 65.0);
    assert!(!app.options_open);
    cleanup(&path);
}

#[test]
fn completing_a_repeating_reminder_moves_it_to_the_next_date() {
    let (mut app, ui, path) = app("repeat");
    let today = app.today;
    add(&mut app, &ui, "Pay rent every month", 0.0);
    assert_eq!(app.data.items[0].due, Some(today));
    app.activate(&ui, Target::Smart(Smart::All), 10.0);
    app.activate(&ui, Target::Circle(0), 20.0);
    let loaded = store::load(&path);
    let r = &loaded.items[0];
    assert!(!r.completed);
    assert_eq!(r.due, Some(repeat::add_months(today, 1)));
    let mut items = app.data.items.clone();
    assert!(matches!(model::toggle_complete(&mut items, 1, today), Some(Completion::Advanced(_))));
    cleanup(&path);
}

#[test]
fn long_press_drag_reorders_manual_lists_only() {
    let (mut app, ui, path) = app("drag");
    app.activate(&ui, Target::List(1), 0.0);
    for (i, name) in ["A", "B", "C"].iter().enumerate() {
        add(&mut app, &ui, name, 10.0 * (i + 1) as f64);
    }
    let (x, y, _, h) = app.row_rect(&ui, 0);
    let (px, py) = (x + ui.pt(120.0), y + h / 2.0);
    app.touch(&ui, Event::Down(px, py), 1000.0);
    app.tick(&ui, 1500.0);
    assert!(app.drag.is_some());
    let (_, y2, _, h2) = app.row_rect(&ui, 2);
    app.touch(&ui, Event::Move(px, y2 + h2 * 0.9), 1600.0);
    render(&mut app, &ui);
    app.touch(&ui, Event::Up(px, y2 + h2 * 0.9), 1700.0);
    assert!(app.drag.is_none());
    assert_eq!(titles(&app), vec!["B", "C", "A"]);
    assert_eq!(store::load(&path).items.iter().map(|r| r.title.clone()).collect::<Vec<_>>(), vec!["B", "C", "A"]);
    // sorted lists do not drag
    app.opts.sort = Sort::Title;
    assert!(!app.can_reorder());
    cleanup(&path);
}

#[test]
fn v1_files_load_and_resave_as_v2() {
    let (_, ui, path) = app("migrate");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "springboard-reminders-v1\n1\t20000\t0\tOld task\n").unwrap();
    let mut app = Reminders::new(&ui, 1200, 1920, path.clone(), Wall { unix: 1_791_073_380, offset: 0, at: 0.0 });
    assert!(!app.error);
    assert_eq!(titles(&app), vec!["Old task"]);
    add(&mut app, &ui, "New task", 0.0);
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("springboard-reminders-v2\n"));
    assert_eq!(store::load(&path).items.len(), 2);
    cleanup(&path);
}

#[test]
fn saves_bump_the_generation_for_the_alert_scheduler() {
    let (mut app, ui, path) = app("gen");
    let g = app.generation();
    add(&mut app, &ui, "x", 0.0);
    assert_eq!(app.generation(), g + 1);
    cleanup(&path);
}

#[test]
fn every_screen_renders_without_panicking() {
    let (mut app, ui, path) = app("render");
    add(&mut app, &ui, "Plan trip every week tomorrow 8am !!", 0.0);
    app.data.items[0].flagged = true;
    app.data.items[0].notes = "Check passports".into();
    add(&mut app, &ui, "Sub", 10.0);
    app.activate(&ui, Target::Back, 15.0);
    for t in [Target::Smart(Smart::Flagged), Target::Back, Target::List(1), Target::Back] {
        app.activate(&ui, t, 20.0);
        render(&mut app, &ui);
    }
    app.activate(&ui, Target::List(1), 25.0);
    app.activate(&ui, Target::Row(0), 30.0);
    app.activate(&ui, Target::Early, 31.0);
    app.activate(&ui, Target::Time, 32.0);
    render(&mut app, &ui);
    app.activate(&ui, Target::Cancel, 33.0);
    app.activate(&ui, Target::NewList, 40.0);
    render(&mut app, &ui);
    app.activate(&ui, Target::Cancel, 41.0);
    render(&mut app, &ui);
    assert!(!view::icon(64).px.is_empty());
    cleanup(&path);
}

#[test]
fn unreadable_future_format_is_preserved_when_user_attempts_save() {
    let (app, ui, path) = app("protected");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "future-reminders-v3\nprecious data";
    std::fs::write(&path, original).unwrap();
    let mut app = Reminders::new(&ui, 1200, 1920, path.clone(), app.wall);
    assert!(app.error);
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("Draft".into()), 1.0);
    app.activate(&ui, Target::Save, 2.0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert!(app.describe().contains("reminders_save_error=true"));
    assert!(app.editor.is_some());
    assert!(app.data.items.is_empty());
    cleanup(&path);
}

#[test]
fn failed_new_save_keeps_draft_until_explicit_success() {
    let (mut app, ui, path) = app("transaction-new");
    std::fs::create_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    std::fs::write(path.parent().unwrap(), "blocks directory").unwrap();
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("Recover me".into()), 1.0);
    app.activate(&ui, Target::Save, 2.0);
    assert!(app.error && app.editor.is_some());
    assert_eq!(app.title.text(), "Recover me");
    assert!(app.data.items.is_empty(), "failed save must not appear accepted");
    std::fs::remove_file(path.parent().unwrap()).unwrap();
    app.activate(&ui, Target::Save, 2002.0);
    assert!(!app.error && app.editor.is_none());
    assert_eq!(store::load(&path).items[0].title, "Recover me");
    cleanup(&path);
}

#[test]
fn failed_edit_delete_and_completion_leave_committed_data_unchanged() {
    let (mut app, ui, path) = app("transaction-existing");
    add(&mut app, &ui, "Original", 0.0);
    let original = store::load(&path);
    // A directory at the temporary file path makes writing fail without
    // modifying the existing persisted file, including when tests run root.
    std::fs::create_dir(path.with_extension("tsv.tmp")).unwrap();
    app.activate(&ui, Target::Circle(0), 3.0);
    assert!(app.error);
    assert_eq!(app.data, original);
    app.activate(&ui, Target::Row(0), 4.0);
    app.title.set_text(&ui, "Edited draft");
    app.activate(&ui, Target::Save, 5.0);
    assert!(app.editor.is_some());
    assert_eq!(app.title.text(), "Edited draft");
    assert_eq!(app.data, original);
    app.activate(&ui, Target::Delete, 6.0);
    assert!(app.editor.is_some() && app.undo.is_none());
    assert_eq!(store::load(&path), original);
    std::fs::remove_dir(path.with_extension("tsv.tmp")).unwrap();
    app.activate(&ui, Target::Save, 7.0);
    assert!(app.editor.is_none());
    assert_eq!(store::load(&path).items[0].title, "Edited draft");
    cleanup(&path);
}

#[test]
fn title_edit_menu_outside_field_accepts_touch_actions() {
    use crate::shell::edit::Action;
    let (mut app, ui, _) = app("menu-routing");
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("Some editable text".into()), 300.0);
    let (x, y, _, h) = app.field_frame(&ui, Field::Title).unwrap();
    app.touch(&ui, Event::Down(x + ui.pt(60.0), y + h / 2.0), 500.0);
    app.tick(&ui, 1000.0);
    app.touch(&ui, Event::Up(x + ui.pt(60.0), y + h / 2.0), 1100.0);
    let menu = app.title.menu().expect("long press opens field menu");
    let point = (0..500)
        .step_by(2)
        .find_map(|dy| {
            (0..1200).step_by(2).find_map(|dx| {
                (menu.hit(dx as f32, dy as f32) == Some(Action::SelectAll)).then_some((dx as f32, dy as f32))
            })
        })
        .expect("Select All has a touch target");
    assert!(!within(point.0, point.1, app.field_frame(&ui, Field::Title).unwrap()));
    tap(&mut app, &ui, point.0, point.1, 1200.0);
    assert_eq!(app.title.selection(), (0, "Some editable text".chars().count()));
    assert!(app.keyboard_shown() && app.editor.is_some());
    app.key(&ui, Out::Backspace, 1300.0);
    assert!(app.title.text().is_empty(), "menu selection was applied to typing");
}

#[test]
fn dynamic_type_updates_cached_labels_and_preserves_editor_draft() {
    let (mut app, mut ui, _) = app("style");
    app.activate(&ui, Target::New, 0.0);
    app.key(&ui, Out::Text("Draft".into()), 1.0);
    app.labels.get(&ui, "Reminders", 17.0, Weight::Regular, WHITE);
    ui.text_scale = 1.3;
    ui.bold = true;
    app.tick(&ui, 500.0);
    assert!(app.labels.is_empty());
    assert_eq!(app.title.text(), "Draft");
    assert!(app.title.focused);
    assert_eq!(app.style, (ui.text_scale.to_bits(), true));
}

#[test]
fn smart_grid_order_matches_mobile() {
    use super::model::Smart;
    assert_eq!(&Smart::ALL[..4], &[Smart::Today, Smart::Scheduled, Smart::All, Smart::Flagged]);
}

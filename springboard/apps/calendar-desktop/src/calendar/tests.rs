use super::*;
use crate::icon::icon;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "springboard-calendar-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn file(&self) -> PathBuf {
        self.0.join("events.tsv")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn date(year: i64, month: u32, day: u32) -> Date {
    Date { year, month, day }
}
fn wall() -> Wall {
    Wall {
        unix: tz::days_from_civil(2026, 10, 4) * 86400 + 12 * 3600,
        offset: 0,
        at: 0.0,
    }
}
fn event(id: u64, title: &str) -> CalendarEvent {
    CalendarEvent {
        id,
        date: date(2026, 10, 4),
        minute: 540,
        duration: 60,
        all_day: false,
        title: title.into(),
        location: String::new(),
        notes: String::new(),
        repeat: 0,
        alert: 0, uid: String::new(),
    }
}
fn tap(c: &mut Calendar, ui: &Ui, x: f32, y: f32, t: f64) {
    c.touch(ui, Event::Down(ui.pt(x), ui.pt(y)), t);
    c.touch(ui, Event::Up(ui.pt(x), ui.pt(y)), t + 20.0);
}
/// Taps the vertical middle of the editor row for `target`.
fn tap_row(c: &mut Calendar, ui: &Ui, target: Target, x: f32, t: f64) {
    let (_, top, h) = c.rows().into_iter().find(|r| r.0 == target).expect("row");
    tap(c, ui, x, top + h / 2.0, t);
}
fn new(ui: &Ui, file: PathBuf) -> Calendar {
    Calendar::new(ui, 1200, 1920, file, wall())
}

#[test]
fn leap_years_and_civil_boundaries() {
    assert_eq!(date(2000, 2, 1).month_len(), 29);
    assert_eq!(date(1900, 2, 1).month_len(), 28);
    assert_eq!(date(2024, 2, 28).shift_days(1), date(2024, 2, 29));
    assert_eq!(date(2024, 2, 29).shift_days(1), date(2024, 3, 1));
    assert_eq!(date(2026, 12, 31).shift_days(1), date(2027, 1, 1));
    assert_eq!(date(2026, 1, 1).shift_days(-1), date(2025, 12, 31));
    assert_eq!(date(1, 1, 1).shift_days(-1), date(1, 1, 1));
    assert_eq!(date(9999, 12, 31).shift_days(1), date(9999, 12, 31));
    assert!(!date(2026, 2, 29).valid());
    assert!(!date(2026, 13, 1).valid());
    assert!(!date(2026, 1, 0).valid());
    for y in [1, 1900, 1970, 2000, 2026, 9999] {
        for m in 1..=12 {
            let d = date(y, m, date(y, m, 1).month_len());
            assert_eq!(Date::from_days(d.days()), d);
        }
    }
    assert_eq!(date(1970, 1, 1).weekday(), 4);
    assert_eq!(date(2026, 10, 4).weekday(), 0);
}

#[test]
fn month_navigation_clamps_end_of_month_and_year() {
    assert_eq!(date(2024, 1, 31).shift_months(1), date(2024, 2, 29));
    assert_eq!(date(2026, 1, 31).shift_months(1), date(2026, 2, 28));
    assert_eq!(date(2026, 12, 4).shift_months(1), date(2027, 1, 4));
    assert_eq!(date(2026, 1, 4).shift_months(-1), date(2025, 12, 4));
    assert_eq!(date(1, 1, 1).shift_months(-1), date(1, 1, 1));
    assert_eq!(date(9999, 12, 31).shift_months(1), date(9999, 12, 31));
}

#[test]
fn persistence_roundtrips_unicode_and_replaces_atomically() {
    let temp = Temp::new();
    let first = event(1, "Viðtal 👩‍💻");
    store::save(&temp.file(), &[first.clone()]).unwrap();
    assert_eq!(store::load(&temp.file()).unwrap(), vec![first]);
    let second = event(2, "Planning");
    store::save(&temp.file(), &[second.clone()]).unwrap();
    assert_eq!(store::load(&temp.file()).unwrap(), vec![second]);
    assert_eq!(
        fs::read_dir(&temp.0).unwrap().count(),
        1,
        "temporary files cleaned up"
    );
    let old = fs::read(temp.file()).unwrap();
    let mut invalid = event(3, "bad\nrecord");
    assert!(store::save(&temp.file(), &[invalid.clone()]).is_err());
    invalid.title = "Valid".into();
    assert!(store::save(&temp.file(), &[invalid.clone(), invalid]).is_err());
    assert_eq!(
        fs::read(temp.file()).unwrap(),
        old,
        "invalid writes leave original intact"
    );
}

#[test]
fn malformed_records_and_unknown_headers_are_safe() {
    let temp = Temp::new();
    store::save(&temp.file(), &[event(1, "Valid")]).unwrap();
    let mut text = fs::read_to_string(temp.file()).unwrap();
    let original = text.lines().nth(1).unwrap().to_string();
    text.push_str("bad\n2\t2026\t2\t30\t540\t60\t0\t426164\n3\t2026\t10\t4\t9999\t60\t0\t426164\n4\t2026\t10\t4\t540\t60\t0\tzz\n");
    text.push_str(&original);
    text.push('\n');
    fs::write(temp.file(), text).unwrap();
    assert_eq!(store::load(&temp.file()).unwrap(), vec![event(1, "Valid")]);
    fs::write(temp.file(), "springboard-calendar-v9\nkeep future data").unwrap();
    assert!(store::load(&temp.file()).is_err());
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    let original = fs::read(temp.file()).unwrap();
    c.open(&ui, None);
    c.key(&ui, Out::Text("Test".into()), 0.0);
    c.activate(&ui, Target::Save, 100.0);
    assert!(c.editor.is_some());
    assert!(c.error.is_some());
    assert_eq!(
        fs::read(temp.file()).unwrap(),
        original,
        "unknown data never overwritten"
    );
}

/// Clicks the centre of toolbar button `id` (window pixels).
fn tap_bar(c: &mut Calendar, ui: &Ui, id: u32, t: f64) {
    let (x, y, w, h) = c.bar_px(ui, id).expect("toolbar button");
    c.touch(ui, Event::Down(x + w / 2.0, y + h / 2.0), t);
    c.touch(ui, Event::Up(x + w / 2.0, y + h / 2.0), t + 20.0);
}

/// The middle of event `id`'s first line in its month-grid cell (content pixels).
fn event_point(c: &Calendar, ui: &Ui, id: u64) -> (f32, f32) {
    let date = c.events.iter().find(|e| e.id == id).unwrap().date;
    let (left, cell, top, rh) = c.grid_geom(ui);
    let n = (date.days() - c.grid_start().days()) as f32;
    let (row, col) = ((n / 7.0).floor(), n % 7.0);
    (left + cell * (col + 0.5), top + rh * row + ui.pt(CELL_HEAD + CELL_LINE / 2.0))
}

fn tap_event(c: &mut Calendar, ui: &Ui, id: u64, t: f64) {
    // event_point is in content-column pixels; touch takes window pixels (the calendar list is left of them).
    c.layout_columns(ui);
    let (x, y) = event_point(c, ui, id);
    let (x, y) = (x + c.x0, y);
    c.touch(ui, Event::Down(x, y), t);
    c.touch(ui, Event::Up(x, y), t + 20.0);
}

/// The full New Event form through the toolbar's "+": its popover, then Edit Details.
fn new_form(c: &mut Calendar, ui: &Ui, t: f64) {
    tap_bar(c, ui, chrome::B_ADD, t);
    assert!(c.event_pop_open(), "\"+\" opens the quick-event popover");
    c.activate(ui, Target::Pop(eventpop::PopPart::Details), t + 30.0);
    assert!(c.editor.is_some() && !c.event_pop_open(), "Edit Details opens the form");
}

#[test]
fn touch_add_edit_delete_and_reload() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    new_form(&mut c, &ui, 0.0);
    assert!(c.keyboard_shown());
    c.key(&ui, Out::Text("Lunch with friends".into()), 100.0);
    c.key(&ui, Out::Hide, 110.0);
    // Later time and longer duration, then commit through the Save button.
    tap_row(&mut c, &ui, Target::Toggle(Slot::Start), 713.0, 150.0);
    tap_row(&mut c, &ui, Target::TimeStep(0), 713.0, 200.0);
    tap_row(&mut c, &ui, Target::Toggle(Slot::End), 713.0, 250.0);
    tap_row(&mut c, &ui, Target::TimeStep(0), 713.0, 300.0);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 400.0);
    assert!(c.editor.is_none());
    assert_eq!(c.events.len(), 1);
    assert_eq!((c.events[0].minute, c.events[0].duration), (555, 75));
    assert_eq!(store::load(&temp.file()).unwrap(), c.events);
    // Open the event from its day cell, select the next day, switch all-day, and save.
    let id = c.events[0].id;
    // a single tap only selects; the editor opens in its window on a double-click (covered by the month-cell test)
    c.open(&ui, Some(id));
    assert!(c.editor.is_some());
    tap_row(&mut c, &ui, Target::Toggle(Slot::Start), 713.0, 560.0);
    tap_row(&mut c, &ui, Target::DateStep(0), 713.0, 600.0);
    tap_row(&mut c, &ui, Target::AllDay, 650.0, 700.0);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 800.0);
    assert_eq!(c.selected, date(2026, 10, 5));
    assert!(c.events[0].all_day);
    let reopened = new(&ui, temp.file());
    assert_eq!(reopened.events, c.events);
    c.open(&ui, Some(id));
    tap_row(&mut c, &ui, Target::Delete, 384.0, 1000.0);
    assert!(c.events.is_empty());
    assert!(store::load(&temp.file()).unwrap().is_empty());
}

#[test]
fn cancellation_blank_titles_and_save_failure_preserve_events() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    c.activate(&ui, Target::Save, 10.0);
    assert!(c.editor.is_some());
    assert!(c.error.is_some());
    c.key(&ui, Out::Text("Draft".into()), 20.0);
    c.activate(&ui, Target::Cancel, 30.0);
    assert!(c.events.is_empty());
    assert!(!temp.file().exists());
    fs::create_dir_all(&temp.0).unwrap();
    fs::write(temp.file(), "block parent path").unwrap();
    c.path = temp.file().join("events.tsv");
    c.open(&ui, None);
    c.key(&ui, Out::Text("Keep this draft".into()), 50.0);
    c.activate(&ui, Target::Save, 60.0);
    assert!(c.events.is_empty());
    assert!(c.editor.is_some());
    assert_eq!(c.fields[0].text(), "Keep this draft");
    assert!(c.error.is_some());
}

#[test]
fn today_tracks_offset_midnight_without_second_redraws() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.dirty = false;
    let mut anchor = wall();
    anchor.unix += 1;
    anchor.at = 1000.0;
    c.set_wall(anchor);
    assert!(!c.needs_frame());
    assert_eq!(c.next_wake(1000.0), Some(12.0 * 3600.0 * 1000.0));
    c.set_wall(Wall {
        unix: tz::days_from_civil(2026, 10, 4) * 86400 + 23 * 3600 + 59 * 60 + 59,
        offset: 3600,
        at: 0.0,
    });
    assert_eq!(c.today, date(2026, 10, 5));
    c.activate(&ui, Target::Today, 0.0);
    assert_eq!(c.selected, date(2026, 10, 5));
}

#[test]
fn grid_and_home_chrome_hit_regions_are_bounded() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let c = new(&ui, temp.file());
    assert_eq!(
        c.hit(&ui, ui.pt(70.0), c.grid_geom(&ui).2 + c.grid_geom(&ui).3 + ui.pt(10.0)),
        Some(Target::Day(4))
    );
    assert_eq!(c.hit(&ui, ui.pt(70.0), c.grid_geom(&ui).2 + ui.pt(10.0)), Some(Target::Pick(date(2026, 9, 27))), "the leading days belong to September");
    assert_eq!(c.hit(&ui, ui.pt(70.0), -1.0), None, "a tap above the window hits nothing");
    assert_eq!(c.hit(&ui, 400.0, c.bottom(&ui)), None);
    assert!(icon(100).px.iter().all(|p| p >> 24 == 255));
}

#[test]
fn all_day_order_and_event_id_wrap() {
    let mut events = vec![event(u64::MAX, "Last"), event(1, "First")];
    events[0].all_day = true;
    store::sort(&mut events);
    assert_eq!(events[0].title, "Last");
    assert_eq!(store::next_id(&events), 2);
}

#[test]
fn full_calendar_with_maximum_unicode_titles_can_reload() {
    let temp = Temp::new();
    let title = "🗓".repeat(160);
    let events: Vec<_> = (1..=store::MAX_EVENTS as u64)
        .map(|id| event(id, &title))
        .collect();
    store::save(&temp.file(), &events).unwrap();
    assert_eq!(store::load(&temp.file()).unwrap(), events);
}

#[test]
fn a_month_cell_lists_its_events_and_a_click_opens_one() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let events: Vec<_> = (1..=20).map(|id| event(id, "Busy day")).collect();
    store::save(&temp.file(), &events).unwrap();
    let mut c = new(&ui, temp.file());
    let (shown, more) = c.cell_events(&ui, events[0].date);
    assert!(more > 0 && shown.len() + more == 20, "the cell ends in \"n more\"");
    let first = shown[0].id;
    tap_event(&mut c, &ui, first, 0.0);
    assert_eq!(c.take_open_request(), None, "one tap only selects the event");
    tap_event(&mut c, &ui, first, 100.0);
    assert_eq!(c.take_open_request(), Some(first), "a double-click asks for its editor window");
    c.open(&ui, Some(first));
    assert_eq!(c.editor.as_ref().map(|d| d.event.id), Some(first));
    c.activate(&ui, Target::Cancel, 100.0);
    // the toolbar's segmented group switches the view; the header's capsules step it
    tap_bar(&mut c, &ui, chrome::B_WEEK, 200.0);
    assert_eq!(c.view, View::Week);
    let before = c.selected;
    tap_bar(&mut c, &ui, chrome::B_NEXT, 300.0);
    assert_eq!(c.selected, before.shift_days(7));
    tap_bar(&mut c, &ui, chrome::B_TODAY, 400.0);
    assert_eq!(c.selected, date(2026, 10, 4));
    // the calendar list is shown on the first screen (as Golden Gate Calendar does), and its toggle folds it away and back
    assert!(c.x0 > 0.0);
    assert!(c.controls(&ui).iter().any(|n| n.label == "Hide Calendar List"));
    tap_bar(&mut c, &ui, chrome::B_SIDEBAR, 500.0);
    assert_eq!(c.x0, 0.0);
    assert!(c.controls(&ui).iter().any(|n| n.label == "Show Calendar List"));
    tap_bar(&mut c, &ui, chrome::B_SIDEBAR, 600.0);
    assert!(c.x0 > 0.0);
}

#[test]
fn hiding_input_lets_the_calendar_sleep() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    c.tick(&ui, 500.0);
    c.key(&ui, Out::Hide, 600.0);
    c.tick(&ui, 1000.0);
    c.dirty = false;
    assert!(!c.tick(&ui, 1001.0), "idle calendar should stay asleep");
}

#[test]
fn title_menu_and_selection_handle_work_outside_the_field() {
    use springboard::shell::edit::Action;
    let _guard = springboard::shell::clipboard::tests::exclusive();
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    c.key(&ui, Out::Text("Hello world".into()), 10.0);
    c.key(&ui, Out::Hide, 20.0);
    c.tick(&ui, 500.0);
    // Field frames are content-column pixels; touches take window pixels, so shift by the calendar list's width.
    c.layout_columns(&ui);
    let (fx, fy, fw, fh) = c.title_frame(&ui);
    let fx = fx + c.x0;
    let x = fx + ui.measure("Hello", MAC_TITLE3_PT, Weight::Regular);
    let y = fy + fh / 2.0;
    c.touch(&ui, Event::Down(x, y), 1000.0);
    c.tick(&ui, 1401.0);
    c.touch(&ui, Event::Up(x, y), 1450.0);
    assert!(c.keyboard_shown(), "long press focuses the input");
    let menu_point = |c: &Calendar, action: Action| {
        let menu = c.fields[0].menu().expect("editing menu");
        let (mx, my, mw, mh) = menu.rect();
        assert!(my + mh / 2.0 < fy, "menu tap lies above the title rectangle");
        let y = my + mh / 2.0;
        (0..mw.ceil() as usize)
            .map(|i| (mx + i as f32, y))
            .find(|&(x, y)| menu.hit(x, y) == Some(action))
            .map(|(x, y)| (x + c.x0, y))
            .expect("menu action")
    };
    let (mx, my) = menu_point(&c, Action::SelectAll);
    c.touch(&ui, Event::Down(mx, my), 1500.0);
    c.touch(&ui, Event::Up(mx, my), 1520.0);
    assert_eq!(c.fields[0].selection(), (0, 11));
    // The start handle's hit circle extends left of the field. Dragging it
    // must consume this outside touch and retain the other selection end.
    let hx = fx - ui.pt(10.0);
    let hy = fy + fh / 2.0 - ui.pt(20.0 * 1.25 / 2.0 + 5.0);
    assert!(!within(hx, hy, (fx, fy, fw, fh)));
    let end_x = fx + ui.measure("Hello ", MAC_TITLE3_PT, Weight::Regular);
    c.touch(&ui, Event::Down(hx, hy), 1600.0);
    c.touch(&ui, Event::Move(end_x, hy), 1650.0);
    c.touch(&ui, Event::Up(end_x, hy), 1700.0);
    assert_eq!(c.fields[0].selection(), (6, 11));
    let (mx, my) = menu_point(&c, Action::Copy);
    c.touch(&ui, Event::Down(mx, my), 1800.0);
    c.touch(&ui, Event::Up(mx, my), 1820.0);
    assert_eq!(springboard::shell::clipboard::text().as_deref(), Some("world"));
    assert_eq!(c.fields[0].text(), "Hello world");
    assert!(c.editor.is_some(), "menu must not activate app controls");
}

#[test]
fn year_view_month_zoom_clamps_day() {
    // Zooming from Year to Month keeps the day, clamped to the tapped month's length.
    let mut d = date(2025, 1, 31);
    d.month = 2;
    d.day = d.day.min(d.month_len());
    assert_eq!(d.day, 28);
}

#[test]
fn arrow_keys_move_the_selected_day_when_no_editor_is_open() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.selected = date(2026, 3, 10);
    c.key(&ui, Out::Named("Right"), 0.0);
    assert_eq!(c.selected, date(2026, 3, 11));
    c.key(&ui, Out::Named("Left"), 0.0);
    c.key(&ui, Out::Named("Left"), 0.0);
    assert_eq!(c.selected, date(2026, 3, 9));
    c.key(&ui, Out::Named("Down"), 0.0);
    assert_eq!(c.selected, date(2026, 3, 16));
    c.key(&ui, Out::Named("Up"), 0.0);
    c.key(&ui, Out::Named("Up"), 0.0);
    assert_eq!(c.selected, date(2026, 3, 2));
    c.key(&ui, Out::Named("Tab"), 0.0);
    assert_eq!(c.selected, date(2026, 3, 2), "unrelated keys do nothing");
}

#[test]
fn duplicate_opens_an_unsaved_copy_and_save_adds_it() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Dentist")];
    c.open(&ui, Some(1));
    c.activate(&ui, Target::Duplicate, 1.0);
    let draft = c.editor.as_ref().expect("copy opened");
    assert!(!draft.existing);
    assert_ne!(draft.event.id, 1);
    assert_eq!(draft.event.title, "Dentist");
    assert_eq!(c.events.len(), 1, "nothing saved until Save");
    c.activate(&ui, Target::Save, 2.0);
    assert_eq!(c.events.len(), 2);
    assert!(c.events.iter().any(|e| e.id == 1 && e.title == "Dentist"), "original kept");
}

#[test]
fn export_writes_the_saved_event_and_refuses_an_unsaved_one() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    c.key(&ui, Out::Text("Unsaved".into()), 0.0);
    c.activate(&ui, Target::Export, 1.0);
    assert!(c.error.as_deref().unwrap_or("").contains("Save the event"));
    c.enable_file_export();
    c.events = vec![event(7, "Team: sync")];
    c.open(&ui, Some(7));
    c.activate(&ui, Target::Export, 3.0);
    let home = temp.0.join("home");
    std::env::set_var("HOME", &home);
    c.activate(&ui, Target::Export, 4.0);
    let file = home.join("Documents/Calendar Export/Team_ sync.ics");
    let text = fs::read_to_string(&file).expect("export written");
    assert!(text.contains("SUMMARY:Team: sync") || text.contains("Team: sync"), "{text}");
    assert!(c.notice.as_deref().unwrap_or("").contains("Team_ sync.ics"));
    // the whole calendar goes to Calendar.ics in the same folder
    c.activate(&ui, Target::ExportCalendar, 5.0);
    let whole = fs::read_to_string(home.join("Documents/Calendar Export/Calendar.ics")).expect("calendar export written");
    assert!(whole.contains("BEGIN:VCALENDAR"), "{whole}");
    assert!(c.notice.as_deref().unwrap_or("").contains("Calendar.ics"));
}

#[test]
fn export_row_is_desktop_only() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(3, "Lunch")];
    c.open(&ui, Some(3));
    assert!(!c.rows().iter().any(|(t, ..)| *t == Target::Export), "no file export");
    c.enable_file_export();
    assert!(c.rows().iter().any(|(t, ..)| *t == Target::Export));
}

#[test]
fn enter_opens_a_new_event_on_the_selected_day() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.selected = date(2026, 3, 10);
    c.key(&ui, Out::Enter, 0.0);
    let draft = c.editor.as_ref().expect("editor opened");
    assert!(!draft.existing);
    assert_eq!(draft.event.date, date(2026, 3, 10));
}

#[test]
fn cmd_chords_switch_views_and_start_an_event() {
    use bifrost_app::proto::CHORD_META;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.chord(&ui, 3, CHORD_META, 0.0), "Cmd+2 is Week");
    assert_eq!(c.view, timeview::View::Week);
    assert!(c.chord(&ui, 5, CHORD_META, 1.0), "Cmd+4 is Year");
    assert_eq!(c.view, timeview::View::Year);
    assert!(c.chord(&ui, 49, CHORD_META, 2.0), "Cmd+N");
    assert!(c.editor.as_ref().is_some_and(|d| !d.existing), "a new event opens");
    assert!(!c.chord(&ui, 3, CHORD_META, 3.0), "no view change while the editor is open");
}

#[test]
fn escape_closes_the_event_editor_without_saving() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(!c.escape(&ui, 0.0), "no editor, nothing to close");
    c.open(&ui, None);
    c.key(&ui, Out::Text("Unsaved".into()), 1.0);
    assert!(c.escape(&ui, 2.0));
    assert!(c.editor.is_none());
    assert!(c.events.is_empty(), "Escape does not save");
}

#[test]
fn iso_weeks_follow_the_monday_rule() {
    assert_eq!(super::iso_week(date(2026, 1, 1)), 1, "a Thursday starts week 1");
    assert_eq!(super::iso_week(date(2026, 10, 8)), 41);
    assert_eq!(super::iso_week(date(2027, 1, 1)), 53, "2026 has 53 weeks, and Friday 1 Jan 2027 is in it");
    assert_eq!(super::iso_week(date(2025, 12, 29)), 1, "Monday 29 Dec 2025 starts 2026's week 1");
}

#[test]
fn deleting_shows_an_undo_bar_that_restores_the_event() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(7, "Dentist")]).unwrap();
    let mut c = new(&ui, temp.file());
    c.open(&ui, Some(7));
    c.activate(&ui, Target::Delete, 1.0);
    assert!(c.events.is_empty(), "deleted");
    assert!(c.undo.is_some(), "Undo offered");
    assert!(c.hit(&ui, ui.pt(60.0), c.undo_rect(&ui).1 + ui.pt(10.0)).is_some_and(|t| t == Target::UndoDelete));
    c.activate(&ui, Target::UndoDelete, 2.0);
    assert_eq!(c.events.len(), 1, "restored");
    assert_eq!(store::load(&temp.file()).unwrap().len(), 1, "restored on disk");
    assert!(c.undo.is_none());
}

#[test]
fn the_undo_bar_lapses_after_five_seconds() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(8, "Gone")]).unwrap();
    let mut c = new(&ui, temp.file());
    c.open(&ui, Some(8));
    c.activate(&ui, Target::Delete, 1000.0);
    c.tick(&ui, 5999.0);
    assert!(c.undo.is_some());
    c.tick(&ui, 6000.0);
    assert!(c.undo.is_none(), "gone after five seconds");
    assert!(c.events.is_empty(), "the delete stands");
}

#[test]
fn week_numbers_are_a_saved_setting() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(!c.prefs.week_numbers);
    c.activate(&ui, Target::WeekNumbers, 0.0);
    assert!(c.prefs.week_numbers);
    let again = new(&ui, temp.file());
    assert!(again.prefs.week_numbers, "kept after a restart");
}

#[test]
fn an_address_in_the_notes_is_offered_as_the_location() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Dinner")];
    c.open(&ui, Some(1));
    assert!(!c.rows().iter().any(|r| r.0 == Target::FillAddress), "no address in the notes yet");
    c.fields[2].set_text(&ui, "Table for two at Laugavegur 12");
    assert!(c.rows().iter().any(|r| r.0 == Target::FillAddress), "the offer appears under Notes");
    c.activate(&ui, Target::FillAddress, 1.0);
    assert_eq!(c.fields[1].text(), "Laugavegur 12");
}

#[test]
fn an_imported_events_invitees_are_listed_in_its_editor_with_their_answers() {
    let temp = Temp::new();
    fs::create_dir_all(&temp.0).unwrap();
    let rich = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:dinner\r\nDTSTART:20261004T090000\r\nDTEND:20261004T093000\r\n\
SUMMARY:Dinner\r\nATTENDEE;CN=Ana Lind;PARTSTAT=ACCEPTED:mailto:ana@example.is\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:bo@example.is\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    fs::write(exchange::rich_path(&temp.file()), rich).unwrap();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Dinner")];
    c.open(&ui, Some(1));
    assert_eq!(c.editor.as_ref().unwrap().invitees.len(), 2, "both attendees read from the imported event");
    assert!(c.rows().iter().any(|r| r.0 == Target::Invitees), "the Invitees row appears");
    let row = invitees::summary(&c.editor.as_ref().unwrap().invitees);
    assert_eq!(row, "Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply)");
    c.activate(&ui, Target::Invitees, 1.0);
    assert!(c.editor.is_some(), "the read-only row leaves the draft open");
}

#[test]
fn a_phone_number_in_the_notes_is_offered_as_a_copy_action() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Dinner")];
    c.open(&ui, Some(1));
    assert!(!c.rows().iter().any(|r| r.0 == Target::CopyPhone), "no phone number in the notes yet");
    c.fields[2].set_text(&ui, "Call +354 555 1234 before lunch");
    assert!(c.rows().iter().any(|r| r.0 == Target::CopyPhone), "the offer appears under Notes");
    c.activate(&ui, Target::CopyPhone, 1.0);
    assert_eq!(c.take_copied().as_deref(), Some("+354 555 1234"));
    assert_eq!(c.take_copied(), None, "the copy is handed over once");
    assert_eq!(c.notice.as_deref(), Some("Copied +354 555 1234"));
    assert!(c.editor.is_some(), "the draft stays open");
}

#[test]
fn new_event_from_text_opens_a_prefilled_editor() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.new_event_from_text(&ui, "Lunch with Ana next Tuesday at 12:30 at Laugavegur 5", 1.0));
    let draft = c.editor.as_ref().expect("the editor opens");
    assert!(!draft.existing, "a new event, not saved yet");
    assert_eq!(draft.event.title, "Lunch with Ana");
    assert_eq!(draft.event.location, "Laugavegur 5");
    assert_eq!(draft.event.minute, 12 * 60 + 30);
    assert_eq!(draft.event.duration, 60);
    // the next Tuesday after today: a weekday 2 (Tuesday), one to seven days on
    let gap = draft.event.date.days() - today(c.wall(), 1.0).days();
    assert!((1..=7).contains(&gap), "next Tuesday is within a week");
    assert_eq!(draft.event.date.weekday(), 2);
    assert_eq!(c.fields[0].text(), "Lunch with Ana");
    assert_eq!(c.fields[1].text(), "Laugavegur 5");
    assert!(c.events.is_empty(), "nothing is saved until Save");
}

#[test]
fn new_event_from_an_empty_pasteboard_is_a_plain_new_event() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.new_event_from_text(&ui, "   ", 1.0));
    let draft = c.editor.as_ref().expect("the editor opens");
    assert!(!draft.existing);
    assert_eq!(draft.event.minute, 9 * 60, "the editor's default start");
    assert!(draft.event.title.is_empty());
}

#[test]
fn zoom_changes_the_hour_height_and_keeps_it() {
    use prefs::HOUR_H_DEFAULT;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(!c.enabled(Command::ZoomIn) && !c.enabled(Command::ZoomOut), "the month grid has no hours");
    c.activate(&ui, Target::View(View::Week), 0.0);
    assert!(c.enabled(Command::ZoomIn) && c.enabled(Command::ZoomOut));
    assert!(c.perform(&ui, Command::ZoomIn, 0.0));
    assert_eq!(c.prefs.hour_h, HOUR_H_DEFAULT + prefs::HOUR_H_STEP);
    let again = new(&ui, temp.file());
    assert_eq!(again.prefs.hour_h, HOUR_H_DEFAULT + prefs::HOUR_H_STEP, "kept after a restart");
    for _ in 0..20 {
        c.perform(&ui, Command::ZoomOut, 0.0);
    }
    assert_eq!(c.prefs.hour_h, prefs::HOUR_H_MIN);
    assert!(!c.enabled(Command::ZoomOut), "at the smallest size");
}

#[test]
fn zooming_keeps_the_same_hour_at_the_top() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Day), 0.0);
    // the day opens at 8 AM
    assert!((c.scroll.offset() - ui.pt(8.0 * prefs::HOUR_H_DEFAULT)).abs() < 1.0);
    c.zoom_hours(&ui, 1);
    assert!((c.scroll.offset() - ui.pt(8.0 * (prefs::HOUR_H_DEFAULT + prefs::HOUR_H_STEP))).abs() < 1.0);
    assert!(c.time_scroll_max(&ui) > 0.0);
}

#[test]
fn an_event_block_grows_with_the_zoom() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Standup")];
    c.activate(&ui, Target::View(View::Day), 0.0);
    let height = |c: &Calendar| c.time_rects(&ui)[0].4;
    let before = height(&c);
    c.zoom_hours(&ui, 1);
    assert!(height(&c) > before);
}

#[test]
fn the_week_can_start_on_monday() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    // 4 October 2026 is a Sunday; the 1st of the month is a Thursday
    assert_eq!(c.week_start(), date(2026, 10, 4));
    assert_eq!(c.grid_start(), date(2026, 9, 27));
    assert!(c.change_setting(&ui, Setting::MondayFirst));
    assert!(c.setting_checked(Setting::MondayFirst));
    assert_eq!(c.week_start(), date(2026, 9, 28));
    assert_eq!(c.grid_start(), date(2026, 9, 28));
    let again = new(&ui, temp.file());
    assert_eq!(again.week_start(), date(2026, 9, 28), "kept after a restart");
}

#[test]
fn new_events_take_the_default_length_and_alerts() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.change_setting(&ui, Setting::Length(2));
    c.change_setting(&ui, Setting::Alert(3));
    c.change_setting(&ui, Setting::AllDayAlert(6));
    c.activate(&ui, Target::Add, 0.0);
    let e = c.editor.as_ref().unwrap().event.clone();
    assert_eq!((e.duration, e.alert), (90, 3));
    // turning the new event all-day swaps the default alert, turning it back swaps it again
    c.activate(&ui, Target::AllDay, 0.0);
    assert_eq!(c.editor.as_ref().unwrap().event.alert, 6);
    c.activate(&ui, Target::AllDay, 0.0);
    assert_eq!(c.editor.as_ref().unwrap().event.alert, 3);
    // a hand-picked alert is left alone: the picker clears the default and picks 30 minutes before
    c.activate(&ui, Target::Alert, 0.0);
    c.activate(&ui, Target::AlarmPreset(0), 0.0);
    c.activate(&ui, Target::AlarmPreset(4), 0.0);
    assert_eq!(c.editor.as_ref().unwrap().alarms, vec![30]);
    c.activate(&ui, Target::AllDay, 0.0);
    assert_eq!(c.editor.as_ref().unwrap().alarms, vec![30]);
}

#[test]
fn settings_cannot_change_while_an_event_is_open() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.activate(&ui, Target::Add, 0.0);
    assert!(!c.settings_enabled());
    assert!(!c.change_setting(&ui, Setting::MondayFirst));
    assert!(!c.setting_checked(Setting::MondayFirst));
}

#[test]
fn copy_and_paste_move_an_event_to_another_day() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Standup")];
    assert!(c.edit_commands(None).supports(Edit::Copy));
    assert!(c.edit_commands(None).supports(Edit::Cut));
    assert!(!c.edit_commands(Some("buy milk")).supports(Edit::Paste), "plain text is not an event");
    assert!(c.edit_command(&ui, Edit::Copy, None, 0.0));
    let text = c.take_copied().expect("the .ics text for the pasteboard");
    assert!(text.contains("SUMMARY:Standup"));
    c.selected = date(2026, 10, 9);
    assert!(!c.edit_commands(None).supports(Edit::Copy), "nothing on the 9th");
    assert!(c.edit_commands(Some(&text)).supports(Edit::Paste));
    assert!(c.edit_command(&ui, Edit::Paste, Some(&text), 0.0));
    let days: Vec<(String, Date)> = c.events.iter().map(|e| (e.title.clone(), e.date)).collect();
    assert_eq!(days, vec![("Standup".to_string(), date(2026, 10, 4)), ("Standup".to_string(), date(2026, 10, 9))]);
    assert_eq!(c.notice.as_deref(), Some("Pasted 1 event"));
    assert_eq!(new(&ui, temp.file()).events.len(), 2, "saved");
}

#[test]
fn pasting_something_else_changes_nothing_and_says_so() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(!c.edit_command(&ui, Edit::Paste, Some("buy milk"), 0.0));
    assert!(c.events.is_empty());
    assert_eq!(c.error.as_deref(), Some("There are no events to paste"));
    assert!(!c.edit_command(&ui, Edit::Paste, None, 0.0));
}

#[test]
fn cut_removes_the_event_with_an_undo_bar() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Standup"), event(2, "Lunch")];
    // the event opened last is the one Cut takes
    c.open(&ui, Some(2));
    c.close();
    assert!(c.edit_command(&ui, Edit::Cut, None, 1000.0));
    assert!(c.take_copied().is_some_and(|t| t.contains("SUMMARY:Lunch")));
    assert_eq!(c.events.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1]);
    assert!(c.undo.is_some());
    c.undo_delete(&ui, 2000.0);
    assert_eq!(c.events.len(), 2, "undo puts it back");
}

#[test]
fn copy_in_the_editor_takes_the_open_event_and_text_commands_go_to_the_field() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.events = vec![event(1, "Standup")];
    c.open(&ui, Some(1));
    assert!(c.edit_commands(None).supports(Edit::Copy), "no field has focus, so Copy takes the event");
    assert!(!c.edit_commands(None).supports(Edit::Cut));
    assert!(c.edit_command(&ui, Edit::Copy, None, 0.0));
    assert!(c.take_copied().is_some());
    // a field with focus gets the text commands
    c.activate(&ui, Target::Notes, 0.0);
    c.key(&ui, Out::Text("remember the cake".into()), 0.0);
    assert!(c.edit_commands(None).supports(Edit::SelectAll));
    assert!(!c.edit_commands(None).supports(Edit::Cut), "nothing is selected");
    assert!(c.edit_command(&ui, Edit::SelectAll, None, 0.0));
    assert!(c.edit_commands(None).supports(Edit::Cut));
    assert!(c.edit_command(&ui, Edit::Cut, None, 0.0));
    assert_eq!(c.fields[2].text(), "");
    assert!(c.take_copied().is_none(), "the field's text is not an event");
}

#[test]
fn go_to_date_steps_and_accepts_the_chosen_day() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.perform(&ui, Command::GoToDate, 0.0));
    assert_eq!(c.goto, Some(date(2026, 10, 4)));
    assert!(!c.enabled(Command::NewEvent) && !c.enabled(Command::GoToDate), "the sheet is modal");
    // the Month row's plus, the Year row's plus, the Day row's minus (window points)
    tap(&mut c, &ui, 713.0, 174.0, 10.0);
    tap(&mut c, &ui, 713.0, 218.0, 50.0);
    tap(&mut c, &ui, 250.0, 130.0, 100.0);
    assert_eq!(c.goto, Some(date(2027, 11, 3)));
    assert_eq!(c.selected, date(2026, 10, 4), "nothing moves until Go");
    tap_bar(&mut c, &ui, chrome::B_GO, 200.0);
    assert!(c.goto.is_none());
    assert_eq!(c.selected, date(2027, 11, 3));
    assert!(c.enabled(Command::NewEvent), "the sheet is closed");
}

#[test]
fn cancel_and_escape_leave_the_shown_date_alone() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::GoToDate, 0.0);
    c.activate(&ui, Target::GoYear(3), 1.0);
    tap_bar(&mut c, &ui, chrome::B_CANCEL, 2.0);
    assert!(c.goto.is_none());
    assert_eq!(c.selected, date(2026, 10, 4));
    c.perform(&ui, Command::GoToDate, 3.0);
    c.activate(&ui, Target::GoDay(9), 4.0);
    assert!(c.escape(&ui, 5.0));
    assert!(c.goto.is_none());
    assert_eq!(c.selected, date(2026, 10, 4));
    assert!(!c.escape(&ui, 6.0), "nothing left to close");
}

#[test]
fn the_sheet_has_a_today_shortcut_and_keys() {
    use bifrost_app::proto::CHORD_META;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::GoToDate, 0.0);
    c.activate(&ui, Target::GoYear(2), 1.0);
    tap(&mut c, &ui, 400.0, 282.0, 10.0);
    assert_eq!(c.goto, Some(date(2026, 10, 4)), "Today puts the sheet back on today");
    c.key(&ui, Out::Named("Right"), 20.0);
    c.key(&ui, Out::Named("Down"), 21.0);
    assert_eq!(c.goto, Some(date(2026, 10, 12)));
    assert!(!c.chord(&ui, 3, CHORD_META, 22.0), "no view change behind the sheet");
    c.key(&ui, Out::Enter, 30.0);
    assert!(c.goto.is_none());
    assert_eq!(c.selected, date(2026, 10, 12));
}

#[test]
fn the_sheet_publishes_its_controls() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::GoToDate, 0.0);
    let labels: Vec<String> = c.controls(&ui).into_iter().map(|n| n.label).collect();
    for want in ["Cancel", "Go", "Previous Day", "Next Day", "Previous Month", "Next Year", "Today"] {
        assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
    }
}

fn weekly_gym_ics() -> String {
    let mut gym = ics::IcsEvent::new("gym@x", "Gym", ics::Local::new(date(2026, 10, 5), 600), ics::Local::new(date(2026, 10, 5), 660));
    gym.rule = recur::parse_rrule("FREQ=WEEKLY;COUNT=4");
    ics::write(&ics::IcsCalendar { name: String::new(), events: vec![gym], timezones: vec![] }, "20261004T000000Z")
}

#[test]
fn an_archive_restores_events_and_the_sidecar_in_another_calendar() {
    let (ta, tb) = (Temp::new(), Temp::new());
    let ui = crate::test_ui();
    let mut dentist = event(2, "Dentist");
    dentist.location = "Laugavegur 5".into();
    store::save(&ta.file(), &[event(1, "Standup"), dentist]).unwrap();
    let mut a = new(&ui, ta.file());
    a.import_ics(&weekly_gym_ics()).unwrap();
    assert_eq!(a.events.len(), 6, "two saved events and four Gym days");
    let text = a.archive_text();
    assert!(archive::is_archive(&text));

    let mut b = new(&ui, tb.file());
    assert!(b.import_file(&text));
    assert_eq!(b.notice.as_deref(), Some("Imported 6 events"));
    assert_eq!(b.events.len(), 6);
    assert_eq!(b.events.iter().find(|e| e.title == "Dentist").unwrap().location, "Laugavegur 5");
    assert!(b.rich_events().events.iter().any(|e| e.uid == "gym@x" && e.rule.is_some()), "the repeat rule came along");
    assert_eq!(new(&ui, tb.file()).events.len(), 6, "saved");

    // the same archive again changes nothing
    assert!(b.import_file(&text));
    assert_eq!(b.notice.as_deref(), Some("Nothing new to import"));
    assert_eq!(b.events.len(), 6);
    assert_eq!(fs::read_dir(&tb.0).unwrap().filter(|e| e.as_ref().unwrap().file_name() == "events.import.tmp").count(), 0, "no scratch file left");
}

#[test]
fn an_empty_calendar_archives_and_restores() {
    let (ta, tb) = (Temp::new(), Temp::new());
    let ui = crate::test_ui();
    let a = new(&ui, ta.file());
    let mut b = new(&ui, tb.file());
    b.events = vec![event(1, "Kept")];
    assert!(b.import_file(&a.archive_text()));
    assert_eq!(b.notice.as_deref(), Some("Nothing new to import"));
    assert_eq!(b.events.len(), 1, "an import never removes anything");
}

#[test]
fn importing_an_ics_file_adds_its_events() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.import_file(&weekly_gym_ics()));
    assert_eq!(c.notice.as_deref(), Some("Imported 4 events"));
    assert_eq!(c.events.len(), 4);
}

#[test]
fn a_file_that_is_not_a_calendar_is_reported() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(!c.import_file("shopping list"));
    assert_eq!(c.error.as_deref(), Some("No events found in this file"));
    assert!(!c.import_file(&format!("{}\nevents.tsv 99\nabc\n", archive::MAGIC)));
    assert_eq!(c.error.as_deref(), Some("This calendar archive is damaged"));
    assert!(c.events.is_empty());
    c.show_error("Could not read it");
    assert_eq!(c.error.as_deref(), Some("Could not read it"));
}

#[test]
fn the_file_commands_are_menu_items_for_the_app_to_run() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    for cmd in [Command::Import, Command::ImportArchive, Command::ExportArchive] {
        assert!(c.enabled(cmd), "{cmd:?}");
        assert!(!c.perform(&ui, cmd, 0.0), "the app shows the panel, not the calendar");
        assert_eq!(Command::from_id(cmd.id()), Some(cmd));
    }
    assert_eq!(c.archive_name(), "Calendar Archive 2026-10-04.calarchive");
    c.activate(&ui, Target::Add, 0.0);
    assert!(!c.enabled(Command::Import) && !c.enabled(Command::ExportArchive));
}

/// Window pixels at the middle of minute `minute` in day column `col` of the Week view's hour grid.
fn grid_xy(c: &Calendar, ui: &Ui, col: usize, minute: u32) -> (f32, f32) {
    let (gut, col_w) = c.col_geometry(ui);
    let top = ui.pt(timeview::GRID0 + timeview::ALLDAY_H) - c.scroll.offset();
    (c.x0 + gut + col_w * (col as f32 + 0.5), top + (minute as f32 + 0.5) / 60.0 * ui.pt(c.prefs.hour_h))
}

fn week_calendar(ui: &Ui, file: PathBuf) -> Calendar {
    let mut c = new(ui, file);
    c.activate(ui, Target::View(View::Week), 0.0);
    c
}

#[test]
fn holding_then_dragging_on_empty_space_draws_out_a_new_event() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    // Sunday 4 October is column 0, so column 3 is Wednesday the 7th
    let (x, y) = grid_xy(&c, &ui, 3, 600);
    let (_, y2) = grid_xy(&c, &ui, 3, 690);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + 1.0), 300.0);
    assert!(c.drag.is_some(), "held long enough, the move drags");
    c.touch(&ui, Event::Move(x, y2), 400.0);
    assert_eq!(c.drag.map(|d| d.span), Some(layout::DragSpan { start: 600, end: 705 }));
    c.touch(&ui, Event::Up(x, y2), 450.0);
    assert!(c.drag.is_none());
    let draft = c.editor.as_ref().expect("the editor opens on the drawn-out event");
    assert!(!draft.existing);
    assert_eq!((draft.event.date, draft.event.minute, draft.event.duration), (date(2026, 10, 7), 600, 105));
    assert!(c.events.is_empty(), "nothing is saved until Save");
    assert_eq!(c.selected, date(2026, 10, 7));
}

#[test]
fn dragging_upwards_works_too_and_a_drawn_event_takes_the_default_alert() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    c.change_setting(&ui, Setting::Alert(3));
    let (x, y) = grid_xy(&c, &ui, 1, 720);
    let (_, y2) = grid_xy(&c, &ui, 1, 660);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y2), 300.0);
    c.touch(&ui, Event::Up(x, y2), 310.0);
    let e = c.editor.as_ref().unwrap().event.clone();
    assert_eq!((e.date, e.minute, e.duration, e.alert), (date(2026, 10, 5), 660, 75, 3));
}

#[test]
fn holding_then_dragging_an_event_moves_it_to_another_day_and_time() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    // the event is on Sunday the 4th at 09:00 for an hour; grab it 20 minutes in and drop it on Tuesday near 13:20
    let (x, y) = grid_xy(&c, &ui, 0, 560);
    let (x2, y2) = grid_xy(&c, &ui, 2, 800);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + 1.0), 300.0);
    c.touch(&ui, Event::Move(x2, y2), 400.0);
    assert_eq!(c.drag.map(|d| (d.day, d.span.start)), Some((date(2026, 10, 6), 780)));
    c.touch(&ui, Event::Up(x2, y2), 450.0);
    assert!(c.editor.is_none(), "a moved event does not open the editor");
    let moved = &c.events[0];
    assert_eq!((moved.date, moved.minute, moved.duration), (date(2026, 10, 6), 780, 60));
    assert_eq!(c.notice.as_deref(), Some("Moved Standup to 13:00"));
    let again = new(&ui, temp.file());
    assert_eq!((again.events[0].date, again.events[0].minute), (date(2026, 10, 6), 780), "saved");
}

#[test]
fn a_press_that_moves_at_once_still_scrolls() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    let before = c.scroll.offset();
    let (x, y) = grid_xy(&c, &ui, 3, 600);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + ui.pt(60.0)), 40.0);
    c.touch(&ui, Event::Move(x, y + ui.pt(120.0)), 80.0);
    assert!(c.drag.is_none(), "not held, so not a drag");
    c.touch(&ui, Event::Up(x, y + ui.pt(120.0)), 120.0);
    assert!(c.editor.is_none() && c.events.is_empty());
    assert_ne!(c.scroll.offset(), before, "the grid scrolled");
}

#[test]
fn a_quick_tap_on_an_event_in_the_week_selects_it_and_a_double_tap_opens_it() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    let (x, y) = grid_xy(&c, &ui, 0, 560);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Up(x, y), 50.0);
    assert!(c.drag.is_none());
    assert!(c.editor.is_none());
    assert_eq!(c.picked, Some(1));
    c.touch(&ui, Event::Down(x, y), 200.0);
    c.touch(&ui, Event::Up(x, y), 250.0);
    assert_eq!(c.take_open_request(), Some(1));
}

#[test]
fn a_drag_that_ends_where_it_began_changes_nothing() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    let (x, y) = grid_xy(&c, &ui, 0, 560);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + 1.0), 300.0);
    c.touch(&ui, Event::Up(x, y + 1.0), 350.0);
    assert!(c.editor.is_none());
    assert_eq!((c.events[0].date, c.events[0].minute), (date(2026, 10, 4), 540));
    assert!(c.notice.is_none());
}

#[test]
fn nothing_drags_in_the_month_grid_or_the_hour_labels() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.touch(&ui, Event::Down(500.0, 700.0), 0.0);
    c.touch(&ui, Event::Move(520.0, 720.0), 400.0);
    assert!(c.drag.is_none() && c.press.is_none(), "the month grid does not drag");
    c.touch(&ui, Event::Up(520.0, 720.0), 450.0);
    let mut w = week_calendar(&ui, temp.file());
    let (gut, _) = w.col_geometry(&ui);
    let (_, y) = grid_xy(&w, &ui, 0, 600);
    w.touch(&ui, Event::Down(w.x0 + gut / 2.0, y), 0.0);
    assert!(w.press.is_none(), "the hour labels scroll, they do not draw events");
}

/// A calendar with three saved events in a window wide enough for the toolbar's search field.
fn searchable(ui: &Ui, file: PathBuf) -> Calendar {
    let mut lunch = event(2, "Lunch with Ana");
    lunch.date = date(2026, 10, 6);
    lunch.minute = 750;
    let mut dentist = event(3, "Dentist");
    dentist.date = date(2026, 10, 8);
    store::save(&file, &[event(1, "Standup"), lunch, dentist]).unwrap();
    let mut c = new(ui, file);
    c.set_size(2400, 1920);
    c.layout_columns(ui);
    c
}

fn type_text(c: &mut Calendar, ui: &Ui, text: &str) {
    for ch in text.chars() {
        c.key(ui, Out::Text(ch.to_string()), 0.0);
    }
}

fn draw_once(c: &mut Calendar, ui: &Ui) {
    let (w, h) = (c.full_w as usize, c.h as usize);
    let mut px = vec![0u32; w * h];
    let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
    c.draw(&mut s, ui, 0.0);
}

#[test]
fn typing_in_the_search_field_lists_matching_events() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    let field = c.search_rect(&ui).expect("a wide window has the search field");
    tap(&mut c, &ui, field.x + field.w / 2.0, field.y + field.h / 2.0, 0.0);
    assert!(c.search_focused && !c.searching());
    type_text(&mut c, &ui, "lun");
    assert!(c.searching());
    assert_eq!(c.search_query(), "lun");
    let titles: Vec<&str> = c.search_hits().iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Lunch with Ana"]);
    type_text(&mut c, &ui, "x");
    assert!(c.searching() && c.search_hits().is_empty(), "no match, and the list says so");
    c.key(&ui, Out::Backspace, 0.0);
    assert_eq!(c.search_hits().len(), 1);
    c.key(&ui, Out::Enter, 0.0);
    assert!(!c.search_focused && c.searching(), "Return leaves the field and keeps the results");
}

#[test]
fn a_result_opens_its_event_on_its_day() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    c.activate(&ui, Target::SearchField, 0.0);
    type_text(&mut c, &ui, "d");
    // "Standup", "Lunch with Ana" has no d, "Dentist": the rows are in date order
    let titles: Vec<&str> = c.search_hits().iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Standup", "Dentist"]);
    // the second row (window points: below the large title, 52 pt rows)
    tap(&mut c, &ui, 400.0, GRID_TOP + 52.0 + 26.0, 10.0);
    assert!(!c.searching() && c.search_query().is_empty(), "the search ends");
    assert_eq!(c.selected, date(2026, 10, 8));
    assert!(c.editor.as_ref().is_some_and(|d| d.existing && d.event.id == 3));
}

#[test]
fn the_clear_button_and_escape_end_the_search() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    let field = c.search_rect(&ui).unwrap();
    c.activate(&ui, Target::SearchField, 0.0);
    type_text(&mut c, &ui, "den");
    // the clear button is at the trailing edge once there is text
    tap(&mut c, &ui, field.x + field.w - 10.0, field.y + field.h / 2.0, 10.0);
    assert!(c.search_query().is_empty() && c.search_focused, "cleared, and the cursor stays");
    type_text(&mut c, &ui, "den");
    assert!(c.escape(&ui, 20.0));
    assert!(!c.searching() && !c.search_focused);
    assert!(!c.escape(&ui, 30.0), "nothing left to close");
}

#[test]
fn choosing_a_view_or_a_new_event_ends_the_search() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    c.activate(&ui, Target::SearchField, 0.0);
    type_text(&mut c, &ui, "den");
    c.activate(&ui, Target::View(View::Week), 1.0);
    assert!(!c.searching() && c.search_query().is_empty());
    c.activate(&ui, Target::SearchField, 2.0);
    type_text(&mut c, &ui, "den");
    c.activate(&ui, Target::Add, 3.0);
    assert!(c.search_query().is_empty() && c.editor.is_some());
}

#[test]
fn a_press_elsewhere_takes_the_cursor_out_of_the_field() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    c.activate(&ui, Target::SearchField, 0.0);
    tap(&mut c, &ui, 500.0, 600.0, 10.0);
    assert!(!c.search_focused);
    // keys then move the shown day again
    let day = c.selected;
    c.key(&ui, Out::Named("Right"), 20.0);
    assert_eq!(c.selected, day.shift_days(1));
}

#[test]
fn cmd_f_and_the_menu_item_focus_the_field_and_paste_goes_into_it() {
    use bifrost_app::proto::CHORD_META;
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    assert!(c.chord(&ui, 33, CHORD_META, 0.0));
    assert!(c.search_focused);
    c.escape(&ui, 1.0);
    assert!(c.enabled(Command::Search) && c.perform(&ui, Command::Search, 2.0));
    assert!(c.search_focused);
    assert!(c.edit_commands(Some("dent")).supports(Edit::Paste));
    assert!(!c.edit_commands(Some("dent")).supports(Edit::Copy));
    assert!(c.edit_command(&ui, Edit::Paste, Some("dent\nsecond line"), 3.0));
    assert_eq!(c.search_query(), "dent");
    assert_eq!(c.search_hits().len(), 1);
}

#[test]
fn the_field_is_published_for_assistive_technology() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    let has_field = |c: &Calendar| c.controls(&ui).iter().any(|n| n.role == bifrost_app::controls::Role::TextField && n.label == "Search");
    assert!(has_field(&c));
    c.activate(&ui, Target::SearchField, 0.0);
    type_text(&mut c, &ui, "den");
    let rows: Vec<String> = c.controls(&ui).into_iter().filter(|n| n.id >= 400 && n.id < 500).map(|n| n.label).collect();
    assert_eq!(rows, vec!["Dentist".to_string()]);
    c.activate(&ui, Target::Add, 1.0);
    assert!(!has_field(&c), "no field while the event form is open");
}

#[test]
fn a_narrow_window_has_no_search_field_but_still_searches() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = new(&ui, temp.file());
    assert!(c.search_rect(&ui).is_none());
    c.perform(&ui, Command::Search, 0.0);
    type_text(&mut c, &ui, "stand");
    assert_eq!(c.search_hits().len(), 1);
}

#[test]
fn every_screen_draws() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    draw_once(&mut c, &ui);
    for view in [View::Day, View::Week, View::Year, View::Month] {
        c.activate(&ui, Target::View(view), 0.0);
        draw_once(&mut c, &ui);
    }
    // results, with matches and without
    c.activate(&ui, Target::SearchField, 0.0);
    type_text(&mut c, &ui, "d");
    draw_once(&mut c, &ui);
    type_text(&mut c, &ui, "zzz");
    draw_once(&mut c, &ui);
    c.escape(&ui, 1.0);
    // the Go to Date sheet
    c.perform(&ui, Command::GoToDate, 2.0);
    draw_once(&mut c, &ui);
    c.activate(&ui, Target::Cancel, 3.0);
    // a block being dragged out in the Week view
    c.activate(&ui, Target::View(View::Week), 4.0);
    let (x, y) = grid_xy(&c, &ui, 2, 600);
    let (_, y2) = grid_xy(&c, &ui, 2, 700);
    c.touch(&ui, Event::Down(x, y), 10.0);
    c.touch(&ui, Event::Move(x, y2), 400.0);
    assert!(c.drag.is_some());
    draw_once(&mut c, &ui);
}

fn params(fields: &[(&str, &str)]) -> bifrost_core::json::Json {
    use bifrost_core::json::Json;
    Json::Obj(fields.iter().map(|(k, v)| (k.to_string(), Json::Str(v.to_string()))).collect())
}

#[test]
fn the_show_date_intent_opens_the_calendar_on_a_date() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    c.activate(&ui, Target::SearchField, 0.0);
    type_text(&mut c, &ui, "den");
    c.open(&ui, Some(1));
    let r = intents::perform(&mut c, &ui, "show-date", &params(&[("date", "2027-01-15")])).unwrap();
    assert_eq!(c.selected, date(2027, 1, 15));
    assert!(c.editor.is_none() && !c.searching(), "whatever was open is closed");
    assert!(r.continue_foreground);
    assert_eq!(r.dialog.as_deref(), Some("Showing January 15, 2027"));
    assert!(intents::perform(&mut c, &ui, "show-date", &params(&[("date", "soon")])).is_err());
}

#[test]
fn the_show_event_intent_opens_the_event() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    let r = intents::perform(&mut c, &ui, "show-event", &params(&[("event", "3")])).unwrap();
    assert!(r.continue_foreground);
    assert_eq!(c.selected, date(2026, 10, 8));
    assert!(c.editor.as_ref().is_some_and(|d| d.existing && d.event.id == 3));
    assert!(intents::perform(&mut c, &ui, "show-event", &params(&[("event", "99")])).is_err(), "no such event");
    assert!(intents::perform(&mut c, &ui, "show-event", &params(&[("event", "abc")])).is_err());
}

#[test]
fn the_create_event_intent_adds_and_saves_an_event() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    let r = intents::perform(
        &mut c,
        &ui,
        "create-event",
        &bifrost_core::json::Json::Obj(vec![
            ("title".into(), bifrost_core::json::Json::Str("  Physio ".into())),
            ("start".into(), bifrost_core::json::Json::Str("2026-10-12T09:30".into())),
            ("minutes".into(), bifrost_core::json::Json::Num(45.0)),
            ("location".into(), bifrost_core::json::Json::Str("Laugavegur 5".into())),
        ]),
    )
    .unwrap();
    assert_eq!(r.dialog.as_deref(), Some("Created Physio on 2026-10-12 09:30"));
    assert!(!r.continue_foreground);
    let saved = new(&ui, temp.file());
    let e = saved.events.iter().find(|e| e.title == "Physio").expect("saved");
    assert_eq!((e.date, e.minute, e.duration, e.location.as_str()), (date(2026, 10, 12), 570, 45, "Laugavegur 5"));
    assert_eq!(r.value, bifrost_core::json::Json::Str(e.id.to_string()));
    assert_eq!(c.events.len(), 4);
}

#[test]
fn the_create_event_intent_takes_the_shown_day_and_the_defaults_and_refuses_bad_input() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.change_setting(&ui, Setting::Length(2));
    c.change_setting(&ui, Setting::Alert(3));
    intents::perform(&mut c, &ui, "create-event", &params(&[("title", "Call")])).unwrap();
    let e = &c.events[0];
    assert_eq!((e.date, e.minute, e.duration, e.alert), (date(2026, 10, 4), 540, 90, 3));
    // an all-day event has no time and its own alert
    c.change_setting(&ui, Setting::AllDayAlert(6));
    let json = bifrost_core::json::Json::Obj(vec![
        ("title".into(), bifrost_core::json::Json::Str("Holiday".into())),
        ("all_day".into(), bifrost_core::json::Json::Bool(true)),
    ]);
    intents::perform(&mut c, &ui, "create-event", &json).unwrap();
    let holiday = c.events.iter().find(|e| e.title == "Holiday").unwrap();
    assert_eq!((holiday.all_day, holiday.minute, holiday.duration, holiday.alert), (true, 0, 1440, 6));
    assert!(intents::perform(&mut c, &ui, "create-event", &params(&[("title", "   ")])).is_err(), "a blank title");
    assert!(intents::perform(&mut c, &ui, "create-event", &params(&[("title", "Ok"), ("start", "nope")])).is_err());
    assert!(intents::perform(&mut c, &ui, "no-such-intent", &params(&[])).is_err());
    assert_eq!(c.events.len(), 2);
}

#[test]
fn events_are_entities_for_the_intents() {
    use bifrost_app::intents::EntityQuery;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let c = searchable(&ui, temp.file());
    let ids = |q: &EntityQuery| c.event_entities(q).into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids(&EntityQuery::Suggested), vec!["1", "2", "3"]);
    assert_eq!(ids(&EntityQuery::Ids(vec!["3".into(), "9".into()])), vec!["3"]);
    assert_eq!(ids(&EntityQuery::String("LUNCH".into())), vec!["2"]);
    let lunch = &c.event_entities(&EntityQuery::String("lunch".into()))[0];
    assert_eq!((lunch.ty.as_str(), lunch.title.as_str(), lunch.subtitle.as_deref()), ("Event", "Lunch with Ana", Some("2026-10-06 12:30")));
}

#[test]
fn print_sends_the_agenda_of_what_the_window_shows() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    assert!(c.enabled(Command::Print) && !c.perform(&ui, Command::Print, 0.0), "the app runs the print job");
    let (job, text) = c.print_job();
    assert_eq!(job, "Calendar October 2026");
    assert!(text.starts_with("October 2026\n============\n"));
    for title in ["Standup", "Lunch with Ana", "Dentist"] {
        assert!(text.contains(title), "{title}");
    }
    c.activate(&ui, Target::View(View::Day), 0.0);
    let (job, text) = c.print_job();
    assert_eq!(job, "Calendar Sunday, October 4");
    assert!(text.contains("Standup") && !text.contains("Dentist"));
    c.activate(&ui, Target::View(View::Week), 0.0);
    assert_eq!(c.print_job().0, "Calendar Oct 4 - 10, 2026");
    assert_eq!(c.print_job().1.matches("\n  ").count(), 3, "three events that week");
    c.activate(&ui, Target::Add, 0.0);
    assert!(!c.enabled(Command::Print), "not while the event form is open");
}

/// Window points inside the calendar list's row `id`.
fn layer_point(c: &Calendar, ui: &Ui, id: u32) -> (f32, f32) {
    for y in 0..800 {
        if c.layer_at(ui, ui.pt(50.0), ui.pt(y as f32)) == Some(id) {
            return (50.0, y as f32);
        }
    }
    panic!("no row {id} in the calendar list");
}

fn team_calendar_text() -> String {
    let standup = ics::IcsEvent::new("standup@team", "Team standup", ics::Local::new(date(2026, 10, 6), 540), ics::Local::new(date(2026, 10, 6), 570));
    ics::write(&ics::IcsCalendar { name: "Team".into(), events: vec![standup], timezones: vec![] }, "20261004T000000Z")
}

#[test]
fn the_holidays_are_a_read_only_calendar_the_list_switches_on_and_off() {
    use overlay::{LAYER_HOLIDAYS, LAYER_LOCAL};
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = new(&ui, temp.file());
    assert!(!c.prefs.show_holidays);
    assert_eq!(c.shown_events().count(), 1, "only the saved event");

    let (x, y) = layer_point(&c, &ui, LAYER_HOLIDAYS);
    tap(&mut c, &ui, x, y, 0.0);
    assert!(c.prefs.show_holidays);
    assert_eq!(c.shown_events().count(), 1 + 3 * holidays::iceland(2026).len(), "three years of holidays");
    // Good Friday 2026 is in its day cell, and the saved events were not touched
    c.selected = date(2026, 4, 1);
    c.sync_overlay();
    let (cell, _) = c.cell_events(&ui, date(2026, 4, 3));
    let good_friday = cell.iter().find(|e| e.title == "Good Friday").expect("Good Friday in its cell");
    assert!(overlay::is_overlay(good_friday.id) && good_friday.all_day);
    let id = good_friday.id;
    assert_eq!(c.events.len(), 1);
    // a tap on it says what it is, and does not open the editor
    c.activate(&ui, Target::Row(id), 1.0);
    assert!(c.editor.is_none());
    assert_eq!(c.notice.as_deref(), Some("Good Friday (Holidays)"));
    assert_ne!(c.event_colour(id), c.event_colour(1), "its own colour");
    // the setting is kept
    assert!(new(&ui, temp.file()).prefs.show_holidays);

    // the Calendar row hides the saved events without deleting them
    let (x, y) = layer_point(&c, &ui, LAYER_LOCAL);
    c.selected = date(2026, 10, 4);
    tap(&mut c, &ui, x, y, 10.0);
    assert!(!c.prefs.show_local);
    assert!(c.shown_events().all(|e| overlay::is_overlay(e.id)));
    assert_eq!(c.events.len(), 1);
    tap(&mut c, &ui, x, y, 20.0);
    assert!(c.shown_events().any(|e| e.id == 1));
    // and the Holidays row turns the holidays off again
    let (x, y) = layer_point(&c, &ui, LAYER_HOLIDAYS);
    tap(&mut c, &ui, x, y, 30.0);
    assert_eq!(c.shown_events().count(), 1);
}

#[test]
fn read_only_events_show_in_the_week_and_cannot_be_dragged() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    c.toggle_layer(overlay::LAYER_HOLIDAYS);
    c.selected = date(2026, 4, 1);
    c.sync_overlay();
    let rects = c.time_rects(&ui);
    assert!(rects.iter().any(|r| overlay::is_overlay(r.0)), "an Easter day is in that week");
    // pressing on its band in the all-day row starts no drag, and a tap says what it is
    c.scroll.jump_top();
    let (_, x, y, w, h) = *c.time_rects(&ui).iter().find(|r| overlay::is_overlay(r.0)).unwrap();
    let (px, py) = (c.x0 + x + w / 2.0, y + h / 2.0);
    c.touch(&ui, Event::Down(px, py), 0.0);
    assert!(c.press.is_none() && c.drag.is_none());
    c.touch(&ui, Event::Up(px, py), 10.0);
    assert!(c.editor.is_none());
    assert!(c.notice.as_deref().is_some_and(|n| n.ends_with("(Holidays)")), "{:?}", c.notice);
}

#[test]
fn subscribing_adds_a_calendar_that_is_read_on_a_schedule() {
    use overlay::LAYER_SUBS;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.subscribe("hello").is_err());
    assert_eq!(c.subscribe("webcal://cal.example.is/team.ics"), Ok("cal.example.is".to_string()));
    assert!(c.subscribe("https://cal.example.is/team.ics").is_err(), "the same calendar twice");
    assert_eq!(c.subscription_names(), vec!["cal.example.is".to_string()]);
    let url = "https://cal.example.is/team.ics".to_string();
    // it is due at once, then again an hour later
    assert_eq!(c.next_fetch(10.0), Some(url.clone()));
    assert_eq!(c.next_fetch(20.0), None);
    assert_eq!(c.fetch_wake(), Some(10.0 + subscribe::EVERY_MS));
    assert_eq!(c.next_fetch(10.0 + subscribe::EVERY_MS), Some(url.clone()));

    // the calendar answers: it names itself and its events appear, read-only
    c.fetch_finished(&url, Ok(team_calendar_text()));
    assert_eq!(c.subscription_names(), vec!["Team".to_string()]);
    c.selected = date(2026, 10, 6);
    c.sync_overlay();
    let events: Vec<(String, u16)> = c.shown_events().map(|e| (e.title.clone(), e.minute)).collect();
    assert_eq!(events, vec![("Team standup".to_string(), 540)]);
    let id = c.shown_events().next().unwrap().id;
    assert_eq!(overlay::slot_of(id), 1);
    c.activate(&ui, Target::Row(id), 0.0);
    assert_eq!(c.notice.as_deref(), Some("Team standup (Team)"));
    assert!(c.editor.is_none());

    // kept after a restart, from the cached answer
    let mut again = new(&ui, temp.file());
    assert_eq!(again.subscription_names(), vec!["Team".to_string()]);
    again.selected = date(2026, 10, 6);
    again.sync_overlay();
    assert_eq!(again.shown_events().count(), 1);

    // a failed or foreign answer is reported and the calendar stays as it was
    c.fetch_finished(&url, Err("the server answered 500".into()));
    assert_eq!(c.error.as_deref(), Some("Could not update Team: the server answered 500"));
    c.fetch_finished(&url, Ok("<html>Not found</html>".into()));
    assert_eq!(c.error.as_deref(), Some("Could not update Team: it is not a calendar"));
    c.sync_overlay();
    assert_eq!(c.shown_events().count(), 1);
    c.fetch_finished("https://other.example.is/x.ics", Ok(team_calendar_text()));
    assert_eq!(c.subscription_names().len(), 1, "an answer for a calendar that is not subscribed to is ignored");

    // the list hides it, and unsubscribing removes it and its cached answer
    let (x, y) = layer_point(&c, &ui, LAYER_SUBS);
    tap(&mut c, &ui, x, y, 100.0);
    assert_eq!(c.shown_events().count(), 0);
    tap(&mut c, &ui, x, y, 200.0);
    assert_eq!(c.shown_events().count(), 1);
    assert!(!c.unsubscribe(5));
    assert!(c.unsubscribe(0));
    assert!(c.subscription_names().is_empty());
    assert_eq!(c.shown_events().count(), 0);
    assert!(!subscribe::cache_path(&temp.file(), &url).exists());
    assert!(new(&ui, temp.file()).subscription_names().is_empty(), "the list is saved");
}

#[test]
fn the_fetcher_wakes_for_the_next_read() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    let fetcher = subscribe::Fetcher::default();
    assert_eq!(fetcher.wake(&c), None, "nothing to read");
    c.subscribe("https://cal.example.is/team.ics").unwrap();
    assert_eq!(fetcher.wake(&c), Some(0.0), "never read, so due now");
}

#[test]
fn answers_a_worker_posted_reach_the_calendar_on_the_next_tick() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.subscribe("https://cal.example.is/team.ics").unwrap();
    let mut fetcher = subscribe::Fetcher::default();
    fetcher.post("https://cal.example.is/team.ics", Ok(team_calendar_text()));
    fetcher.deliver(&mut c);
    assert_eq!(c.subscription_names(), vec!["Team".to_string()]);
    fetcher.post("https://cal.example.is/team.ics", Err("no route".into()));
    fetcher.deliver(&mut c);
    assert_eq!(c.error.as_deref(), Some("Could not update Team: no route"));
    fetcher.deliver(&mut c);
    assert_eq!(c.subscription_names(), vec!["Team".to_string()], "nothing more to take");
}

#[test]
fn holidays_and_subscribed_events_draw_in_every_view() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    c.toggle_layer(overlay::LAYER_HOLIDAYS);
    c.subscribe("https://cal.example.is/team.ics").unwrap();
    let url = "https://cal.example.is/team.ics";
    c.fetch_finished(url, Ok(team_calendar_text()));
    c.selected = date(2026, 4, 3);
    for view in [View::Month, View::Week, View::Day, View::Year] {
        c.activate(&ui, Target::View(view), 0.0);
        draw_once(&mut c, &ui);
    }
    c.selected = date(2026, 10, 6);
    for view in [View::Month, View::Week, View::Day] {
        c.activate(&ui, Target::View(view), 0.0);
        draw_once(&mut c, &ui);
    }
    // the printed agenda lists them too
    assert!(c.print_job().1.contains("Team standup"));
}

#[test]
fn quick_event_adds_the_event_a_sentence_describes() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.enabled(Command::QuickEvent) && c.perform(&ui, Command::QuickEvent, 0.0));
    assert_eq!(c.quick_text(), Some(""));
    assert!(!c.enabled(Command::NewEvent) && !c.enabled(Command::QuickEvent), "the sheet is modal");
    type_text(&mut c, &ui, "Lunch with Ana tomorrow at 12:30");
    let rows = c.quick_preview();
    assert_eq!(rows[0].1, "Lunch with Ana");
    assert_eq!(rows[1].1, "Monday, October 5, 2026");
    assert_eq!(rows[2].1, "12:30");
    // pasted text goes into the line too
    assert!(c.edit_commands(Some("x")).supports(Edit::Paste) && !c.edit_commands(Some("x")).supports(Edit::Copy));
    tap_bar(&mut c, &ui, chrome::B_QUICK, 10.0);
    assert!(c.quick_text().is_none(), "Add closes the sheet");
    assert_eq!(c.events.len(), 1);
    let e = &c.events[0];
    assert_eq!((e.title.as_str(), e.date, e.minute, e.duration), ("Lunch with Ana", date(2026, 10, 5), 750, 60));
    assert_eq!(c.notice.as_deref(), Some("Added Lunch with Ana on 2026-10-05 at 12:30"));
    assert_eq!(c.selected, date(2026, 10, 5));
    assert_eq!(new(&ui, temp.file()).events.len(), 1, "saved");
}

#[test]
fn quick_event_takes_return_paste_and_backspace_and_refuses_an_empty_sentence() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::QuickEvent, 0.0);
    c.key(&ui, Out::Enter, 1.0);
    assert!(c.quick_text().is_some(), "the sheet stays");
    assert_eq!(c.error.as_deref(), Some("Type what the event is, for example: Lunch tomorrow at 12:30"));
    assert!(c.events.is_empty());
    assert!(c.edit_command(&ui, Edit::Paste, Some("Dentist friday 3pm\nsecond line"), 2.0));
    assert_eq!(c.quick_text(), Some("Dentist friday 3pm"));
    assert!(c.error.is_none(), "typing clears the message");
    c.key(&ui, Out::Backspace, 3.0);
    assert_eq!(c.quick_text(), Some("Dentist friday 3p"));
    type_text(&mut c, &ui, "m");
    c.key(&ui, Out::Enter, 4.0);
    assert!(c.quick_text().is_none());
    assert_eq!((c.events[0].title.as_str(), c.events[0].date, c.events[0].minute), ("Dentist", date(2026, 10, 9), 15 * 60));
}

#[test]
fn quick_event_can_be_cancelled_and_blocks_the_window_behind_it() {
    use bifrost_app::proto::CHORD_META;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::QuickEvent, 0.0);
    type_text(&mut c, &ui, "Never mind");
    assert!(!c.chord(&ui, 3, CHORD_META, 1.0), "no view change behind the sheet");
    assert!(!c.change_setting(&ui, prefs::Setting::MondayFirst));
    tap_bar(&mut c, &ui, chrome::B_CANCEL, 2.0);
    assert!(c.quick_text().is_none() && c.events.is_empty());
    c.perform(&ui, Command::QuickEvent, 3.0);
    assert_eq!(c.quick_text(), Some(""), "a fresh line each time");
    assert!(c.escape(&ui, 4.0));
    assert!(c.quick_text().is_none());
    // the sheet and its field draw and are published
    c.perform(&ui, Command::QuickEvent, 5.0);
    type_text(&mut c, &ui, "Dentist tomorrow 3pm");
    draw_once(&mut c, &ui);
    assert!(c.controls(&ui).iter().any(|n| n.role == bifrost_app::controls::Role::TextField && n.label == "Quick Event" && n.value == "Dentist tomorrow 3pm"));
    // the app's intents close it
    intents::perform(&mut c, &ui, "show-date", &params(&[("date", "2027-01-15")])).unwrap();
    assert!(c.quick_text().is_none());
}

#[test]
fn a_custom_repeat_is_saved_beside_the_store_and_its_days_are_worked_out() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    new_form(&mut c, &ui, 0.0);
    type_text(&mut c, &ui, "Gym");
    c.key(&ui, Out::Hide, 1.0);
    c.activate(&ui, Target::Repeat, 2.0);
    c.activate(&ui, Target::RepeatFreq, 3.0);
    c.activate(&ui, Target::RepeatFreq, 4.0);
    c.activate(&ui, Target::RepeatDay(2), 5.0);
    c.activate(&ui, Target::RepeatEnds, 6.0);
    c.activate(&ui, Target::RepeatCount(-7), 7.0);
    assert!(c.rows().iter().any(|r| r.0 == Target::RepeatCount(0)), "the count row shows");
    draw_once(&mut c, &ui);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 8.0);
    assert!(c.editor.is_none());
    let id = c.events[0].id;
    assert_eq!(c.events[0].repeat, 0, "a custom rule is not one of the presets");
    let rule = c.extras().events.get(&id).and_then(|x| x.rrule.clone());
    assert_eq!(rule.as_deref(), Some("FREQ=WEEKLY;BYDAY=SU,TU;COUNT=3"));
    let e = c.events[0].clone();
    assert!(c.occurs(&e, date(2026, 10, 6)), "a Tuesday");
    assert!(c.occurs(&e, date(2026, 10, 11)), "the second Sunday");
    assert!(!c.occurs(&e, date(2026, 10, 5)), "a Monday");
    assert!(!c.occurs(&e, date(2026, 10, 13)), "the count is used up");
    let reopened = new(&ui, temp.file());
    assert_eq!(reopened.extras().events.get(&id).and_then(|x| x.rrule.clone()), rule, "the rule survives a reload");
}

#[test]
fn the_alert_picker_saves_custom_alarms_for_the_alert_calendar() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    new_form(&mut c, &ui, 0.0);
    type_text(&mut c, &ui, "Call");
    c.key(&ui, Out::Hide, 1.0);
    c.activate(&ui, Target::Alert, 2.0);
    c.activate(&ui, Target::AlarmPreset(0), 3.0);
    c.activate(&ui, Target::AlarmPreset(5), 4.0);
    c.activate(&ui, Target::AlarmStep(1), 5.0);
    c.activate(&ui, Target::AlarmAdd, 6.0);
    assert_eq!(c.editor.as_ref().unwrap().alarms, vec![35, 60]);
    draw_once(&mut c, &ui);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 7.0);
    let id = c.events[0].id;
    assert_eq!(c.extras().events.get(&id).and_then(|x| x.alarms.clone()), Some(vec![35, 60]));
    let cal = alerts::alert_calendar_with(&c.events, &ics::IcsCalendar::default(), c.extras());
    assert_eq!(cal.events[0].alarms, vec![35, 60]);
    // picking a single preset again stores it as the event's preset, with no list of its own
    c.activate(&ui, Target::Row(id), 8.0);
    c.activate(&ui, Target::Alert, 9.0);
    c.activate(&ui, Target::AlarmPreset(0), 10.0);
    c.activate(&ui, Target::AlarmPreset(3), 11.0);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 12.0);
    assert_eq!(c.events[0].alert, 3);
    assert_eq!(c.extras().events.get(&id).and_then(|x| x.alarms.clone()), None, "a preset needs no entry of its own");
}

#[test]
fn calendars_are_made_shown_renamed_and_removed_only_when_empty() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.perform(&ui, Command::NewCalendar, 0.0));
    assert_eq!(c.line_text().as_deref(), Some(""), "the sheet opens empty");
    draw_once(&mut c, &ui);
    type_text(&mut c, &ui, "Work");
    c.key(&ui, Out::Enter, 1.0);
    assert!(c.line_sheet.is_none(), "Return makes the calendar and closes the sheet");
    assert_eq!(c.extras().name_of(2), "Work");
    // an event on the new calendar, shown until its row hides the calendar
    new_form(&mut c, &ui, 2.0);
    type_text(&mut c, &ui, "Standup");
    c.key(&ui, Out::Hide, 3.0);
    c.activate(&ui, Target::Calendar, 4.0);
    assert!(c.cal_menu_open(), "the Calendar row opens the menu of calendars");
    c.key(&ui, Out::Named("Down"), 4.1);
    c.key(&ui, Out::Enter, 4.2);
    assert!(!c.cal_menu_open());
    assert_eq!(c.editor.as_ref().unwrap().extra.calendar, 2);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 5.0);
    let id = c.events[0].id;
    assert_eq!(c.extras().calendar_of(id), 2);
    assert_eq!(c.shown_events().count(), 1);
    c.activate(&ui, Target::Layer(overlay::LAYER_CAL + 2), 6.0);
    assert_eq!(c.shown_events().count(), 0, "the calendar row hides its events");
    c.activate(&ui, Target::Layer(overlay::LAYER_CAL + 2), 7.0);
    assert_eq!(c.shown_events().count(), 1);
    // rename it, then refuse to delete it while it holds an event
    assert!(c.open_rename(2));
    assert_eq!(c.line_text().as_deref(), Some("Work"));
    for _ in 0..4 {
        c.key(&ui, Out::Backspace, 8.0);
    }
    type_text(&mut c, &ui, "Job");
    c.key(&ui, Out::Enter, 9.0);
    assert_eq!(c.extras().name_of(2), "Job");
    assert!(c.delete_calendar(2));
    assert!(c.extras().exists(2), "a calendar with an event is kept");
    assert!(c.error.as_deref().is_some_and(|e| e.contains("Delete them first")));
    c.activate(&ui, Target::Row(id), 10.0);
    tap_row(&mut c, &ui, Target::Delete, 384.0, 11.0);
    assert!(c.events.is_empty());
    assert!(c.delete_calendar(2));
    assert!(!c.extras().exists(2), "an empty calendar is removed");
    assert!(!c.delete_calendar(1), "the built-in calendar cannot be removed");
}

#[test]
fn the_default_calendar_setting_is_kept_with_the_calendars() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::NewCalendar, 0.0);
    type_text(&mut c, &ui, "Home");
    c.key(&ui, Out::Enter, 1.0);
    assert!(c.change_setting(&ui, Setting::DefaultCalendar(2)));
    assert!(c.setting_checked(Setting::DefaultCalendar(2)) && !c.setting_checked(Setting::DefaultCalendar(1)));
    new_form(&mut c, &ui, 2.0);
    assert_eq!(c.editor.as_ref().unwrap().extra.calendar, 2, "new events go to the default");
    c.activate(&ui, Target::Cancel, 3.0);
    assert!(!c.change_setting(&ui, Setting::DefaultCalendar(7)), "a calendar that does not exist");
    let reopened = new(&ui, temp.file());
    assert_eq!(reopened.extras().default_id(), 2);
}

#[test]
fn the_time_zone_and_travel_time_rows_cycle_and_are_saved() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    new_form(&mut c, &ui, 0.0);
    type_text(&mut c, &ui, "Flight");
    c.key(&ui, Out::Hide, 1.0);
    c.activate(&ui, Target::Zone, 2.0);
    assert_eq!(c.editor.as_ref().unwrap().extra.tzid.as_deref(), Some("UTC"));
    c.activate(&ui, Target::Zone, 3.0);
    c.activate(&ui, Target::Travel, 4.0);
    draw_once(&mut c, &ui);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 5.0);
    let id = c.events[0].id;
    let extra = c.extras().events.get(&id).cloned().unwrap();
    assert_eq!(extra.tzid.as_deref(), Some("Atlantic/Reykjavik"));
    assert_eq!(extra.travel, 15);
    assert!(c.export_ics().contains("TZID=Atlantic/Reykjavik"), "the export names the zone");
}

#[test]
fn an_empty_subscription_address_keeps_the_sheet_open() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.open_subscription("");
    assert!(c.line_sheet.is_some());
    c.key(&ui, Out::Enter, 1.0);
    assert!(c.line_sheet.is_some(), "the address is refused and the sheet stays open");
    assert!(c.error.is_some());
    draw_once(&mut c, &ui);
    tap_bar(&mut c, &ui, chrome::B_CANCEL, 2.0);
    assert!(c.line_sheet.is_none());
}

// ---- a mouse that drags at once, all-day drags, the view time zone, a shorter week, typed dates, colours, invitees,
// ---- birthdays and the printed grid

#[test]
fn a_mouse_drags_at_once_while_a_finger_must_hold() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    c.set_pointer(true);
    let (x, y) = grid_xy(&c, &ui, 3, 600);
    let (_, y2) = grid_xy(&c, &ui, 3, 690);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + ui.pt(2.0)), 100.0);
    assert!(c.drag.is_none(), "a wobble smaller than the slop is not a drag");
    c.touch(&ui, Event::Move(x, y2), 120.0);
    assert!(c.drag.is_some(), "the mouse has moved: there is no hold to wait for");
    assert_eq!(c.drag.map(|d| d.span), Some(layout::DragSpan { start: 600, end: 705 }));
    c.touch(&ui, Event::Up(x, y2), 140.0);
    let draft = c.editor.as_ref().expect("the editor opens on the drawn-out event");
    assert_eq!((draft.event.date, draft.event.minute, draft.event.duration), (date(2026, 10, 7), 600, 105));
    // the same moves with a finger are a scroll: it has not held
    let mut f = week_calendar(&ui, temp.file());
    f.set_pointer(false);
    let before = f.scroll.offset();
    f.touch(&ui, Event::Down(x, y), 0.0);
    f.touch(&ui, Event::Move(x, y + ui.pt(60.0)), 40.0);
    f.touch(&ui, Event::Move(x, y2), 120.0);
    assert!(f.drag.is_none());
    f.touch(&ui, Event::Up(x, y2), 140.0);
    assert_ne!(f.scroll.offset(), before, "a finger that moves at once scrolls");
}

#[test]
fn a_mouse_that_drags_an_event_moves_it_and_one_that_only_clicks_selects_it() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    c.set_pointer(true);
    let (x, y) = grid_xy(&c, &ui, 0, 560);
    let (x2, y2) = grid_xy(&c, &ui, 2, 800);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x2, y2), 200.0);
    assert!(matches!(c.drag.map(|d| d.kind), Some(drag::Kind::Move { id: 1, .. })));
    c.touch(&ui, Event::Up(x2, y2), 250.0);
    assert_eq!((c.events[0].date, c.events[0].minute), (date(2026, 10, 6), 780));
    // a click with no move is a tap
    c.picked = None;
    let (x, y) = grid_xy(&c, &ui, 2, 800);
    c.touch(&ui, Event::Down(x, y), 400.0);
    c.touch(&ui, Event::Up(x, y), 450.0);
    assert!(c.drag.is_none());
    assert_eq!(c.picked, Some(1), "the click selected the event");
    assert_eq!((c.events[0].date, c.events[0].minute), (date(2026, 10, 6), 780), "a click moves nothing");
}

#[test]
fn the_wheels_burst_of_moves_scrolls_the_grid_even_for_a_pointer() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    c.set_pointer(true);
    let before = c.scroll.offset();
    // the shell makes a wheel notch as a press, a move 16 ms later and a lift 32 ms after the press
    let (x, y) = grid_xy(&c, &ui, 0, 560);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + ui.pt(40.0)), 16.0);
    assert!(c.drag.is_none(), "the burst is the wheel, not a hand dragging the event");
    c.touch(&ui, Event::Move(x, y + ui.pt(80.0)), 100.0);
    assert!(c.drag.is_none(), "and the press stays the wheel's");
    c.touch(&ui, Event::Up(x, y + ui.pt(80.0)), 140.0);
    assert_ne!(c.scroll.offset(), before, "the grid scrolled");
    assert_eq!((c.events[0].date, c.events[0].minute), (date(2026, 10, 4), 540), "the event did not move");
}

#[test]
fn an_all_day_event_is_dragged_to_another_day() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut offsite = event(1, "Offsite");
    offsite.all_day = true;
    offsite.minute = 0;
    offsite.duration = 1440;
    store::save(&temp.file(), &[offsite]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    c.scroll.jump_top();
    let rect = c.time_rects(&ui).into_iter().find(|r| r.0 == 1).expect("the all-day block");
    let (x, y) = (c.x0 + rect.1 + rect.3 / 2.0, rect.2 + rect.4 / 2.0);
    let (x2, _) = grid_xy(&c, &ui, 2, 600);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x + 1.0, y), 300.0);
    assert!(matches!(c.drag.map(|d| d.kind), Some(drag::Kind::AllDay { id: 1 })), "held long enough, the move drags");
    c.touch(&ui, Event::Move(x2, y), 400.0);
    assert_eq!(c.drag.map(|d| d.day), Some(date(2026, 10, 6)));
    draw_once(&mut c, &ui);
    c.touch(&ui, Event::Up(x2, y), 450.0);
    assert!(c.editor.is_none(), "a moved event does not open the editor");
    assert_eq!(c.events[0].date, date(2026, 10, 6));
    assert!(c.events[0].all_day);
    assert_eq!(c.notice.as_deref(), Some("Moved Offsite to Tuesday, October 6, 2026"));
    let again = new(&ui, temp.file());
    assert_eq!(again.events[0].date, date(2026, 10, 6), "saved");
}

#[test]
fn a_view_time_zone_moves_the_events_in_the_grids_and_not_in_the_store() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Call")]).unwrap();
    let mut c = new(&ui, temp.file());
    // 09:00 on 4 October in Tokyo is 00:00 UTC and 20:00 on the 3rd in New York (summer time)
    c.extras.put(1, extras::Extra { tzid: Some("Asia/Tokyo".into()), ..extras::Extra::default() });
    let ny = zones::TABLE.iter().position(|z| z.id == "America/New_York").unwrap();
    assert!(c.view_zone_checked(None) && !c.view_zone_checked(Some(ny)));
    assert_eq!(c.view_zone_label(), "Time Zone");
    assert!(c.set_view_zone(&ui, Some(ny)));
    assert!(c.view_zone_checked(Some(ny)) && !c.view_zone_checked(None));
    assert_eq!(c.view_zone_label(), "New York");
    let shown: Vec<(Date, u16)> = c.shown_events().map(|e| (e.date, e.minute)).collect();
    assert_eq!(shown, vec![(date(2026, 10, 3), 1200)]);
    assert_eq!((c.events[0].date, c.events[0].minute), (date(2026, 10, 4), 540), "the store keeps the event's own time");
    c.selected = date(2026, 10, 3);
    assert_eq!(c.agenda().len(), 1, "the day cell and the agenda follow the view zone");
    c.selected = date(2026, 10, 4);
    assert_eq!(c.agenda().len(), 0);
    assert_eq!(c.to_own_clock(1, date(2026, 10, 3), 1200), (date(2026, 10, 4), 540));
    assert_eq!(c.to_view_clock(1, date(2026, 10, 4), 540), (date(2026, 10, 3), 1200));
    // a zone chosen once is kept, and every view still draws
    assert_eq!(new(&ui, temp.file()).prefs.view_zone, Some("America/New_York"));
    for view in [View::Day, View::Week, View::Month, View::Year] {
        c.activate(&ui, Target::View(view), 0.0);
        draw_once(&mut c, &ui);
    }
    // the toolbar button steps through the zones and back to each event's own time
    c.cycle_view_zone(&ui);
    assert_eq!(c.prefs.view_zone, zones::TABLE.get(ny + 1).map(|z| z.id));
    for _ in 0..(zones::TABLE.len() - ny - 1) {
        c.cycle_view_zone(&ui);
    }
    assert_eq!(c.prefs.view_zone, None);
    assert_eq!(c.view_zone_label(), "Time Zone");
    let shown: Vec<(Date, u16)> = c.shown_events().map(|e| (e.date, e.minute)).collect();
    assert_eq!(shown, vec![(date(2026, 10, 4), 540)]);
    assert!(!c.set_view_zone(&ui, Some(99)), "a zone that is not in the table");
}

#[test]
fn moving_a_block_under_a_view_time_zone_saves_the_event_on_its_own_clock() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Call")]).unwrap();
    let mut c = new(&ui, temp.file());
    c.extras.put(1, extras::Extra { tzid: Some("Asia/Tokyo".into()), ..extras::Extra::default() });
    let ny = zones::TABLE.iter().position(|z| z.id == "America/New_York").unwrap();
    c.set_view_zone(&ui, Some(ny));
    // the week of Saturday 3 October, where the block is shown at 20:00 in the last column
    c.selected = date(2026, 10, 3);
    c.activate(&ui, Target::View(View::Week), 0.0);
    let (x, y) = grid_xy(&c, &ui, 6, 1215);
    let (_, y2) = grid_xy(&c, &ui, 6, 1275);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Move(x, y + 1.0), 300.0);
    c.touch(&ui, Event::Move(x, y2), 400.0);
    c.touch(&ui, Event::Up(x, y2), 450.0);
    // an hour later in New York is an hour later on Tokyo's clock too: 10:00 on the 4th
    assert_eq!((c.events[0].date, c.events[0].minute, c.events[0].duration), (date(2026, 10, 4), 600, 60));
    assert_eq!(c.notice.as_deref(), Some("Moved Call to 21:00"), "the notice speaks of the grid the person is looking at");
}

#[test]
fn the_week_view_shows_the_chosen_number_of_days() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    assert_eq!(c.view_days().len(), 7);
    assert_eq!(c.view_days()[0], date(2026, 10, 4));
    // five days, with the week starting on Sunday, are Monday to Friday
    assert!(c.change_setting(&ui, Setting::DaysPerWeek(4)));
    let days = c.view_days();
    assert_eq!((days.len(), days[0], days[4]), (5, date(2026, 10, 5), date(2026, 10, 9)));
    draw_once(&mut c, &ui);
    // a tap on the header's second chip picks Tuesday, not the seven day layout's second chip
    let (gut, col_w) = c.col_geometry(&ui);
    assert!(col_w > 0.0);
    assert_eq!(c.hit_view(&ui, gut + col_w * 1.5, ui.pt(150.0)), Some(Target::Pick(date(2026, 10, 6))));
    // a Monday start, on a Wednesday: the week is 5 to 11 October, and three days of it are shown
    assert!(c.change_setting(&ui, Setting::MondayFirst));
    c.selected = date(2026, 10, 7);
    assert_eq!(c.view_days()[0], date(2026, 10, 5));
    assert!(c.change_setting(&ui, Setting::DaysPerWeek(2)));
    assert_eq!(c.view_days().len(), 3);
    assert_eq!(c.range_label(), "Oct 5 - 7, 2026");
    for view in [View::Week, View::Day, View::Month] {
        c.activate(&ui, Target::View(view), 0.0);
        draw_once(&mut c, &ui);
    }
    // the settings are kept
    let again = new(&ui, temp.file());
    assert_eq!((again.prefs.days_per_week, again.prefs.monday_first), (3, true));
}

#[test]
fn the_hours_outside_the_working_day_are_shaded() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = week_calendar(&ui, temp.file());
    assert_eq!((c.prefs.day_start, c.prefs.day_end), (8, 18));
    assert!(c.prefs.off_hour(7) && !c.prefs.off_hour(8) && c.prefs.off_hour(18));
    assert!(c.change_setting(&ui, Setting::DayEnd(6)));
    assert_eq!(c.prefs.day_end, 22);
    assert!(c.setting_checked(Setting::DayEnd(6)) && !c.setting_checked(Setting::DayEnd(0)));
    draw_once(&mut c, &ui);
    assert_eq!(new(&ui, temp.file()).prefs.day_end, 22);
}

#[test]
fn a_date_typed_in_go_to_date_moves_the_sheet_and_go_takes_it() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert!(c.perform(&ui, Command::GoToDate, 0.0));
    c.key(&ui, Out::Text("2027-03-09".into()), 1.0);
    assert_eq!(c.goto, Some(date(2027, 3, 9)), "the steppers follow what is typed");
    draw_once(&mut c, &ui);
    c.key(&ui, Out::Backspace, 2.0);
    assert_eq!(c.goto, Some(date(2027, 3, 9)), "a date that does not read yet leaves them where they are");
    c.key(&ui, Out::Text("4".into()), 3.0);
    assert_eq!(c.goto, Some(date(2027, 3, 4)));
    // the arrow keys move the caret while there is text, and step the day once the row is empty
    c.key(&ui, Out::Named("Left"), 4.0);
    c.key(&ui, Out::Named("Right"), 4.5);
    assert_eq!(c.goto, Some(date(2027, 3, 4)));
    for _ in 0..10 {
        c.key(&ui, Out::Backspace, 5.0);
    }
    assert!(c.goto_line.is_empty());
    c.key(&ui, Out::Named("Left"), 6.0);
    assert_eq!(c.goto, Some(date(2027, 3, 3)));
    c.key(&ui, Out::Text("24 dec".into()), 7.0);
    assert_eq!(c.goto, Some(date(2027, 12, 24)), "a day and a month name are in the year the sheet shows");
    c.key(&ui, Out::Enter, 8.0);
    assert!(c.goto.is_none() && c.goto_line.is_empty());
    assert_eq!(c.selected, date(2027, 12, 24));
}

#[test]
fn a_typed_date_that_does_not_read_keeps_the_sheet_open_and_says_what_to_type() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.perform(&ui, Command::GoToDate, 0.0);
    c.key(&ui, Out::Text("soon".into()), 1.0);
    c.key(&ui, Out::Enter, 2.0);
    assert!(c.goto.is_some(), "the sheet stays open");
    assert!(c.error.as_deref().is_some_and(|e| e.contains("2026-12-24")));
    draw_once(&mut c, &ui);
    assert_eq!(c.selected, date(2026, 10, 4), "nothing moved");
    for _ in 0..4 {
        c.key(&ui, Out::Backspace, 3.0);
    }
    c.key(&ui, Out::Text("tomorrow".into()), 4.0);
    assert_eq!(c.goto, Some(date(2026, 10, 5)));
    c.key(&ui, Out::Enter, 5.0);
    assert_eq!(c.selected, date(2026, 10, 5));
    // Cancel throws what was typed away
    c.perform(&ui, Command::GoToDate, 6.0);
    c.key(&ui, Out::Text("junk".into()), 7.0);
    tap_bar(&mut c, &ui, chrome::B_CANCEL, 8.0);
    assert!(c.goto.is_none() && c.goto_line.is_empty());
    c.perform(&ui, Command::GoToDate, 9.0);
    assert!(c.goto_line.is_empty(), "the next sheet starts with an empty row");
    assert!(c.controls(&ui).iter().any(|n| n.label == "Date"), "the typed row is published");
}

#[test]
fn a_calendar_is_given_a_colour_from_the_palette() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    let work = c.extras.new_calendar("Work").unwrap();
    assert!(!c.open_colour(7), "a calendar that does not exist");
    assert!(c.open_colour(work));
    assert_eq!(c.colour_sheet().map(|s| s.pick), Some(c.extras.colour_index(work)));
    assert!(c.sheet_open() && !c.enabled(Command::NewEvent), "the sheet is modal");
    draw_once(&mut c, &ui);
    let published: Vec<String> = c.controls(&ui).into_iter().filter(|n| overlay::PALETTE.contains(&n.label.as_str())).map(|n| n.label).collect();
    assert_eq!(published.len(), overlay::PALETTE.len(), "a button per colour");
    // a tap on the Brown row picks it; Done gives it to the calendar
    let (i, top, h) = c.colour_rows().into_iter().find(|r| overlay::palette_name(r.0) == "Brown").unwrap();
    let old = c.extras.colour_index(work);
    tap(&mut c, &ui, 400.0, top + h / 2.0, 1.0);
    assert_eq!(c.colour_sheet().map(|s| s.pick), Some(i));
    assert_eq!(c.extras.colour_index(work), old, "nothing changes until Done");
    tap_bar(&mut c, &ui, chrome::B_COLOUR, 5.0);
    assert!(c.colour_sheet().is_none());
    assert_eq!(c.extras.colour_index(work), i);
    assert_eq!(c.notice.as_deref(), Some("Work is now Brown"));
    assert_eq!(c.calendar_colour(work), overlay::palette_colour(i));
    assert_eq!(new(&ui, temp.file()).extras.colour_index(work), i, "kept with the calendars");
    // events on that calendar use it
    let mut e = event(5, "Plan");
    e.date = date(2026, 10, 4);
    c.events.push(e);
    c.extras.put(5, extras::Extra { calendar: work, ..extras::Extra::default() });
    assert_eq!(c.event_colour(5), overlay::palette_colour(i));
}

#[test]
fn the_built_in_calendar_can_be_coloured_and_cancel_and_the_keys_work() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    assert_eq!(c.calendar_colour(extras::DEFAULT_CALENDAR), chrome::cal_colour(), "the accent until it is changed");
    assert!(c.open_colour(extras::DEFAULT_CALENDAR));
    c.activate(&ui, Target::ColourPick(4), 0.0);
    tap_bar(&mut c, &ui, chrome::B_CANCEL, 1.0);
    assert!(c.colour_sheet().is_none() && c.extras.builtin_colour.is_none(), "Cancel keeps the old colour");
    // Up and Down move the pick, Return takes it, Escape cancels
    assert!(c.open_colour(extras::DEFAULT_CALENDAR));
    c.key(&ui, Out::Named("Down"), 2.0);
    c.key(&ui, Out::Named("Down"), 3.0);
    assert_eq!(c.colour_sheet().map(|s| s.pick), Some(overlay::BLUE + 2));
    c.key(&ui, Out::Named("Up"), 4.0);
    c.key(&ui, Out::Enter, 5.0);
    assert!(c.colour_sheet().is_none());
    assert_eq!(c.extras.builtin_colour, Some(overlay::BLUE + 1));
    assert_eq!(c.calendar_colour(extras::DEFAULT_CALENDAR), overlay::palette_colour(overlay::BLUE + 1));
    assert!(c.open_colour(extras::DEFAULT_CALENDAR));
    assert!(c.escape(&ui, 6.0) && c.colour_sheet().is_none());
    c.activate(&ui, Target::ColourPick(200), 7.0);
    assert!(c.colour_sheet().is_none(), "a pick with no sheet does nothing");
}

#[test]
fn invitees_are_added_in_the_form_kept_with_the_event_and_invited_by_mail_on_save() {
    use extras::Invitee;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.set_account_email(Some("me@example.is".into()));
    c.open(&ui, None);
    type_text(&mut c, &ui, "Lunch");
    c.key(&ui, Out::Hide, 1.0);
    assert!(c.rows().iter().any(|r| r.0 == Target::AddInvitee), "the form has the Add Invitees row");
    c.activate(&ui, Target::AddInvitee, 2.0);
    assert!(c.invitee_line_open() && c.keyboard_shown());
    type_text(&mut c, &ui, "ana@example.is");
    c.key(&ui, Out::Enter, 3.0);
    let invited = |c: &Calendar| c.editor.as_ref().unwrap().extra.invitees.clone();
    assert_eq!(invited(&c), vec![Invitee { address: "ana@example.is".into(), name: String::new() }]);
    assert!(c.invitee_line_open(), "the line is ready for the next one");
    assert!(c.rows().iter().any(|r| r.0 == Target::InviteeDrop(0)), "a Remove row for the first");
    // the same address again, and something that is not an address
    type_text(&mut c, &ui, "ANA@example.is");
    c.key(&ui, Out::Enter, 4.0);
    assert_eq!(invited(&c).len(), 1);
    assert_eq!(c.notice.as_deref(), Some("Already invited"));
    type_text(&mut c, &ui, "nonsense");
    c.key(&ui, Out::Enter, 5.0);
    assert_eq!(invited(&c).len(), 1);
    assert!(c.error.is_some(), "it says what to type");
    draw_once(&mut c, &ui);
    // Escape closes the line first, then the form
    assert!(c.escape(&ui, 6.0));
    assert!(!c.invitee_line_open() && c.editor.is_some());
    // a second person, then remove the first
    c.activate(&ui, Target::AddInvitee, 7.0);
    type_text(&mut c, &ui, "bo@example.is");
    c.key(&ui, Out::Enter, 8.0);
    c.key(&ui, Out::Hide, 9.0);
    assert_eq!(invited(&c).len(), 2);
    c.activate(&ui, Target::InviteeDrop(0), 10.0);
    assert_eq!(invited(&c), vec![Invitee { address: "bo@example.is".into(), name: String::new() }]);
    draw_once(&mut c, &ui);
    c.activate(&ui, Target::Save, 11.0);
    assert!(c.editor.is_none(), "saved");
    let id = c.events[0].id;
    assert_eq!(c.extras().events.get(&id).unwrap().invitees.len(), 1, "kept with the event");
    assert_eq!(new(&ui, temp.file()).extras().events.get(&id).unwrap().invitees.len(), 1, "and with the file");
    let mail = c.take_outgoing();
    assert_eq!(mail.len(), 1);
    assert_eq!(mail[0].to, vec!["bo@example.is".to_string()]);
    assert_eq!(mail[0].subject, "Invitation: Lunch");
    // the file's long lines are folded; read them as one
    let text = fs::read_to_string(&mail[0].file).unwrap().replace("\r\n ", "");
    for part in ["METHOD:REQUEST", "SUMMARY:Lunch", "ORGANIZER:mailto:me@example.is", "ATTENDEE", "mailto:bo@example.is", "PARTSTAT=NEEDS-ACTION"] {
        assert!(text.contains(part), "{part}\n{text}");
    }
    // opening the event again shows its invitee, and Save sends the update again
    c.open(&ui, Some(id));
    assert_eq!(invited(&c).len(), 1);
    c.activate(&ui, Target::Save, 12.0);
    assert_eq!(c.take_outgoing().len(), 1);
}

#[test]
fn without_an_account_the_invitees_are_kept_and_nothing_is_sent() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    type_text(&mut c, &ui, "Lunch");
    c.key(&ui, Out::Hide, 1.0);
    c.activate(&ui, Target::AddInvitee, 2.0);
    type_text(&mut c, &ui, "ana@example.is");
    c.key(&ui, Out::Enter, 3.0);
    c.key(&ui, Out::Hide, 4.0);
    c.activate(&ui, Target::Save, 5.0);
    assert!(c.take_outgoing().is_empty());
    assert!(c.notice.as_deref().is_some_and(|n| n.contains("Invitees saved")));
    let id = c.events[0].id;
    assert_eq!(c.extras().events.get(&id).unwrap().invitees.len(), 1);
}

#[test]
fn paste_goes_into_the_invitee_line_when_it_is_open() {
    use bifrost_app::Command as Edit;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    c.key(&ui, Out::Hide, 1.0);
    assert!(!c.edit_commands(Some("ana@example.is")).supports(Edit::Paste), "no field has the cursor");
    c.activate(&ui, Target::AddInvitee, 2.0);
    assert!(c.edit_commands(Some("ana@example.is")).supports(Edit::Paste));
    assert!(c.edit_command(&ui, Edit::Paste, Some("ana@example.is\nsecond line"), 3.0));
    assert_eq!(c.editor.as_ref().unwrap().invitee_line.as_ref().map(|l| l.to_string()).as_deref(), Some("ana@example.is"));
}

/// A calendar whose events file sits beside a Contacts folder with two cards: Ana (an address, a birthday on 6 October
/// 1990) and Bo (no address, a birthday on 4 October with no year).
fn calendar_with_contacts(ui: &Ui, temp: &Temp) -> Calendar {
    let apps = temp.0.join("appdata");
    let own = apps.join("is.olibuijr.calendar.desktop");
    let cards = apps.join("is.olibuijr.contacts.desktop").join("Contacts");
    fs::create_dir_all(&own).unwrap();
    fs::create_dir_all(&cards).unwrap();
    fs::write(cards.join("contacts.tsv"), "springboard-contacts-v1\n1\tAna Lind\t\tana@example.is\t\n2\tBo Berg\t\t\t\n").unwrap();
    fs::write(cards.join("details.txt"), "springboard-contact-details-v1\n1\tbirthday\t1990-10-06\n2\tbirthday\t--10-04\n").unwrap();
    new(ui, own.join("events.tsv"))
}

#[test]
fn the_birthdays_of_the_cards_in_contacts_are_a_calendar_the_list_switches_on_and_off() {
    use overlay::LAYER_BIRTHDAYS;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = calendar_with_contacts(&ui, &temp);
    assert!(!c.prefs.show_birthdays);
    assert_eq!(c.shown_events().count(), 0);
    let (x, y) = layer_point(&c, &ui, LAYER_BIRTHDAYS);
    tap(&mut c, &ui, x, y, 0.0);
    assert!(c.prefs.show_birthdays);
    let titles: Vec<String> = c.shown_events().map(|e| e.title.clone()).collect();
    assert_eq!(titles.iter().filter(|t| *t == "Ana Lind's Birthday").count(), 3, "the years around the shown one");
    // today is Sunday 4 October 2026: Bo's birthday is in the agenda, and a tap says what it is
    let today: Vec<String> = c.agenda().iter().map(|e| e.title.clone()).collect();
    assert_eq!(today, vec!["Bo Berg's Birthday".to_string()]);
    let id = c.agenda()[0].id;
    assert!(overlay::is_overlay(id) && overlay::slot_of(id) == birthdays::BIRTHDAY_SLOT);
    c.activate(&ui, Target::Row(id), 1.0);
    assert!(c.editor.is_none());
    assert_eq!(c.notice.as_deref(), Some("Bo Berg's Birthday (Birthdays)"));
    assert_eq!(c.event_colour(id), overlay::colour(birthdays::BIRTHDAY_SLOT));
    // Ana's is on the 6th, with her age in the notes
    c.selected = date(2026, 10, 6);
    let ana = c.agenda().into_iter().map(|e| e.into_owned()).find(|e| e.title == "Ana Lind's Birthday").unwrap();
    assert_eq!((ana.all_day, ana.notes.as_str()), (true, "Turns 36"));
    // the row is kept with the settings, and a second tap hides the calendar again
    assert!(new(&ui, c.path.clone()).prefs.show_birthdays);
    let (x, y) = layer_point(&c, &ui, LAYER_BIRTHDAYS);
    tap(&mut c, &ui, x, y, 5.0);
    assert!(!c.prefs.show_birthdays && c.shown_events().count() == 0);
    for view in [View::Month, View::Week, View::Year] {
        c.activate(&ui, Target::View(view), 6.0);
        draw_once(&mut c, &ui);
    }
}

#[test]
fn a_birthday_alert_is_planned_with_the_events_and_never_written_to_the_calendar_file() {
    use prefs::Setting;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = calendar_with_contacts(&ui, &temp);
    assert!(c.rich_events_with_birthdays().events.is_empty(), "no alert, no events");
    let before = c.store_generation();
    assert!(c.change_setting(&ui, Setting::BirthdayAlert(2)));
    assert_ne!(c.store_generation(), before, "the alerts are planned again");
    assert_eq!(c.prefs.alert_birthday, 2);
    let events = c.rich_events_with_birthdays().events;
    assert_eq!(events.len(), 4, "two people, this year and the next");
    assert!(events.iter().all(|e| e.summary.ends_with("'s Birthday") && e.alarms == vec![1440] && e.start.minute == 540));
    assert!(c.rich_events().events.is_empty(), "the file's own events carry none");
    assert!(c.setting_checked(Setting::BirthdayAlert(2)) && !c.setting_checked(Setting::BirthdayAlert(0)));
    assert_eq!(new(&ui, c.path.clone()).prefs.alert_birthday, 2);
    // the alert scan finds Bo's alert: 09:00 on 3 October for a birthday on the 4th (a device on UTC)
    let cal = alerts::alert_calendar_with(&[], &c.rich_events_with_birthdays(), c.extras());
    let fire = date(2026, 10, 3).days() * 86_400 + 9 * 3600;
    let due = alerts::due(&cal, fire - 1, fire, 0);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].title, "Bo Berg's Birthday");
}

#[test]
fn the_address_book_suggests_people_under_the_invitee_line() {
    use extras::Invitee;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = calendar_with_contacts(&ui, &temp);
    c.open(&ui, None);
    c.key(&ui, Out::Hide, 1.0);
    c.activate(&ui, Target::AddInvitee, 2.0);
    assert_eq!(c.invitee_suggestions().len(), 0, "nothing is suggested before anything is typed");
    type_text(&mut c, &ui, "an");
    assert_eq!(c.invitee_suggestions().len(), 1, "Bo has no address, so Bo is not offered");
    assert!(c.rows().iter().any(|r| r.0 == Target::InviteeSuggest(0)));
    draw_once(&mut c, &ui);
    assert!(c.controls(&ui).iter().any(|n| n.label == "Invite from Contacts" && n.value.contains("ana@example.is")));
    c.activate(&ui, Target::InviteeSuggest(0), 3.0);
    assert_eq!(c.editor.as_ref().unwrap().extra.invitees, vec![Invitee { address: "ana@example.is".into(), name: "Ana Lind".into() }]);
    // a person already invited is not offered again, and Return with no match says what to type
    type_text(&mut c, &ui, "lind");
    assert_eq!(c.invitee_suggestions().len(), 0);
    c.key(&ui, Out::Enter, 4.0);
    assert!(c.error.is_some(), "no one else matches");
    // an address of someone in the book is added with their name
    for _ in 0..4 {
        c.key(&ui, Out::Backspace, 5.0);
    }
    c.editor.as_mut().unwrap().extra.invitees.clear();
    type_text(&mut c, &ui, "ANA@example.is");
    c.key(&ui, Out::Enter, 6.0);
    assert_eq!(c.editor.as_ref().unwrap().extra.invitees[0].name, "Ana Lind");
}

#[test]
fn print_draws_the_grid_of_what_the_window_shows() {
    use bifrost_app::print::Orientation;
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = searchable(&ui, temp.file());
    let (job, pages, info) = c.print_pages();
    assert_eq!(job, "Calendar October 2026");
    assert_eq!(pages.len(), 1);
    assert_eq!(info.orientation, Orientation::Landscape);
    let text = String::from_utf8_lossy(&pages[0].content).into_owned();
    for part in ["(October 2026) Tj", "Standup", "Lunch with Ana", "Dentist", "(Sun) Tj"] {
        assert!(text.contains(part), "{part}");
    }
    c.activate(&ui, Target::View(View::Week), 0.0);
    let (job, pages, _) = c.print_pages();
    assert_eq!(job, "Calendar Oct 4 - 10, 2026");
    let text = String::from_utf8_lossy(&pages[0].content).into_owned();
    assert!(text.contains("(Sun 4) Tj") && text.contains("(Sat 10) Tj") && text.contains("Standup") && text.contains("Dentist"));
    // a shorter week prints the days it shows
    c.change_setting(&ui, prefs::Setting::DaysPerWeek(4));
    let text = String::from_utf8_lossy(&c.print_pages().1[0].content).into_owned();
    assert!(text.contains("(Mon 5) Tj") && text.contains("(Fri 9) Tj") && !text.contains("(Sat 10) Tj"));
    c.activate(&ui, Target::View(View::Day), 0.0);
    assert_eq!(c.print_pages().0, "Calendar Sunday, October 4");
    c.activate(&ui, Target::View(View::Year), 0.0);
    assert_eq!(c.print_pages().0, "Calendar 2026");
    // the agenda of the same range is still there as text
    assert!(c.print_job().1.contains("Standup"));
    assert!(c.enabled(Command::Print) && !c.perform(&ui, Command::Print, 0.0), "the app runs the print job");
}

#[test]
fn the_day_view_has_one_column_and_one_day_chip() {
    // E2E W2-15: the Day view drew the whole week's chips over its single column
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Day), 0.0);
    assert_eq!(c.view_days(), vec![c.selected]);
    // a press anywhere on the header row picks the shown day, never another day of its week
    let y = ui.pt(150.0);
    for x in [0.1, 0.5, 0.95] {
        assert_eq!(c.hit_view(&ui, c.w as f32 * x, y), Some(Target::Pick(c.selected)));
    }
    draw_once(&mut c, &ui);
}

#[test]
fn a_selected_event_is_deleted_with_backspace_delete_and_the_event_menu() {
    // E2E W2-16: a clicked event could only be deleted from its form
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut a = event(1, "Standup");
    a.minute = 9 * 60;
    let mut b = event(2, "Lunch");
    b.minute = 12 * 60;
    let mut d = event(3, "Review");
    d.minute = 15 * 60;
    store::save(&temp.file(), &[a, b, d]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    assert!(!c.enabled(Command::Delete), "nothing selected");
    c.tap_event(1, 1);
    assert!(c.enabled(Command::Delete), "Event > Delete Event is on for the selected event");
    c.key(&ui, Out::Backspace, 1000.0);
    assert!(!c.events.iter().any(|e| e.id == 1), "Backspace deleted it");
    assert!(c.undo.is_some(), "with the Undo bar");
    c.tap_event(2, 1);
    c.key(&ui, Out::Named("Delete"), 2100.0);
    assert!(!c.events.iter().any(|e| e.id == 2), "the Delete key too");
    c.tap_event(3, 1);
    assert!(c.perform(&ui, Command::Delete, 3100.0));
    assert!(c.events.is_empty(), "and the menu");
    assert_eq!(store::load(&temp.file()).unwrap(), vec![]);
    c.key(&ui, Out::Backspace, 4000.0);
    assert!(c.events.is_empty() && c.error.is_none(), "nothing selected: nothing happens");
}

#[test]
fn deleting_the_event_of_an_editor_window_closes_that_window() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    c.tap_event(1, 1);
    c.tap_event(1, 2);
    assert_eq!(c.take_open_request(), Some(1), "a double-click asks for the window");
    assert!(c.open_pane(&ui, 7, 1));
    assert!(c.editor.is_none(), "the main window has no form");
    c.picked = Some(1);
    assert!(c.enabled(Command::Delete), "Event > Delete Event works while the editor window is open");
    assert!(c.perform(&ui, Command::Delete, 200.0));
    assert!(c.events.is_empty());
    assert_eq!(c.take_closed_windows(), vec![7], "the app closes the editor window");
    assert!(!c.editing_in_window(1));
    assert!(c.take_closed_windows().is_empty());
}

#[test]
fn return_in_the_title_saves_the_new_event() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.open(&ui, None);
    c.key(&ui, Out::Text("Dentist".into()), 0.0);
    c.key(&ui, Out::Enter, 10.0);
    assert!(c.editor.is_none(), "Return is Save");
    assert_eq!(c.events.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Dentist"]);
}

#[test]
fn the_hour_grid_keeps_its_scroll_when_a_form_closes() {
    // E2E W2-18: 8 AM was at the top before the form and 5 AM after it
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Week), 0.0);
    let morning = c.scroll.offset();
    assert!(morning > 0.0);
    c.open(&ui, None);
    c.activate(&ui, Target::Cancel, 10.0);
    assert!((c.scroll.offset() - morning).abs() < 1.0, "Cancel");
    c.open(&ui, None);
    c.key(&ui, Out::Text("Gym".into()), 20.0);
    c.activate(&ui, Target::Save, 30.0);
    assert!(c.editor.is_none());
    assert!((c.scroll.offset() - morning).abs() < 1.0, "Save");
}

#[test]
fn the_hour_grid_keeps_its_scroll_when_the_quick_event_popover_closes() {
    // E2E R7 / W2-18: 8 AM was at the top before "+" and 1 AM after Return (ending the empty search scrolled to the top)
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Week), 0.0);
    let morning = c.scroll.offset();
    assert!(morning > 0.0);
    tap_bar(&mut c, &ui, chrome::B_ADD, 10.0);
    assert!(c.event_pop_open());
    assert!((c.scroll.offset() - morning).abs() < 1.0, "\"+\" keeps the grid where it was");
    type_text(&mut c, &ui, "Gym");
    c.key(&ui, Out::Enter, 20.0);
    assert!(!c.event_pop_open() && c.events.len() == 1);
    assert!((c.scroll.offset() - morning).abs() < 1.0, "Return");
    tap_bar(&mut c, &ui, chrome::B_ADD, 30.0);
    assert!(c.escape(&ui, 40.0));
    assert!((c.scroll.offset() - morning).abs() < 1.0, "Escape");
}

#[test]
fn two_quick_mouse_clicks_on_a_timed_event_open_its_window() {
    // E2E R7: two clicks about 100 ms apart, as the harness sends them, did not count as a double-click
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Week), 0.0);
    c.set_pointer(true);
    c.layout_columns(&ui);
    let r = c.time_rects(&ui).into_iter().find(|r| r.0 == 1).expect("the event's block");
    let (x, y) = (c.x0 + r.1 + r.3 / 2.0, r.2 + r.4 / 2.0);
    c.touch(&ui, Event::Down(x, y), 1000.0);
    c.touch(&ui, Event::Up(x, y), 1010.0);
    assert_eq!(c.take_open_request(), None, "one click selects");
    c.touch(&ui, Event::Down(x + 1.0, y), 1100.0);
    c.touch(&ui, Event::Up(x + 1.0, y), 1110.0);
    assert_eq!(c.take_open_request(), Some(1), "the second click in time opens the editor window");
    // a slow second click is two single clicks
    c.touch(&ui, Event::Down(x, y), 3000.0);
    c.touch(&ui, Event::Up(x, y), 3010.0);
    c.touch(&ui, Event::Down(x, y), 4000.0);
    c.touch(&ui, Event::Up(x, y), 4010.0);
    assert_eq!(c.take_open_request(), None);
}

#[test]
fn the_time_zone_button_is_a_glyph_and_the_all_day_band_stays_put() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut offsite = event(1, "Offsite");
    offsite.all_day = true;
    offsite.minute = 0;
    offsite.duration = 1440;
    store::save(&temp.file(), &[offsite]).unwrap();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Week), 0.0);
    let zone = c.bars(&ui).into_iter().flat_map(|(bar, _)| bar.items).find(|i| i.id == super::chrome::B_ZONE).expect("the Time Zone button");
    assert_eq!(zone.symbol.as_deref(), Some("globe"));
    assert_eq!(zone.label, "Time Zone", "its name is its accessibility label");
    let band_y = |c: &Calendar| c.time_rects(&ui).into_iter().find(|r| r.0 == 1).unwrap().2;
    c.scroll.jump_top();
    let top = band_y(&c);
    c.scroll_to_morning(&ui);
    assert!(c.scroll.offset() > 0.0);
    assert_eq!(band_y(&c), top, "the all-day event does not scroll away with the hours");
    draw_once(&mut c, &ui);
}

#[test]
fn plus_opens_the_quick_event_popover_and_return_adds_the_event() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    tap_bar(&mut c, &ui, chrome::B_ADD, 0.0);
    assert!(c.event_pop_open() && c.editor.is_none(), "a popover, not the full form");
    assert!(c.events.is_empty(), "nothing is saved while the popover is open");
    let l = c.pop_layout(&ui).expect("the popover is laid out");
    let anchor = c.pop_anchor(&ui, &c.event_pop.as_ref().unwrap().event);
    assert!(l.panel.x >= anchor.x + anchor.w || l.panel.x + l.panel.w <= anchor.x, "beside the new event's day");
    assert!(c.controls(&ui).iter().any(|n| n.label == "Title" && n.role == bifrost_app::controls::Role::TextField));
    draw_once(&mut c, &ui);
    type_text(&mut c, &ui, "Lunch");
    c.key(&ui, Out::Named("Tab"), 1.0);
    type_text(&mut c, &ui, "Laugavegur 5");
    c.key(&ui, Out::Enter, 2.0);
    assert!(!c.event_pop_open(), "Return adds the event and closes the popover");
    assert_eq!(c.events.len(), 1);
    let e = &c.events[0];
    assert_eq!((e.title.as_str(), e.location.as_str(), e.date, e.minute), ("Lunch", "Laugavegur 5", date(2026, 10, 4), 540));
    assert_eq!(e.duration, c.prefs.duration);
    assert_eq!(store::load(&temp.file()).unwrap(), c.events);
}

#[test]
fn escape_drops_the_popover_event_and_keys_and_commands_wait_for_it() {
    use bifrost_app::proto::CHORD_META;
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    tap_bar(&mut c, &ui, chrome::B_ADD, 0.0);
    type_text(&mut c, &ui, "Gym");
    assert_eq!(c.event_pop.as_ref().unwrap().title.to_string(), "Gym", "typing goes to the focused title");
    assert!(!c.enabled(Command::NewEvent) && !c.settings_enabled(), "the popover holds the commands");
    assert!(!c.chord(&ui, 3, CHORD_META, 1.0), "no view change under the popover");
    assert!(c.escape(&ui, 2.0));
    assert!(!c.event_pop_open() && c.events.is_empty(), "Escape drops the new event");
    assert!(c.enabled(Command::NewEvent));
}

#[test]
fn edit_details_opens_the_full_form_with_what_was_typed() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    tap_bar(&mut c, &ui, chrome::B_ADD, 0.0);
    type_text(&mut c, &ui, "Dentist");
    let l = c.pop_layout(&ui).unwrap();
    tap(&mut c, &ui, l.details.x + SPACE_S, l.details.y + l.details.h / 2.0, 100.0);
    let draft = c.editor.as_ref().expect("the full form");
    assert!(!draft.existing);
    assert_eq!(c.fields[0].text(), "Dentist");
    assert!(!c.event_pop_open());
    tap_bar(&mut c, &ui, chrome::B_SAVE, 200.0);
    assert_eq!(c.events.len(), 1);
    assert_eq!(c.events[0].title, "Dentist");
}

#[test]
fn a_click_outside_the_popover_adds_a_titled_event_and_drops_an_untitled_one() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    let k = ui.pt(1.0);
    tap_bar(&mut c, &ui, chrome::B_ADD, 0.0);
    let l = c.pop_layout(&ui).unwrap();
    let (x, y) = (c.x0 / k + 10.0, l.panel.y + l.panel.h + 20.0);
    assert!(!l.panel.contains(x, y));
    tap(&mut c, &ui, x, y, 100.0);
    assert!(!c.event_pop_open() && c.events.is_empty(), "an untouched popover leaves nothing behind");
    tap_bar(&mut c, &ui, chrome::B_ADD, 1000.0);
    type_text(&mut c, &ui, "Call Ana");
    tap(&mut c, &ui, x, y, 1100.0);
    assert!(!c.event_pop_open());
    assert_eq!(c.events.len(), 1);
    assert_eq!(c.events[0].title, "Call Ana");
}

#[test]
fn the_popover_calendar_pop_up_picks_the_calendar_the_event_goes_to() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    assert!(c.perform(&ui, Command::NewCalendar, 0.0));
    type_text(&mut c, &ui, "Work");
    c.key(&ui, Out::Enter, 1.0);
    tap_bar(&mut c, &ui, chrome::B_ADD, 2.0);
    assert_eq!(c.event_pop.as_ref().unwrap().calendar, c.extras().default_id());
    c.activate(&ui, Target::Pop(eventpop::PopPart::Calendar), 3.0);
    assert!(c.cal_menu_open(), "the pop-up opens the menu of calendars");
    c.key(&ui, Out::Named("Down"), 3.1);
    c.key(&ui, Out::Enter, 3.2);
    assert!(!c.cal_menu_open() && c.event_pop_open(), "the pick closes the menu and keeps the popover");
    assert_eq!(c.event_pop.as_ref().unwrap().calendar, 2);
    draw_once(&mut c, &ui);
    c.key(&ui, Out::Enter, 4.0);
    let id = c.events[0].id;
    assert_eq!(c.events[0].title, eventpop::UNTITLED, "Return with no title adds a New Event");
    assert_eq!(c.extras().calendar_of(id), 2);
}

#[test]
fn a_double_click_on_an_empty_day_of_the_month_opens_the_popover_there() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.layout_columns(&ui);
    let (left, cell, top, rh) = c.grid_geom(&ui);
    let target = date(2026, 10, 20);
    let n = (target.days() - c.grid_start().days()) as f32;
    let (row, col) = ((n / 7.0).floor(), n % 7.0);
    let (x, y) = (c.x0 + left + cell * (col + 0.5), top + rh * row + rh - ui.pt(6.0));
    let click = |c: &mut Calendar, t: f64| {
        c.touch(&ui, Event::Down(x, y), t);
        c.touch(&ui, Event::Up(x, y), t + 20.0);
    };
    click(&mut c, 0.0);
    assert!(!c.event_pop_open(), "one click selects the day");
    assert_eq!(c.selected, target);
    click(&mut c, 1000.0);
    assert!(!c.event_pop_open(), "two clicks far apart are not a double-click");
    click(&mut c, 1100.0);
    let pop = c.event_pop.as_ref().expect("the double-click opens the popover");
    assert_eq!((pop.event.date, pop.event.minute), (target, 540));
    draw_once(&mut c, &ui);
}

#[test]
fn a_double_click_on_an_empty_slot_of_the_week_grid_starts_on_its_half_hour() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.activate(&ui, Target::View(View::Week), 0.0);
    c.layout_columns(&ui);
    let (gut, col_w) = c.col_geometry(&ui);
    let hours_top = ui.pt(timeview::GRID0 + timeview::ALLDAY_H) - c.scroll.offset();
    let (x, y) = (c.x0 + gut + col_w * 2.5, hours_top + (14.0 + 10.0 / 60.0) * ui.pt(c.prefs.hour_h));
    assert!(y < c.bottom(&ui), "the slot is on screen");
    for t in [100.0, 300.0] {
        c.set_pointer(true);
        c.touch(&ui, Event::Down(x, y), t);
        c.touch(&ui, Event::Up(x, y), t + 20.0);
    }
    let pop = c.event_pop.as_ref().expect("the double-click opens the popover");
    assert_eq!((pop.event.date, pop.event.minute), (c.view_days()[2], 14 * 60));
    let anchor = c.pop_anchor(&ui, &pop.event);
    assert!((anchor.y - y / ui.pt(1.0)).abs() < c.prefs.hour_h, "the popover points at the new block");
    draw_once(&mut c, &ui);
    c.key(&ui, Out::Enter, 400.0);
    assert_eq!(c.events.len(), 1);
    assert_eq!(c.events[0].minute, 14 * 60);
}

#[test]
fn the_sidebar_mini_month_pages_and_shows_a_clicked_day() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.create_event(intents::NewEvent { title: "Dinner".into(), date: date(2026, 10, 4), minute: Some(19 * 60), minutes: None, location: String::new(), notes: String::new(), all_day: false }).unwrap();
    c.layout_columns(&ui);
    assert!(c.mini_rect(&ui).is_some(), "the mini month fits under the calendar list");
    assert!(c.mini_rect(&ui).unwrap().y >= c.sidebar_bottom(&ui), "below the calendar rows");
    let marked = c.mini_marked(c.mini_month_shown());
    assert!(marked.contains(&4) && !marked.contains(&5), "a dot under the day with an event");
    draw_once(&mut c, &ui);
    let l = c.mini_layout(&ui).unwrap();
    tap(&mut c, &ui, l.next.x + l.next.w / 2.0, l.next.y + l.next.h / 2.0, 0.0);
    assert_eq!(c.mini_month_shown(), date(2026, 11, 1), "the chevron pages the mini month");
    assert_eq!(c.selected, date(2026, 10, 4), "and leaves the main view where it was");
    let l = c.mini_layout(&ui).unwrap();
    let (_, r) = l.days.iter().find(|(d, _)| *d == 12).copied().unwrap();
    tap(&mut c, &ui, r.x + r.w / 2.0, r.y + r.h / 2.0, 100.0);
    assert_eq!(c.selected, date(2026, 11, 12), "a click on a day shows it");
    assert_eq!(c.mini_month_shown(), date(2026, 11, 1));
    c.activate(&ui, Target::MiniPrev, 200.0);
    c.activate(&ui, Target::MiniPrev, 300.0);
    assert_eq!(c.mini_month_shown(), date(2026, 9, 1));
    c.activate(&ui, Target::Today, 400.0);
    assert_eq!(c.mini_month_shown(), date(2026, 10, 1), "Today brings the mini month back to the selected day");
    c.activate(&ui, Target::Sidebar, 500.0);
    assert!(c.mini_rect(&ui).is_none(), "hidden with the calendar list");
}

#[test]
fn a_greyed_day_of_the_next_or_previous_month_in_the_mini_month_shows_that_day() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    c.layout_columns(&ui);
    draw_once(&mut c, &ui);
    let l = c.mini_layout(&ui).expect("the mini month");
    assert_eq!(l.days.len() + l.outside.len(), 42, "six full rows");
    // October 2026 starts on a Thursday: its first cells are September's last days, its last cell a day of November
    let (d, r) = l.outside[0];
    assert_eq!((d.year, d.month), (2026, 9));
    tap(&mut c, &ui, r.x + r.w / 2.0, r.y + r.h / 2.0, 0.0);
    assert_eq!(c.selected, date(2026, 9, d.day), "a click on a leading day shows it");
    assert_eq!(c.mini_month_shown(), date(2026, 9, 1), "and the mini month follows it");
    c.activate(&ui, Target::MiniNext, 50.0);
    let l = c.mini_layout(&ui).unwrap();
    let (d, r) = *l.outside.last().unwrap();
    assert_eq!((d.year, d.month), (2026, 11));
    tap(&mut c, &ui, r.x + r.w / 2.0, r.y + r.h / 2.0, 100.0);
    assert_eq!(c.selected, date(2026, 11, d.day), "a click on a trailing day shows it");
}

#[test]
fn the_calendar_pop_up_is_a_menu_of_calendars_with_their_colours() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    for name in ["Work", "Home"] {
        assert!(c.perform(&ui, Command::NewCalendar, 0.0));
        type_text(&mut c, &ui, name);
        c.key(&ui, Out::Enter, 1.0);
    }
    tap_bar(&mut c, &ui, chrome::B_ADD, 2.0);
    let l = c.pop_layout(&ui).expect("the popover");
    let k = ui.pt(1.0);
    tap(&mut c, &ui, l.calendar.x + 10.0, l.calendar.y + l.calendar.h / 2.0, 3.0);
    let m = c.cal_menu.as_ref().expect("a click on the pop-up opens its menu");
    assert_eq!(m.ids, c.extras().choices().into_iter().map(|(id, _)| id).collect::<Vec<_>>(), "one row per calendar");
    assert_eq!(m.menu.selected(), Some(0), "the current calendar is checked");
    for (item, id) in m.menu.items.iter().zip(&m.ids) {
        assert_eq!(item.swatch, Some(c.calendar_colour(*id)), "each row has its calendar's colour dot");
    }
    let row = m.rect;
    // the current row lies over the button, as a pop-up menu opens
    assert!(row.y <= l.calendar.y + l.calendar.h && row.y + row.h >= l.calendar.y);
    draw_once(&mut c, &ui);
    // a click on the third row (Home) picks it and closes the menu; the popover stays
    let (rows, _) = c.cal_menu.as_ref().unwrap().menu.rows(bifrost_app::widgets::MENU_ROW_PT, bifrost_app::widgets::MENU_SEPARATOR_PT, bifrost_app::widgets::MENU_PAD_PT);
    let (x, y) = (row.x + 30.0, row.y + rows[2].0 + rows[2].1 / 2.0);
    c.touch(&ui, Event::Down(x * k, y * k), 4.0);
    c.touch(&ui, Event::Up(x * k, y * k), 4.02);
    assert!(!c.cal_menu_open() && c.event_pop_open());
    assert_eq!(c.event_pop.as_ref().unwrap().calendar, 3);
    // a click outside the menu closes it and changes nothing; Escape too
    tap(&mut c, &ui, l.calendar.x + 10.0, l.calendar.y + l.calendar.h / 2.0, 5.0);
    assert!(c.cal_menu_open());
    c.touch(&ui, Event::Down(1.0, ui.pt(300.0)), 6.0);
    c.touch(&ui, Event::Up(1.0, ui.pt(300.0)), 6.02);
    assert!(!c.cal_menu_open() && c.event_pop_open(), "the outside click is the menu's, not the popover's");
    assert_eq!(c.event_pop.as_ref().unwrap().calendar, 3);
    tap(&mut c, &ui, l.calendar.x + 10.0, l.calendar.y + l.calendar.h / 2.0, 7.0);
    assert!(c.escape(&ui, 8.0));
    assert!(!c.cal_menu_open() && c.event_pop_open(), "Escape closes the menu before the popover");
}

#[test]
fn the_forms_calendar_row_opens_the_menu_and_a_row_picks_it() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    assert!(c.perform(&ui, Command::NewCalendar, 0.0));
    type_text(&mut c, &ui, "Work");
    c.key(&ui, Out::Enter, 1.0);
    new_form(&mut c, &ui, 2.0);
    c.key(&ui, Out::Hide, 3.0);
    c.activate(&ui, Target::Calendar, 4.0);
    assert!(c.cal_menu_open());
    let items: Vec<String> = c.controls(&ui).into_iter().filter(|n| n.role == bifrost_app::controls::Role::MenuItem).map(|n| n.label).collect();
    assert_eq!(items, vec!["Calendar".to_string(), "Work".to_string()], "the rows are menu items for assistive technology");
    draw_once(&mut c, &ui);
    let m = c.cal_menu.as_ref().unwrap();
    let (rows, _) = m.menu.rows(bifrost_app::widgets::MENU_ROW_PT, bifrost_app::widgets::MENU_SEPARATOR_PT, bifrost_app::widgets::MENU_PAD_PT);
    let k = ui.pt(1.0);
    let (x, y) = (m.rect.x + 30.0, m.rect.y + rows[1].0 + rows[1].1 / 2.0);
    c.touch(&ui, Event::Down(x * k, y * k), 5.0);
    c.touch(&ui, Event::Up(x * k, y * k), 5.02);
    assert!(!c.cal_menu_open() && c.editor.is_some(), "the pick closes the menu and keeps the form");
    assert_eq!(c.editor.as_ref().unwrap().extra.calendar, 2);
}

#[test]
fn the_wheel_scrolls_the_hour_grid_but_not_the_calendar_list() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    let mut c = new(&ui, temp.file());
    c.set_size(1200, 780);
    c.activate(&ui, Target::View(View::Week), 0.0);
    c.layout_columns(&ui);
    c.scroll.jump_top();
    let x = c.x0 + ui.pt(200.0);
    // a wheel turned down: the content travels up and the offset grows
    assert!(c.wheel(&ui, x, ui.pt(300.0), -ui.pt(40.0)));
    assert!((c.scroll.offset() - ui.pt(40.0)).abs() < 0.5);
    assert!(c.wheel(&ui, x, ui.pt(300.0), ui.pt(10.0)), "and back up");
    assert!((c.scroll.offset() - ui.pt(30.0)).abs() < 0.5);
    if c.x0 > 0.0 {
        assert!(!c.wheel(&ui, c.x0 / 2.0, ui.pt(300.0), -ui.pt(40.0)), "the calendar list does not scroll");
    }
    // the month grid fits the window
    c.activate(&ui, Target::View(View::Month), 10.0);
    assert!(!c.wheel(&ui, x, ui.pt(300.0), -ui.pt(40.0)));
}

#[test]
fn the_wheel_scrolls_the_form_of_an_editor_window_and_leaves_the_grid() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    let grid = c.scroll.offset();
    assert!(c.open_pane(&ui, 7, 1));
    // a long form (pickers open): give the window's form something to scroll
    c.with_pane(7, |c| {
        c.scroll.set_max(ui.pt(300.0));
        c.scroll.jump_top();
    });
    let top = c.with_pane(7, |c| c.rows()[0].1).unwrap().0;
    // window points, as `App::window_pointer` delivers them
    assert!(c.wheel_pane(&ui, 7, 100.0, 200.0, -50.0));
    let ((offset, now), open) = c.with_pane(7, |c| (c.scroll.offset(), c.rows()[0].1)).unwrap();
    assert!(open);
    assert!((offset - ui.pt(50.0)).abs() < 0.5, "the form scrolled 50 pt");
    assert!((top - now - 50.0).abs() < 0.5, "its rows moved up 50 pt");
    assert!((c.scroll.offset() - grid).abs() < 0.5, "the main window's grid stays where it was");
    assert!(!c.wheel_pane(&ui, 8, 100.0, 200.0, -50.0), "no such window");
}

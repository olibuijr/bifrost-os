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
        alert: 0,
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

#[test]
fn touch_add_edit_delete_and_reload() {
    let ui = crate::test_ui();
    let temp = Temp::new();
    let mut c = new(&ui, temp.file());
    tap_bar(&mut c, &ui, chrome::B_ADD, 0.0);
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
    tap_event(&mut c, &ui, id, 500.0);
    assert!(c.editor.is_some());
    tap_row(&mut c, &ui, Target::Toggle(Slot::Start), 713.0, 560.0);
    tap_row(&mut c, &ui, Target::DateStep(0), 713.0, 600.0);
    tap_row(&mut c, &ui, Target::AllDay, 650.0, 700.0);
    tap_bar(&mut c, &ui, chrome::B_SAVE, 800.0);
    assert_eq!(c.selected, date(2026, 10, 5));
    assert!(c.events[0].all_day);
    let reopened = new(&ui, temp.file());
    assert_eq!(reopened.events, c.events);
    tap_event(&mut c, &ui, id, 900.0);
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
    assert!(!c.week_numbers);
    c.activate(&ui, Target::WeekNumbers, 0.0);
    assert!(c.week_numbers);
    let again = new(&ui, temp.file());
    assert!(again.week_numbers, "kept after a restart");
}

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
    // a hand-picked alert is left alone
    c.activate(&ui, Target::Alert, 0.0);
    assert_eq!(c.editor.as_ref().unwrap().event.alert, 4);
    c.activate(&ui, Target::AllDay, 0.0);
    assert_eq!(c.editor.as_ref().unwrap().event.alert, 4);
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
fn a_quick_tap_on_an_event_in_the_week_still_opens_it() {
    let temp = Temp::new();
    let ui = crate::test_ui();
    store::save(&temp.file(), &[event(1, "Standup")]).unwrap();
    let mut c = week_calendar(&ui, temp.file());
    let (x, y) = grid_xy(&c, &ui, 0, 560);
    c.touch(&ui, Event::Down(x, y), 0.0);
    c.touch(&ui, Event::Up(x, y), 50.0);
    assert!(c.drag.is_none());
    assert!(c.editor.as_ref().is_some_and(|d| d.existing && d.event.id == 1));
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

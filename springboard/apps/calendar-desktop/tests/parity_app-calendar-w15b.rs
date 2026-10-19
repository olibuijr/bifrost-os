//! Parity app-calendar, wave 15 part two: headless tests of the Birthdays calendar, the invitee addresses, the printed grid,
//! the time zones with daylight saving that the view time zone uses, and the settings the Week view reads (days per week,
//! the working day). No test draws a frame or opens a window.

use bifrost_app::print::{Orientation, PdfPage};
use bifrost_calendar::calendar::birthdays::{self, Person};
use bifrost_calendar::calendar::extras::{clean_address, Extras, Invitee};
use bifrost_calendar::calendar::prefs::{Prefs, Setting};
use bifrost_calendar::calendar::printgrid::{self, Grid};
use bifrost_calendar::calendar::store::CalendarEvent;
use bifrost_calendar::calendar::timeview::View;
use bifrost_calendar::calendar::zones::{self, Clock};
use bifrost_calendar::calendar::Date;
use springboard::shell::contactstore::Contact;

fn day(month: u32, day: u32) -> Date {
    Date { year: 2026, month, day }
}

fn card(id: u64, name: &str, email: &str) -> Contact {
    Contact { id, name: name.into(), email: email.into(), ..Contact::default() }
}

fn saved(id: u64, date: Date, minute: u16, duration: u16, title: &str) -> CalendarEvent {
    CalendarEvent { id, date, minute, duration, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0, uid: String::new() }
}

fn text_of(page: &PdfPage) -> String {
    String::from_utf8_lossy(&page.content).into_owned()
}

#[test]
fn a_birthday_with_and_without_a_year_is_read_from_a_card() {
    assert_eq!(birthdays::parse_birthday("1990-05-17"), Some((Some(1990), 5, 17)));
    assert_eq!(birthdays::parse_birthday("--05-17"), Some((None, 5, 17)));
    assert_eq!(birthdays::parse_birthday("--02-29"), Some((None, 2, 29)));
    assert_eq!(birthdays::parse_birthday("May 17"), None);
}

#[test]
fn the_cards_with_a_birthday_become_all_day_items_each_year() {
    let contacts = vec![card(1, "Ana Lind", "ana@example.is"), card(2, "Bo Berg", "")];
    let details = "springboard-contact-details-v1\n1\tbirthday\t1990-10-06\n2\tbirthday\t--02-29\n9\tbirthday\t2000-01-01\n";
    let who = birthdays::people(&contacts, details);
    assert_eq!(who.len(), 2, "a detail whose card is gone is left out");
    let events = birthdays::overlay_events(&who, 2027, 2028, birthdays::BIRTHDAY_SLOT);
    let bo: Vec<Date> = events.iter().filter(|e| e.title == "Bo Berg's Birthday").map(|e| e.date).collect();
    assert_eq!(bo, vec![Date { year: 2027, month: 3, day: 1 }, Date { year: 2028, month: 2, day: 29 }], "29 February falls on 1 March in a common year");
    assert!(events.iter().all(|e| e.all_day && e.duration == 1440));
    let ana = events.iter().find(|e| e.title == "Ana Lind's Birthday" && e.date.year == 2028).unwrap();
    assert_eq!(ana.notes, "Turns 38");
}

#[test]
fn a_birthday_alert_fires_at_nine_o_clock_on_the_day_it_names() {
    let who = vec![Person { id: 4, name: "Ana".into(), born: None, month: 10, day: 6 }];
    assert!(birthdays::alert_events(&who, 0, 2026).is_empty());
    let events = birthdays::alert_events(&who, 1, 2026);
    assert_eq!(events.len(), 2);
    assert_eq!((events[0].start.date, events[0].start.minute, events[0].alarms.clone()), (day(10, 6), 540, vec![0]));
    assert_eq!(birthdays::ALERTS.len(), 5);
    assert_eq!(birthdays::alert_title(3), "2 days before at 9 AM");
}

#[test]
fn the_address_book_suggests_by_name_or_address_and_skips_cards_without_one() {
    let contacts = vec![card(1, "Ana Lind", "ana@example.is"), card(2, "Bo Berg", ""), card(3, "Anna", "anna@x.is")];
    let book = birthdays::book(&contacts);
    assert_eq!(book.len(), 2);
    let names: Vec<&str> = birthdays::suggest(&book, "an", 5).iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Ana Lind", "Anna"]);
    assert!(birthdays::suggest(&book, "", 5).is_empty());
}

#[test]
fn invitee_addresses_are_cleaned_and_kept_with_the_event() {
    assert_eq!(clean_address(" mailto:Ana@Example.is "), Some("Ana@Example.is".to_string()));
    assert_eq!(clean_address("ana@example"), None);
    assert_eq!(clean_address("ana"), None);
    let mut extras = Extras::default();
    extras.entry(7).invitees.push(Invitee { address: "ana@example.is".into(), name: "Ana Lind".into() });
    let text = extras.to_text();
    assert!(text.contains("invitee\t7\tana@example.is\tAna Lind\n"), "{text}");
    assert_eq!(Extras::parse(&text), extras);
}

#[test]
fn a_calendar_colour_is_a_palette_number_the_file_keeps() {
    let mut extras = Extras::default();
    let work = extras.new_calendar("Work").unwrap();
    assert!(extras.set_colour(work, 7));
    assert!(extras.set_colour(1, 2));
    assert!(!extras.set_colour(work, 99));
    let back = Extras::parse(&extras.to_text());
    assert_eq!((back.colour_index(work), back.colour_index(1)), (7, 2));
}

#[test]
fn the_working_day_and_the_week_length_are_settings_with_their_own_menu_ids() {
    let mut p = Prefs::default();
    p.apply(Setting::DayEnd(2));
    p.apply(Setting::DaysPerWeek(4));
    assert_eq!((p.day_end, p.days_per_week), (18, 5));
    assert_eq!(Prefs::parse(&p.to_text()), p);
    assert_eq!(Setting::from_id(Setting::DaysPerWeek(4).id()), Some(Setting::DaysPerWeek(4)));
    assert_eq!(Setting::from_id(Setting::BirthdayAlert(3).id()), Some(Setting::BirthdayAlert(3)));
    assert_eq!(p.week_view_start(), 1, "five days that start on Sunday run Monday to Friday");
}

#[test]
fn a_time_zone_with_summer_time_converts_across_the_shift() {
    let tokyo = Clock::Zone(zones::find("Asia/Tokyo").unwrap());
    let paris = Clock::Zone(zones::find("Europe/Paris").unwrap());
    // 10:00 in Paris is 17:00 in Tokyo in July (UTC+2) and 18:00 in January (UTC+1)
    assert_eq!(zones::convert_clock(paris, day(7, 10), 600, tokyo), (day(7, 10), 1020));
    assert_eq!(zones::convert_clock(paris, day(1, 10), 600, tokyo), (day(1, 10), 1080));
    assert_eq!(zones::convert_clock(tokyo, day(1, 10), 1080, paris), (day(1, 10), 600));
}

fn week_grid(selected: Date, days: Vec<Date>) -> Grid {
    Grid { view: View::Week, title: "Oct 4 - 10, 2026".into(), selected, days, first_weekday: 0, day_start: 8, day_end: 18 }
}

#[test]
fn the_printed_week_has_a_column_per_day_and_the_times_of_each_block() {
    let days: Vec<Date> = (4..=10).map(|n| day(10, n)).collect();
    let events = vec![saved(1, day(10, 6), 600, 60, "Dentist"), saved(2, day(10, 8), 780, 90, "Review")];
    let occurs = |e: &CalendarEvent, d: Date| e.occurs_on(d);
    let info = printgrid::paper();
    assert_eq!(info.orientation, Orientation::Landscape);
    let pages = printgrid::pages(&week_grid(day(10, 7), days), &events, &occurs, &info);
    assert_eq!(pages.len(), 1);
    let text = text_of(&pages[0]);
    for part in ["(Oct 4 - 10, 2026) Tj", "(Sun 4) Tj", "(Sat 10) Tj", "(Dentist) Tj", "(Review) Tj", "(10:00 - 11:00) Tj", "(13:00 - 14:30) Tj"] {
        assert!(text.contains(part), "{part}");
    }
}

#[test]
fn a_five_day_week_prints_five_columns() {
    let days: Vec<Date> = (5..=9).map(|n| day(10, n)).collect();
    let occurs = |e: &CalendarEvent, d: Date| e.occurs_on(d);
    let pages = printgrid::pages(&week_grid(day(10, 7), days), &[], &occurs, &printgrid::paper());
    let text = text_of(&pages[0]);
    assert!(text.contains("(Mon 5) Tj") && text.contains("(Fri 9) Tj"));
    assert!(!text.contains("(Sun 4) Tj") && !text.contains("(Sat 10) Tj"));
}

#[test]
fn the_printed_month_table_has_the_rows_its_weeks_need() {
    assert_eq!(printgrid::month_cells(day(10, 15), 0).len(), 5);
    assert_eq!(printgrid::month_cells(day(8, 1), 0).len(), 6);
    assert_eq!(printgrid::month_cells(day(2, 10), 0).len(), 4);
    assert_eq!(printgrid::hour_range(8, 18, &[(300, 360)]), (5, 18));
}

#[test]
fn text_is_measured_in_helvetica_widths_and_cut_with_dots() {
    assert!((printgrid::text_width("M", 10.0) - 8.33).abs() < 1e-3);
    assert_eq!(printgrid::fit("Standup", 10.0, 200.0), "Standup");
    assert!(printgrid::fit("A very long meeting title indeed", 10.0, 60.0).ends_with("..."));
}

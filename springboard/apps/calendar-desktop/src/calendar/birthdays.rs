//! The Birthdays calendar: the birthdays on the cards of Contacts, shown read-only as all-day items, and the people of the
//! address book for the editor's invitee field. Contacts keeps its cards in `Contacts/contacts.tsv` and the extra card
//! fields (the birthday among them) in `Contacts/details.txt`, both in its own data folder beside Calendar's
//! (`appdata/is.olibuijr.contacts.desktop/`, like Reminders). Calendar only reads them: a card is never written from
//! here, and the files are read when the calendar is shown or the editor's invitee field opens, never in the background.
//! Only names, birthdays and email addresses are read, and they stay on this machine.
use super::ics::{IcsEvent, Local};
use super::overlay;
use super::store::CalendarEvent;
use super::Date;
use springboard::shell::contactstore::{self, Contact};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The data folder names Contacts may use under the shell's appdata folder, newest first.
pub const STORE_DIRS: [&str; 2] = ["is.olibuijr.contacts.desktop", "is.olibuijr.contacts"];
/// The calendar slot of the birthdays: far above the holidays (0) and the subscriptions, clear of the reminders' slot.
pub const BIRTHDAY_SLOT: usize = 1 << 18;
/// The header of `details.txt`; a file with another header is not read.
const DETAILS_HEADER: &str = "springboard-contact-details-v1";
/// The detail code of a birthday.
const BIRTHDAY_CODE: &str = "birthday";
/// Files larger than this are not read (Contacts has the same limits).
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Longest title a birthday gets, in characters.
const TITLE_MAX: usize = 150;
/// The minute of the day a birthday alert counts from: 09:00.
const ALERT_AT: u16 = 9 * 60;

/// The default birthday alerts: the name the Settings menu shows and the minutes before 09:00 on the birthday.
pub const ALERTS: [(&str, Option<u32>); 5] = [
    ("None", None),
    ("On the day at 9 AM", Some(0)),
    ("1 day before at 9 AM", Some(1440)),
    ("2 days before at 9 AM", Some(2880)),
    ("1 week before at 9 AM", Some(10_080)),
];

/// The name of default birthday alert `v` (an index into `ALERTS`).
pub fn alert_title(v: u8) -> &'static str {
    ALERTS.get(v as usize).map_or("None", |a| a.0)
}

/// The minutes before 09:00 of the birthday that alert `v` fires, None for no alert.
pub fn alert_minutes(v: u8) -> Option<u32> {
    ALERTS.get(v as usize).and_then(|a| a.1)
}

/// A card with a birthday.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub id: u64,
    pub name: String,
    /// The year of birth when the card has one.
    pub born: Option<i64>,
    pub month: u32,
    pub day: u32,
}

/// A person of the address book who has an email address: a name to suggest and the address an invitation goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub email: String,
}

/// Reverses the details file's escaping: `\\`, `\t`, `\n` and `\r`.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// A birthday as Contacts writes it: `YYYY-MM-DD`, or `--MM-DD` with no year. The year when known, the month and the day;
/// None for anything else. 29 February is a real birthday.
pub fn parse_birthday(text: &str) -> Option<(Option<i64>, u32, u32)> {
    let t = text.trim();
    let (year, rest) = if let Some(rest) = t.strip_prefix("--") {
        (None, rest.replace('-', ""))
    } else {
        let (y, rest) = t.split_once('-')?;
        if y.len() != 4 {
            return None;
        }
        (Some(y.parse::<i64>().ok()?), rest.replace('-', ""))
    };
    if rest.len() != 4 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (month, day) = (rest[..2].parse::<u32>().ok()?, rest[2..].parse::<u32>().ok()?);
    // 2000 is a leap year, so a birthday on 29 February passes
    Date { year: 2000, month, day }.valid().then_some((year, month, day))
}

/// The people with a birthday: the cards of `contacts` that `details` (the text of `details.txt`) gives a birthday, by
/// month and day, then name. A birthday that does not read, and a detail of a card that is gone, are left out.
pub fn people(contacts: &[Contact], details: &str) -> Vec<Person> {
    let mut lines = details.lines();
    if lines.next() != Some(DETAILS_HEADER) {
        return vec![];
    }
    let by_id: HashMap<u64, &Contact> = contacts.iter().map(|c| (c.id, c)).collect();
    let mut out: Vec<Person> = vec![];
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let &[id, code, value] = fields.as_slice() else { continue };
        if code != BIRTHDAY_CODE {
            continue;
        }
        let Ok(id) = id.parse::<u64>() else { continue };
        let Some((born, month, day)) = parse_birthday(&unescape(value)) else { continue };
        let Some(card) = by_id.get(&id) else { continue };
        if out.iter().any(|p| p.id == id) {
            continue;
        }
        out.push(Person { id, name: card.display_name().trim().to_string(), born, month, day });
    }
    out.sort_by(|a, b| (a.month, a.day, &a.name).cmp(&(b.month, b.day, &b.name)));
    out
}

/// The people of the address book who have an email address, by name.
pub fn book(contacts: &[Contact]) -> Vec<Entry> {
    let mut out: Vec<Entry> = contacts
        .iter()
        .filter(|c| c.email.contains('@'))
        .map(|c| Entry { name: c.display_name().trim().to_string(), email: c.email.trim().to_string() })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// At most `limit` people of `book` whose name or address holds `typed` (case ignored), in name order. Nothing for text
/// that is blank.
pub fn suggest<'a>(book: &'a [Entry], typed: &str, limit: usize) -> Vec<&'a Entry> {
    let needle = typed.trim().to_lowercase();
    if needle.is_empty() {
        return vec![];
    }
    book.iter().filter(|e| e.name.to_lowercase().contains(&needle) || e.email.to_lowercase().contains(&needle)).take(limit).collect()
}

/// The folder of Contacts' cards for the Calendar events file `events_path`, when the address book is there.
pub fn folder(events_path: &Path) -> Option<PathBuf> {
    let appdata = events_path.parent()?.parent()?;
    STORE_DIRS.iter().map(|dir| appdata.join(dir).join("Contacts")).find(|dir| dir.join("contacts.tsv").exists())
}

/// A text file no larger than `MAX_BYTES`; empty when it is missing, too large or not text.
fn read_small(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_BYTES => std::fs::read_to_string(path).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The cards of the address book next to Calendar's folder. No address book gives no cards.
fn load_contacts(events_path: &Path) -> Option<(PathBuf, Vec<Contact>)> {
    let dir = folder(events_path)?;
    let contacts = contactstore::load(&dir.join("contacts.tsv"));
    Some((dir, contacts))
}

/// The people with a birthday in the address book next to `events_path`.
pub fn load_people(events_path: &Path) -> Vec<Person> {
    let Some((dir, contacts)) = load_contacts(events_path) else { return vec![] };
    people(&contacts, &read_small(&dir.join("details.txt")))
}

/// The people with an email address in the address book next to `events_path`.
pub fn load_book(events_path: &Path) -> Vec<Entry> {
    load_contacts(events_path).map_or_else(Vec::new, |(_, contacts)| book(&contacts))
}

/// The day of `year` a birthday on `month` and `day` falls on: 29 February is 1 March in a year with no 29 February.
pub fn occurrence(month: u32, day: u32, year: i64) -> Date {
    let date = Date { year, month, day };
    if date.valid() {
        date
    } else {
        Date { year, month: 3, day: 1 }
    }
}

/// "Ana's Birthday", cut to `TITLE_MAX` characters.
pub fn title_of(person: &Person) -> String {
    let name: String = person.name.chars().take(TITLE_MAX - 11).collect();
    format!("{name}'s Birthday")
}

/// The birthdays of the years `first..=last` as all-day events in calendar `slot`, with the age in the notes when the card
/// has a year of birth.
pub fn overlay_events(people: &[Person], first: i64, last: i64, slot: usize) -> Vec<CalendarEvent> {
    let mut out = vec![];
    for year in first..=last {
        for p in people {
            let notes = match p.born {
                Some(born) if year > born => format!("Turns {}", year - born),
                _ => String::new(),
            };
            out.push(CalendarEvent {
                id: overlay::id_for(slot, out.len()),
                date: occurrence(p.month, p.day, year),
                minute: 0,
                duration: 1440,
                all_day: true,
                title: title_of(p),
                location: String::new(),
                notes,
                repeat: 0,
                alert: 0,
                uid: String::new(),
            });
        }
    }
    out
}

/// The events that fire the birthday alert `alert` (an index into `ALERTS`): one timed event at 09:00 on each person's
/// birthday in `year` and in the year after, so the alert of a birthday just after New Year is planned in December. No
/// alert gives none.
pub fn alert_events(people: &[Person], alert: u8, year: i64) -> Vec<IcsEvent> {
    let Some(minutes) = alert_minutes(alert) else { return vec![] };
    let mut out = vec![];
    for y in [year, year + 1] {
        for p in people {
            let start = Local::new(occurrence(p.month, p.day, y), ALERT_AT);
            let mut e = IcsEvent::new(&format!("birthday-{}-{y}@springboard.tab", p.id), &title_of(p), start, start.add_minutes(15));
            e.alarms = vec![minutes];
            out.push(e);
        }
    }
    out
}

/// `alert_events` for the address book next to `events_path`. The files are not read when there is no alert.
pub fn alert_events_at(events_path: &Path, alert: u8, year: i64) -> Vec<IcsEvent> {
    if alert_minutes(alert).is_none() {
        return vec![];
    }
    alert_events(&load_people(events_path), alert, year)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: u64, name: &str, email: &str) -> Contact {
        Contact { id, name: name.into(), email: email.into(), ..Contact::default() }
    }
    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }
    const DETAILS: &str = "springboard-contact-details-v1\n1\tnickname\tAna\n1\tbirthday\t1990-05-17\n2\tbirthday\t--02-29\n3\tbirthday\tsoon\n9\tbirthday\t2000-01-01\n";

    #[test]
    fn birthdays_read_with_and_without_a_year() {
        assert_eq!(parse_birthday("1990-05-17"), Some((Some(1990), 5, 17)));
        assert_eq!(parse_birthday("--05-17"), Some((None, 5, 17)));
        assert_eq!(parse_birthday("--0517"), Some((None, 5, 17)));
        assert_eq!(parse_birthday("--02-29"), Some((None, 2, 29)), "29 February is a real birthday");
        assert_eq!(parse_birthday("--02-30"), None);
        assert_eq!(parse_birthday("--13-01"), None);
        assert_eq!(parse_birthday("May 17"), None);
        assert_eq!(parse_birthday("90-05-17"), None);
        assert_eq!(parse_birthday(""), None);
    }

    #[test]
    fn people_join_the_details_to_their_cards_in_calendar_order() {
        let contacts = vec![card(1, "Ana Lind", "ana@example.is"), card(2, "Bo", ""), card(3, "Cy", "")];
        let got = people(&contacts, DETAILS);
        let names: Vec<(&str, u32, u32)> = got.iter().map(|p| (p.name.as_str(), p.month, p.day)).collect();
        // card 3's birthday does not read and card 9 is gone; February comes before May
        assert_eq!(names, vec![("Bo", 2, 29), ("Ana Lind", 5, 17)]);
        assert_eq!(got[1].born, Some(1990));
        assert_eq!(got[0].born, None);
        assert!(people(&contacts, "something else\n1\tbirthday\t1990-05-17\n").is_empty(), "another header is not read");
        assert!(people(&[], DETAILS).is_empty());
    }

    #[test]
    fn a_leap_day_birthday_falls_on_the_first_of_march_in_a_common_year() {
        assert_eq!(occurrence(2, 29, 2028), d(2028, 2, 29));
        assert_eq!(occurrence(2, 29, 2027), d(2027, 3, 1));
        assert_eq!(occurrence(5, 17, 2027), d(2027, 5, 17));
    }

    #[test]
    fn the_calendar_lists_each_birthday_every_year_as_an_all_day_item() {
        let contacts = vec![card(1, "Ana Lind", ""), card(2, "Bo", "")];
        let who = people(&contacts, DETAILS);
        let events = overlay_events(&who, 2026, 2027, BIRTHDAY_SLOT);
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|e| e.all_day && overlay::is_overlay(e.id) && overlay::slot_of(e.id) == BIRTHDAY_SLOT));
        let ana: Vec<(Date, &str)> = events.iter().filter(|e| e.title == "Ana Lind's Birthday").map(|e| (e.date, e.notes.as_str())).collect();
        assert_eq!(ana, vec![(d(2026, 5, 17), "Turns 36"), (d(2027, 5, 17), "Turns 37")]);
        let bo: Vec<Date> = events.iter().filter(|e| e.title == "Bo's Birthday").map(|e| e.date).collect();
        assert_eq!(bo, vec![d(2026, 3, 1), d(2027, 3, 1)]);
        let mut ids: Vec<u64> = events.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn a_long_name_is_cut_so_the_title_stays_valid() {
        let p = Person { id: 1, name: "x".repeat(200), born: None, month: 1, day: 1 };
        assert!(title_of(&p).chars().count() <= TITLE_MAX);
        assert!(title_of(&p).ends_with("'s Birthday"));
    }

    #[test]
    fn an_alert_fires_at_nine_on_the_birthday_or_before() {
        let who = vec![Person { id: 7, name: "Ana".into(), born: Some(1990), month: 5, day: 17 }];
        assert!(alert_events(&who, 0, 2026).is_empty(), "no alert, no events");
        let events = alert_events(&who, 2, 2026);
        assert_eq!(events.len(), 2, "this year and the next");
        assert_eq!(events[0].start.date, d(2026, 5, 17));
        assert_eq!(events[0].start.minute, 540);
        assert_eq!(events[0].alarms, vec![1440]);
        assert_eq!(events[1].start.date, d(2027, 5, 17));
        assert_eq!(events[0].summary, "Ana's Birthday");
        assert_ne!(events[0].uid, events[1].uid);
        // the alerts are the ones the Settings menu lists
        assert_eq!(ALERTS.len(), super::super::prefs::BIRTHDAY_ALERT_CHOICES as usize);
        assert_eq!(alert_title(0), "None");
        assert_eq!(alert_minutes(4), Some(10_080));
        assert_eq!(alert_minutes(9), None);
    }

    #[test]
    fn a_birthday_alert_is_planned_by_the_alert_scan() {
        use super::super::alerts::due;
        use super::super::ics::IcsCalendar;
        let who = vec![Person { id: 7, name: "Ana".into(), born: None, month: 5, day: 17 }];
        let cal = IcsCalendar { name: String::new(), events: alert_events(&who, 2, 2026), timezones: vec![] };
        // a device on UTC: 09:00 on 16 May 2026 is the alert of a birthday on the 17th
        let fire = (d(2026, 5, 16).days() * 86_400) + 9 * 3600;
        let got = due(&cal, fire - 1, fire, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Ana's Birthday");
    }

    #[test]
    fn the_address_book_suggests_by_name_or_address() {
        let contacts = vec![card(1, "Ana Lind", "ana@example.is"), card(2, "Bo Berg", "bo@work.is"), card(3, "No Mail", ""), card(4, "Anna", "anna@x.is")];
        let all = book(&contacts);
        assert_eq!(all.len(), 3, "a card with no address is not offered");
        let got: Vec<&str> = suggest(&all, "AN", 5).iter().map(|e| e.name.as_str()).collect();
        assert_eq!(got, vec!["Ana Lind", "Anna"]);
        assert_eq!(suggest(&all, "work", 5).len(), 1);
        assert_eq!(suggest(&all, "a", 1).len(), 1, "the limit holds");
        assert!(suggest(&all, "  ", 5).is_empty());
        assert!(suggest(&all, "zzz", 5).is_empty());
    }

    #[test]
    fn no_address_book_gives_nothing() {
        let nowhere = Path::new("/nonexistent-springboard/appdata/is.olibuijr.calendar.desktop/events.tsv");
        assert_eq!(folder(nowhere), None);
        assert!(load_people(nowhere).is_empty());
        assert!(load_book(nowhere).is_empty());
        assert!(alert_events_at(nowhere, 2, 2026).is_empty());
    }
}

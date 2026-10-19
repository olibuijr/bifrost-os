//! What the saved events file cannot hold, kept beside it as `calendar-extras.txt`: a custom repeat rule, a list of
//! alarms, the local calendar an event is in, its time zone and its travel time, plus the local calendars themselves.
//! Entries are keyed by the store id of the event. The file is plain tab-separated text and every line is read on its
//! own, so a damaged line costs that line only.
use super::ics::IcsCalendar;
use super::recur;
use super::zones;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The built-in calendar every event belongs to when it names none. It is the calendar list's "Calendar" row.
pub const DEFAULT_CALENDAR: u32 = 1;
/// Most calendars, the built-in one included. Their ids are 1..=MAX_CALENDARS, so each fits a Settings menu group.
pub const MAX_CALENDARS: u32 = 10;
/// Longest calendar name kept, in characters.
pub const NAME_MAX: usize = 60;
/// The time zones an event can name: the device's own zone (`None`) and the zones of `zones::TABLE`, in its order,
/// which know their daylight saving rules.
pub const ZONES: [Option<&str>; 12] = [
    None,
    Some("UTC"),
    Some("Atlantic/Reykjavik"),
    Some("Europe/London"),
    Some("Europe/Paris"),
    Some("Europe/Helsinki"),
    Some("America/New_York"),
    Some("America/Chicago"),
    Some("America/Denver"),
    Some("America/Los_Angeles"),
    Some("Asia/Kolkata"),
    Some("Asia/Tokyo"),
];
/// The travel times an event can name (minutes); 0 is none.
pub const TRAVEL: [u16; 5] = [0, 15, 30, 45, 60];
/// Custom alarms step by this many minutes, between one step and a week.
pub const ALARM_STEP: u32 = 5;
pub const ALARM_MAX: u32 = 7 * 24 * 60;

/// How many colours a calendar can be given (`overlay::PALETTE`).
pub const PALETTE_LEN: u8 = 9;
/// Most invitees one event keeps.
pub const MAX_INVITEES: usize = 20;
/// Longest address kept for an invitee, in characters.
pub const ADDRESS_MAX: usize = 254;

/// A person invited to an event from the editor: the email address an invitation goes to and the name shown for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitee {
    pub address: String,
    pub name: String,
}

/// An invitee's address, cleaned: spaces and control characters dropped, a leading `mailto:` removed. None when what is
/// left is not an address (it needs one `@` with text on both sides and a dot after it).
pub fn clean_address(text: &str) -> Option<String> {
    let t = text.trim();
    let t = match t.get(..7) {
        Some(head) if head.eq_ignore_ascii_case("mailto:") => &t[7..],
        _ => t,
    };
    let address: String = t.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    if address.chars().count() > ADDRESS_MAX || address.contains(['<', '>', ',', ';', '"']) {
        return None;
    }
    let (local, domain) = address.split_once('@')?;
    let ok = !local.is_empty() && !domain.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.');
    ok.then_some(address)
}

/// The extras of one event. A field left at its default is the same as no entry.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Extra {
    /// The people invited from the editor, in the order they were added (`MAX_INVITEES` at most).
    pub invitees: Vec<Invitee>,
    /// The RRULE of a custom repeat. None when the event repeats with one of the presets, or does not repeat.
    pub rrule: Option<String>,
    /// Minutes before the start of each alarm. None when the alarm is the event's preset.
    pub alarms: Option<Vec<u32>>,
    /// The calendar id, or 0 for the default calendar.
    pub calendar: u32,
    /// The zone the event's times are in (`ZONES`), None for the device's zone.
    pub tzid: Option<String>,
    /// Minutes to leave before the event, shown in the editor.
    pub travel: u16,
}

impl Extra {
    pub fn is_empty(&self) -> bool {
        self.rrule.is_none() && self.alarms.is_none() && self.calendar == 0 && self.tzid.is_none() && self.travel == 0 && self.invitees.is_empty()
    }
}

/// A calendar the user made. Its colour is an index into the calendar palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalCal {
    pub id: u32,
    pub name: String,
    pub colour: u8,
    pub shown: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Extras {
    pub events: BTreeMap<u64, Extra>,
    /// The calendars the user made, in the order they were made. The built-in calendar is not in this list.
    pub calendars: Vec<LocalCal>,
    /// The calendar new events go to (Settings > Default Calendar). 0 or a missing calendar means the built-in one.
    pub default_calendar: u32,
    /// The colour of the built-in calendar (an index into `overlay::PALETTE`); None keeps the accent.
    pub builtin_colour: Option<u8>,
}

/// The settings file beside the events.
pub fn path(events: &Path) -> PathBuf {
    events.with_file_name("calendar-extras.txt")
}

/// Minutes before the start of each `store::ALERTS` choice (see `alerts::ALERT_MINUTES`).
pub fn preset_alarms(alert: u8) -> Vec<u32> {
    super::alerts::ALERT_MINUTES.get(alert as usize).copied().flatten().into_iter().collect()
}

/// The standard offset in minutes east of UTC of a zone in `ZONES`, None for an unknown name. A zone with daylight
/// saving is shifted by `zones::offset_at`.
pub fn zone_minutes(tzid: &str) -> Option<i32> {
    zones::find(tzid).map(|z| z.standard)
}

/// Gives the store events of `cal` the zone they name, so the .ics text says which zone their times are in. A zone the
/// calendar does not list yet is added with its offset. All-day events have no zone.
pub fn apply_zones(cal: &mut IcsCalendar, extras: &Extras) {
    for e in cal.events.iter_mut().filter(|e| !e.all_day) {
        let Some(id) = e.uid.strip_suffix("@springboard.tab").and_then(|i| i.parse::<u64>().ok()) else { continue };
        let Some(zone) = extras.events.get(&id).and_then(|x| x.tzid.clone()) else { continue };
        if !cal.timezones.iter().any(|(n, _)| *n == zone) {
            if let Some(minutes) = zone_minutes(&zone) {
                cal.timezones.push((zone.clone(), minutes));
            }
        }
        e.tzid = Some(zone);
    }
}

/// The editor's name for a zone choice.
pub fn zone_title(tzid: Option<&str>) -> &'static str {
    match tzid {
        None => "Device Time Zone",
        Some(id) => zones::find(id).map_or("Other Time Zone", |z| z.title),
    }
}

/// "At time of event", "5 minutes before", "2 hours before", "1 day before": one alarm's name.
pub fn alarm_title(minutes: u32) -> String {
    match minutes {
        0 => "At time of event".into(),
        m if m % 1440 == 0 && m / 1440 == 1 => "1 day before".into(),
        m if m % 1440 == 0 => format!("{} days before", m / 1440),
        m if m % 60 == 0 && m / 60 == 1 => "1 hour before".into(),
        m if m % 60 == 0 => format!("{} hours before", m / 60),
        m if m == 1 => "1 minute before".into(),
        m => format!("{m} minutes before"),
    }
}

/// The travel time's name: "None", "15 minutes", "1 hour".
pub fn travel_title(minutes: u16) -> String {
    match minutes {
        0 => "None".into(),
        60 => "1 hour".into(),
        m => format!("{m} minutes"),
    }
}

/// Sorts the alarms and drops repeats and anything out of range.
pub fn clean_alarms(list: &[u32]) -> Vec<u32> {
    let set: BTreeSet<u32> = list.iter().copied().filter(|m| *m <= ALARM_MAX).collect();
    set.into_iter().collect()
}

/// The name a calendar gets: control characters and tabs are dropped, the length is capped.
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect::<String>().trim().to_string()
}

impl Extras {
    /// The calendar an event belongs to: its own when that calendar exists, else the default.
    pub fn calendar_of(&self, id: u64) -> u32 {
        match self.events.get(&id).map(|x| x.calendar).filter(|c| *c != 0) {
            Some(cal) if self.exists(cal) => cal,
            _ => self.default_id(),
        }
    }

    /// The calendar new events go to.
    pub fn default_id(&self) -> u32 {
        if self.default_calendar != 0 && self.exists(self.default_calendar) {
            self.default_calendar
        } else {
            DEFAULT_CALENDAR
        }
    }

    /// Whether `cal` is the built-in calendar or one of the calendars made.
    pub fn exists(&self, cal: u32) -> bool {
        cal == DEFAULT_CALENDAR || self.calendars.iter().any(|c| c.id == cal)
    }

    /// The name of a calendar; the built-in one is "Calendar".
    pub fn name_of(&self, cal: u32) -> String {
        match self.calendars.iter().find(|c| c.id == cal) {
            Some(c) if cal != DEFAULT_CALENDAR => c.name.clone(),
            _ => "Calendar".into(),
        }
    }

    /// The local calendars in their order, with the built-in one first, as (id, name) pairs for the pickers.
    pub fn choices(&self) -> Vec<(u32, String)> {
        let mut out = vec![(DEFAULT_CALENDAR, self.name_of(DEFAULT_CALENDAR))];
        out.extend(self.calendars.iter().map(|c| (c.id, c.name.clone())));
        out
    }

    /// The next calendar id that is free, if the limit is not reached.
    fn free_id(&self) -> Option<u32> {
        (DEFAULT_CALENDAR + 1..=MAX_CALENDARS).find(|id| !self.calendars.iter().any(|c| c.id == *id))
    }

    /// Makes a calendar named `name` (cleaned first). None when the name is empty or the limit is reached.
    pub fn new_calendar(&mut self, name: &str) -> Option<u32> {
        let name = clean_name(name);
        if name.is_empty() {
            return None;
        }
        let id = self.free_id()?;
        // the palette starts after the first colour, which the holidays use
        let colour = (self.calendars.len() + 1) as u8 % 6;
        self.calendars.push(LocalCal { id, name, colour, shown: true });
        Some(id)
    }

    /// Renames a calendar. False for the built-in one, an unknown id or an empty name.
    pub fn rename(&mut self, cal: u32, name: &str) -> bool {
        let name = clean_name(name);
        if name.is_empty() || cal == DEFAULT_CALENDAR {
            return false;
        }
        match self.calendars.iter_mut().find(|c| c.id == cal) {
            Some(c) => {
                c.name = name;
                true
            }
            None => false,
        }
    }

    /// Takes a calendar away. Its events fall back to the default calendar. The built-in calendar cannot be removed.
    pub fn remove(&mut self, cal: u32) -> bool {
        let before = self.calendars.len();
        self.calendars.retain(|c| c.id != cal);
        if self.default_calendar == cal {
            self.default_calendar = 0;
        }
        for x in self.events.values_mut().filter(|x| x.calendar == cal) {
            x.calendar = 0;
        }
        self.calendars.len() != before
    }

    /// The palette colour (an index into `overlay::PALETTE`) of calendar `cal`: its own, or the built-in calendar's choice,
    /// which is blue until it is changed.
    pub fn colour_index(&self, cal: u32) -> u8 {
        if cal == DEFAULT_CALENDAR {
            return self.builtin_colour.unwrap_or(super::overlay::BLUE);
        }
        self.calendars.iter().find(|c| c.id == cal).map_or(super::overlay::BLUE, |c| c.colour)
    }

    /// Gives calendar `cal` palette colour `colour`. False for a colour past the palette or a calendar that does not exist.
    pub fn set_colour(&mut self, cal: u32, colour: u8) -> bool {
        if colour >= PALETTE_LEN {
            return false;
        }
        if cal == DEFAULT_CALENDAR {
            self.builtin_colour = Some(colour);
            return true;
        }
        match self.calendars.iter_mut().find(|c| c.id == cal) {
            Some(c) => {
                c.colour = colour;
                true
            }
            None => false,
        }
    }

    /// Drops the entries of events that are gone, so a later event that reuses an id starts clean.
    pub fn retain_events(&mut self, live: &[u64]) {
        let live: BTreeSet<u64> = live.iter().copied().collect();
        self.events.retain(|id, _| live.contains(id));
    }

    /// The entry of event `id`, made when it is missing.
    pub fn entry(&mut self, id: u64) -> &mut Extra {
        self.events.entry(id).or_default()
    }

    /// Stores the entry of event `id`, or drops it when it holds nothing.
    pub fn put(&mut self, id: u64, extra: Extra) {
        if extra.is_empty() {
            self.events.remove(&id);
        } else {
            self.events.insert(id, extra);
        }
    }

    /// Writes the file. Lines are `default`, `calendar`, `event` and `#` comments; the order is fixed so the file diffs.
    pub fn to_text(&self) -> String {
        let mut out = String::from("springboard-calendar-extras-v1\n");
        out.push_str(&format!("default\t{}\n", self.default_calendar));
        if let Some(colour) = self.builtin_colour {
            out.push_str(&format!("colour\t{colour}\n"));
        }
        for c in &self.calendars {
            out.push_str(&format!("calendar\t{}\t{}\t{}\t{}\n", c.id, c.colour, u8::from(c.shown), c.name));
        }
        for (id, x) in &self.events {
            let rrule = x.rrule.clone().unwrap_or_else(|| "-".into());
            let alarms = match &x.alarms {
                Some(list) if list.is_empty() => "none".to_string(),
                Some(list) => list.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
                None => "-".into(),
            };
            let tz = x.tzid.clone().unwrap_or_else(|| "-".into());
            out.push_str(&format!("event\t{id}\t{}\t{rrule}\t{alarms}\t{tz}\t{}\n", x.calendar, x.travel));
            for p in &x.invitees {
                out.push_str(&format!("invitee\t{id}\t{}\t{}\n", p.address, clean_name(&p.name)));
            }
        }
        out
    }

    /// Reads the file. Unknown and damaged lines are skipped; the built-in calendar is never listed twice.
    pub fn parse(text: &str) -> Extras {
        let mut out = Extras::default();
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            match f.first().copied() {
                Some("default") if f.len() == 2 => {
                    if let Ok(id) = f[1].parse::<u32>() {
                        out.default_calendar = id;
                    }
                }
                Some("colour") if f.len() == 2 => {
                    out.builtin_colour = f[1].parse::<u8>().ok().filter(|c| *c < PALETTE_LEN);
                }
                Some("calendar") if f.len() == 5 => {
                    let (Ok(id), Ok(colour), Some(name)) = (f[1].parse::<u32>(), f[2].parse::<u8>(), Some(clean_name(f[4]))) else {
                        continue;
                    };
                    if (DEFAULT_CALENDAR + 1..=MAX_CALENDARS).contains(&id) && !name.is_empty() && !out.calendars.iter().any(|c| c.id == id) {
                        out.calendars.push(LocalCal { id, name, colour: colour % PALETTE_LEN, shown: f[3] != "0" });
                    }
                }
                Some("invitee") if f.len() == 4 => {
                    let (Ok(id), Some(address)) = (f[1].parse::<u64>(), clean_address(f[2])) else { continue };
                    let entry = out.events.entry(id).or_default();
                    if entry.invitees.len() < MAX_INVITEES && !entry.invitees.iter().any(|p| p.address.eq_ignore_ascii_case(&address)) {
                        entry.invitees.push(Invitee { address, name: clean_name(f[3]) });
                    }
                }
                Some("event") if f.len() == 7 => {
                    let Ok(id) = f[1].parse::<u64>() else { continue };
                    let calendar = f[2].parse::<u32>().unwrap_or(0);
                    let rrule = (f[3] != "-").then(|| f[3].to_string()).filter(|r| recur::parse_rrule(r).is_some());
                    let alarms = match f[4] {
                        "-" => None,
                        "none" => Some(vec![]),
                        list => list.split(',').map(|m| m.parse::<u32>().ok()).collect::<Option<Vec<u32>>>().map(|l| clean_alarms(&l)),
                    };
                    let tzid = (f[5] != "-").then(|| f[5].to_string()).filter(|z| zone_minutes(z).is_some());
                    let travel = f[6].parse::<u16>().ok().filter(|t| TRAVEL.contains(t)).unwrap_or(0);
                    // invitee lines may come first in a damaged file: keep what they gave
                    let invitees = out.events.get(&id).map(|x| x.invitees.clone()).unwrap_or_default();
                    out.events.insert(id, Extra { invitees, rrule, alarms, calendar, tzid, travel });
                }
                _ => {}
            }
        }
        out
    }

    pub fn load(events: &Path) -> Extras {
        std::fs::read_to_string(path(events)).map(|t| Extras::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, events: &Path) -> std::io::Result<()> {
        springboard::shell::fsutil::write_atomic(&path(events), &self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_calendar_takes_the_next_id_and_a_palette_colour() {
        let mut x = Extras::default();
        assert_eq!(x.new_calendar("  Work\t"), Some(2));
        assert_eq!(x.new_calendar("Home"), Some(3));
        assert_eq!(x.calendars[0].name, "Work");
        assert_ne!(x.calendars[0].colour, x.calendars[1].colour);
        assert!(x.calendars.iter().all(|c| c.shown));
    }

    #[test]
    fn names_are_cleaned_and_empty_names_refused() {
        let mut x = Extras::default();
        assert_eq!(x.new_calendar("   "), None);
        assert_eq!(x.new_calendar("\t\n"), None);
        assert_eq!(clean_name("a\u{7}b\nc"), "abc");
        assert_eq!(clean_name(&"x".repeat(200)).chars().count(), NAME_MAX);
    }

    #[test]
    fn the_limit_is_the_settings_group_size() {
        let mut x = Extras::default();
        let made: Vec<Option<u32>> = (0..12).map(|i| x.new_calendar(&format!("c{i}"))).collect();
        assert_eq!(made.iter().filter(|m| m.is_some()).count(), (MAX_CALENDARS - 1) as usize);
        assert_eq!(x.calendars.last().map(|c| c.id), Some(MAX_CALENDARS));
    }

    #[test]
    fn the_built_in_calendar_cannot_be_renamed_or_removed() {
        let mut x = Extras::default();
        assert!(!x.rename(DEFAULT_CALENDAR, "Mine"));
        assert!(!x.remove(DEFAULT_CALENDAR));
        let work = x.new_calendar("Work").unwrap();
        assert!(x.rename(work, "Job"));
        assert_eq!(x.name_of(work), "Job");
        assert!(!x.rename(work, "  "));
    }

    #[test]
    fn removing_a_calendar_moves_its_events_and_the_default_back() {
        let mut x = Extras::default();
        let work = x.new_calendar("Work").unwrap();
        x.default_calendar = work;
        x.put(5, Extra { calendar: work, ..Extra::default() });
        assert_eq!(x.calendar_of(5), work);
        assert!(x.remove(work));
        assert_eq!(x.default_calendar, 0);
        assert_eq!(x.calendar_of(5), DEFAULT_CALENDAR);
        assert_eq!(x.default_id(), DEFAULT_CALENDAR);
    }

    #[test]
    fn events_without_a_calendar_use_the_default() {
        let mut x = Extras::default();
        let home = x.new_calendar("Home").unwrap();
        x.put(1, Extra { calendar: home, ..Extra::default() });
        x.put(2, Extra { calendar: 9, ..Extra::default() });
        assert_eq!(x.calendar_of(1), home);
        assert_eq!(x.calendar_of(2), DEFAULT_CALENDAR, "a calendar that does not exist");
        assert_eq!(x.calendar_of(3), DEFAULT_CALENDAR);
    }

    #[test]
    fn an_empty_entry_is_not_kept() {
        let mut x = Extras::default();
        x.put(4, Extra { travel: 15, ..Extra::default() });
        assert!(x.events.contains_key(&4));
        x.put(4, Extra::default());
        assert!(x.events.is_empty());
    }

    #[test]
    fn entries_of_deleted_events_are_dropped() {
        let mut x = Extras::default();
        for id in [1, 2, 3] {
            x.put(id, Extra { travel: 30, ..Extra::default() });
        }
        x.retain_events(&[1, 3]);
        assert_eq!(x.events.keys().copied().collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn extras_round_trip_through_the_file_text() {
        let mut x = Extras::default();
        let work = x.new_calendar("Work, Ltd").unwrap();
        x.calendars[0].shown = false;
        x.default_calendar = work;
        x.builtin_colour = Some(8);
        x.put(
            7,
            Extra {
                rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE".into()),
                alarms: Some(vec![5, 1440]),
                calendar: work,
                tzid: Some("UTC".into()),
                travel: 30,
                invitees: vec![
                    Invitee { address: "ana@example.is".into(), name: "Ana Lind".into() },
                    Invitee { address: "bo@example.is".into(), name: String::new() },
                ],
            },
        );
        x.put(8, Extra { alarms: Some(vec![]), ..Extra::default() });
        x.put(9, Extra { invitees: vec![Invitee { address: "cy@example.is".into(), name: "Cy".into() }], ..Extra::default() });
        let text = x.to_text();
        assert_eq!(Extras::parse(&text), x);
        assert_eq!(Extras::parse(&text).to_text(), text, "writing twice gives the same bytes");
    }

    #[test]
    fn addresses_are_cleaned_and_must_look_like_addresses() {
        assert_eq!(clean_address(" ana@example.is "), Some("ana@example.is".into()));
        assert_eq!(clean_address("mailto:ana@example.is"), Some("ana@example.is".into()));
        assert_eq!(clean_address("MAILTO:ana@example.is"), Some("ana@example.is".into()));
        assert_eq!(clean_address("ana @example.is"), Some("ana@example.is".into()), "spaces are dropped");
        for bad in ["", "ana", "ana@", "@example.is", "ana@example", "ana@@example.is", "ana@.is", "ana@example.", "a<b>@x.is", "a,b@x.is"] {
            assert_eq!(clean_address(bad), None, "{bad:?}");
        }
        assert_eq!(clean_address(&format!("{}@x.is", "a".repeat(300))), None, "too long");
    }

    #[test]
    fn invitees_keep_their_order_and_stop_at_the_limit_and_at_repeats() {
        let mut text = String::from("springboard-calendar-extras-v1\n");
        for n in 0..30 {
            text.push_str(&format!("invitee\t5\tp{n}@example.is\tP{n}\n"));
        }
        text.push_str("invitee\t5\tP0@EXAMPLE.is\tAgain\ninvitee\t5\tnot-an-address\tNo\ninvitee\tx\ta@b.is\tBad id\n");
        let x = Extras::parse(&text);
        let kept = &x.events.get(&5).unwrap().invitees;
        assert_eq!(kept.len(), MAX_INVITEES);
        assert_eq!(kept[0], Invitee { address: "p0@example.is".into(), name: "P0".into() });
        assert_eq!(kept[19].address, "p19@example.is");
        assert!(!x.events.contains_key(&0));
        let dup = Extras::parse("springboard-calendar-extras-v1\ninvitee\t1\ta@b.is\tA\ninvitee\t1\tA@B.IS\tAgain\n");
        assert_eq!(dup.events.get(&1).unwrap().invitees.len(), 1, "an address is invited once");
    }

    #[test]
    fn an_event_line_after_its_invitee_lines_keeps_them() {
        let x = Extras::parse("springboard-calendar-extras-v1\ninvitee\t4\ta@b.is\tA\nevent\t4\t0\t-\t-\t-\t15\n");
        let e = x.events.get(&4).unwrap();
        assert_eq!((e.travel, e.invitees.len()), (15, 1));
    }

    #[test]
    fn an_event_with_only_invitees_is_kept() {
        let mut x = Extras::default();
        x.put(3, Extra { invitees: vec![Invitee { address: "a@b.is".into(), name: String::new() }], ..Extra::default() });
        assert!(x.events.contains_key(&3));
        x.put(3, Extra::default());
        assert!(x.events.is_empty());
    }

    #[test]
    fn calendars_take_a_palette_colour_and_the_built_in_one_starts_blue() {
        let mut x = Extras::default();
        let work = x.new_calendar("Work").unwrap();
        assert_eq!(x.colour_index(DEFAULT_CALENDAR), super::super::overlay::BLUE);
        assert!(x.set_colour(work, 7));
        assert_eq!(x.colour_index(work), 7);
        assert!(x.set_colour(DEFAULT_CALENDAR, 2));
        assert_eq!(x.colour_index(DEFAULT_CALENDAR), 2);
        assert!(!x.set_colour(work, PALETTE_LEN), "past the palette");
        assert!(!x.set_colour(9, 1), "a calendar that does not exist");
        // the choices are kept in the file, and a colour past the palette is not read
        let back = Extras::parse(&x.to_text());
        assert_eq!((back.colour_index(work), back.colour_index(DEFAULT_CALENDAR)), (7, 2));
        let odd = Extras::parse("springboard-calendar-extras-v1\ncolour\t99\ncalendar\t2\t40\t1\tOdd\n");
        assert_eq!(odd.builtin_colour, None);
        assert_eq!(odd.calendars[0].colour, 40 % PALETTE_LEN);
        assert_eq!(PALETTE_LEN as usize, super::super::overlay::PALETTE.len());
    }

    #[test]
    fn damaged_lines_cost_only_themselves() {
        let text = "springboard-calendar-extras-v1\nevent\tx\t0\t-\t-\t-\t0\n\
            event\t3\t0\tRRULE-NOT-A-RULE\t5,oops\tMars/Olympus\t999\n\
            calendar\t99\t0\t1\tToo high\ncalendar\t2\t1\t0\tWork\nunknown\tline\n\
            event\t4\t2\tFREQ=DAILY\t15,30\tUTC\t45\n";
        let x = Extras::parse(text);
        assert_eq!(x.calendars.len(), 1);
        assert!(!x.calendars[0].shown);
        let kept = x.events.get(&3).expect("the id is fine, its fields are not");
        assert_eq!((kept.rrule.clone(), kept.alarms.clone(), kept.tzid.clone(), kept.travel), (None, None, None, 0));
        assert_eq!(x.events.get(&4).unwrap().alarms, Some(vec![15, 30]));
        assert!(!x.events.contains_key(&0));
    }

    #[test]
    fn alarms_sort_and_drop_duplicates_and_out_of_range() {
        assert_eq!(clean_alarms(&[60, 5, 60, ALARM_MAX + 1]), vec![5, 60]);
        assert_eq!(preset_alarms(0), Vec::<u32>::new());
        assert_eq!(preset_alarms(6), vec![1440]);
    }

    #[test]
    fn alarm_and_travel_titles_read_naturally() {
        assert_eq!(alarm_title(0), "At time of event");
        assert_eq!(alarm_title(1440), "1 day before");
        assert_eq!(alarm_title(2880), "2 days before");
        assert_eq!(alarm_title(90), "90 minutes before");
        assert_eq!(alarm_title(120), "2 hours before");
        assert_eq!(travel_title(0), "None");
        assert_eq!(travel_title(60), "1 hour");
        assert_eq!(travel_title(45), "45 minutes");
    }

    #[test]
    fn the_zones_are_the_table_and_the_device() {
        assert_eq!(zone_minutes("Atlantic/Reykjavik"), Some(0));
        assert_eq!(zone_minutes("Europe/Paris"), Some(60));
        assert_eq!(zone_minutes("America/New_York"), Some(-300));
        assert_eq!(zone_minutes("Mars/Olympus"), None);
        assert_eq!(zone_title(None), "Device Time Zone");
        assert_eq!(zone_title(Some("Europe/Paris")), "Paris");
        assert_eq!(zone_title(Some("Zone/X")), "Other Time Zone");
        assert_eq!(ZONES.len(), zones::TABLE.len() + 1);
        for (choice, zone) in ZONES.iter().skip(1).zip(zones::TABLE.iter()) {
            assert_eq!(*choice, Some(zone.id));
        }
    }

    #[test]
    fn export_names_the_zone_of_timed_events_only() {
        use super::super::ics::{IcsEvent, Local};
        use super::super::Date;
        let date = Date { year: 2026, month: 10, day: 13 };
        let timed = IcsEvent::new("7@springboard.tab", "Call", Local::new(date, 540), Local::new(date, 600));
        let mut all_day = IcsEvent::new("8@springboard.tab", "Holiday", Local::new(date, 0), Local::new(date, 0));
        all_day.all_day = true;
        let other = IcsEvent::new("uid@elsewhere", "Imported", Local::new(date, 540), Local::new(date, 600));
        let mut cal = IcsCalendar { name: String::new(), events: vec![timed, all_day, other], timezones: vec![] };
        let mut x = Extras::default();
        x.put(7, Extra { tzid: Some("Atlantic/Reykjavik".into()), ..Extra::default() });
        x.put(8, Extra { tzid: Some("UTC".into()), ..Extra::default() });
        apply_zones(&mut cal, &x);
        assert_eq!(cal.events[0].tzid.as_deref(), Some("Atlantic/Reykjavik"));
        assert_eq!(cal.events[1].tzid, None, "all-day events have no zone");
        assert_eq!(cal.events[2].tzid, None, "events that are not the app's are left alone");
        assert_eq!(cal.timezones, vec![("Atlantic/Reykjavik".to_string(), 0)]);
    }

    #[test]
    fn the_choices_list_the_built_in_calendar_first() {
        let mut x = Extras::default();
        x.new_calendar("Work");
        assert_eq!(x.choices(), vec![(1, "Calendar".to_string()), (2, "Work".to_string())]);
    }
}

//! "Found in" suggestions: the events, flights, contacts and addresses a message mentions, offered back to the person
//! as things to add to the calendar, look up, save or open in the map. Everything runs on the device, over text the
//! app already shows; nothing is sent anywhere and nothing is kept except the keys of the suggestions the person
//! turned down ([`Dismissed`]).
//!
//! [`extract`] reads the text: a meeting time ([`crate::proposed_event`]), a flight number with the airports named
//! beside it (the table in [`airport`] gives each airport's city and time zone), and the emails, phone numbers and
//! street addresses of [`crate::detect`]. Each result is a [`Found`] that knows the app and message it came from. A
//! [`Pager`] is the model of the popover that shows them one at a time ("2 of 3"), with next, previous, dismiss and
//! accept. Accepting hands back an [`Action`] for the app to run; the view and the calendar, contacts and map stores
//! belong to the shell and the apps.

use crate::detect::{self, Kind};
use crate::proposed_event::{self, ProposedEvent};
use std::collections::BTreeSet;
use std::ops::Range;

/// An airport the suggestion engine knows: its IATA code, the city it serves and the time zone of that city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Airport {
    pub code: &'static str,
    pub city: &'static str,
    /// The tz database name of the airport's time zone.
    pub zone: &'static str,
}

const fn ap(code: &'static str, city: &'static str, zone: &'static str) -> Airport {
    Airport { code, city, zone }
}

static AIRPORTS: &[Airport] = &[
    ap("KEF", "Reykjavik", "Atlantic/Reykjavik"),
    ap("RKV", "Reykjavik", "Atlantic/Reykjavik"),
    ap("AEY", "Akureyri", "Atlantic/Reykjavik"),
    ap("EGS", "Egilsstadir", "Atlantic/Reykjavik"),
    ap("LHR", "London", "Europe/London"),
    ap("LGW", "London", "Europe/London"),
    ap("MAN", "Manchester", "Europe/London"),
    ap("EDI", "Edinburgh", "Europe/London"),
    ap("DUB", "Dublin", "Europe/Dublin"),
    ap("CPH", "Copenhagen", "Europe/Copenhagen"),
    ap("OSL", "Oslo", "Europe/Oslo"),
    ap("ARN", "Stockholm", "Europe/Stockholm"),
    ap("HEL", "Helsinki", "Europe/Helsinki"),
    ap("AMS", "Amsterdam", "Europe/Amsterdam"),
    ap("BRU", "Brussels", "Europe/Brussels"),
    ap("FRA", "Frankfurt", "Europe/Berlin"),
    ap("MUC", "Munich", "Europe/Berlin"),
    ap("BER", "Berlin", "Europe/Berlin"),
    ap("CDG", "Paris", "Europe/Paris"),
    ap("ZRH", "Zurich", "Europe/Zurich"),
    ap("VIE", "Vienna", "Europe/Vienna"),
    ap("MAD", "Madrid", "Europe/Madrid"),
    ap("BCN", "Barcelona", "Europe/Madrid"),
    ap("LIS", "Lisbon", "Europe/Lisbon"),
    ap("FCO", "Rome", "Europe/Rome"),
    ap("ATH", "Athens", "Europe/Athens"),
    ap("IST", "Istanbul", "Europe/Istanbul"),
    ap("DXB", "Dubai", "Asia/Dubai"),
    ap("DOH", "Doha", "Asia/Qatar"),
    ap("DEL", "Delhi", "Asia/Kolkata"),
    ap("SIN", "Singapore", "Asia/Singapore"),
    ap("HKG", "Hong Kong", "Asia/Hong_Kong"),
    ap("NRT", "Tokyo", "Asia/Tokyo"),
    ap("SYD", "Sydney", "Australia/Sydney"),
    ap("JFK", "New York", "America/New_York"),
    ap("EWR", "New York", "America/New_York"),
    ap("BOS", "Boston", "America/New_York"),
    ap("IAD", "Washington", "America/New_York"),
    ap("ATL", "Atlanta", "America/New_York"),
    ap("ORD", "Chicago", "America/Chicago"),
    ap("DFW", "Dallas", "America/Chicago"),
    ap("DEN", "Denver", "America/Denver"),
    ap("SEA", "Seattle", "America/Los_Angeles"),
    ap("SFO", "San Francisco", "America/Los_Angeles"),
    ap("LAX", "Los Angeles", "America/Los_Angeles"),
    ap("YYZ", "Toronto", "America/Toronto"),
    ap("YVR", "Vancouver", "America/Vancouver"),
    ap("MEX", "Mexico City", "America/Mexico_City"),
    ap("GRU", "Sao Paulo", "America/Sao_Paulo"),
];

/// The airport with this IATA code (letters in any case), or `None` when the table does not have it.
pub fn airport(code: &str) -> Option<&'static Airport> {
    AIRPORTS.iter().find(|a| a.code.eq_ignore_ascii_case(code))
}

/// Where a suggestion was found: the app and the item (a mail subject, a conversation name) shown to the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub app: String,
    pub title: String,
}

/// The kinds of suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuggestionKind {
    Event,
    Flight,
    Contact,
    Address,
}

/// What was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Event(ProposedEvent),
    Flight { airline: String, number: String, route: Option<(String, String)> },
    /// An email address or a phone number. A phone number is in international form when the region's rules accept it,
    /// otherwise as written.
    Contact { email: Option<String>, phone: Option<String> },
    Address(String),
}

/// What accepting a suggestion asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    AddEvent(ProposedEvent),
    ShowFlight { airline: String, number: String },
    AddContact { email: Option<String>, phone: Option<String> },
    OpenInMaps(String),
}

/// One suggestion with the row the popover shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub kind: SuggestionKind,
    pub content: Content,
    /// The bytes of the message the suggestion was read from.
    pub range: Range<usize>,
    /// The first line of the row.
    pub title: String,
    /// The second line of the row; empty when there is nothing to add.
    pub subtitle: String,
    pub source: Source,
}

impl Found {
    /// The line that says where the suggestion came from, such as "Found in Mail".
    pub fn found_in(&self) -> String {
        format!("Found in {}", self.source.app)
    }

    /// Identifies the suggestion, so one that was turned down is not offered again from the same item.
    pub fn key(&self) -> String {
        format!("{:?}|{}|{}|{}", self.kind, self.source.app, self.source.title, self.title.to_lowercase())
    }

    /// What accepting the suggestion does.
    pub fn action(&self) -> Action {
        match &self.content {
            Content::Event(e) => Action::AddEvent(e.clone()),
            Content::Flight { airline, number, .. } => Action::ShowFlight { airline: airline.clone(), number: number.clone() },
            Content::Contact { email, phone } => Action::AddContact { email: email.clone(), phone: phone.clone() },
            Content::Address(a) => Action::OpenInMaps(a.clone()),
        }
    }
}

/// The IATA codes of known airports named in `text`, outside `skip`, in reading order.
fn airports_named(text: &str, skip: &Range<usize>) -> Vec<&'static Airport> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (start, c.is_ascii_alphabetic()) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                let word = &text[s..i];
                let inside = s >= skip.start && i <= skip.end;
                if word.len() == 3 && word.chars().all(|c| c.is_ascii_uppercase()) && !inside {
                    if let Some(a) = airport(word) {
                        out.push(a);
                    }
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn minutes_label(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn day_label(e: &ProposedEvent) -> String {
    match e.day {
        proposed_event::Day::Today => "Today".to_string(),
        proposed_event::Day::Tomorrow => "Tomorrow".to_string(),
        proposed_event::Day::Weekday(d) => ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][(d % 7) as usize].to_string(),
    }
}

/// Reads the suggestions out of `text`, in the order they appear. `region` is the region code (such as `IS`) whose
/// phone rules turn a number into international form.
pub fn extract(source: &Source, text: &str, region: &str) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    if let Some(event) = proposed_event::find_proposal(text) {
        let range = event.span.clone();
        let subtitle = format!("{} at {}", day_label(&event), minutes_label(event.start_minute));
        out.push(Found { kind: SuggestionKind::Event, title: text[range.clone()].to_string(), subtitle, range, content: Content::Event(event), source: source.clone() });
    }
    for m in detect::find(text, &[Kind::Flight, Kind::Email, Kind::Phone, Kind::Address]) {
        let shown = m.text(text).to_string();
        let found = match m.kind {
            Kind::Flight => {
                let Some((airline, number)) = detect::flight_parts(&shown) else { continue };
                let named = airports_named(text, &m.range);
                let route = (named.len() >= 2).then(|| (named[0].code.to_string(), named[1].code.to_string()));
                let subtitle = if named.len() >= 2 { format!("{} to {}", named[0].city, named[1].city) } else { String::new() };
                Found {
                    kind: SuggestionKind::Flight,
                    content: Content::Flight { airline: airline.to_string(), number: number.to_string(), route },
                    title: format!("Flight {airline} {number}"),
                    subtitle,
                    range: m.range.clone(),
                    source: source.clone(),
                }
            }
            Kind::Email => Found {
                kind: SuggestionKind::Contact,
                content: Content::Contact { email: Some(shown.clone()), phone: None },
                title: shown,
                subtitle: String::new(),
                range: m.range.clone(),
                source: source.clone(),
            },
            Kind::Phone => {
                let phone = detect::phone_e164(&m, text, region).unwrap_or_else(|| shown.clone());
                Found {
                    kind: SuggestionKind::Contact,
                    content: Content::Contact { email: None, phone: Some(phone) },
                    title: shown,
                    subtitle: String::new(),
                    range: m.range.clone(),
                    source: source.clone(),
                }
            }
            Kind::Address => Found {
                kind: SuggestionKind::Address,
                content: Content::Address(shown.clone()),
                title: shown,
                subtitle: String::new(),
                range: m.range.clone(),
                source: source.clone(),
            },
            _ => continue,
        };
        out.push(found);
    }
    out.sort_by_key(|f| f.range.start);
    out
}

/// The keys of suggestions the person turned down.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dismissed {
    keys: BTreeSet<String>,
}

impl Dismissed {
    pub fn add(&mut self, found: &Found) {
        self.keys.insert(found.key());
    }

    pub fn contains(&self, found: &Found) -> bool {
        self.keys.contains(&found.key())
    }

    /// `items` without the ones turned down.
    pub fn filter(&self, items: Vec<Found>) -> Vec<Found> {
        items.into_iter().filter(|f| !self.contains(f)).collect()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// The popover that pages through suggestions one at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pager {
    items: Vec<Found>,
    index: usize,
}

impl Pager {
    pub fn new(items: Vec<Found>) -> Self {
        Pager { items, index: 0 }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The suggestion on show.
    pub fn current(&self) -> Option<&Found> {
        self.items.get(self.index)
    }

    /// "1 of 3" for the page on show, or `None` when there is nothing to show. A single suggestion needs no pager, so
    /// its label is `None` too.
    pub fn label(&self) -> Option<String> {
        (self.items.len() > 1).then(|| format!("{} of {}", self.index + 1, self.items.len()))
    }

    /// Moves to the next suggestion. False at the last one.
    pub fn next_page(&mut self) -> bool {
        if self.index + 1 < self.items.len() {
            self.index += 1;
            true
        } else {
            false
        }
    }

    /// Moves to the previous suggestion. False at the first one.
    pub fn previous_page(&mut self) -> bool {
        if self.index > 0 {
            self.index -= 1;
            true
        } else {
            false
        }
    }

    fn take_current(&mut self) -> Option<Found> {
        if self.index >= self.items.len() {
            return None;
        }
        let found = self.items.remove(self.index);
        if self.index >= self.items.len() {
            self.index = self.items.len().saturating_sub(1);
        }
        Some(found)
    }

    /// Turns down the suggestion on show and returns it, so the caller can remember it in [`Dismissed`]. The page
    /// that follows (or the one before, at the end) takes its place.
    pub fn dismiss(&mut self) -> Option<Found> {
        self.take_current()
    }

    /// Accepts the suggestion on show: it leaves the pager and its action is returned for the app to run.
    pub fn accept(&mut self) -> Option<Action> {
        self.take_current().map(|f| f.action())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail() -> Source {
        Source { app: "Mail".into(), title: "Plans".into() }
    }

    #[test]
    fn the_airport_table_has_unique_three_letter_codes_and_zones() {
        let mut codes: Vec<&str> = AIRPORTS.iter().map(|a| a.code).collect();
        assert!(codes.iter().all(|c| c.len() == 3 && c.chars().all(|ch| ch.is_ascii_uppercase())));
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), AIRPORTS.len());
        assert!(AIRPORTS.iter().all(|a| a.zone.contains('/') && !a.city.is_empty()));
        assert_eq!(airport("kef").map(|a| a.city), Some("Reykjavik"));
        assert_eq!(airport("JFK").map(|a| a.zone), Some("America/New_York"));
        assert_eq!(airport("XXX"), None);
    }

    #[test]
    fn a_meeting_time_becomes_an_event_suggestion() {
        let found = extract(&mail(), "Lunch tomorrow at 1pm", "IS");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SuggestionKind::Event);
        assert_eq!(found[0].title, "tomorrow at 1pm");
        assert_eq!(found[0].subtitle, "Tomorrow at 13:00");
        match found[0].action() {
            Action::AddEvent(e) => assert_eq!(e.start_minute, 13 * 60),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_flight_carries_its_route_from_the_airports_named_beside_it() {
        let text = "Your flight FI 552 from KEF to JFK is on time";
        let found = extract(&mail(), text, "IS");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SuggestionKind::Flight);
        assert_eq!(found[0].title, "Flight FI 552");
        assert_eq!(found[0].subtitle, "Reykjavik to New York");
        assert_eq!(
            found[0].content,
            Content::Flight { airline: "FI".into(), number: "552".into(), route: Some(("KEF".into(), "JFK".into())) }
        );
        assert_eq!(found[0].action(), Action::ShowFlight { airline: "FI".into(), number: "552".into() });
    }

    #[test]
    fn a_flight_without_airports_has_no_route() {
        let found = extract(&mail(), "Boarding BA117 now", "IS");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subtitle, "");
        assert!(matches!(&found[0].content, Content::Flight { route: None, .. }));
    }

    #[test]
    fn emails_phones_and_addresses_are_contacts_and_places() {
        let found = extract(&mail(), "write to ana@example.is", "IS");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, Content::Contact { email: Some("ana@example.is".into()), phone: None });
        assert_eq!(found[0].action(), Action::AddContact { email: Some("ana@example.is".into()), phone: None });

        let phone = extract(&mail(), "call 555 1234 later", "IS");
        assert_eq!(phone.len(), 1);
        assert_eq!(phone[0].kind, SuggestionKind::Contact);
        assert!(matches!(&phone[0].content, Content::Contact { email: None, phone: Some(p) } if !p.is_empty()));

        let place = extract(&mail(), "meet at Laugavegur 12", "IS");
        assert_eq!(place.len(), 1);
        assert_eq!(place[0].content, Content::Address("Laugavegur 12".into()));
        assert_eq!(place[0].action(), Action::OpenInMaps("Laugavegur 12".into()));
    }

    #[test]
    fn suggestions_come_in_reading_order_and_say_where_they_were_found() {
        let text = "Lunch tomorrow at 1pm, then flight FI 552 KEF to JFK. Mail ana@example.is";
        let found = extract(&mail(), text, "IS");
        let kinds: Vec<SuggestionKind> = found.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [SuggestionKind::Event, SuggestionKind::Flight, SuggestionKind::Contact]);
        assert!(found.windows(2).all(|w| w[0].range.start <= w[1].range.start));
        assert_eq!(found[0].found_in(), "Found in Mail");
        assert_eq!(&text[found[2].range.clone()], "ana@example.is");
    }

    #[test]
    fn plain_text_has_no_suggestions() {
        assert!(extract(&mail(), "nothing to see here", "IS").is_empty());
        assert!(extract(&mail(), "", "IS").is_empty());
    }

    #[test]
    fn the_pager_pages_dismisses_and_accepts() {
        let text = "Lunch tomorrow at 1pm, flight FI 552 KEF to JFK, mail ana@example.is";
        let mut pager = Pager::new(extract(&mail(), text, "IS"));
        assert_eq!(pager.len(), 3);
        assert_eq!(pager.label().as_deref(), Some("1 of 3"));
        assert_eq!(pager.current().map(|f| f.kind), Some(SuggestionKind::Event));
        assert!(!pager.previous_page());
        assert!(pager.next_page());
        assert!(pager.next_page());
        assert!(!pager.next_page());
        assert_eq!(pager.label().as_deref(), Some("3 of 3"));

        // Dismissing the last page shows the one before it.
        let gone = pager.dismiss().unwrap();
        assert_eq!(gone.kind, SuggestionKind::Contact);
        assert_eq!(pager.label().as_deref(), Some("2 of 2"));
        assert_eq!(pager.current().map(|f| f.kind), Some(SuggestionKind::Flight));

        // Accepting hands back the action, and the single page left needs no pager.
        assert_eq!(pager.accept(), Some(Action::ShowFlight { airline: "FI".into(), number: "552".into() }));
        assert_eq!(pager.len(), 1);
        assert_eq!(pager.label(), None);
        assert!(pager.accept().is_some());
        assert!(pager.is_empty());
        assert_eq!(pager.dismiss(), None);
        assert_eq!(pager.accept(), None);
        assert_eq!(pager.current(), None);
    }

    #[test]
    fn a_dismissed_suggestion_is_not_offered_again_from_the_same_item() {
        let text = "Mail ana@example.is or flight FI 552";
        let mut dismissed = Dismissed::default();
        assert!(dismissed.is_empty());
        let first = extract(&mail(), text, "IS");
        assert_eq!(first.len(), 2);
        let mut pager = Pager::new(first);
        let turned_down = pager.dismiss().unwrap();
        dismissed.add(&turned_down);
        assert_eq!(dismissed.len(), 1);

        let again = dismissed.filter(extract(&mail(), text, "IS"));
        assert_eq!(again.len(), 1);
        assert!(!dismissed.contains(&again[0]));
        // The same words in another message are a new suggestion.
        let other = Source { app: "Mail".into(), title: "Other".into() };
        assert_eq!(dismissed.filter(extract(&other, text, "IS")).len(), 2);
    }
}

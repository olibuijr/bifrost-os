//! Case-insensitive event search: every whitespace-separated term must occur
//! in at least one of the event's text fields.
use super::ics::IcsEvent;
use super::store::CalendarEvent;
use super::Date;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub date: Date,
    pub minute: u16,
    pub all_day: bool,
    pub title: String,
    pub location: String,
}

/// An empty query matches nothing.
pub fn matches(query: &str, fields: &[&str]) -> bool {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return false;
    }
    let hay: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    terms.iter().all(|t| hay.iter().any(|h| h.contains(t.as_str())))
}

/// Hits from stored events and rich (imported) events, de-duplicated by
/// date, time and title, ordered by date then time.
pub fn search(query: &str, store: &[CalendarEvent], rich: &[IcsEvent]) -> Vec<Hit> {
    // a saved event matches on its title, its location and its notes
    let mut hits: Vec<Hit> = store
        .iter()
        .filter(|e| matches(query, &[e.title.as_str(), e.location.as_str(), e.notes.as_str()]))
        .map(|e| Hit {
            date: e.date,
            minute: e.minute,
            all_day: e.all_day,
            title: e.title.clone(),
            location: e.location.clone(),
        })
        .collect();
    for e in rich {
        if !matches(query, &[e.summary.as_str(), e.location.as_str(), e.notes.as_str()]) {
            continue;
        }
        if let Some(h) = hits
            .iter_mut()
            .find(|h| h.date == e.start.date && h.minute == e.start.minute && h.title == e.summary)
        {
            h.location = e.location.clone();
            continue;
        }
        hits.push(Hit {
            date: e.start.date,
            minute: e.start.minute,
            all_day: e.all_day,
            title: e.summary.clone(),
            location: e.location.clone(),
        });
    }
    hits.sort_by(|a, b| (a.date, !a.all_day, a.minute, &a.title).cmp(&(b.date, !b.all_day, b.minute, &b.title)));
    hits
}

#[cfg(test)]
mod tests {
    use super::super::ics::Local;
    use super::*;

    fn d(day: u32) -> Date {
        Date { year: 2026, month: 10, day }
    }
    fn ev(id: u64, day: u32, title: &str) -> CalendarEvent {
        CalendarEvent { id, date: d(day), minute: 540, duration: 60, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0 }
    }

    #[test]
    fn terms_are_anded_and_case_insensitive() {
        assert!(matches("DENT appoint", &["Dentist appointment"]));
        assert!(!matches("dentist lunch", &["Dentist appointment"]));
        assert!(matches("room", &["Standup", "Meeting Room 4"]));
        assert!(!matches("   ", &["anything"]));
    }

    #[test]
    fn unicode_case_folding() {
        assert!(matches("\u{00fe}orri", &["\u{00de}orri blot"]));
    }

    #[test]
    fn search_orders_and_merges_rich_details() {
        let store = vec![ev(1, 9, "Team sync"), ev(2, 5, "Sync with Ann"), ev(3, 6, "Lunch")];
        let mut rich = IcsEvent::new("u", "Team sync", Local::new(d(9), 540), Local::new(d(9), 600));
        rich.location = "Room 4".into();
        let mut only_rich = IcsEvent::new("v", "Offsite", Local::new(d(20), 0), Local::new(d(20), 60));
        only_rich.notes = "sync deck".into();
        let hits = search("sync", &store, &[rich, only_rich]);
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, vec!["Sync with Ann", "Team sync", "Offsite"]);
        assert_eq!(hits[1].location, "Room 4");
    }

    #[test]
    fn a_saved_event_matches_on_its_location_and_notes() {
        let mut lunch = ev(4, 7, "Lunch");
        lunch.location = "Laugavegur 5".into();
        lunch.notes = "Bring the report".into();
        let hits = search("laugavegur report", &[lunch.clone()], &[]);
        assert_eq!(hits.len(), 1, "every term matches somewhere in the event");
        assert_eq!(hits[0].location, "Laugavegur 5");
        assert!(search("parking", &[lunch], &[]).is_empty());
    }

    #[test]
    fn empty_query_has_no_hits() {
        assert!(search("", &[ev(1, 1, "x")], &[]).is_empty());
    }
}

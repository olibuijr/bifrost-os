//! Invitees of an imported event: who was invited and how each person answered. The .ics file holds the attendee
//! lines (ATTENDEE with CN, ROLE, CUTYPE and PARTSTAT); this module turns them into the read-only
//! `Participant` values the editor shows. Apps cannot change attendance here, so there are no setters.
use super::exchange::occurrences;
use super::ics::IcsCalendar;
use super::Date;
use bifrost_app::controls::calendar_participant::{Kind, Participant, Role, Status};

/// Names the editor's Invitees row lists before "and n more".
pub const SHOWN: usize = 3;

/// The ROLE parameter of an attendee line.
pub fn role_of(value: &str) -> Role {
    match value.trim().to_ascii_uppercase().as_str() {
        "REQ-PARTICIPANT" => Role::Required,
        "OPT-PARTICIPANT" => Role::Optional,
        "CHAIR" => Role::Chair,
        "NON-PARTICIPANT" => Role::NonParticipant,
        _ => Role::Unknown,
    }
}

/// The CUTYPE parameter of an attendee line.
pub fn kind_of(value: &str) -> Kind {
    match value.trim().to_ascii_uppercase().as_str() {
        "INDIVIDUAL" => Kind::Person,
        "ROOM" => Kind::Room,
        "RESOURCE" => Kind::Resource,
        "GROUP" => Kind::Group,
        _ => Kind::Unknown,
    }
}

/// The PARTSTAT parameter of an attendee line.
pub fn status_of(value: &str) -> Status {
    match value.trim().to_ascii_uppercase().as_str() {
        "NEEDS-ACTION" => Status::Pending,
        "ACCEPTED" => Status::Accepted,
        "DECLINED" => Status::Declined,
        "TENTATIVE" => Status::Tentative,
        "DELEGATED" => Status::Delegated,
        "COMPLETED" => Status::Completed,
        "IN-PROCESS" => Status::InProcess,
        _ => Status::Unknown,
    }
}

/// One attendee from an ATTENDEE line: `value` is the URL (`mailto:ana@example.is`), the rest are its parameters.
/// A value without a scheme is dropped, as the participant model refuses it.
pub fn attendee(value: &str, cn: Option<&str>, role: Option<&str>, cutype: Option<&str>, partstat: Option<&str>) -> Option<Participant> {
    let role = role.map_or(Role::Unknown, role_of);
    let kind = cutype.map_or(Kind::Unknown, kind_of);
    let status = partstat.map_or(Status::Unknown, status_of);
    Participant::new(value, cn, role, kind, status, false).ok()
}

/// The word the editor shows for a response.
pub fn status_label(status: Status) -> &'static str {
    match status {
        Status::Accepted => "Accepted",
        Status::Declined => "Declined",
        Status::Tentative => "Maybe",
        Status::Pending => "Awaiting reply",
        Status::Delegated => "Delegated",
        Status::Completed => "Completed",
        Status::InProcess => "In progress",
        Status::Unknown => "No reply",
    }
}

/// The Invitees row's text: "Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply) and 2 more".
/// An empty list gives an empty string (the row is then left out).
pub fn summary(people: &[Participant]) -> String {
    if people.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = people
        .iter()
        .take(SHOWN)
        .map(|p| format!("{} ({})", p.display_name(), status_label(p.status())))
        .collect();
    let mut text = format!("Invitees: {}", shown.join(", "));
    let more = people.len().saturating_sub(SHOWN);
    if more > 0 {
        text.push_str(&format!(" and {more} more"));
    }
    text
}

/// The invitees of the imported event a store entry came from. The match is the one the export uses: the same
/// title, the same all-day flag, the same start minute (timed events) and an occurrence on `date`. The first
/// matching event that has attendees wins; a store entry with no match has no invitees.
pub fn for_event(rich: &IcsCalendar, title: &str, date: Date, minute: u16, all_day: bool) -> Vec<Participant> {
    index_for(rich, title, date, minute, all_day).map_or_else(Vec::new, |i| rich.events[i].attendees.clone())
}

/// The position in `rich.events` of the imported event with invitees that a store entry came from (the match `for_event`
/// describes). Rsvp writes the answer into that event.
pub fn index_for(rich: &IcsCalendar, title: &str, date: Date, minute: u16, all_day: bool) -> Option<usize> {
    rich.events.iter().position(|r| {
        !r.attendees.is_empty()
            && r.summary == title
            && r.all_day == all_day
            && (all_day || r.start.minute == minute)
            && occurrences(r, date, date).contains(&date)
    })
}

/// The ATTENDEE line for one invitee, as the .ics writer keeps it. The name is quoted when it holds a separator.
pub fn attendee_line(p: &Participant) -> String {
    let role = match p.role() {
        Role::Required => "REQ-PARTICIPANT",
        Role::Optional => "OPT-PARTICIPANT",
        Role::Chair => "CHAIR",
        Role::NonParticipant => "NON-PARTICIPANT",
        Role::Unknown => "REQ-PARTICIPANT",
    };
    let cutype = match p.kind() {
        Kind::Person => "INDIVIDUAL",
        Kind::Room => "ROOM",
        Kind::Resource => "RESOURCE",
        Kind::Group => "GROUP",
        Kind::Unknown => "UNKNOWN",
    };
    let partstat = match p.status() {
        Status::Pending => "NEEDS-ACTION",
        Status::Accepted => "ACCEPTED",
        Status::Declined => "DECLINED",
        Status::Tentative => "TENTATIVE",
        Status::Delegated => "DELEGATED",
        Status::Completed => "COMPLETED",
        Status::InProcess => "IN-PROCESS",
        Status::Unknown => "NEEDS-ACTION",
    };
    let mut line = String::from("ATTENDEE");
    if let Some(name) = p.name() {
        line.push_str(&format!(";CN=\"{}\"", name.replace('"', "")));
    }
    line.push_str(&format!(";ROLE={role};CUTYPE={cutype};PARTSTAT={partstat}:{}", p.url()));
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::ics::{self, IcsEvent, Local};

    fn d(day: u32) -> Date {
        Date { year: 2026, month: 10, day }
    }

    fn who(url: &str, name: Option<&str>, status: Status) -> Participant {
        Participant::new(url, name, Role::Required, Kind::Person, status, false).unwrap()
    }

    #[test]
    fn partstat_values_map_to_statuses() {
        assert_eq!(status_of("ACCEPTED"), Status::Accepted);
        assert_eq!(status_of("needs-action"), Status::Pending);
        assert_eq!(status_of("TENTATIVE"), Status::Tentative);
        assert_eq!(status_of("IN-PROCESS"), Status::InProcess);
        assert_eq!(status_of("sometimes"), Status::Unknown);
    }

    #[test]
    fn role_and_type_parameters_map_to_the_model() {
        assert_eq!(role_of("OPT-PARTICIPANT"), Role::Optional);
        assert_eq!(role_of("CHAIR"), Role::Chair);
        assert_eq!(role_of(""), Role::Unknown);
        assert_eq!(kind_of("ROOM"), Kind::Room);
        assert_eq!(kind_of("INDIVIDUAL"), Kind::Person);
        assert_eq!(kind_of("UNKNOWN"), Kind::Unknown);
    }

    #[test]
    fn an_attendee_line_becomes_a_participant_with_its_parameters() {
        let p = attendee("mailto:ana@example.is", Some("Ana Lind"), Some("REQ-PARTICIPANT"), Some("INDIVIDUAL"), Some("ACCEPTED"))
            .unwrap();
        assert_eq!(p.display_name(), "Ana Lind");
        assert_eq!(p.address(), "ana@example.is");
        assert_eq!(p.role(), Role::Required);
        assert_eq!(p.kind(), Kind::Person);
        assert_eq!(p.status(), Status::Accepted);
        assert!(!p.is_current_user());
    }

    #[test]
    fn a_missing_parameter_is_unknown_and_a_schemeless_value_is_dropped() {
        let p = attendee("mailto:bo@example.is", None, None, None, None).unwrap();
        assert_eq!(p.role(), Role::Unknown);
        assert_eq!(p.kind(), Kind::Unknown);
        assert_eq!(p.status(), Status::Unknown);
        assert_eq!(p.display_name(), "bo@example.is");
        assert!(attendee("bo@example.is", None, None, None, None).is_none());
    }

    #[test]
    fn the_summary_lists_names_with_their_responses() {
        let people = vec![
            who("mailto:ana@example.is", Some("Ana Lind"), Status::Accepted),
            who("mailto:bo@example.is", None, Status::Pending),
        ];
        assert_eq!(summary(&people), "Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply)");
    }

    #[test]
    fn the_summary_counts_the_people_past_the_first_three() {
        let people: Vec<Participant> = (1..=5)
            .map(|n| who(&format!("mailto:p{n}@example.is"), None, Status::Tentative))
            .collect();
        assert_eq!(
            summary(&people),
            "Invitees: p1@example.is (Maybe), p2@example.is (Maybe), p3@example.is (Maybe) and 2 more"
        );
        assert_eq!(summary(&[]), "");
    }

    fn timed(summary: &str, minute: u16, attendees: Vec<Participant>) -> IcsEvent {
        let start = Local::new(d(12), minute);
        let mut e = IcsEvent::new("uid-1", summary, start, start.add_minutes(60));
        e.attendees = attendees;
        e
    }

    #[test]
    fn invitees_come_from_the_imported_event_at_the_same_time_and_title() {
        let rich = IcsCalendar {
            name: String::new(),
            events: vec![timed("Standup", 540, vec![who("mailto:ana@example.is", Some("Ana"), Status::Accepted)])],
            timezones: vec![],
        };
        let found = for_event(&rich, "Standup", d(12), 540, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].display_name(), "Ana");
        assert!(for_event(&rich, "Standup", d(12), 600, false).is_empty(), "another start time has no invitees");
        assert!(for_event(&rich, "Standup", d(13), 540, false).is_empty(), "another day has no invitees");
        assert!(for_event(&rich, "Lunch", d(12), 540, false).is_empty(), "another title has no invitees");
    }

    #[test]
    fn an_attendee_line_survives_a_write_and_a_parse() {
        let p = attendee("mailto:ana@example.is", Some("Ana, Lind"), Some("OPT-PARTICIPANT"), Some("INDIVIDUAL"), Some("TENTATIVE"))
            .unwrap();
        let line = attendee_line(&p);
        assert_eq!(
            line,
            "ATTENDEE;CN=\"Ana, Lind\";ROLE=OPT-PARTICIPANT;CUTYPE=INDIVIDUAL;PARTSTAT=TENTATIVE:mailto:ana@example.is"
        );
        let text = format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261012T090000\r\nSUMMARY:Sync\r\n{line}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
        let cal = ics::parse(&text);
        assert_eq!(cal.events[0].attendees, vec![p]);
    }
}

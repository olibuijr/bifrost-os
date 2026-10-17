//! Answers to invitations (iTIP, RFC 5546). An imported event lists its invitees with ATTENDEE lines; when the account's
//! own address is one of them with PARTSTAT=NEEDS-ACTION, the user can Accept, Maybe or Decline. The answer is written
//! into the event's PARTSTAT in calendar.ics, and a METHOD:REPLY calendar holding only the replying ATTENDEE line is
//! written to `itip-replies/` beside the events file. The reply opens in Mail (`mail_compose`) addressed to the organiser,
//! with that .ics attached. An event the account organises, with invitees, sends a METHOD:REQUEST to them the same way
//! when it is saved (`invite_attendees`). Both go through the outbox (`Calendar::take_outgoing`), which the app drains.
use super::ics::{self, IcsCalendar, IcsEvent};
use super::invitees;
use super::store::CalendarEvent;
use super::{exchange, Calendar};
use bifrost_app::controls::calendar_participant::{Participant, Status};
use bifrost_app::controls::internet_accounts::{self, Service};
use bifrost_app::dav_sync::first_account;
use std::path::{Path, PathBuf};

/// The folder beside the events file that holds the replies waiting to be sent.
pub const REPLY_DIR: &str = "itip-replies";
/// The folder beside the events file that holds the invitations (METHOD:REQUEST) sent for saved events.
pub const REQUEST_DIR: &str = "itip-requests";

/// A message for Mail: the recipients, the subject and the body, with the .ics file to attach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    pub file: PathBuf,
}

/// The three answers the editor offers, in the order it lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Maybe,
    Decline,
}

impl Answer {
    pub const ALL: [Answer; 3] = [Answer::Accept, Answer::Maybe, Answer::Decline];

    /// The PARTSTAT the answer writes.
    pub fn status(self) -> Status {
        match self {
            Answer::Accept => Status::Accepted,
            Answer::Maybe => Status::Tentative,
            Answer::Decline => Status::Declined,
        }
    }

    /// The button's words.
    pub fn label(self) -> &'static str {
        match self {
            Answer::Accept => "Accept",
            Answer::Maybe => "Maybe",
            Answer::Decline => "Decline",
        }
    }
}

/// The address of a `mailto:` URL without its scheme (the scheme may be upper case).
pub fn mailto_address(url: &str) -> &str {
    url.split_once(':').filter(|(scheme, _)| scheme.eq_ignore_ascii_case("mailto")).map_or(url, |(_, rest)| rest)
}

/// The address of an attendee without its `mailto:` scheme, compared without case.
fn address_of(p: &Participant) -> &str {
    mailto_address(p.url())
}

/// The attendee line of the account's own address, if the account is one of the invitees.
pub fn mine<'a>(attendees: &'a [Participant], email: &str) -> Option<&'a Participant> {
    let email = email.trim();
    attendees.iter().find(|p| !email.is_empty() && address_of(p).eq_ignore_ascii_case(email))
}

/// True when the account is invited and has not answered yet: the editor then shows Accept, Maybe and Decline.
pub fn awaiting(attendees: &[Participant], email: &str) -> bool {
    mine(attendees, email).is_some_and(|p| p.status() == Status::Pending)
}

/// True when the account declined the event.
pub fn declined(attendees: &[Participant], email: &str) -> bool {
    mine(attendees, email).is_some_and(|p| p.status() == Status::Declined)
}

/// Whether an event is left out of the grids: the account declined it and View > Show Declined Events is off.
pub fn hidden_by_decline(attendees: &[Participant], email: &str, show_declined: bool) -> bool {
    !show_declined && declined(attendees, email)
}

/// The METHOD:REPLY calendar for one attendee, RFC 5546 section 3.2.3: the organiser's address, and only the replying
/// attendee's ATTENDEE line with its PARTSTAT. Lines end in CRLF, as the rest of the calendar files do.
pub fn reply_text(organizer: &str, attendee: &Participant, uid: &str, stamp: &str) -> String {
    let mut out = String::new();
    let mut add = |line: String| out.push_str(&ics::fold(&line));
    add("BEGIN:VCALENDAR".into());
    add("VERSION:2.0".into());
    add("PRODID:-//BifrOSt//Calendar//EN".into());
    add("METHOD:REPLY".into());
    add("BEGIN:VEVENT".into());
    add(format!("UID:{}", ics::escape(uid)));
    add(format!("DTSTAMP:{stamp}"));
    add(format!("ORGANIZER:{organizer}"));
    add(invitees::attendee_line(attendee));
    add("END:VEVENT".into());
    add("END:VCALENDAR".into());
    out
}

/// Records `answer` for the account on imported event `index` of `rich` and returns the reply to send. Only an invitee
/// who has not answered yet can answer, and only an event with an organiser has a reply to send.
pub fn respond(rich: &mut IcsCalendar, index: usize, email: &str, answer: Answer, stamp: &str) -> Result<String, &'static str> {
    let event: &mut IcsEvent = rich.events.get_mut(index).ok_or("This event is not in the calendar file.")?;
    let organizer = event.organizer.clone().ok_or("This invitation has no organiser to reply to.")?;
    let at = event.attendees.iter().position(|p| address_of(p).eq_ignore_ascii_case(email.trim())).ok_or("You are not invited to this event.")?;
    let old = &event.attendees[at];
    if old.status() != Status::Pending {
        return Err("This invitation already has an answer.");
    }
    let new = Participant::new(old.url(), old.name(), old.role(), old.kind(), answer.status(), true).map_err(|_| "The invitee address is not valid.")?;
    event.attendees[at] = new.clone();
    Ok(reply_text(&organizer, &new, &event.uid, stamp))
}

/// A file name for the reply of event `uid`: the uid with anything but letters, digits and `-_.@` replaced by `_`.
pub fn reply_file_name(uid: &str) -> String {
    let clean: String = uid.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.@".contains(c) { c } else { '_' }).take(120).collect();
    format!("{clean}.ics")
}

/// Writes the reply beside the events file, in `itip-replies/`. Returns the path written.
pub fn save_reply(events_path: &Path, uid: &str, text: &str) -> std::io::Result<PathBuf> {
    save_in(events_path, REPLY_DIR, uid, text)
}

/// Writes an iTIP file for event `uid` into `dir` beside the events file. Returns the path written.
pub fn save_in(events_path: &Path, dir: &str, uid: &str, text: &str) -> std::io::Result<PathBuf> {
    let dir = events_path.parent().unwrap_or(Path::new(".")).join(dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(reply_file_name(uid));
    springboard::shell::fsutil::write_atomic(&path, &text.to_string())?;
    Ok(path)
}

/// The Mail message that carries an answer to the organiser: the verb of the answer as the subject, and the reply file.
pub fn reply_mail(organizer: &str, answer: Answer, title: &str, file: PathBuf) -> Outgoing {
    let verb = match answer {
        Answer::Accept => "Accepted",
        Answer::Maybe => "Tentative",
        Answer::Decline => "Declined",
    };
    Outgoing {
        to: vec![mailto_address(organizer).to_string()],
        subject: format!("{verb}: {title}"),
        body: format!("The reply to \"{title}\" is attached.\n"),
        file,
    }
}

/// The METHOD:REQUEST calendar for a saved event: the event's own lines with its invitees and organiser (RFC 5546 3.2.2).
pub fn request_text(event: &IcsEvent, timezones: &[(String, i32)], stamp: &str) -> String {
    ics::write_single(event, timezones, stamp).replacen("VERSION:2.0\r\n", "VERSION:2.0\r\nMETHOD:REQUEST\r\n", 1)
}

impl Calendar {
    /// The messages for Mail that the last actions queued (replies and invitations). The app sends them.
    pub fn take_outgoing(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outbox)
    }

    /// After an event is saved: when the account organises the imported event `original` (the event before the edit,
    /// matched as `invitees::index_for` matches it), its invitees get the saved event as a METHOD:REQUEST in the outbox.
    /// An event the account does not organise, or with no invitees, sends nothing.
    pub(super) fn invite_attendees(&mut self, original: &CalendarEvent, saved: &CalendarEvent) {
        let Some(email) = self.account_email.clone() else { return };
        let rich = self.rich_events();
        let Some(index) = invitees::index_for(&rich, &original.title, original.date, original.minute, original.all_day) else { return };
        let imported = &rich.events[index];
        let organised = imported.organizer.as_deref().is_some_and(|o| mailto_address(o).eq_ignore_ascii_case(email.trim()));
        if !organised {
            return;
        }
        let to: Vec<String> = imported
            .attendees
            .iter()
            .map(|p| address_of(p).to_string())
            .filter(|a| !a.eq_ignore_ascii_case(email.trim()))
            .collect();
        if to.is_empty() {
            return;
        }
        let mut event = ics::from_calendar_event(saved);
        event.uid = imported.uid.clone();
        event.location = saved.location.clone();
        event.notes = saved.notes.clone();
        event.organizer = imported.organizer.clone();
        event.attendees = imported.attendees.clone();
        let stamp = ics::stamp(self.wall.unix);
        let text = request_text(&event, &rich.timezones, &stamp);
        match save_in(&self.path, REQUEST_DIR, &event.uid, &text) {
            Ok(file) => self.outbox.push(Outgoing {
                to,
                subject: format!("Invitation: {}", saved.title),
                body: format!("You are invited to \"{}\". The invitation is attached.\n", saved.title),
                file,
            }),
            Err(e) => self.notice = Some(format!("The invitation could not be written: {e}")),
        }
    }

    /// Whether the account has an invitation to answer among `people` (the open event's invitees).
    pub(super) fn awaiting_reply(&self, people: &[Participant]) -> bool {
        self.account_email.as_deref().is_some_and(|email| awaiting(people, email))
    }

    /// Sets the account address that answers and declined events use. The declined events are worked out again.
    pub fn set_account_email(&mut self, email: Option<String>) {
        self.account_email = email;
        self.declined_for = None;
    }

    /// Accept, Maybe or Decline on the open invitation. The answer is written into calendar.ics, the reply is saved in
    /// `itip-replies/` and queued for Mail, addressed to the organiser, and the editor reads its invitees again.
    pub(super) fn respond_to_invitation(&mut self, answer: Answer) {
        let Some(draft) = self.editor.as_ref() else { return };
        let (title, date, minute, all_day) = (draft.event.title.clone(), draft.event.date, draft.event.minute, draft.event.all_day);
        let Some(email) = self.account_email.clone() else {
            self.notice = Some("No account address to answer with.".into());
            return;
        };
        let mut rich = self.rich_events();
        let Some(index) = invitees::index_for(&rich, &title, date, minute, all_day) else {
            self.notice = Some("This invitation is not in the calendar file.".into());
            return;
        };
        let uid = rich.events[index].uid.clone();
        let stamp = ics::stamp(self.wall.unix);
        match respond(&mut rich, index, &email, answer, &stamp) {
            Ok(reply) => {
                let organizer = rich.events[index].organizer.clone().unwrap_or_default();
                let saved = springboard::shell::fsutil::write_atomic(&exchange::rich_path(&self.path), &ics::write(&rich, &stamp))
                    .map_err(|e| e.to_string())
                    .and_then(|_| save_reply(&self.path, &uid, &reply).map_err(|e| e.to_string()));
                match saved {
                    Ok(file) => self.outbox.push(reply_mail(&organizer, answer, &title, file)),
                    Err(message) => {
                        self.notice = Some(message);
                        return;
                    }
                }
                let people = invitees::for_event(&rich, &title, date, minute, all_day);
                if let Some(draft) = self.editor.as_mut() {
                    draft.invitees = people;
                }
                self.declined_for = None;
                self.notice = Some(format!("{} saved. The reply opens in Mail.", answer.label()));
                self.dirty = true;
            }
            Err(message) => self.notice = Some(message.into()),
        }
    }
}

/// The account's own address: the username of the first CalDAV account that has Calendars switched on, when it is an
/// email address. None when no such account exists.
pub fn account_email() -> Option<String> {
    let path = internet_accounts::store_path()?;
    let all = internet_accounts::load(&path).ok()?;
    first_account(&all, Service::Calendars).map(|a| a.username).filter(|u| u.contains('@'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::calendar_participant::{Kind, Role};

    const ORGANISED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:evt-1@x\r\nSUMMARY:Review\r\n\
        DTSTART:20261020T090000Z\r\nDTEND:20261020T100000Z\r\nORGANIZER;CN=Bo:mailto:bo@example.is\r\n\
        ATTENDEE;CN=\"Ana Lind\";ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION:mailto:ana@example.is\r\n\
        ATTENDEE;PARTSTAT=ACCEPTED:mailto:cy@example.is\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn ana(status: Status) -> Participant {
        Participant::new("mailto:ana@example.is", Some("Ana Lind"), Role::Required, Kind::Person, status, false).unwrap()
    }

    #[test]
    fn partstat_and_organizer_are_read_from_the_attendee_lines() {
        let cal = ics::parse(ORGANISED);
        let event = &cal.events[0];
        assert_eq!(event.organizer.as_deref(), Some("mailto:bo@example.is"));
        assert_eq!(event.attendees.len(), 2);
        let me = mine(&event.attendees, "ANA@example.is").expect("ana is invited");
        assert_eq!(me.status(), Status::Pending);
        assert!(awaiting(&event.attendees, "ana@example.is"));
        assert!(!awaiting(&event.attendees, "bo@example.is"), "the organiser is not an invitee here");
        assert!(!awaiting(&event.attendees, ""), "a blank account address matches nobody");
        assert_eq!(event.attendees[1].status(), Status::Accepted);
    }

    #[test]
    fn an_uppercase_mailto_scheme_still_matches_the_account() {
        let p = Participant::new("MAILTO:ana@example.is", None, Role::Required, Kind::Person, Status::Pending, false).unwrap();
        assert!(mine(&[p], "ana@example.is").is_some());
    }

    #[test]
    fn the_reply_is_a_method_reply_with_only_the_one_attendee_line() {
        let text = reply_text("mailto:bo@example.is", &ana(Status::Accepted), "evt-1@x", "20261017T090000Z");
        // long lines are folded with CRLF and a space (RFC 5545 3.1), so the check unfolds them first
        assert!(text.ends_with("END:VCALENDAR\r\n") && text.contains("\r\n"));
        assert_eq!(
            ics::unfold(&text),
            vec![
                "BEGIN:VCALENDAR",
                "VERSION:2.0",
                "PRODID:-//BifrOSt//Calendar//EN",
                "METHOD:REPLY",
                "BEGIN:VEVENT",
                "UID:evt-1@x",
                "DTSTAMP:20261017T090000Z",
                "ORGANIZER:mailto:bo@example.is",
                "ATTENDEE;CN=\"Ana Lind\";ROLE=REQ-PARTICIPANT;CUTYPE=INDIVIDUAL;PARTSTAT=ACCEPTED:mailto:ana@example.is",
                "END:VEVENT",
                "END:VCALENDAR",
            ]
        );
        assert_eq!(text.matches("ATTENDEE").count(), 1);
    }

    #[test]
    fn each_answer_writes_its_partstat_into_the_reply() {
        assert_eq!(Answer::Maybe.status(), Status::Tentative);
        let text = reply_text("mailto:bo@example.is", &ana(Answer::Decline.status()), "u", "20261017T090000Z");
        assert!(ics::unfold(&text).iter().any(|l| l.ends_with("PARTSTAT=DECLINED:mailto:ana@example.is")));
        assert_eq!(Answer::ALL.map(Answer::label), ["Accept", "Maybe", "Decline"]);
    }

    #[test]
    fn responding_updates_partstat_and_returns_the_reply() {
        let mut cal = ics::parse(ORGANISED);
        let reply = respond(&mut cal, 0, "ana@example.is", Answer::Maybe, "20261017T090000Z").unwrap();
        let lines = ics::unfold(&reply);
        assert!(lines.iter().any(|l| l == "METHOD:REPLY") && lines.iter().any(|l| l.ends_with("PARTSTAT=TENTATIVE:mailto:ana@example.is")));
        let me = mine(&cal.events[0].attendees, "ana@example.is").unwrap();
        assert_eq!(me.status(), Status::Tentative);
        assert!(me.is_current_user(), "the answered line is marked as the account's own");
        assert_eq!(respond(&mut cal, 0, "ana@example.is", Answer::Accept, "t"), Err("This invitation already has an answer."));
        assert_eq!(respond(&mut cal, 0, "cy@example.is", Answer::Accept, "t"), Err("This invitation already has an answer."));
        assert_eq!(respond(&mut cal, 0, "zed@example.is", Answer::Accept, "t"), Err("You are not invited to this event."));
        assert!(respond(&mut cal, 9, "ana@example.is", Answer::Accept, "t").is_err());
    }

    #[test]
    fn an_event_without_an_organiser_cannot_be_answered() {
        let text = ORGANISED.replace("ORGANIZER;CN=Bo:mailto:bo@example.is\r\n", "");
        let mut cal = ics::parse(&text);
        assert_eq!(respond(&mut cal, 0, "ana@example.is", Answer::Accept, "t"), Err("This invitation has no organiser to reply to."));
    }

    #[test]
    fn declined_events_are_hidden_only_while_the_view_option_is_off() {
        let declined_list = vec![ana(Status::Declined)];
        let pending_list = vec![ana(Status::Pending)];
        assert!(hidden_by_decline(&declined_list, "ana@example.is", false));
        assert!(!hidden_by_decline(&declined_list, "ana@example.is", true));
        assert!(!hidden_by_decline(&pending_list, "ana@example.is", false));
        assert!(!hidden_by_decline(&declined_list, "other@example.is", false), "someone else's answer does not hide it");
    }

    #[test]
    fn reply_files_are_named_by_a_safe_uid() {
        assert_eq!(reply_file_name("a/b c@x"), "a_b_c@x.ics");
        assert_eq!(reply_file_name("evt-1@x"), "evt-1@x.ics");
    }
}

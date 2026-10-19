//! Adding invitees in the event editor. The Add Invitees row opens a line in the form: an email address, or the first
//! letters of a name in Contacts, which shows up to three matching people under it (`birthdays::suggest`, from the
//! address book the Contacts app keeps; nothing is looked up anywhere else). Return adds the typed address, or the first
//! match; a tap on a match adds that person. Each added person has a Remove row. The invitees are kept with the event
//! (`extras.rs`), and saving an event that has some writes a METHOD:REQUEST invitation (RFC 5546) for them into
//! `itip-requests/`, addressed from the account's own address, and queues it for Mail (`rsvp.rs`); Calendar never sends
//! mail itself. Without an account address the invitees are kept and the notice says why nothing was sent.
use super::birthdays::{self, Entry};
use super::extras::{self, clean_address, Invitee, MAX_INVITEES};
use super::*;
use bifrost_app::controls::calendar_participant::{Kind, Role, Status};
use bifrost_app::controls::TextField as Line;

/// How many matches of the address book show under the invitee line.
pub(super) const SUGGESTIONS: usize = 3;

/// The participant an invitation to `who` lists: a required person who has not answered.
pub fn participant(who: &Invitee) -> Option<Participant> {
    let name = (!who.name.is_empty()).then_some(who.name.as_str());
    Participant::new(&format!("mailto:{}", who.address), name, Role::Required, Kind::Person, Status::Pending, false).ok()
}

/// The row of the invitee list for `who`: "Ana Lind" with the address after it, or the address alone.
pub fn row_label(who: &Invitee) -> String {
    if who.name.is_empty() {
        who.address.clone()
    } else {
        format!("{}  {}", who.name, who.address)
    }
}

impl Calendar {
    /// The people of the address book the typed line matches, leaving out the ones already invited.
    pub(super) fn invitee_suggestions(&self) -> Vec<Entry> {
        let Some(draft) = &self.editor else { return vec![] };
        let (Some(line), Some(book)) = (&draft.invitee_line, &self.book) else { return vec![] };
        birthdays::suggest(book, line, usize::MAX)
            .into_iter()
            .filter(|e| !draft.extra.invitees.iter().any(|p| p.address.eq_ignore_ascii_case(&e.email)))
            .take(SUGGESTIONS)
            .cloned()
            .collect()
    }

    /// The invitee rows under the Add Invitees row: the matches of the typed line, then a Remove row per invitee.
    pub(super) fn invitee_targets(&self, draft: &Draft) -> Vec<Target> {
        let mut out: Vec<Target> = (0..self.invitee_suggestions().len()).map(Target::InviteeSuggest).collect();
        out.extend((0..draft.extra.invitees.len()).map(Target::InviteeDrop));
        out
    }

    /// A tap on one of the invitee rows.
    pub(super) fn invitee_tap(&mut self, target: Target) {
        match target {
            Target::AddInvitee => self.open_invitee_line(),
            Target::InviteeDrop(i) => {
                if let Some(draft) = self.editor.as_mut() {
                    if i < draft.extra.invitees.len() {
                        draft.extra.invitees.remove(i);
                    }
                }
            }
            Target::InviteeSuggest(i) => {
                if let Some(entry) = self.invitee_suggestions().get(i).cloned() {
                    self.add_invitee(&entry.email, &entry.name);
                }
            }
            _ => {}
        }
    }

    /// Opens the invitee line, and reads the address book the first time.
    fn open_invitee_line(&mut self) {
        self.focus(None);
        if self.book.is_none() {
            self.book = Some(birthdays::load_book(&self.path));
        }
        if let Some(draft) = self.editor.as_mut() {
            if draft.invitee_line.is_none() {
                draft.invitee_line = Some(Line::new());
            }
        }
    }

    /// Whether the invitee line is open and has the cursor.
    pub(super) fn invitee_line_open(&self) -> bool {
        self.editor.as_ref().is_some_and(|d| d.invitee_line.is_some())
    }

    /// A key for the invitee line, when it is open: text and Backspace edit it, Left and Right move in it, Return adds
    /// the person, and Hide closes it. False when the line is not open (the key is for the fields then).
    pub(super) fn invitee_key(&mut self, out: Out) -> bool {
        let Some(draft) = self.editor.as_mut() else { return false };
        if draft.invitee_line.is_none() {
            return false;
        }
        // a message from the last key is gone with the next one; this key may leave a new one
        self.error = None;
        self.notice = None;
        match out {
            Out::Hide => draft.invitee_line = None,
            Out::Enter => self.add_typed_invitee(),
            Out::Text(text) => {
                if let Some(line) = draft.invitee_line.as_mut() {
                    line.push_str(&text);
                }
            }
            Out::Backspace => {
                if let Some(line) = draft.invitee_line.as_mut() {
                    line.pop();
                }
            }
            Out::Named("Left") => {
                if let Some(line) = draft.invitee_line.as_mut() {
                    line.move_caret(false);
                }
            }
            Out::Named("Right") => {
                if let Some(line) = draft.invitee_line.as_mut() {
                    line.move_caret(true);
                }
            }
            _ => {}
        }
        self.dirty = true;
        true
    }

    /// Text pasted into the invitee line (Edit > Paste): the first line of it. False when the line is not open.
    pub(super) fn invitee_paste(&mut self, text: &str) -> bool {
        let Some(line) = self.editor.as_mut().and_then(|d| d.invitee_line.as_mut()) else { return false };
        line.push_str(text.lines().next().unwrap_or(""));
        self.error = None;
        self.dirty = true;
        true
    }

    /// Return in the line: an empty line closes it; an address is added; anything else adds the first match of the address
    /// book, or says what to type.
    fn add_typed_invitee(&mut self) {
        let typed = self.editor.as_ref().and_then(|d| d.invitee_line.as_ref()).map(|l| l.to_string()).unwrap_or_default();
        if typed.trim().is_empty() {
            if let Some(draft) = self.editor.as_mut() {
                draft.invitee_line = None;
            }
            return;
        }
        let (address, name) = match clean_address(&typed) {
            Some(address) => {
                let known = self.book.as_ref().and_then(|b| b.iter().find(|e| e.email.eq_ignore_ascii_case(&address))).map(|e| e.name.clone());
                (address, known.unwrap_or_default())
            }
            None => match self.invitee_suggestions().first() {
                Some(entry) => (entry.email.clone(), entry.name.clone()),
                None => {
                    self.error = Some("Type an email address, or the name of someone in Contacts".into());
                    return;
                }
            },
        };
        self.add_invitee(&address, &name);
    }

    /// Invites `address` (named `name` when it has a name): the line is emptied for the next one. False, with the reason
    /// in the notice or the error, when the address is not one, is invited already, or the event has its limit.
    pub(super) fn add_invitee(&mut self, address: &str, name: &str) -> bool {
        let Some(address) = clean_address(address) else {
            self.error = Some("That is not an email address".into());
            return false;
        };
        let name = extras::clean_name(name);
        let Some(draft) = self.editor.as_mut() else { return false };
        let already = draft.extra.invitees.iter().any(|p| p.address.eq_ignore_ascii_case(&address));
        let full = draft.extra.invitees.len() >= MAX_INVITEES;
        if !already && !full {
            draft.extra.invitees.push(Invitee { address, name });
        }
        draft.invitee_line = Some(Line::new());
        if already {
            self.notice = Some("Already invited".into());
        } else if full {
            self.error = Some(format!("An event can have {MAX_INVITEES} invitees"));
        }
        !already && !full
    }

    /// After an event is saved: its invitees from the editor get the saved event as a METHOD:REQUEST in the outbox, from
    /// the account's own address as the organiser. Nothing is queued for an event with no invitees. Without an account
    /// address the invitees are kept and the notice says the invitations were not sent.
    pub(super) fn invite_new_attendees(&mut self, saved: &CalendarEvent) {
        let Some(extra) = self.extras.events.get(&saved.id).cloned() else { return };
        if extra.invitees.is_empty() {
            return;
        }
        let Some(email) = self.account_email.clone() else {
            self.notice = Some("Invitees saved. Set up a calendar account in Internet Accounts to send invitations".into());
            return;
        };
        let mut event = ics::from_calendar_event(saved);
        event.location = saved.location.clone();
        event.notes = saved.notes.clone();
        event.organizer = Some(format!("mailto:{email}"));
        event.attendees = extra.invitees.iter().filter_map(participant).collect();
        event.rule = match extra.rrule.as_deref() {
            Some(rule) => recur::parse_rrule(rule),
            None => alerts::REPEAT_RULES.get(saved.repeat as usize).and_then(|r| recur::parse_rrule(r)),
        };
        let mut zones: Vec<(String, i32)> = vec![];
        if let (Some(zone), false) = (extra.tzid.clone(), saved.all_day) {
            if let Some(minutes) = extras::zone_minutes(&zone) {
                zones.push((zone.clone(), minutes));
                event.tzid = Some(zone);
            }
        }
        let stamp = ics::stamp(self.wall.unix);
        let text = rsvp::request_text(&event, &zones, &stamp);
        match rsvp::save_in(&self.path, rsvp::REQUEST_DIR, &event.uid, &text) {
            Ok(file) => {
                let to: Vec<String> = extra.invitees.iter().map(|p| p.address.clone()).collect();
                self.outbox.push(rsvp::Outgoing {
                    to,
                    subject: format!("Invitation: {}", saved.title),
                    body: format!("You are invited to \"{}\". The invitation is attached.\n", saved.title),
                    file,
                });
                self.notice = Some("The invitation opens in Mail".into());
            }
            Err(e) => self.notice = Some(format!("The invitation could not be written: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(address: &str, name: &str) -> Invitee {
        Invitee { address: address.into(), name: name.into() }
    }

    #[test]
    fn an_invitee_is_a_required_person_who_has_not_answered() {
        let p = participant(&who("ana@example.is", "Ana Lind")).unwrap();
        assert_eq!(p.address(), "ana@example.is");
        assert_eq!(p.display_name(), "Ana Lind");
        assert_eq!((p.role(), p.kind(), p.status()), (Role::Required, Kind::Person, Status::Pending));
        let bare = participant(&who("bo@example.is", "")).unwrap();
        assert_eq!(bare.display_name(), "bo@example.is");
    }

    #[test]
    fn the_invite_line_shows_the_name_and_the_address() {
        assert_eq!(row_label(&who("ana@example.is", "Ana Lind")), "Ana Lind  ana@example.is");
        assert_eq!(row_label(&who("bo@example.is", "")), "bo@example.is");
    }

    #[test]
    fn an_invitation_names_each_invitee_as_awaiting_a_reply() {
        let attendees: Vec<Participant> = [who("ana@example.is", "Ana Lind"), who("bo@example.is", "")].iter().filter_map(participant).collect();
        let line = invitees::attendee_line(&attendees[0]);
        assert_eq!(line, "ATTENDEE;CN=\"Ana Lind\";ROLE=REQ-PARTICIPANT;CUTYPE=INDIVIDUAL;PARTSTAT=NEEDS-ACTION:mailto:ana@example.is");
        assert_eq!(invitees::attendee_line(&attendees[1]), "ATTENDEE;ROLE=REQ-PARTICIPANT;CUTYPE=INDIVIDUAL;PARTSTAT=NEEDS-ACTION:mailto:bo@example.is");
    }
}

//! Participants and virtual conferences of a calendar event. `Roster` holds the people, groups, rooms and resources
//! invited to an event (`NativeParticipant`) and counts how the attendees answered. `ConferenceProvider` holds the room
//! types a conference service offers and the conference behind each room (`NativeConferenceProvider`,
//! `NativeRoomTypeDescriptor`, `NativeConferenceDescriptor`). The calendar store is read-only for participants, so this
//! module checks what it is given and never sends an invitation.

/// The kind of a participant (`NativeParticipantType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticipantType {
    Unknown,
    Person,
    Group,
    Room,
    Resource,
}

/// The role of a participant in an event (`NativeParticipantRole`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticipantRole {
    Unknown,
    Required,
    Optional,
    Chair,
    NonParticipant,
}

/// The attendance status of a participant (`NativeParticipantStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticipantStatus {
    Unknown,
    Pending,
    Accepted,
    Declined,
    Tentative,
    Delegated,
    Completed,
    InProcess,
}

/// Why a value was refused. A refused change leaves the roster or the provider as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConferenceError {
    /// A participant address needs a scheme such as `mailto:`, and a join link needs `https://`.
    InvalidUrl,
    /// A title or name is blank, or an identifier is empty or contains whitespace.
    InvalidText,
    /// A conference needs at least one join link.
    NoLinks,
    DuplicateParticipant,
    UnknownParticipant,
    DuplicateRoomType,
    UnknownRoomType,
    /// The room exists but no conference is set for it.
    UnknownConference,
}

fn text_ok(text: &str) -> bool {
    !text.trim().is_empty()
}

fn token_ok(token: &str) -> bool {
    !token.is_empty() && !token.chars().any(char::is_whitespace)
}

/// A participant address: a scheme made of letters, digits, `+`, `-` or `.` that starts with a letter, then `:` and
/// a non-empty rest. Whitespace is never allowed.
fn valid_address(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    starts_with_letter
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !rest.is_empty()
        && token_ok(url)
}

/// A join link must be `https://` followed by something, with no whitespace.
fn valid_join_url(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| !rest.is_empty()) && token_ok(url)
}

/// One person, group, room or resource invited to an event (`NativeParticipant`). `url` is the address that names
/// the participant, such as a `mailto:` address or a room URL, and it is the key of a roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    pub name: Option<String>,
    pub url: String,
    pub kind: ParticipantType,
    pub role: ParticipantRole,
    pub status: ParticipantStatus,
    /// True when the participant is the owner of the account that reads the event.
    pub is_current_user: bool,
}

impl Participant {
    /// A participant at `url` with an unknown kind, role and status. Fails when the address has no valid scheme.
    pub fn new(url: &str) -> Result<Self, ConferenceError> {
        if !valid_address(url) {
            return Err(ConferenceError::InvalidUrl);
        }
        Ok(Participant {
            name: None,
            url: url.to_string(),
            kind: ParticipantType::Unknown,
            role: ParticipantRole::Unknown,
            status: ParticipantStatus::Unknown,
            is_current_user: false,
        })
    }
}

/// How the people and groups of a roster answered (`NativeParticipantStatus` counted over attendees).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attendance {
    pub pending: usize,
    pub accepted: usize,
    pub declined: usize,
    pub tentative: usize,
    /// Unknown, delegated, completed and in-process answers.
    pub other: usize,
}

/// The participants of one event. Addresses are compared without case, so one address cannot be added twice.
#[derive(Debug, Default)]
pub struct Roster {
    participants: Vec<Participant>,
}

impl Roster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn participants(&self) -> &[Participant] {
        &self.participants
    }

    pub fn add(&mut self, participant: Participant) -> Result<(), ConferenceError> {
        if self.index_of(&participant.url).is_some() {
            return Err(ConferenceError::DuplicateParticipant);
        }
        self.participants.push(participant);
        Ok(())
    }

    /// Records the answer of the participant at `url`.
    pub fn set_status(&mut self, url: &str, status: ParticipantStatus) -> Result<(), ConferenceError> {
        let index = self.index_of(url).ok_or(ConferenceError::UnknownParticipant)?;
        self.participants[index].status = status;
        Ok(())
    }

    /// The participant that represents the account owner, if the roster has one.
    pub fn current_user(&self) -> Option<&Participant> {
        self.participants.iter().find(|p| p.is_current_user)
    }

    /// Counts the answers of people and groups that take part. Rooms, resources and non-participants do not answer
    /// an invitation, so they are left out.
    pub fn attendance(&self) -> Attendance {
        let mut count = Attendance::default();
        for participant in &self.participants {
            let answers = matches!(participant.kind, ParticipantType::Person | ParticipantType::Group)
                && participant.role != ParticipantRole::NonParticipant;
            if !answers {
                continue;
            }
            match participant.status {
                ParticipantStatus::Pending => count.pending += 1,
                ParticipantStatus::Accepted => count.accepted += 1,
                ParticipantStatus::Declined => count.declined += 1,
                ParticipantStatus::Tentative => count.tentative += 1,
                _ => count.other += 1,
            }
        }
        count
    }

    fn index_of(&self, url: &str) -> Option<usize> {
        self.participants.iter().position(|p| p.url.eq_ignore_ascii_case(url))
    }
}

/// A place where virtual conferences happen, such as a personal room (`NativeRoomTypeDescriptor`). `identifier` is
/// chosen by the service and has no whitespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomType {
    pub identifier: String,
    pub title: String,
}

impl RoomType {
    pub fn new(identifier: &str, title: &str) -> Result<Self, ConferenceError> {
        if !token_ok(identifier) || !text_ok(title) {
            return Err(ConferenceError::InvalidText);
        }
        Ok(RoomType { identifier: identifier.to_string(), title: title.to_string() })
    }
}

/// One way to join a conference (`NativeConferenceUrlDescriptor`). Only `https://` links are accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinLink {
    pub title: String,
    pub url: String,
}

impl JoinLink {
    pub fn new(title: &str, url: &str) -> Result<Self, ConferenceError> {
        if !text_ok(title) {
            return Err(ConferenceError::InvalidText);
        }
        if !valid_join_url(url) {
            return Err(ConferenceError::InvalidUrl);
        }
        Ok(JoinLink { title: title.to_string(), url: url.to_string() })
    }
}

/// The details of one virtual conference (`NativeConferenceDescriptor`): a title, at least one join link and an
/// optional note for the people who join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conference {
    pub title: String,
    pub links: Vec<JoinLink>,
    pub details: Option<String>,
}

impl Conference {
    pub fn new(title: &str, links: Vec<JoinLink>) -> Result<Self, ConferenceError> {
        if !text_ok(title) {
            return Err(ConferenceError::InvalidText);
        }
        if links.is_empty() {
            return Err(ConferenceError::NoLinks);
        }
        Ok(Conference { title: title.to_string(), links, details: None })
    }
}

/// The room types a conference service offers and the conference behind each room (`NativeConferenceProvider`).
#[derive(Debug, Default)]
pub struct ConferenceProvider {
    room_types: Vec<RoomType>,
    conferences: Vec<(String, Conference)>,
}

impl ConferenceProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rooms a user can pick for an event (`fetchAvailableRoomTypes`).
    pub fn room_types(&self) -> &[RoomType] {
        &self.room_types
    }

    pub fn add_room_type(&mut self, room: RoomType) -> Result<(), ConferenceError> {
        if self.room_types.iter().any(|r| r.identifier == room.identifier) {
            return Err(ConferenceError::DuplicateRoomType);
        }
        self.room_types.push(room);
        Ok(())
    }

    /// Sets the conference for a room, replacing an earlier one. The room must already be offered.
    pub fn set_conference(&mut self, room_identifier: &str, conference: Conference) -> Result<(), ConferenceError> {
        if !self.has_room(room_identifier) {
            return Err(ConferenceError::UnknownRoomType);
        }
        match self.conferences.iter_mut().find(|(id, _)| id.as_str() == room_identifier) {
            Some(entry) => entry.1 = conference,
            None => self.conferences.push((room_identifier.to_string(), conference)),
        }
        Ok(())
    }

    /// The conference for a room (`fetchVirtualConference(identifier:)`).
    pub fn fetch_conference(&self, room_identifier: &str) -> Result<&Conference, ConferenceError> {
        if !self.has_room(room_identifier) {
            return Err(ConferenceError::UnknownRoomType);
        }
        self.conferences
            .iter()
            .find(|(id, _)| id.as_str() == room_identifier)
            .map(|(_, conference)| conference)
            .ok_or(ConferenceError::UnknownConference)
    }

    fn has_room(&self, room_identifier: &str) -> bool {
        self.room_types.iter().any(|r| r.identifier == room_identifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str) -> Result<JoinLink, ConferenceError> {
        JoinLink::new("Join", url)
    }

    #[test]
    fn participant_addresses_need_a_scheme() {
        assert!(Participant::new("mailto:ann@example.is").is_ok());
        assert!(Participant::new("https://rooms.example.is/eld").is_ok());
        assert_eq!(Participant::new("ann@example.is").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(Participant::new("mailto:").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(Participant::new("1mail:ann").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(Participant::new("mailto:ann @example.is").unwrap_err(), ConferenceError::InvalidUrl);
    }

    #[test]
    fn roster_refuses_duplicate_addresses_ignoring_case() {
        let mut roster = Roster::new();
        roster.add(Participant::new("mailto:ann@example.is").unwrap()).unwrap();
        let again = Participant::new("MAILTO:ANN@EXAMPLE.IS").unwrap();
        assert_eq!(roster.add(again), Err(ConferenceError::DuplicateParticipant));
        assert_eq!(roster.participants().len(), 1);

        assert_eq!(
            roster.set_status("mailto:bob@example.is", ParticipantStatus::Accepted),
            Err(ConferenceError::UnknownParticipant)
        );
        roster.set_status("MAILTO:ANN@EXAMPLE.IS", ParticipantStatus::Accepted).unwrap();
        assert_eq!(roster.participants()[0].status, ParticipantStatus::Accepted);
    }

    #[test]
    fn attendance_counts_people_and_groups_only() {
        let mut roster = Roster::new();
        let mut add = |url: &str, kind: ParticipantType, role: ParticipantRole, status: ParticipantStatus| {
            let mut p = Participant::new(url).unwrap();
            p.kind = kind;
            p.role = role;
            p.status = status;
            roster.add(p).unwrap();
        };
        add("mailto:ann@example.is", ParticipantType::Person, ParticipantRole::Required, ParticipantStatus::Accepted);
        add("mailto:bob@example.is", ParticipantType::Person, ParticipantRole::Optional, ParticipantStatus::Declined);
        add("mailto:team@example.is", ParticipantType::Group, ParticipantRole::Required, ParticipantStatus::Pending);
        add("mailto:kim@example.is", ParticipantType::Person, ParticipantRole::Unknown, ParticipantStatus::Unknown);
        // Rooms and resources answer nothing, and a non-participant is not asked.
        add("https://rooms.example.is/eld", ParticipantType::Room, ParticipantRole::Required, ParticipantStatus::Accepted);
        add("mailto:pen@example.is", ParticipantType::Resource, ParticipantRole::Required, ParticipantStatus::Declined);
        add("mailto:eve@example.is", ParticipantType::Person, ParticipantRole::NonParticipant, ParticipantStatus::Tentative);

        assert_eq!(
            roster.attendance(),
            Attendance { pending: 1, accepted: 1, declined: 1, tentative: 0, other: 1 }
        );
    }

    #[test]
    fn current_user_is_the_flagged_participant() {
        let mut roster = Roster::new();
        roster.add(Participant::new("mailto:ann@example.is").unwrap()).unwrap();
        assert!(roster.current_user().is_none());

        let mut me = Participant::new("mailto:olibuijr@example.is").unwrap();
        me.is_current_user = true;
        roster.add(me).unwrap();
        assert_eq!(roster.current_user().unwrap().url, "mailto:olibuijr@example.is");
    }

    #[test]
    fn conference_is_set_for_an_offered_room_only() {
        let mut provider = ConferenceProvider::new();
        provider.add_room_type(RoomType::new("personal", "Personal Room").unwrap()).unwrap();
        assert_eq!(
            provider.add_room_type(RoomType::new("personal", "Again").unwrap()),
            Err(ConferenceError::DuplicateRoomType)
        );

        let conference = Conference::new("Weekly sync", vec![link("https://meet.example.is/sync").unwrap()]).unwrap();
        assert_eq!(provider.set_conference("team", conference.clone()), Err(ConferenceError::UnknownRoomType));
        assert_eq!(provider.fetch_conference("personal"), Err(ConferenceError::UnknownConference));

        provider.set_conference("personal", conference).unwrap();
        assert_eq!(provider.fetch_conference("personal").unwrap().title, "Weekly sync");
        assert_eq!(provider.room_types().len(), 1);
    }

    #[test]
    fn join_links_must_be_https_and_conferences_need_one() {
        assert!(link("https://meet.example.is/abc").is_ok());
        assert_eq!(link("http://meet.example.is/abc").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(link("javascript:alert(1)").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(link("https://").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(link("https://meet.example.is/a b").unwrap_err(), ConferenceError::InvalidUrl);
        assert_eq!(JoinLink::new("  ", "https://meet.example.is/abc").unwrap_err(), ConferenceError::InvalidText);
        assert_eq!(Conference::new("Sync", Vec::new()).unwrap_err(), ConferenceError::NoLinks);
    }

    #[test]
    fn room_identifiers_have_no_whitespace() {
        assert!(RoomType::new("team-room", "Team Room").is_ok());
        assert_eq!(RoomType::new("team room", "Team Room").unwrap_err(), ConferenceError::InvalidText);
        assert_eq!(RoomType::new("", "Team Room").unwrap_err(), ConferenceError::InvalidText);
        assert_eq!(RoomType::new("team", " ").unwrap_err(), ConferenceError::InvalidText);
    }
}

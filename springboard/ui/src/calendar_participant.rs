//! A person, group, room or resource invited to a calendar event: the read-only view an app gets of one attendee.
//! Apps cannot add participants or change their status, so the type has no setters. The store builds a `Participant`
//! from the attendee record it holds, and the caller reads the name, the address, the role, the type and the status.

use std::fmt;

/// The role a participant has in an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Unknown,
    /// Attendance is required.
    Required,
    /// Attendance is optional.
    Optional,
    /// The participant chairs the event.
    Chair,
    /// The participant has no active role in the event.
    NonParticipant,
}

/// What kind of participant this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Unknown,
    Person,
    Room,
    Resource,
    Group,
}

/// The attendance status of a participant for an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Unknown,
    /// Has not responded yet.
    Pending,
    Accepted,
    Declined,
    Tentative,
    /// Handed attendance to another participant.
    Delegated,
    /// The event has completed for this participant.
    Completed,
    /// The event is in progress for this participant.
    InProcess,
}

/// Why a participant could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The address is empty or has no scheme (for example `ana@example.is` without `mailto:`).
    InvalidUrl,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidUrl => f.write_str("the participant address needs a scheme, such as mailto:"),
        }
    }
}

impl std::error::Error for Error {}

/// One attendee of an event. The address is kept as the URL the store gave, such as `mailto:ana@example.is`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    name: Option<String>,
    url: String,
    role: Role,
    kind: Kind,
    status: Status,
    current_user: bool,
}

impl Participant {
    /// Builds a read-only participant. The address must have a scheme; a blank name counts as no name.
    pub fn new(url: &str, name: Option<&str>, role: Role, kind: Kind, status: Status, current_user: bool) -> Result<Self, Error> {
        let url = url.trim();
        match url.split_once(':') {
            Some((scheme, rest)) if !scheme.is_empty() && !rest.is_empty() => {}
            _ => return Err(Error::InvalidUrl),
        }
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map(String::from);
        Ok(Participant { name, url: url.to_string(), role, kind, status, current_user })
    }

    /// The participant's name, if the store has one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The URL that represents the participant, as stored.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The address without its scheme: `ana@example.is` for `mailto:ana@example.is`, or the whole URL for any other scheme.
    pub fn address(&self) -> &str {
        match self.url.split_once(':') {
            Some(("mailto", rest)) => rest,
            _ => &self.url,
        }
    }

    /// The name to show: the name when there is one, otherwise the address.
    pub fn display_name(&self) -> &str {
        self.name().unwrap_or_else(|| self.address())
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// True when this participant is the owner of the account the store belongs to.
    pub fn is_current_user(&self) -> bool {
        self.current_user
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(url: &str, name: Option<&str>) -> Participant {
        Participant::new(url, name, Role::Required, Kind::Person, Status::Pending, false).unwrap()
    }

    #[test]
    fn an_address_without_a_scheme_is_refused() {
        assert_eq!(
            Participant::new("ana@example.is", None, Role::Required, Kind::Person, Status::Pending, false),
            Err(Error::InvalidUrl)
        );
        assert_eq!(
            Participant::new("   ", None, Role::Required, Kind::Person, Status::Pending, false),
            Err(Error::InvalidUrl)
        );
        assert_eq!(
            Participant::new("mailto:", None, Role::Required, Kind::Person, Status::Pending, false),
            Err(Error::InvalidUrl)
        );
    }

    #[test]
    fn the_address_drops_a_mailto_scheme_and_keeps_other_urls_whole() {
        assert_eq!(person("mailto:ana@example.is", None).address(), "ana@example.is");
        assert_eq!(person("  https://example.is/room/7 ", None).url(), "https://example.is/room/7");
        assert_eq!(person("https://example.is/room/7", None).address(), "https://example.is/room/7");
    }

    #[test]
    fn the_display_name_falls_back_to_the_address_when_the_name_is_blank() {
        assert_eq!(person("mailto:ana@example.is", Some("Ana Lind")).display_name(), "Ana Lind");
        assert_eq!(person("mailto:ana@example.is", Some("   ")).name(), None);
        assert_eq!(person("mailto:ana@example.is", Some("   ")).display_name(), "ana@example.is");
        assert_eq!(person("mailto:ana@example.is", None).display_name(), "ana@example.is");
    }

    #[test]
    fn the_name_is_trimmed() {
        assert_eq!(person("mailto:bo@example.is", Some("  Bo Ingi ")).name(), Some("Bo Ingi"));
    }

    #[test]
    fn role_type_and_status_are_returned_as_given() {
        let room = Participant::new("https://example.is/room/3", Some("Fundarsalur"), Role::Optional, Kind::Room, Status::Tentative, false)
            .unwrap();
        assert_eq!(room.role(), Role::Optional);
        assert_eq!(room.kind(), Kind::Room);
        assert_eq!(room.status(), Status::Tentative);
        assert!(!room.is_current_user());
    }

    #[test]
    fn the_owner_of_the_account_is_marked_as_the_current_user() {
        let me = Participant::new("mailto:owner@example.is", None, Role::Chair, Kind::Person, Status::Accepted, true).unwrap();
        assert!(me.is_current_user());
        assert_eq!(me.role(), Role::Chair);
    }
}

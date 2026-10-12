//! Virtual conference rooms for calendar events, as a provider app holds them: the rooms the provider offers (each with
//! a title and an identifier the provider chose), and for each room the conference that a user's event joins. The
//! calendar asks for the room list, then for the conference of the room the user picked. Both lookups are pure: the
//! provider fills the table, and `fetch_*` answers from it. Nothing here opens a network connection.

use std::collections::BTreeMap;
use std::fmt;

/// A room where virtual conferences take place, such as a personal room or a team room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomType {
    title: String,
    identifier: String,
}

impl RoomType {
    /// Builds a room. The title and the identifier must not be blank; the identifier is kept as given, trimmed.
    pub fn new(title: &str, identifier: &str) -> Result<Self, Error> {
        let title = title.trim();
        let identifier = identifier.trim();
        if title.is_empty() || identifier.is_empty() {
            return Err(Error::Blank);
        }
        Ok(RoomType { title: title.to_string(), identifier: identifier.to_string() })
    }

    /// The name the user sees for the room.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The identifier the provider chose; the calendar passes it back when it asks for the conference.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

/// One address a user can join a conference by, such as a dial-in or a web link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinUrl {
    title: String,
    url: String,
}

impl JoinUrl {
    /// Builds a join address. The URL must have a scheme, such as `https:`.
    pub fn new(title: &str, url: &str) -> Result<Self, Error> {
        let title = title.trim();
        let url = url.trim();
        match url.split_once(':') {
            Some((scheme, rest)) if !scheme.is_empty() && !rest.is_empty() => {}
            _ => return Err(Error::InvalidUrl),
        }
        if title.is_empty() {
            return Err(Error::Blank);
        }
        Ok(JoinUrl { title: title.to_string(), url: url.to_string() })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

/// A virtual conference that an event joins: a title, the addresses to join by, and optional details for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conference {
    title: String,
    join_urls: Vec<JoinUrl>,
    details: Option<String>,
}

impl Conference {
    /// Builds a conference. The title must not be blank. Blank details count as none.
    pub fn new(title: &str, join_urls: Vec<JoinUrl>, details: Option<&str>) -> Result<Self, Error> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::Blank);
        }
        let details = details.map(str::trim).filter(|d| !d.is_empty()).map(String::from);
        Ok(Conference { title: title.to_string(), join_urls, details })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The addresses to join by, in the order the provider gave them.
    pub fn join_urls(&self) -> &[JoinUrl] {
        &self.join_urls
    }

    /// Extra information for the user, if the provider gave any.
    pub fn details(&self) -> Option<&str> {
        self.details.as_deref()
    }
}

/// Why a provider table or a lookup failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A title, an identifier or a join title is empty.
    Blank,
    /// A join address has no scheme.
    InvalidUrl,
    /// Two rooms share an identifier. The first one stays.
    DuplicateRoom(String),
    /// No room has this identifier.
    UnknownRoom(String),
    /// The room exists but the provider has no conference for it.
    NoConference(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Blank => f.write_str("a title or an identifier is blank"),
            Error::InvalidUrl => f.write_str("a join address needs a scheme, such as https:"),
            Error::DuplicateRoom(id) => write!(f, "two rooms share the identifier {id}"),
            Error::UnknownRoom(id) => write!(f, "no room has the identifier {id}"),
            Error::NoConference(id) => write!(f, "the room {id} has no conference"),
        }
    }
}

impl std::error::Error for Error {}

/// The rooms a provider offers, and the conference for each room.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provider {
    rooms: Vec<RoomType>,
    conferences: BTreeMap<String, Conference>,
}

impl Provider {
    /// An empty provider with no rooms.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a room. Its identifier must not match a room already added.
    pub fn add_room(&mut self, room: RoomType) -> Result<(), Error> {
        if self.rooms.iter().any(|r| r.identifier == room.identifier) {
            return Err(Error::DuplicateRoom(room.identifier));
        }
        self.rooms.push(room);
        Ok(())
    }

    /// The rooms, in the order they were added. The calendar shows them as the choices for an event's location.
    pub fn fetch_available_room_types(&self) -> &[RoomType] {
        &self.rooms
    }

    /// Sets the conference for a room the provider already offers. A second call for the same room replaces the first.
    pub fn set_conference(&mut self, room_id: &str, conference: Conference) -> Result<(), Error> {
        let room_id = room_id.trim();
        if !self.rooms.iter().any(|r| r.identifier == room_id) {
            return Err(Error::UnknownRoom(room_id.to_string()));
        }
        self.conferences.insert(room_id.to_string(), conference);
        Ok(())
    }

    /// The conference for the room the user picked. Fails when the room is unknown or has no conference yet.
    pub fn fetch_virtual_conference(&self, room_id: &str) -> Result<&Conference, Error> {
        let room_id = room_id.trim();
        if !self.rooms.iter().any(|r| r.identifier == room_id) {
            return Err(Error::UnknownRoom(room_id.to_string()));
        }
        self.conferences.get(room_id).ok_or_else(|| Error::NoConference(room_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str) -> RoomType {
        RoomType::new("Team room", id).unwrap()
    }

    #[test]
    fn a_room_needs_a_title_and_an_identifier() {
        assert_eq!(RoomType::new("  ", "team-1"), Err(Error::Blank));
        assert_eq!(RoomType::new("Team room", ""), Err(Error::Blank));
        let r = RoomType::new(" Personal room ", " p-7 ").unwrap();
        assert_eq!(r.title(), "Personal room");
        assert_eq!(r.identifier(), "p-7");
    }

    #[test]
    fn a_join_address_needs_a_scheme() {
        assert_eq!(JoinUrl::new("Web", "meet.example.is/abc"), Err(Error::InvalidUrl));
        assert_eq!(JoinUrl::new("", "https://meet.example.is/abc"), Err(Error::Blank));
        let j = JoinUrl::new("Web", "https://meet.example.is/abc").unwrap();
        assert_eq!(j.url(), "https://meet.example.is/abc");
    }

    #[test]
    fn rooms_are_listed_in_the_order_they_were_added() {
        let mut p = Provider::new();
        p.add_room(room("b")).unwrap();
        p.add_room(room("a")).unwrap();
        let ids: Vec<&str> = p.fetch_available_room_types().iter().map(|r| r.identifier()).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }

    #[test]
    fn two_rooms_with_one_identifier_are_refused_and_the_first_stays() {
        let mut p = Provider::new();
        p.add_room(RoomType::new("First", "same").unwrap()).unwrap();
        assert_eq!(
            p.add_room(RoomType::new("Second", "same").unwrap()),
            Err(Error::DuplicateRoom("same".to_string()))
        );
        assert_eq!(p.fetch_available_room_types()[0].title(), "First");
        assert_eq!(p.fetch_available_room_types().len(), 1);
    }

    #[test]
    fn a_conference_is_returned_for_its_room() {
        let mut p = Provider::new();
        p.add_room(room("team-1")).unwrap();
        let join = JoinUrl::new("Web", "https://meet.example.is/abc").unwrap();
        let conf = Conference::new("Weekly sync", vec![join], Some("  Bring the agenda  ")).unwrap();
        p.set_conference("team-1", conf).unwrap();
        let got = p.fetch_virtual_conference("team-1").unwrap();
        assert_eq!(got.title(), "Weekly sync");
        assert_eq!(got.details(), Some("Bring the agenda"));
        assert_eq!(got.join_urls()[0].title(), "Web");
    }

    #[test]
    fn an_unknown_room_and_a_room_without_a_conference_are_different_errors() {
        let mut p = Provider::new();
        p.add_room(room("team-1")).unwrap();
        assert_eq!(p.fetch_virtual_conference("nope"), Err(Error::UnknownRoom("nope".to_string())));
        assert_eq!(p.fetch_virtual_conference("team-1"), Err(Error::NoConference("team-1".to_string())));
        assert_eq!(
            p.set_conference("nope", Conference::new("Sync", vec![], None).unwrap()),
            Err(Error::UnknownRoom("nope".to_string()))
        );
    }

    #[test]
    fn a_blank_detail_counts_as_no_details() {
        let conf = Conference::new("Sync", vec![], Some("   ")).unwrap();
        assert_eq!(conf.details(), None);
        assert_eq!(Conference::new("  ", vec![], None), Err(Error::Blank));
    }
}

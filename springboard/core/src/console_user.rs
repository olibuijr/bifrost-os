//! The console user: the account whose session is on the console seat right now (task theme-17). It answers the
//! question "who is at this machine", which a system configuration call answers for apps on the native platform.
//!
//! This is a pure model. It reads no session manager, no login record and no dynamic store, and it starts no
//! watcher. A caller reads its own session listing, turns the text into [`Session`] values with [`parse_sessions`],
//! and asks [`console_user`] for the answer.
//!
//! Rules of this model:
//! - A session can be the console user only when it is on the console seat (`seat0`), has class `user` and is not
//!   remote.
//! - An `active` session wins over an `online` one. A `closing` session never wins, and neither does a session in
//!   any other state.
//! - Ties go to the lowest session id, so the answer does not depend on the order of the records.
//! - When no session qualifies, there is no console user: the login screen is showing.
//! - A session record has no group id, so the answer carries the user name and uid only.
//!
//! The record text is one `Key=Value` per line, with a blank line between sessions. The keys are `Id`, `UID`,
//! `Name`, `Seat` (empty when the session has no seat), `Class`, `State` and `Remote` (`yes` or `no`). Lines without
//! an `=` are ignored, and a repeated key takes its last value, so a newer listing can add fields without breaking
//! this reader.

use std::fmt;

/// The seat whose session can hold the console user.
pub const CONSOLE_SEAT: &str = "seat0";

/// What a session is for. Only [`Class::User`] can be the console user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A session a person signed in to.
    User,
    /// The sign-in screen that runs before anyone signs in.
    Greeter,
    /// A session that is locked: the user is still signed in.
    LockScreen,
    /// A service session with no person behind it.
    Background,
    /// Any class this module does not know. It is never the console user.
    Other,
}

impl Class {
    fn parse(text: &str) -> Class {
        match text {
            "user" => Class::User,
            "greeter" => Class::Greeter,
            "lock-screen" => Class::LockScreen,
            "background" => Class::Background,
            _ => Class::Other,
        }
    }
}

/// Where a session is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The session holds the seat and is shown on the screen.
    Active,
    /// The session is signed in but another session holds the seat.
    Online,
    /// The session is being torn down.
    Closing,
    /// Any state this module does not know.
    Other,
}

impl State {
    fn parse(text: &str) -> State {
        match text {
            "active" => State::Active,
            "online" => State::Online,
            "closing" => State::Closing,
            _ => State::Other,
        }
    }

    /// The order of preference for the console user: lower wins. `None` means the state can never be the console user.
    fn rank(self) -> Option<u8> {
        match self {
            State::Active => Some(0),
            State::Online => Some(1),
            State::Closing | State::Other => None,
        }
    }
}

/// One login session as a caller reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: u32,
    pub uid: u32,
    pub name: String,
    /// The seat the session is on, or `None` when it has none.
    pub seat: Option<String>,
    pub class: Class,
    pub state: State,
    pub remote: bool,
}

/// The account at the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleUser {
    /// The session the answer came from.
    pub session_id: u32,
    pub uid: u32,
    pub name: String,
}

/// Why a session record could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// A required key is missing from a session record.
    MissingKey(&'static str),
    /// A numeric key does not hold a number.
    BadNumber(&'static str, String),
    /// `Remote` is neither `yes` nor `no`.
    BadFlag(&'static str, String),
    /// The session has an empty user name. The field is the session id.
    EmptyName(u32),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::MissingKey(key) => write!(f, "session record has no {key} field"),
            ParseError::BadNumber(key, value) => write!(f, "{key} is not a number: {value:?}"),
            ParseError::BadFlag(key, value) => write!(f, "{key} must be yes or no, found {value:?}"),
            ParseError::EmptyName(id) => write!(f, "session {id} has an empty user name"),
        }
    }
}

/// Reads session records from text. Records are separated by blank lines.
pub fn parse_sessions(text: &str) -> Result<Vec<Session>, ParseError> {
    let mut out = Vec::new();
    let mut block: Vec<(&str, &str)> = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if !block.is_empty() {
                out.push(session_from(&block)?);
                block.clear();
            }
        } else if let Some((key, value)) = line.split_once('=') {
            block.push((key.trim(), value.trim()));
        }
    }
    Ok(out)
}

/// The value of the last field with this key.
fn field<'a>(fields: &[(&str, &'a str)], key: &'static str) -> Result<&'a str, ParseError> {
    fields
        .iter()
        .rev()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .ok_or(ParseError::MissingKey(key))
}

fn number(value: &str, key: &'static str) -> Result<u32, ParseError> {
    value.parse::<u32>().map_err(|_| ParseError::BadNumber(key, value.to_string()))
}

fn session_from(fields: &[(&str, &str)]) -> Result<Session, ParseError> {
    let id = number(field(fields, "Id")?, "Id")?;
    let uid = number(field(fields, "UID")?, "UID")?;
    let name = field(fields, "Name")?;
    if name.is_empty() {
        return Err(ParseError::EmptyName(id));
    }
    let seat = field(fields, "Seat")?;
    let remote = match field(fields, "Remote")? {
        "yes" => true,
        "no" => false,
        other => return Err(ParseError::BadFlag("Remote", other.to_string())),
    };
    Ok(Session {
        id,
        uid,
        name: name.to_string(),
        seat: if seat.is_empty() { None } else { Some(seat.to_string()) },
        class: Class::parse(field(fields, "Class")?),
        state: State::parse(field(fields, "State")?),
        remote,
    })
}

/// The console user under the rules in the module notes, or `None` when no session qualifies.
pub fn console_user(sessions: &[Session]) -> Option<ConsoleUser> {
    sessions
        .iter()
        .filter(|s| s.seat.as_deref() == Some(CONSOLE_SEAT) && s.class == Class::User && !s.remote)
        .filter_map(|s| s.state.rank().map(|rank| (rank, s)))
        .min_by_key(|(rank, s)| (*rank, s.id))
        .map(|(_, s)| ConsoleUser { session_id: s.id, uid: s.uid, name: s.name.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: u32, seat: Option<&str>, class: Class, state: State, remote: bool) -> Session {
        Session {
            id,
            uid: 1000 + id,
            name: format!("user{id}"),
            seat: seat.map(String::from),
            class,
            state,
            remote,
        }
    }

    #[test]
    fn parses_two_records_and_an_empty_seat() {
        let text = "Id=2\nUID=1000\nName=olibuijr\nSeat=seat0\nClass=user\nState=active\nRemote=no\n\n\
                    Id=3\nUID=1001\nName=gest\nSeat=\nClass=user\nState=online\nRemote=yes\n";
        let list = parse_sessions(text).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0],
            Session {
                id: 2,
                uid: 1000,
                name: "olibuijr".to_string(),
                seat: Some("seat0".to_string()),
                class: Class::User,
                state: State::Active,
                remote: false,
            }
        );
        assert_eq!(list[1].seat, None);
        assert!(list[1].remote);
        assert_eq!(list[1].state, State::Online);
    }

    #[test]
    fn a_missing_key_names_the_key() {
        let text = "Id=2\nUID=1000\nName=olibuijr\nSeat=seat0\nClass=user\nRemote=no\n";
        let err = parse_sessions(text).unwrap_err();
        assert_eq!(err, ParseError::MissingKey("State"));
        assert_eq!(err.to_string(), "session record has no State field");
    }

    #[test]
    fn a_non_numeric_uid_is_refused() {
        let text = "Id=2\nUID=abc\nName=olibuijr\nSeat=seat0\nClass=user\nState=active\nRemote=no\n";
        assert_eq!(parse_sessions(text).unwrap_err(), ParseError::BadNumber("UID", "abc".to_string()));
    }

    #[test]
    fn an_active_session_beats_an_online_one_and_ties_go_to_the_lowest_id() {
        let list = vec![
            session(1, Some("seat0"), Class::User, State::Online, false),
            session(2, Some("seat0"), Class::User, State::Active, false),
        ];
        let got = console_user(&list).unwrap();
        assert_eq!(got, ConsoleUser { session_id: 2, uid: 1002, name: "user2".to_string() });

        let tie = vec![
            session(5, Some("seat0"), Class::User, State::Active, false),
            session(3, Some("seat0"), Class::User, State::Active, false),
        ];
        assert_eq!(console_user(&tie).unwrap().session_id, 3);
    }

    #[test]
    fn remote_and_closing_sessions_never_qualify() {
        let list = vec![
            session(1, Some("seat0"), Class::User, State::Active, true),
            session(4, Some("seat0"), Class::User, State::Closing, false),
        ];
        assert_eq!(console_user(&list), None);
    }

    #[test]
    fn the_sign_in_screen_means_no_console_user() {
        let list = vec![session(7, Some("seat0"), Class::Greeter, State::Active, false)];
        assert_eq!(console_user(&list), None);
    }

    #[test]
    fn a_session_on_another_seat_is_ignored() {
        let list = vec![
            session(2, Some("seat1"), Class::User, State::Active, false),
            session(9, Some("seat0"), Class::User, State::Online, false),
        ];
        assert_eq!(console_user(&list).unwrap().session_id, 9);
    }
}

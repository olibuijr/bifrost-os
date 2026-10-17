//! Peer sessions: named listeners and anonymous endpoints that accept connections, and request and reply messages
//! inside a session between a client side and a server side.
//!
//! This is the pure model of a connection-based channel between services. A [`Hub`] stands in for the routing
//! layer: it owns every listener, endpoint and session, so the whole exchange runs in one process and needs no
//! sockets. A listener or endpoint accepts a connection only when the peer satisfies its [`Requirement`]. The
//! client gets a [`SessionId`] from [`Hub::connect`]; the service takes the same session from [`Hub::accept`] and
//! talks to it as [`Side::Server`]. A message sent with `wants_reply` must be answered with [`Hub::reply`] before
//! the sender can take that reply with [`Hub::take_reply`].
//!
//! Remaining gaps: there is no process boundary, so an [`Endpoint`] cannot be sent to another process; a [`Peer`]
//! carries a user id and an app id only, with no code-signing or entitlement check; and nothing launches a service
//! on demand or shuts it down when idle, which the service supervisor owns.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Longest listener name, in bytes.
pub const MAX_NAME_LEN: usize = 256;
/// Largest message body, in bytes, in either direction.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// The process on the other end of a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub uid: u32,
    pub app_id: String,
}

/// Which peers a listener or endpoint accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Any peer.
    Any,
    /// Peers that run as this user id.
    SameUser(u32),
    /// Peers with this app id.
    App(String),
    /// Peers that satisfy both requirements.
    Both(Box<Requirement>, Box<Requirement>),
}

impl Requirement {
    /// True when `peer` may connect.
    pub fn accepts(&self, peer: &Peer) -> bool {
        match self {
            Requirement::Any => true,
            Requirement::SameUser(uid) => peer.uid == *uid,
            Requirement::App(id) => peer.app_id == *id,
            Requirement::Both(a, b) => a.accepts(peer) && b.accepts(peer),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidName,
    NameTaken,
    NoListener,
    Rejected,
    UnknownSession,
    Closed,
    TooLarge,
    UnknownMessage,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::InvalidName => "invalid listener name",
            Error::NameTaken => "listener name already in use",
            Error::NoListener => "no listener at that target",
            Error::Rejected => "peer does not meet the requirement",
            Error::UnknownSession => "unknown session",
            Error::Closed => "session is closed",
            Error::TooLarge => "message is larger than the limit",
            Error::UnknownMessage => "no such message awaiting a reply",
        };
        f.write_str(text)
    }
}

/// An anonymous listener. It exists only inside the hub that created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint(u64);

/// Where a connection is made: a named listener or an anonymous endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Named(String),
    Endpoint(Endpoint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(u64);

/// Which end of a session a call acts for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Client,
    Server,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::Client => 0,
            Side::Server => 1,
        }
    }

    fn other(self) -> Side {
        match self {
            Side::Client => Side::Server,
            Side::Server => Side::Client,
        }
    }
}

/// A message as the receiving side sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedMessage {
    pub id: u64,
    pub body: Vec<u8>,
    pub wants_reply: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GateKey {
    Named(String),
    Endpoint(u64),
}

fn gate_key(target: &Target) -> GateKey {
    match target {
        Target::Named(name) => GateKey::Named(name.clone()),
        Target::Endpoint(e) => GateKey::Endpoint(e.0),
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_LEN && !name.chars().any(char::is_control)
}

/// A listener or endpoint: its requirement and the sessions it has accepted but the server has not taken yet.
struct Gate {
    requirement: Requirement,
    accepted: VecDeque<SessionId>,
}

struct Session {
    peer: Peer,
    closed: bool,
    /// Messages waiting for each side, indexed by [`Side::index`].
    inbox: [VecDeque<ReceivedMessage>; 2],
    /// Messages a side received with `wants_reply` and has not answered yet.
    awaiting: [BTreeSet<u64>; 2],
    /// Replies waiting for each side to take, keyed by the id of the message they answer.
    replies: [BTreeMap<u64, Vec<u8>>; 2],
}

impl Session {
    fn new(peer: Peer) -> Session {
        Session {
            peer,
            closed: false,
            inbox: [VecDeque::new(), VecDeque::new()],
            awaiting: [BTreeSet::new(), BTreeSet::new()],
            replies: [BTreeMap::new(), BTreeMap::new()],
        }
    }
}

/// Routes connections and owns every session. Ids count up from 1 in the order they are issued.
pub struct Hub {
    gates: BTreeMap<GateKey, Gate>,
    sessions: BTreeMap<SessionId, Session>,
    next_endpoint: u64,
    next_session: u64,
    next_message: u64,
}

impl Default for Hub {
    fn default() -> Self {
        Hub::new()
    }
}

impl Hub {
    pub fn new() -> Hub {
        Hub {
            gates: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_endpoint: 1,
            next_session: 1,
            next_message: 1,
        }
    }

    /// Registers a named listener. Names are unique within the hub.
    pub fn listen(&mut self, name: &str, requirement: Requirement) -> Result<(), Error> {
        if !valid_name(name) {
            return Err(Error::InvalidName);
        }
        let key = GateKey::Named(name.to_string());
        if self.gates.contains_key(&key) {
            return Err(Error::NameTaken);
        }
        self.gates.insert(key, Gate { requirement, accepted: VecDeque::new() });
        Ok(())
    }

    /// Creates an anonymous endpoint that accepts peers meeting `requirement`.
    pub fn anonymous_endpoint(&mut self, requirement: Requirement) -> Endpoint {
        let id = self.next_endpoint;
        self.next_endpoint += 1;
        self.gates.insert(GateKey::Endpoint(id), Gate { requirement, accepted: VecDeque::new() });
        Endpoint(id)
    }

    /// Removes a listener or endpoint. Sessions it already accepted stay open. Returns false if it was not there.
    pub fn cancel(&mut self, target: &Target) -> bool {
        self.gates.remove(&gate_key(target)).is_some()
    }

    /// Opens a session for a client. The peer must meet the target's requirement.
    pub fn connect(&mut self, target: &Target, peer: Peer) -> Result<SessionId, Error> {
        let key = gate_key(target);
        let gate = self.gates.get(&key).ok_or(Error::NoListener)?;
        if !gate.requirement.accepts(&peer) {
            return Err(Error::Rejected);
        }
        let id = SessionId(self.next_session);
        self.next_session += 1;
        self.sessions.insert(id, Session::new(peer));
        if let Some(gate) = self.gates.get_mut(&key) {
            gate.accepted.push_back(id);
        }
        Ok(id)
    }

    /// Takes the oldest connection the target accepted but nobody has taken yet.
    pub fn accept(&mut self, target: &Target) -> Option<SessionId> {
        self.gates.get_mut(&gate_key(target))?.accepted.pop_front()
    }

    /// The peer that opened a session.
    pub fn peer(&self, session: SessionId) -> Option<&Peer> {
        self.sessions.get(&session).map(|s| &s.peer)
    }

    /// Sends a message from `from` to the other side. Returns the message id. The body is copied.
    pub fn send(&mut self, session: SessionId, from: Side, body: &[u8], wants_reply: bool) -> Result<u64, Error> {
        if body.len() > MAX_MESSAGE_BYTES {
            return Err(Error::TooLarge);
        }
        let id = self.next_message;
        let s = self.sessions.get_mut(&session).ok_or(Error::UnknownSession)?;
        if s.closed {
            return Err(Error::Closed);
        }
        let to = from.other().index();
        s.inbox[to].push_back(ReceivedMessage { id, body: body.to_vec(), wants_reply });
        if wants_reply {
            s.awaiting[to].insert(id);
        }
        self.next_message += 1;
        Ok(id)
    }

    /// Takes the next message waiting for `side`.
    pub fn receive(&mut self, session: SessionId, side: Side) -> Result<Option<ReceivedMessage>, Error> {
        let s = self.sessions.get_mut(&session).ok_or(Error::UnknownSession)?;
        Ok(s.inbox[side.index()].pop_front())
    }

    /// Answers message `message`, which `side` received with `wants_reply`. Each message can be answered once.
    pub fn reply(&mut self, session: SessionId, side: Side, message: u64, body: &[u8]) -> Result<(), Error> {
        if body.len() > MAX_MESSAGE_BYTES {
            return Err(Error::TooLarge);
        }
        let s = self.sessions.get_mut(&session).ok_or(Error::UnknownSession)?;
        if s.closed {
            return Err(Error::Closed);
        }
        if !s.awaiting[side.index()].remove(&message) {
            return Err(Error::UnknownMessage);
        }
        s.replies[side.other().index()].insert(message, body.to_vec());
        Ok(())
    }

    /// Takes the reply to message `message` that `side` sent. A reply can be taken once.
    pub fn take_reply(&mut self, session: SessionId, side: Side, message: u64) -> Result<Option<Vec<u8>>, Error> {
        let s = self.sessions.get_mut(&session).ok_or(Error::UnknownSession)?;
        Ok(s.replies[side.index()].remove(&message))
    }

    /// Ends a session. New sends and replies fail with `Closed`. Messages already queued can still be received.
    pub fn close(&mut self, session: SessionId) -> Result<(), Error> {
        let s = self.sessions.get_mut(&session).ok_or(Error::UnknownSession)?;
        s.closed = true;
        s.awaiting = [BTreeSet::new(), BTreeSet::new()];
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(uid: u32, app: &str) -> Peer {
        Peer { uid, app_id: app.to_string() }
    }

    fn named(name: &str) -> Target {
        Target::Named(name.to_string())
    }

    #[test]
    fn connect_and_accept_pair_the_two_sides() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::Any).unwrap();
        let client = hub.connect(&named("svc"), peer(1000, "a")).unwrap();
        assert_eq!(client, SessionId(1));
        assert_eq!(hub.accept(&named("svc")), Some(SessionId(1)));
        assert_eq!(hub.accept(&named("svc")), None);
        assert_eq!(hub.peer(client), Some(&peer(1000, "a")));
    }

    #[test]
    fn requirement_rejects_other_users_and_apps() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::SameUser(1000)).unwrap();
        assert_eq!(hub.connect(&named("svc"), peer(1001, "a")), Err(Error::Rejected));
        assert!(hub.connect(&named("svc"), peer(1000, "a")).is_ok());

        let both = Requirement::Both(
            Box::new(Requirement::SameUser(1000)),
            Box::new(Requirement::App("com.x.y".to_string())),
        );
        assert!(!both.accepts(&peer(1000, "com.x.z")));
        assert!(both.accepts(&peer(1000, "com.x.y")));
    }

    #[test]
    fn listener_names_are_unique_and_checked() {
        let mut hub = Hub::new();
        assert_eq!(hub.listen("", Requirement::Any), Err(Error::InvalidName));
        assert_eq!(hub.listen("bad\nname", Requirement::Any), Err(Error::InvalidName));
        hub.listen("svc", Requirement::Any).unwrap();
        assert_eq!(hub.listen("svc", Requirement::Any), Err(Error::NameTaken));
        assert_eq!(hub.connect(&named("other"), peer(1, "a")), Err(Error::NoListener));
    }

    #[test]
    fn request_and_reply_round_trip() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::Any).unwrap();
        let client = hub.connect(&named("svc"), peer(1, "a")).unwrap();
        let server = hub.accept(&named("svc")).unwrap();

        let m = hub.send(client, Side::Client, b"ping", true).unwrap();
        assert_eq!(m, 1);
        let got = hub.receive(server, Side::Server).unwrap().unwrap();
        assert_eq!(got, ReceivedMessage { id: 1, body: b"ping".to_vec(), wants_reply: true });
        assert_eq!(hub.receive(server, Side::Server).unwrap(), None);

        hub.reply(server, Side::Server, 1, b"pong").unwrap();
        assert_eq!(hub.take_reply(client, Side::Client, 1), Ok(Some(b"pong".to_vec())));
        assert_eq!(hub.take_reply(client, Side::Client, 1), Ok(None));
        // Answering the same message again is refused.
        assert_eq!(hub.reply(server, Side::Server, 1, b"again"), Err(Error::UnknownMessage));
    }

    #[test]
    fn reply_needs_a_message_that_asked_for_one() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::Any).unwrap();
        let client = hub.connect(&named("svc"), peer(1, "a")).unwrap();
        let server = hub.accept(&named("svc")).unwrap();
        let m = hub.send(client, Side::Client, b"fire", false).unwrap();
        assert_eq!(hub.reply(server, Side::Server, m, b"x"), Err(Error::UnknownMessage));
        // A side cannot take a reply it did not send, and the reply sits with the sender.
        assert_eq!(hub.take_reply(server, Side::Server, m), Ok(None));
    }

    #[test]
    fn closed_session_refuses_new_traffic_but_keeps_queued_messages() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::Any).unwrap();
        let client = hub.connect(&named("svc"), peer(1, "a")).unwrap();
        let server = hub.accept(&named("svc")).unwrap();
        hub.send(client, Side::Client, b"queued", false).unwrap();
        hub.close(client).unwrap();
        assert_eq!(hub.send(client, Side::Client, b"late", false), Err(Error::Closed));
        let got = hub.receive(server, Side::Server).unwrap().unwrap();
        assert_eq!(got.body, b"queued".to_vec());
    }

    #[test]
    fn anonymous_endpoint_checks_peers_and_can_be_cancelled() {
        let mut hub = Hub::new();
        let ep = hub.anonymous_endpoint(Requirement::App("com.a".to_string()));
        let target = Target::Endpoint(ep);
        assert_eq!(hub.connect(&target, peer(1, "com.b")), Err(Error::Rejected));
        let s = hub.connect(&target, peer(1, "com.a")).unwrap();
        assert_eq!(hub.accept(&target), Some(s));
        assert!(hub.cancel(&target));
        assert!(!hub.cancel(&target));
        assert_eq!(hub.connect(&target, peer(1, "com.a")), Err(Error::NoListener));
        // The session accepted before the cancel stays usable.
        assert!(hub.send(s, Side::Client, b"still open", false).is_ok());
    }

    #[test]
    fn message_size_limit_is_inclusive() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::Any).unwrap();
        let client = hub.connect(&named("svc"), peer(1, "a")).unwrap();
        let exact = vec![0u8; MAX_MESSAGE_BYTES];
        let over = vec![0u8; MAX_MESSAGE_BYTES + 1];
        assert!(hub.send(client, Side::Client, &exact, false).is_ok());
        assert_eq!(hub.send(client, Side::Client, &over, false), Err(Error::TooLarge));
    }

    #[test]
    fn session_ids_and_message_ids_count_up_in_issue_order() {
        let mut hub = Hub::new();
        hub.listen("svc", Requirement::Any).unwrap();
        let a = hub.connect(&named("svc"), peer(1, "a")).unwrap();
        let b = hub.connect(&named("svc"), peer(2, "b")).unwrap();
        assert_eq!((a, b), (SessionId(1), SessionId(2)));
        assert_eq!(hub.accept(&named("svc")), Some(a));
        assert_eq!(hub.accept(&named("svc")), Some(b));
        assert_eq!(hub.send(a, Side::Client, b"1", false), Ok(1));
        assert_eq!(hub.send(b, Side::Client, b"2", false), Ok(2));
        assert_eq!(hub.send(SessionId(9), Side::Client, b"x", false), Err(Error::UnknownSession));
    }
}

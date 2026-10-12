//! Peer mesh: a session that nearby peers join by invitation, with a cap on how many peers it holds and two delivery
//! modes for data.
//!
//! This is the pure model. A [`Mesh`] belongs to one local peer. Other peers reach it through invitations: each
//! incoming invitation is `Pending` until [`Mesh::respond`] accepts or declines it. Accepting connects the peer only
//! while the session has room. The limit counts the local peer, so a session with a limit of 3 holds two remote
//! peers. Data goes to connected peers only, in the order it was sent. [`Mode::Reliable`] fails before anything is
//! delivered when a named peer is not connected. [`Mode::Unreliable`] skips such peers and reports them as dropped.
//!
//! Remaining gaps: no discovery or advertising, no transport, no streams or file resources, no encryption choice,
//! and no automatic disconnect when the app leaves the foreground. The value of the platform's own peer limit
//! constant is not in the fetched documentation text, so [`MAX_SUPPORTED_PEERS`] is this model's bound.

use std::collections::{BTreeMap, VecDeque};

/// Largest session size the model accepts, counting the local peer.
pub const MAX_SUPPORTED_PEERS: usize = 8;
/// Largest payload, in bytes, for one [`Mesh::send`].
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// How data is delivered to a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The call fails, and sends nothing, unless every named peer is connected.
    Reliable,
    /// Connected peers get the data. Peers that are not connected are counted as dropped.
    Unreliable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteState {
    Pending,
    Accepted,
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidName,
    BadLimit,
    AlreadyConnected,
    Full,
    UnknownInvitation,
    NotConnected,
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::InvalidName => "invalid peer name",
            Error::BadLimit => "session limit must be between 1 and MAX_SUPPORTED_PEERS",
            Error::AlreadyConnected => "peer is already connected",
            Error::Full => "session has no room for another peer",
            Error::UnknownInvitation => "no pending invitation with that id",
            Error::NotConnected => "a named peer is not connected",
            Error::TooLarge => "payload is larger than the limit",
        };
        f.write_str(text)
    }
}

/// The category an app switches on. The names match the platform's peer error codes (`invalidParameter`, `notConnected`
/// and so on), which [`Code::name`] returns. The model returns only `InvalidParameter`, `Unavailable` and `NotConnected`;
/// the other variants exist so that an app's handling matches the platform's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Cancelled,
    InvalidParameter,
    NotConnected,
    TimedOut,
    Unavailable,
    Unknown,
    Unsupported,
}

impl Code {
    /// The platform's spelling of this code.
    pub fn name(self) -> &'static str {
        match self {
            Code::Cancelled => "cancelled",
            Code::InvalidParameter => "invalidParameter",
            Code::NotConnected => "notConnected",
            Code::TimedOut => "timedOut",
            Code::Unavailable => "unavailable",
            Code::Unknown => "unknown",
            Code::Unsupported => "unsupported",
        }
    }
}

impl Error {
    /// The category of this error. Bad names, limits, invitation ids and payload sizes are `InvalidParameter`. A full
    /// session is `Unavailable` (it may have room later), and a reliable send to a missing peer is `NotConnected`.
    pub fn code(&self) -> Code {
        match self {
            Error::InvalidName
            | Error::BadLimit
            | Error::AlreadyConnected
            | Error::UnknownInvitation
            | Error::TooLarge => Code::InvalidParameter,
            Error::Full => Code::Unavailable,
            Error::NotConnected => Code::NotConnected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Invitation {
    from: String,
    state: InviteState,
}

/// What one [`Mesh::send`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub sent: usize,
    pub dropped: usize,
}

/// One local session. Ids for invitations count up from 1.
pub struct Mesh {
    local: String,
    limit: usize,
    invitations: BTreeMap<u64, Invitation>,
    next_invitation: u64,
    /// Connected remote peers and the data waiting to be received from each.
    connected: BTreeMap<String, VecDeque<(Mode, Vec<u8>)>>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

impl Mesh {
    /// A session for `local` that holds at most `limit` peers, the local peer included.
    pub fn new(local: &str, limit: usize) -> Result<Mesh, Error> {
        if !valid_name(local) {
            return Err(Error::InvalidName);
        }
        if limit == 0 || limit > MAX_SUPPORTED_PEERS {
            return Err(Error::BadLimit);
        }
        Ok(Mesh {
            local: local.to_string(),
            limit,
            invitations: BTreeMap::new(),
            next_invitation: 1,
            connected: BTreeMap::new(),
        })
    }

    pub fn local(&self) -> &str {
        &self.local
    }

    /// The most peers the session holds, counting the local peer.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Peers currently connected, not counting the local peer.
    pub fn connected_names(&self) -> Vec<String> {
        self.connected.keys().cloned().collect()
    }

    pub fn is_connected(&self, peer: &str) -> bool {
        self.connected.contains_key(peer)
    }

    /// Records an invitation from `from`. It stays `Pending` until answered. A peer that is already connected cannot
    /// invite again.
    pub fn receive_invitation(&mut self, from: &str) -> Result<u64, Error> {
        if !valid_name(from) {
            return Err(Error::InvalidName);
        }
        if self.connected.contains_key(from) {
            return Err(Error::AlreadyConnected);
        }
        let id = self.next_invitation;
        self.next_invitation += 1;
        self.invitations.insert(id, Invitation { from: from.to_string(), state: InviteState::Pending });
        Ok(id)
    }

    /// Accepts or declines a pending invitation. Accepting connects the peer if the session has room: the local peer
    /// and the connected peers together must stay below the limit before a new peer is added.
    pub fn respond(&mut self, invitation: u64, accept: bool) -> Result<InviteState, Error> {
        let pending = match self.invitations.get(&invitation) {
            Some(inv) if inv.state == InviteState::Pending => inv.from.clone(),
            _ => return Err(Error::UnknownInvitation),
        };
        if !accept {
            self.set_state(invitation, InviteState::Declined);
            return Ok(InviteState::Declined);
        }
        // Local peer plus connected peers, then one more must still fit under the limit.
        if self.connected.len() + 1 >= self.limit {
            return Err(Error::Full);
        }
        self.connected.insert(pending, VecDeque::new());
        self.set_state(invitation, InviteState::Accepted);
        Ok(InviteState::Accepted)
    }

    fn set_state(&mut self, invitation: u64, state: InviteState) {
        if let Some(inv) = self.invitations.get_mut(&invitation) {
            inv.state = state;
        }
    }

    /// Sends `data` to the named peers, or to every connected peer when `to` is empty. Peers are visited in name
    /// order. Reliable sends are checked first, so a missing peer leaves no delivery behind.
    pub fn send(&mut self, to: &[&str], data: &[u8], mode: Mode) -> Result<Delivery, Error> {
        if data.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::TooLarge);
        }
        let targets: Vec<String> = if to.is_empty() {
            self.connected.keys().cloned().collect()
        } else {
            let mut names: Vec<String> = to.iter().map(|s| s.to_string()).collect();
            names.sort();
            names.dedup();
            names
        };
        let missing = targets.iter().filter(|t| !self.connected.contains_key(t.as_str())).count();
        if mode == Mode::Reliable && missing > 0 {
            return Err(Error::NotConnected);
        }
        let mut delivery = Delivery { sent: 0, dropped: 0 };
        for name in &targets {
            match self.connected.get_mut(name) {
                Some(queue) => {
                    queue.push_back((mode, data.to_vec()));
                    delivery.sent += 1;
                }
                None => delivery.dropped += 1,
            }
        }
        Ok(delivery)
    }

    /// Takes the next message from `peer`, with the mode it was sent in.
    pub fn receive(&mut self, peer: &str) -> Option<(Mode, Vec<u8>)> {
        self.connected.get_mut(peer)?.pop_front()
    }

    /// Removes a peer and the data still waiting from it. Returns false if it was not connected.
    pub fn disconnect(&mut self, peer: &str) -> bool {
        self.connected.remove(peer).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(limit: usize) -> Mesh {
        Mesh::new("me", limit).unwrap()
    }

    #[test]
    fn limit_must_be_between_one_and_the_supported_maximum() {
        assert!(matches!(Mesh::new("me", 0), Err(Error::BadLimit)));
        assert!(matches!(Mesh::new("me", MAX_SUPPORTED_PEERS + 1), Err(Error::BadLimit)));
        assert!(Mesh::new("me", MAX_SUPPORTED_PEERS).is_ok());
        assert!(matches!(Mesh::new("", 3), Err(Error::InvalidName)));
    }

    #[test]
    fn limit_counts_the_local_peer() {
        // Limit 3 means local plus two remote peers. The third acceptance has no room.
        let mut m = mesh(3);
        let a = m.receive_invitation("a").unwrap();
        let b = m.receive_invitation("b").unwrap();
        let c = m.receive_invitation("c").unwrap();
        assert_eq!(m.respond(a, true), Ok(InviteState::Accepted));
        assert_eq!(m.respond(b, true), Ok(InviteState::Accepted));
        assert_eq!(m.respond(c, true), Err(Error::Full));
        // The refused invitation is still pending and can be declined.
        assert_eq!(m.respond(c, false), Ok(InviteState::Declined));
        assert_eq!(m.connected_names(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn limit_of_one_admits_no_remote_peer() {
        let mut m = mesh(1);
        let a = m.receive_invitation("a").unwrap();
        assert_eq!(m.respond(a, true), Err(Error::Full));
        assert!(!m.is_connected("a"));
    }

    #[test]
    fn invitations_answer_once_and_connected_peers_cannot_reinvite() {
        let mut m = mesh(4);
        let a = m.receive_invitation("a").unwrap();
        assert_eq!(m.respond(a, false), Ok(InviteState::Declined));
        assert_eq!(m.respond(a, true), Err(Error::UnknownInvitation));
        assert_eq!(m.respond(99, true), Err(Error::UnknownInvitation));

        let b = m.receive_invitation("b").unwrap();
        m.respond(b, true).unwrap();
        assert_eq!(m.receive_invitation("b"), Err(Error::AlreadyConnected));
    }

    #[test]
    fn reliable_send_to_a_missing_peer_delivers_nothing() {
        let mut m = mesh(4);
        let a = m.receive_invitation("a").unwrap();
        m.respond(a, true).unwrap();
        assert_eq!(m.send(&["a", "ghost"], b"hi", Mode::Reliable), Err(Error::NotConnected));
        assert_eq!(m.receive("a"), None);
    }

    #[test]
    fn unreliable_send_counts_missing_peers_as_dropped() {
        let mut m = mesh(4);
        let a = m.receive_invitation("a").unwrap();
        m.respond(a, true).unwrap();
        let d = m.send(&["a", "ghost"], b"hi", Mode::Unreliable).unwrap();
        assert_eq!(d, Delivery { sent: 1, dropped: 1 });
        assert_eq!(m.receive("a"), Some((Mode::Unreliable, b"hi".to_vec())));
        assert_eq!(m.receive("a"), None);
    }

    #[test]
    fn empty_target_list_sends_to_every_connected_peer_in_name_order() {
        let mut m = mesh(4);
        for name in ["b", "a"] {
            let id = m.receive_invitation(name).unwrap();
            m.respond(id, true).unwrap();
        }
        let d = m.send(&[], b"x", Mode::Reliable).unwrap();
        assert_eq!(d, Delivery { sent: 2, dropped: 0 });
        assert_eq!(m.connected_names(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(m.receive("a"), Some((Mode::Reliable, b"x".to_vec())));
        assert_eq!(m.receive("b"), Some((Mode::Reliable, b"x".to_vec())));
    }

    #[test]
    fn payloads_arrive_in_send_order_and_the_size_limit_is_inclusive() {
        let mut m = mesh(4);
        let a = m.receive_invitation("a").unwrap();
        m.respond(a, true).unwrap();
        m.send(&["a"], b"1", Mode::Reliable).unwrap();
        m.send(&["a"], b"2", Mode::Unreliable).unwrap();
        assert_eq!(m.receive("a"), Some((Mode::Reliable, b"1".to_vec())));
        assert_eq!(m.receive("a"), Some((Mode::Unreliable, b"2".to_vec())));

        let exact = vec![7u8; MAX_PAYLOAD_BYTES];
        let over = vec![7u8; MAX_PAYLOAD_BYTES + 1];
        assert!(m.send(&["a"], &exact, Mode::Reliable).is_ok());
        assert_eq!(m.send(&["a"], &over, Mode::Reliable), Err(Error::TooLarge));
    }

    #[test]
    fn errors_map_to_the_platform_codes_by_cause() {
        let mut m = mesh(2);
        let a = m.receive_invitation("a").unwrap();
        m.respond(a, true).unwrap();
        // The session of limit 2 is full once "a" is in, so a second acceptance has no room.
        let b = m.receive_invitation("b").unwrap();
        assert_eq!(m.respond(b, true).unwrap_err().code(), Code::Unavailable);
        assert_eq!(m.send(&["ghost"], b"x", Mode::Reliable).unwrap_err().code(), Code::NotConnected);
        assert_eq!(m.send(&["a"], &vec![0u8; MAX_PAYLOAD_BYTES + 1], Mode::Reliable).unwrap_err().code(), Code::InvalidParameter);
        assert_eq!(Error::AlreadyConnected.code(), Code::InvalidParameter);
        assert_eq!(Code::TimedOut.name(), "timedOut");
        assert_eq!(Code::InvalidParameter.name(), "invalidParameter");
    }

    #[test]
    fn disconnect_drops_the_queue_and_frees_a_slot() {
        let mut m = mesh(2);
        let a = m.receive_invitation("a").unwrap();
        m.respond(a, true).unwrap();
        m.send(&["a"], b"pending", Mode::Reliable).unwrap();
        assert!(m.disconnect("a"));
        assert!(!m.disconnect("a"));
        assert_eq!(m.receive("a"), None);
        assert!(!m.is_connected("a"));
        // The slot is free again, so a new peer can join a session of limit 2.
        let b = m.receive_invitation("b").unwrap();
        assert_eq!(m.respond(b, true), Ok(InviteState::Accepted));
    }
}

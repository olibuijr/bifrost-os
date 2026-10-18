//! Secure session management: the table of open secure channels (`secure_channel`) a process holds, one per peer, with
//! the rules that keep them safe. A session ends when it is older than the policy's lifetime, when it has been idle
//! longer than the policy allows, when its direction counter is used up, or when the person or the app closes it.
//! Adopting a new channel for a peer ends that peer's earlier session, so a reconnect never leaves two live channels for
//! one peer. A closed session drops its channel, which wipes the keys.
//!
//! Pure model: times are whole seconds passed in by the caller, and nothing here reads a clock, a file or a socket.

use crate::secure_channel::{self, Channel};
use std::collections::BTreeMap;

/// The id of an open session. Ids are not reused while the manager lives.
pub type SessionId = u32;

/// How long sessions live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Seconds from adoption until a session must be set up again.
    pub max_age_s: u64,
    /// Seconds without a message after which a session ends.
    pub idle_s: u64,
    /// The most sessions open at once.
    pub max_sessions: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { max_age_s: 24 * 60 * 60, idle_s: 15 * 60, max_sessions: 64 }
    }
}

/// Why a session call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No open session has this id (it never existed, or it was closed or expired).
    Unknown,
    /// The session was older or idler than the policy allows and has just been closed.
    Expired,
    /// The manager already holds `max_sessions` sessions for other peers.
    Full,
    /// The channel refused the message.
    Channel(secure_channel::Error),
}

/// Why a session was closed by a sweep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    TooOld,
    Idle,
}

struct Session {
    peer: String,
    channel: Channel,
    opened_at: u64,
    last_used: u64,
}

impl Session {
    fn expiry(&self, policy: &Policy, now: u64) -> Option<Reason> {
        if now.saturating_sub(self.opened_at) >= policy.max_age_s {
            Some(Reason::TooOld)
        } else if now.saturating_sub(self.last_used) >= policy.idle_s {
            Some(Reason::Idle)
        } else {
            None
        }
    }
}

/// The open sessions of a process.
pub struct Manager {
    policy: Policy,
    sessions: BTreeMap<SessionId, Session>,
    next_id: SessionId,
}

impl Manager {
    pub fn new(policy: Policy) -> Manager {
        Manager { policy, sessions: BTreeMap::new(), next_id: 1 }
    }

    /// How many sessions are open.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Takes over an established channel for `peer` and returns its session id. The peer's earlier session, if any, is
    /// closed first. `Full` when the manager is at its limit and `peer` has no session to replace.
    pub fn adopt(&mut self, peer: &str, channel: Channel, now: u64) -> Result<SessionId, Error> {
        let earlier = self.find(peer);
        if earlier.is_none() && self.sessions.len() >= self.policy.max_sessions {
            return Err(Error::Full);
        }
        if let Some(id) = earlier {
            self.sessions.remove(&id);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.sessions.insert(id, Session { peer: peer.to_string(), channel, opened_at: now, last_used: now });
        Ok(id)
    }

    /// The open session of `peer`.
    pub fn find(&self, peer: &str) -> Option<SessionId> {
        self.sessions.iter().find(|(_, s)| s.peer == peer).map(|(id, _)| *id)
    }

    /// The peer a session belongs to.
    pub fn peer(&self, id: SessionId) -> Option<&str> {
        self.sessions.get(&id).map(|s| s.peer.as_str())
    }

    /// Looks the session up, closing it first when it has expired.
    fn live(&mut self, id: SessionId, now: u64) -> Result<&mut Session, Error> {
        let expired = match self.sessions.get(&id) {
            None => return Err(Error::Unknown),
            Some(s) => s.expiry(&self.policy, now).is_some(),
        };
        if expired {
            self.sessions.remove(&id);
            return Err(Error::Expired);
        }
        self.sessions.get_mut(&id).ok_or(Error::Unknown)
    }

    /// Seals a message on a session.
    pub fn seal(&mut self, id: SessionId, aad: &[u8], plaintext: &[u8], now: u64) -> Result<Vec<u8>, Error> {
        let session = self.live(id, now)?;
        let out = session.channel.seal(aad, plaintext).map_err(Error::Channel)?;
        session.last_used = now;
        Ok(out)
    }

    /// Opens a message on a session. A message that fails to open does not count as use, so garbage cannot keep a
    /// session alive.
    pub fn open(&mut self, id: SessionId, aad: &[u8], message: &[u8], now: u64) -> Result<Vec<u8>, Error> {
        let session = self.live(id, now)?;
        let out = session.channel.open(aad, message).map_err(Error::Channel)?;
        session.last_used = now;
        Ok(out)
    }

    /// Closes a session; false when there was none.
    pub fn close(&mut self, id: SessionId) -> bool {
        self.sessions.remove(&id).is_some()
    }

    /// Closes every session the policy has run out for and reports them.
    pub fn sweep(&mut self, now: u64) -> Vec<(SessionId, String, Reason)> {
        let due: Vec<(SessionId, Reason)> =
            self.sessions.iter().filter_map(|(id, s)| s.expiry(&self.policy, now).map(|r| (*id, r))).collect();
        due.into_iter()
            .filter_map(|(id, reason)| self.sessions.remove(&id).map(|s| (id, s.peer.clone(), reason)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secure_channel::{begin, Role};
    use crate::x25519::PrivateKey;

    fn public(private: &[u8; 32]) -> [u8; 32] {
        PrivateKey::from_raw(private).unwrap().public_key().raw()
    }

    /// Two ends of one channel; `salt` varies the ephemeral keys so channels differ.
    fn ends(salt: u8) -> (Channel, Channel) {
        let (a_static, b_static) = ([0x11u8; 32], [0x22u8; 32]);
        let (a, a_pub) = begin(Role::Initiator, &a_static, &public(&b_static), &[salt; 32]).unwrap();
        let (b, b_pub) = begin(Role::Responder, &b_static, &public(&a_static), &[salt ^ 0x55; 32]).unwrap();
        (a.finish(&b_pub).unwrap(), b.finish(&a_pub).unwrap())
    }

    fn short() -> Policy {
        Policy { max_age_s: 100, idle_s: 30, max_sessions: 2 }
    }

    #[test]
    fn a_session_carries_messages_both_ways() {
        let (a_end, b_end) = ends(1);
        let mut left = Manager::new(short());
        let mut right = Manager::new(short());
        let l = left.adopt("b", a_end, 0).unwrap();
        let r = right.adopt("a", b_end, 0).unwrap();
        let wire = left.seal(l, b"", b"ping", 1).unwrap();
        assert_eq!(right.open(r, b"", &wire, 2).unwrap(), b"ping");
        assert_eq!(left.peer(l), Some("b"));
        assert_eq!(left.find("b"), Some(l));
    }

    #[test]
    fn a_new_channel_for_a_peer_replaces_its_session() {
        let mut m = Manager::new(short());
        let first = m.adopt("b", ends(1).0, 0).unwrap();
        let second = m.adopt("b", ends(2).0, 5).unwrap();
        assert_ne!(first, second);
        assert_eq!(m.len(), 1);
        assert_eq!(m.seal(first, b"", b"x", 6), Err(Error::Unknown));
        assert!(m.seal(second, b"", b"x", 6).is_ok());
    }

    #[test]
    fn the_limit_counts_peers_not_reconnects() {
        let mut m = Manager::new(short());
        m.adopt("a", ends(1).0, 0).unwrap();
        m.adopt("b", ends(2).0, 0).unwrap();
        assert_eq!(m.adopt("c", ends(3).0, 0), Err(Error::Full));
        assert!(m.adopt("a", ends(4).0, 0).is_ok());
    }

    #[test]
    fn an_idle_or_old_session_ends_when_used_or_swept() {
        let mut m = Manager::new(short());
        let a = m.adopt("a", ends(1).0, 0).unwrap();
        let b = m.adopt("b", ends(2).0, 0).unwrap();
        // b stays busy, a goes idle
        assert!(m.seal(b, b"", b"x", 20).is_ok());
        assert_eq!(m.seal(a, b"", b"x", 31), Err(Error::Expired));
        assert_eq!(m.seal(a, b"", b"x", 31), Err(Error::Unknown), "the expired session is gone");
        // keep b alive until its age runs out
        for t in [40u64, 60, 80, 99] {
            assert!(m.seal(b, b"", b"x", t).is_ok());
        }
        assert_eq!(m.sweep(100), vec![(b, "b".to_string(), Reason::TooOld)]);
        assert!(m.is_empty());
    }

    #[test]
    fn garbage_does_not_keep_a_session_alive() {
        let (_, b_end) = ends(1);
        let mut m = Manager::new(short());
        let id = m.adopt("a", b_end, 0).unwrap();
        assert!(matches!(m.open(id, b"", &[0u8; 40], 25), Err(Error::Channel(_))));
        assert_eq!(m.sweep(30), vec![(id, "a".to_string(), Reason::Idle)]);
    }

    #[test]
    fn close_removes_the_session() {
        let mut m = Manager::new(short());
        let id = m.adopt("a", ends(1).0, 0).unwrap();
        assert!(m.close(id));
        assert!(!m.close(id));
        assert_eq!(m.open(id, b"", b"", 1), Err(Error::Unknown));
    }
}

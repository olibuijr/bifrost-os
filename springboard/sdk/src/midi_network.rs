//! MIDI network sessions: the session model (`MIDINetworkSession`, `MIDINetworkHost`, `MIDINetworkConnection`)
//! and the RTP-MIDI session packets that invite a host to join.
//!
//! A session packet is sent to a host's control port. All fields are big endian: `FF FF`, a two-letter command
//! (`IN` invitation, `OK` accept, `NO` reject, `BY` leave), protocol version 2, the initiator token, the sender's
//! SSRC and, for `IN` and `OK`, the sender's name as a null-terminated UTF-8 string. Nothing here opens a socket.

/// The default control port; the data port is the next one.
pub const DEFAULT_PORT: u16 = 5004;
/// The protocol version in every session packet.
pub const PROTOCOL_VERSION: u32 = 2;

const SIGNATURE: [u8; 2] = [0xFF, 0xFF];

/// The command of a session packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Invitation,
    InvitationAccepted,
    InvitationRejected,
    EndSession,
}

impl Command {
    fn code(self) -> [u8; 2] {
        match self {
            Command::Invitation => *b"IN",
            Command::InvitationAccepted => *b"OK",
            Command::InvitationRejected => *b"NO",
            Command::EndSession => *b"BY",
        }
    }

    fn from_code(c: [u8; 2]) -> Option<Command> {
        match &c {
            b"IN" => Some(Command::Invitation),
            b"OK" => Some(Command::InvitationAccepted),
            b"NO" => Some(Command::InvitationRejected),
            b"BY" => Some(Command::EndSession),
            _ => None,
        }
    }

    fn carries_name(self) -> bool {
        matches!(self, Command::Invitation | Command::InvitationAccepted)
    }
}

/// Why bytes are not a session packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotSessionPacket,
    Truncated,
    UnknownCommand,
    UnsupportedVersion(u32),
    BadName,
}

/// One session packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPacket {
    pub command: Command,
    pub initiator_token: u32,
    pub ssrc: u32,
    /// Empty for `NO` and `BY`.
    pub name: String,
}

impl SessionPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(17 + self.name.len());
        out.extend_from_slice(&SIGNATURE);
        out.extend_from_slice(&self.command.code());
        out.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        out.extend_from_slice(&self.initiator_token.to_be_bytes());
        out.extend_from_slice(&self.ssrc.to_be_bytes());
        if self.command.carries_name() {
            out.extend_from_slice(self.name.as_bytes());
            out.push(0);
        }
        out
    }

    pub fn decode(b: &[u8]) -> Result<SessionPacket, Error> {
        if b.len() < 2 || b[..2] != SIGNATURE {
            return Err(Error::NotSessionPacket);
        }
        if b.len() < 16 {
            return Err(Error::Truncated);
        }
        let command = Command::from_code([b[2], b[3]]).ok_or(Error::UnknownCommand)?;
        let word = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let version = word(4);
        if version != PROTOCOL_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let name = if command.carries_name() {
            let tail = &b[16..];
            let end = tail.iter().position(|&x| x == 0).ok_or(Error::Truncated)?;
            String::from_utf8(tail[..end].to_vec()).map_err(|_| Error::BadName)?
        } else {
            String::new()
        };
        Ok(SessionPacket { command, initiator_token: word(8), ssrc: word(12), name })
    }
}

/// A remote session (`MIDINetworkHost`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    pub address: String,
    pub port: u16,
}

impl Host {
    pub fn new(name: &str, address: &str, port: u16) -> Host {
        Host { name: name.to_string(), address: address.to_string(), port }
    }
}

/// A connection to a host (`MIDINetworkConnection`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub host: Host,
}

/// Who may connect to us (`MIDINetworkConnectionPolicy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionPolicy {
    NoOne,
    HostsInContactList,
    Anyone,
}

/// The state of one invitation we sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invitation {
    Pending,
    Accepted,
    Rejected,
}

/// The network session of the device (`MIDINetworkSession`).
#[derive(Debug)]
pub struct NetworkSession {
    pub enabled: bool,
    pub network_port: u16,
    pub network_name: String,
    pub local_name: String,
    pub connection_policy: ConnectionPolicy,
    contacts: Vec<Host>,
    connections: Vec<Connection>,
    ssrc: u32,
    invitations: Vec<(u32, Host, Invitation)>,
}

impl NetworkSession {
    pub fn new(name: &str, ssrc: u32) -> Self {
        NetworkSession {
            enabled: false,
            network_port: DEFAULT_PORT,
            network_name: name.to_string(),
            local_name: name.to_string(),
            connection_policy: ConnectionPolicy::HostsInContactList,
            contacts: Vec::new(),
            connections: Vec::new(),
            ssrc,
            invitations: Vec::new(),
        }
    }

    pub fn contacts(&self) -> &[Host] {
        &self.contacts
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Adds a contact; `false` when the same name and address is already there.
    pub fn add_contact(&mut self, host: Host) -> bool {
        if self.contacts.iter().any(|h| h.name == host.name && h.address == host.address) {
            return false;
        }
        self.contacts.push(host);
        true
    }

    pub fn remove_contact(&mut self, host: &Host) -> bool {
        let before = self.contacts.len();
        self.contacts.retain(|h| h != host);
        self.contacts.len() != before
    }

    /// Adds a connection; `false` when it exists.
    pub fn add_connection(&mut self, host: Host) -> bool {
        if self.connections.iter().any(|c| c.host == host) {
            return false;
        }
        self.connections.push(Connection { host });
        true
    }

    pub fn remove_connection(&mut self, host: &Host) -> bool {
        let before = self.connections.len();
        self.connections.retain(|c| &c.host != host);
        self.connections.len() != before
    }

    /// The invitation to send to `host`'s control port. `None` while the session is disabled.
    pub fn invite(&mut self, host: &Host, token: u32) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        self.invitations.push((token, host.clone(), Invitation::Pending));
        Some(
            SessionPacket {
                command: Command::Invitation,
                initiator_token: token,
                ssrc: self.ssrc,
                name: self.local_name.clone(),
            }
            .encode(),
        )
    }

    /// The state of the invitation sent with `token`.
    pub fn invitation_state(&self, token: u32) -> Option<Invitation> {
        self.invitations.iter().find(|(t, _, _)| *t == token).map(|(_, _, s)| *s)
    }

    fn allows(&self, name: &str) -> bool {
        self.enabled
            && match self.connection_policy {
                ConnectionPolicy::NoOne => false,
                ConnectionPolicy::Anyone => true,
                ConnectionPolicy::HostsInContactList => self.contacts.iter().any(|h| h.name == name),
            }
    }

    /// Takes a packet from a host and returns the bytes to send back, if any. `OK` and `NO` settle the invitation
    /// with that token, and `OK` adds the connection. `BY` removes the connection of the invitation with that
    /// token. An invitation is accepted when the policy allows it: `Anyone`, or `HostsInContactList` and the
    /// sender's name is a contact.
    pub fn handle(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        let packet = SessionPacket::decode(bytes).ok()?;
        match packet.command {
            Command::InvitationAccepted | Command::InvitationRejected => {
                let accepted = packet.command == Command::InvitationAccepted;
                let entry = self
                    .invitations
                    .iter_mut()
                    .find(|(t, _, s)| *t == packet.initiator_token && *s == Invitation::Pending)?;
                entry.2 = if accepted { Invitation::Accepted } else { Invitation::Rejected };
                let host = entry.1.clone();
                if accepted {
                    self.add_connection(host);
                }
                None
            }
            Command::EndSession => {
                let host = self.invitations.iter().find(|(t, _, _)| *t == packet.initiator_token).map(|(_, h, _)| h.clone());
                if let Some(host) = host {
                    self.remove_connection(&host);
                }
                None
            }
            Command::Invitation => {
                let command = if self.allows(&packet.name) {
                    Command::InvitationAccepted
                } else {
                    Command::InvitationRejected
                };
                Some(
                    SessionPacket {
                        command,
                        initiator_token: packet.initiator_token,
                        ssrc: self.ssrc,
                        name: self.local_name.clone(),
                    }
                    .encode(),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitation_matches_the_wire_layout() {
        let p = SessionPacket {
            command: Command::Invitation,
            initiator_token: 0x0102_0304,
            ssrc: 0xAABB_CCDD,
            name: "Mac".into(),
        };
        let expected = [
            0xFF, 0xFF, b'I', b'N', 0, 0, 0, 2, 1, 2, 3, 4, 0xAA, 0xBB, 0xCC, 0xDD, b'M', b'a', b'c', 0,
        ];
        assert_eq!(p.encode(), expected);
        assert_eq!(SessionPacket::decode(&expected), Ok(p));
    }

    #[test]
    fn end_and_reject_carry_no_name() {
        for command in [Command::EndSession, Command::InvitationRejected] {
            let p = SessionPacket { command, initiator_token: 7, ssrc: 9, name: String::new() };
            let bytes = p.encode();
            assert_eq!(bytes.len(), 16);
            assert_eq!(SessionPacket::decode(&bytes), Ok(p));
        }
    }

    #[test]
    fn bad_packets_are_refused() {
        assert_eq!(SessionPacket::decode(&[0x80, 0x61]), Err(Error::NotSessionPacket));
        assert_eq!(SessionPacket::decode(&[0xFF, 0xFF, b'I', b'N']), Err(Error::Truncated));
        let mut b = SessionPacket { command: Command::Invitation, initiator_token: 1, ssrc: 2, name: "x".into() }.encode();
        b[2] = b'Z';
        assert_eq!(SessionPacket::decode(&b), Err(Error::UnknownCommand));
        b[2] = b'I';
        b[7] = 3;
        assert_eq!(SessionPacket::decode(&b), Err(Error::UnsupportedVersion(3)));
        b[7] = 2;
        b.pop();
        assert_eq!(SessionPacket::decode(&b), Err(Error::Truncated));
        let bad = [0xFF, 0xFF, b'I', b'N', 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0xC0, 0];
        assert_eq!(SessionPacket::decode(&bad), Err(Error::BadName));
    }

    fn enabled(name: &str, ssrc: u32) -> NetworkSession {
        let mut s = NetworkSession::new(name, ssrc);
        s.enabled = true;
        s
    }

    #[test]
    fn a_disabled_session_sends_no_invitation() {
        let mut s = NetworkSession::new("Me", 1);
        assert_eq!(s.network_port, DEFAULT_PORT);
        assert!(s.invite(&Host::new("Peer", "10.0.0.2", 5004), 5).is_none());
    }

    #[test]
    fn an_accepted_invitation_adds_the_connection() {
        let host = Host::new("Peer", "10.0.0.2", 5004);
        let mut me = enabled("Me", 1);
        let mut peer = enabled("Peer", 2);
        peer.connection_policy = ConnectionPolicy::Anyone;
        let invite = me.invite(&host, 77).unwrap();
        assert_eq!(me.invitation_state(77), Some(Invitation::Pending));
        let reply = peer.handle(&invite).expect("peer answers");
        assert_eq!(SessionPacket::decode(&reply).unwrap().command, Command::InvitationAccepted);
        assert!(me.handle(&reply).is_none());
        assert_eq!(me.invitation_state(77), Some(Invitation::Accepted));
        assert_eq!(me.connections(), &[Connection { host: host.clone() }]);
        let bye = SessionPacket { command: Command::EndSession, initiator_token: 77, ssrc: 2, name: String::new() }.encode();
        me.handle(&bye);
        assert!(me.connections().is_empty());
    }

    #[test]
    fn a_rejected_invitation_adds_no_connection() {
        let host = Host::new("Peer", "10.0.0.2", 5004);
        let mut me = enabled("Me", 1);
        let mut peer = enabled("Peer", 2);
        peer.connection_policy = ConnectionPolicy::NoOne;
        let reply = peer.handle(&me.invite(&host, 5).unwrap()).unwrap();
        me.handle(&reply);
        assert_eq!(me.invitation_state(5), Some(Invitation::Rejected));
        assert!(me.connections().is_empty());
    }

    #[test]
    fn policy_decides_who_is_accepted() {
        let invite =
            SessionPacket { command: Command::Invitation, initiator_token: 1, ssrc: 9, name: "Stranger".into() }.encode();
        let answer = |s: &mut NetworkSession| SessionPacket::decode(&s.handle(&invite).unwrap()).unwrap().command;
        let mut s = enabled("Me", 1);
        assert_eq!(answer(&mut s), Command::InvitationRejected);
        s.add_contact(Host::new("Stranger", "10.0.0.9", 5004));
        assert_eq!(answer(&mut s), Command::InvitationAccepted);
        s.connection_policy = ConnectionPolicy::NoOne;
        assert_eq!(answer(&mut s), Command::InvitationRejected);
        s.connection_policy = ConnectionPolicy::Anyone;
        s.enabled = false;
        assert_eq!(answer(&mut s), Command::InvitationRejected);
    }

    #[test]
    fn contacts_and_connections_do_not_duplicate() {
        let host = Host::new("Peer", "10.0.0.2", 5004);
        let mut s = NetworkSession::new("Me", 1);
        assert!(s.add_contact(host.clone()));
        assert!(!s.add_contact(host.clone()));
        assert!(s.remove_contact(&host));
        assert!(!s.remove_contact(&host));
        assert!(s.add_connection(host.clone()));
        assert!(!s.add_connection(host.clone()));
        assert!(s.remove_connection(&host));
    }
}

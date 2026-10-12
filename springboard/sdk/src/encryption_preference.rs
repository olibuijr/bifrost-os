//! Encryption preference for a nearby peer session: whether traffic between peers is encrypted. The three choices
//! mirror the official `optional`, `required` and `none` cases. The rule is the one the documentation states: a
//! connection uses encryption when every peer chose optional or required; if any peer chose none, the session is not
//! encrypted; and a required side refuses to connect to a peer that chose none.
//!
//! This is the pure decision only. No key is exchanged and no byte is encrypted here; the transport asks `negotiate`
//! or `session_encrypted` and then does (or does not) encrypt.
//!
//! Remaining gaps: the transport does not encrypt yet (the nearby sharing path is an HMAC over the header), and no
//! certificate or identity check is made.

use std::fmt;

/// What a session asks for: encryption if the other side agrees, encryption always, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncryptionPreference {
    /// Prefers encryption, but accepts an unencrypted connection.
    Optional,
    /// Refuses any connection that is not encrypted.
    Required,
    /// Never encrypts.
    None,
}

/// A required side met a peer that chose none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncryptionError {
    Refused,
}

impl fmt::Display for EncryptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a required encryption setting cannot connect to a peer with encryption off")
    }
}

/// Decides one pair of sides. `Ok(true)` means the connection is encrypted, `Ok(false)` that it is not. Refuses when
/// one side is required and the other chose none.
pub fn negotiate(local: EncryptionPreference, remote: EncryptionPreference) -> Result<bool, EncryptionError> {
    let off = |p: EncryptionPreference| p == EncryptionPreference::None;
    let required = |p: EncryptionPreference| p == EncryptionPreference::Required;
    if (required(local) && off(remote)) || (required(remote) && off(local)) {
        return Err(EncryptionError::Refused);
    }
    Ok(!off(local) && !off(remote))
}

/// Decides a whole session: `local` against every remote peer. The session is encrypted only if the local side and
/// every peer chose optional or required. Any refusal from `negotiate` fails the session. With no peers, the session
/// is encrypted when the local side is not none.
pub fn session_encrypted(local: EncryptionPreference, peers: &[EncryptionPreference]) -> Result<bool, EncryptionError> {
    let mut encrypted = local != EncryptionPreference::None;
    for &peer in peers {
        encrypted &= negotiate(local, peer)?;
    }
    Ok(encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use EncryptionPreference::{None as Off, Optional, Required};

    #[test]
    fn optional_with_optional_or_required_is_encrypted() {
        assert_eq!(negotiate(Optional, Optional), Ok(true));
        assert_eq!(negotiate(Optional, Required), Ok(true));
        assert_eq!(negotiate(Required, Required), Ok(true));
    }

    #[test]
    fn none_on_either_side_is_not_encrypted_unless_refused() {
        assert_eq!(negotiate(Off, Optional), Ok(false));
        assert_eq!(negotiate(Optional, Off), Ok(false));
        assert_eq!(negotiate(Off, Off), Ok(false));
    }

    #[test]
    fn a_required_side_refuses_a_peer_that_chose_none() {
        assert_eq!(negotiate(Required, Off), Err(EncryptionError::Refused));
        assert_eq!(negotiate(Off, Required), Err(EncryptionError::Refused));
    }

    #[test]
    fn session_is_encrypted_only_when_every_peer_agrees() {
        assert_eq!(session_encrypted(Optional, &[Optional, Required]), Ok(true));
        assert_eq!(session_encrypted(Optional, &[Optional, Off]), Ok(false));
        assert_eq!(session_encrypted(Required, &[Optional, Required]), Ok(true));
    }

    #[test]
    fn a_required_session_fails_when_any_peer_chose_none() {
        assert_eq!(session_encrypted(Required, &[Optional, Off]), Err(EncryptionError::Refused));
        assert_eq!(session_encrypted(Off, &[Required]), Err(EncryptionError::Refused));
    }

    #[test]
    fn a_session_with_no_peers_follows_the_local_side() {
        assert_eq!(session_encrypted(Optional, &[]), Ok(true));
        assert_eq!(session_encrypted(Off, &[]), Ok(false));
    }
}

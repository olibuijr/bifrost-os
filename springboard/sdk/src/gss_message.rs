//! GSS-API message protection (`gss_get_mic`, `gss_verify_mic`, `gss_wrap`, `gss_unwrap`, `gss_wrap_size_limit`): a
//! MIC that proves a message was not changed, and the integrity-only wrap built on it, under a session key the caller
//! holds.
//!
//! The MIC is HMAC-SHA256 (`digest::hmac_sha256`) over the message, keyed with the session key. A MIC token is the tag
//! `'M'` and the 32-byte MAC: 33 bytes. A wrapped message is the tag `'W'`, the 32-byte MAC, and the message in the
//! clear: 33 bytes of overhead. The MAC is compared with `digest::constant_time_eq`, so a wrong tag does not reveal how
//! many leading bytes matched. The token tags are this module's own framing, not a mechanism's.
//!
//! Confidentiality is not built. No cipher is implemented here, so a request for it is `GSS_S_UNAVAILABLE`, and every
//! unwrapped message reports `conf_state` false. The session key is supplied by the caller: no security context here
//! derives one, because no mechanism stands behind a context.
//!
//! Not here: sequence numbers and replay detection, and the release calls, which drop replaces.

use crate::digest;
use crate::gss_status::RoutineError;
use std::fmt;

/// The tag of a MIC token.
pub const TAG_MIC: u8 = b'M';
/// The tag of a wrapped message.
pub const TAG_WRAP: u8 = b'W';
/// The length of a MAC: HMAC-SHA256 output.
const MAC_LEN: usize = 32;
/// The length of a MIC token: the tag and the MAC.
pub const MIC_LEN: usize = 1 + MAC_LEN;
/// The bytes a wrapped message adds to its message: the tag and the MAC.
pub const WRAP_OVERHEAD: usize = 1 + MAC_LEN;

/// A session key for message protection. `Debug` shows its length, never its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct MessageKey {
    bytes: Vec<u8>,
}

impl MessageKey {
    /// A key from its secret bytes. An empty key is refused with `GSS_S_NO_CONTEXT`, since no context holds one.
    pub fn new(bytes: &[u8]) -> Result<MessageKey, RoutineError> {
        if bytes.is_empty() {
            Err(RoutineError::NoContext)
        } else {
            Ok(MessageKey { bytes: bytes.to_vec() })
        }
    }
}

impl fmt::Debug for MessageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MessageKey({} bytes)", self.bytes.len())
    }
}

fn mac(key: &MessageKey, message: &[u8]) -> [u8; MAC_LEN] {
    digest::hmac_sha256(&key.bytes, message)
}

/// Compares a received MAC with the MAC of `message`. A mismatch is `GSS_S_BAD_MIC`.
fn check(received: &[u8], key: &MessageKey, message: &[u8]) -> Result<(), RoutineError> {
    if digest::constant_time_eq(received, &mac(key, message)) {
        Ok(())
    } else {
        Err(RoutineError::BadMic)
    }
}

/// `gss_get_mic`: the MIC token for `message`.
pub fn get_mic(key: &MessageKey, message: &[u8]) -> Vec<u8> {
    let mut token = Vec::with_capacity(MIC_LEN);
    token.push(TAG_MIC);
    token.extend_from_slice(&mac(key, message));
    token
}

/// `gss_verify_mic`: `Ok` when `token` is the MIC of `message` under `key`. A token of the wrong shape is
/// `GSS_S_DEFECTIVE_TOKEN`; a MAC that does not match is `GSS_S_BAD_MIC`.
pub fn verify_mic(key: &MessageKey, message: &[u8], token: &[u8]) -> Result<(), RoutineError> {
    if token.len() != MIC_LEN || token[0] != TAG_MIC {
        return Err(RoutineError::DefectiveToken);
    }
    check(&token[1..], key, message)
}

/// `gss_wrap`: the message with its MAC in front, in the clear. A request for confidentiality is `GSS_S_UNAVAILABLE`.
pub fn wrap(key: &MessageKey, message: &[u8], conf_req: bool) -> Result<Vec<u8>, RoutineError> {
    if conf_req {
        return Err(RoutineError::Unavailable);
    }
    let mut token = Vec::with_capacity(WRAP_OVERHEAD + message.len());
    token.push(TAG_WRAP);
    token.extend_from_slice(&mac(key, message));
    token.extend_from_slice(message);
    Ok(token)
}

/// `gss_unwrap`: the message from a wrapped token, once its MAC checks out. The flag returned with it is `conf_state`,
/// which is always false because confidentiality is not built.
pub fn unwrap(key: &MessageKey, token: &[u8]) -> Result<(Vec<u8>, bool), RoutineError> {
    if token.len() < WRAP_OVERHEAD || token[0] != TAG_WRAP {
        return Err(RoutineError::DefectiveToken);
    }
    let message = &token[WRAP_OVERHEAD..];
    check(&token[1..WRAP_OVERHEAD], key, message)?;
    Ok((message.to_vec(), false))
}

/// `gss_wrap_size_limit`: the largest message whose wrapped token fits in `max_token` bytes. `None` when the token
/// cannot hold the overhead, or when confidentiality is requested, since it is not built.
pub fn wrap_size_limit(max_token: usize, conf_req: bool) -> Option<usize> {
    if conf_req {
        return None;
    }
    max_token.checked_sub(WRAP_OVERHEAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOX: &[u8] = b"The quick brown fox jumps over the lazy dog";

    fn key() -> MessageKey {
        MessageKey::new(b"key").unwrap()
    }

    #[test]
    fn the_mic_is_the_tag_followed_by_the_hmac_of_the_message() {
        // HMAC-SHA256 with key "key" over the fox sentence is the published value f7bc83f4...2d1a3cd8.
        let token = get_mic(&key(), FOX);
        assert_eq!(token.len(), 33);
        assert_eq!(token[0], b'M');
        let expected: [u8; 32] = [
            0xf7, 0xbc, 0x83, 0xf4, 0x30, 0x53, 0x84, 0x24, 0xb1, 0x32, 0x98, 0xe6, 0xaa, 0x6f, 0xb1, 0x43, 0xef, 0x4d,
            0x59, 0xa1, 0x49, 0x46, 0x17, 0x59, 0x97, 0x47, 0x9d, 0xbc, 0x2d, 0x1a, 0x3c, 0xd8,
        ];
        assert_eq!(&token[1..], &expected[..]);
    }

    #[test]
    fn a_mic_verifies_under_the_same_key_and_message() {
        let token = get_mic(&key(), FOX);
        assert_eq!(verify_mic(&key(), FOX, &token), Ok(()));
    }

    #[test]
    fn a_changed_message_fails_the_mic_check() {
        let token = get_mic(&key(), FOX);
        let changed = b"The quick brown fox jumps over the lazy cat";
        assert_eq!(verify_mic(&key(), changed, &token), Err(RoutineError::BadMic));
    }

    #[test]
    fn a_mic_from_another_key_fails_the_check() {
        let token = get_mic(&key(), FOX);
        let other = MessageKey::new(b"other").unwrap();
        assert_eq!(verify_mic(&other, FOX, &token), Err(RoutineError::BadMic));
    }

    #[test]
    fn a_wrapped_message_round_trips_with_33_bytes_of_overhead() {
        // The fox sentence is 43 bytes, so the wrapped token is 33 + 43 = 76 bytes.
        let token = wrap(&key(), FOX, false).unwrap();
        assert_eq!(token.len(), 76);
        assert_eq!(token[0], b'W');
        assert_eq!(&token[33..], FOX);
        assert_eq!(unwrap(&key(), &token), Ok((FOX.to_vec(), false)));
    }

    #[test]
    fn a_tampered_wrapped_message_fails_the_mic_check() {
        let mut token = wrap(&key(), FOX, false).unwrap();
        let last = token.len() - 1;
        token[last] ^= 0x01;
        assert_eq!(unwrap(&key(), &token), Err(RoutineError::BadMic));
    }

    #[test]
    fn confidentiality_is_refused_because_it_is_not_built() {
        assert_eq!(wrap(&key(), FOX, true), Err(RoutineError::Unavailable));
    }

    #[test]
    fn a_token_of_the_wrong_shape_is_defective() {
        let mic = get_mic(&key(), FOX);
        let wrapped = wrap(&key(), FOX, false).unwrap();
        // A MIC is not a wrapped message, and a wrapped message is not a MIC token.
        assert_eq!(unwrap(&key(), &mic), Err(RoutineError::DefectiveToken));
        assert_eq!(verify_mic(&key(), FOX, &wrapped), Err(RoutineError::DefectiveToken));
        // A wrapped token shorter than its overhead is defective, not an out-of-range read.
        assert_eq!(unwrap(&key(), &[b'W', 1, 2]), Err(RoutineError::DefectiveToken));
    }

    #[test]
    fn the_wrap_size_limit_subtracts_the_overhead_and_refuses_what_cannot_fit() {
        assert_eq!(wrap_size_limit(100, false), Some(67));
        assert_eq!(wrap_size_limit(33, false), Some(0));
        assert_eq!(wrap_size_limit(32, false), None);
        assert_eq!(wrap_size_limit(100, true), None);
    }

    #[test]
    fn an_empty_key_is_refused_and_a_key_never_prints_its_bytes() {
        assert_eq!(MessageKey::new(&[]), Err(RoutineError::NoContext));
        assert_eq!(format!("{:?}", key()), "MessageKey(3 bytes)");
    }
}

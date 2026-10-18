//! A secure channel between two peers that each hold a long-term X25519 key and know the other's public key (pinned at
//! pairing). The handshake is one ephemeral key each way. The traffic keys come from three Diffie-Hellman results
//! (ephemeral with ephemeral, and each side's static key with the other's ephemeral), so only a peer that holds the
//! pinned static key can finish it, and a later leak of a static key does not open traffic recorded before. After the
//! handshake each direction has its own ChaCha20-Poly1305 key and a message counter that is the nonce: a message that
//! is replayed, reordered or altered is refused, and a channel stops after `MAX_MESSAGES` per direction.
//!
//! Pure model: the caller supplies the random private keys and carries the 32-byte ephemeral public keys over any
//! transport. Nothing here reads a clock, a file or a socket.
//!
//! ```ignore
//! let (a, a_eph) = secure_channel::begin(Role::Initiator, &a_static, &b_static_public, &a_random)?;
//! let (b, b_eph) = secure_channel::begin(Role::Responder, &b_static, &a_static_public, &b_random)?;
//! let (mut a, mut b) = (a.finish(&b_eph)?, b.finish(&a_eph)?);
//! let wire = a.seal(b"hdr", b"hello")?;
//! assert_eq!(b.open(b"hdr", &wire)?, b"hello");
//! ```

use crate::x25519::{PrivateKey, PublicKey};
use bifrost_core::{chacha_poly, hkdf, sha256};

/// Bytes of the message counter in front of each sealed message.
pub const COUNTER_LEN: usize = 8;
/// What a sealed message adds to its plaintext: the counter and the authentication tag.
pub const OVERHEAD: usize = COUNTER_LEN + chacha_poly::TAG_LEN;
/// The most messages one direction seals before the channel must be set up again.
pub const MAX_MESSAGES: u64 = 1 << 32;

const LABEL: &[u8] = b"bifrost secure channel v1";
/// Two 32-byte traffic keys and a 16-byte channel binding.
const OKM_LEN: usize = 80;

/// Which side of the handshake a peer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The peer that sends its ephemeral key first.
    Initiator,
    /// The peer that answers.
    Responder,
}

/// Why a handshake or a message was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A key is not 32 bytes, or the agreement hit a low-order point.
    Key,
    /// The message does not authenticate: wrong key, altered bytes or different additional data.
    Open,
    /// The message is shorter than its counter and tag.
    Malformed,
    /// The message counter is behind the next expected one: a replay.
    Replay,
    /// The message counter is ahead of the next expected one: a message was lost or reordered.
    OutOfOrder,
    /// This direction has sealed `MAX_MESSAGES` messages.
    Exhausted,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::Key => "bad key",
            Error::Open => "message does not authenticate",
            Error::Malformed => "message too short",
            Error::Replay => "message replayed",
            Error::OutOfOrder => "message out of order",
            Error::Exhausted => "channel used up, set it up again",
        })
    }
}

impl std::error::Error for Error {}

/// A handshake that has sent its ephemeral key and waits for the peer's.
pub struct Pending {
    role: Role,
    static_private: PrivateKey,
    peer_static: PublicKey,
    ephemeral_private: PrivateKey,
}

/// Starts a handshake. `static_private` is this peer's long-term key, `peer_static_public` the pinned public key of the
/// peer it expects, and `ephemeral_private` fresh random bytes used once. Returns the pending handshake and the
/// ephemeral public key to send to the peer.
pub fn begin(role: Role, static_private: &[u8], peer_static_public: &[u8], ephemeral_private: &[u8]) -> Result<(Pending, [u8; 32]), Error> {
    let static_private = PrivateKey::from_raw(static_private).map_err(|_| Error::Key)?;
    let peer_static = PublicKey::from_raw(peer_static_public).map_err(|_| Error::Key)?;
    let ephemeral_private = PrivateKey::from_raw(ephemeral_private).map_err(|_| Error::Key)?;
    let public = ephemeral_private.public_key().raw();
    Ok((Pending { role, static_private, peer_static, ephemeral_private }, public))
}

impl Pending {
    /// Finishes the handshake with the peer's ephemeral public key and returns the channel.
    pub fn finish(self, peer_ephemeral_public: &[u8]) -> Result<Channel, Error> {
        let peer_eph = PublicKey::from_raw(peer_ephemeral_public).map_err(|_| Error::Key)?;
        let eph = &self.ephemeral_private;
        let stat = &self.static_private;
        let dh = |k: &PrivateKey, p: &PublicKey| k.shared_secret(p).map_err(|_| Error::Key);
        // The three results are the same on both sides: (e_i, e_r), (e_i, s_r) and (s_i, e_r).
        let (dh1, dh2, dh3) = match self.role {
            Role::Initiator => (dh(eph, &peer_eph)?, dh(eph, &self.peer_static)?, dh(stat, &peer_eph)?),
            Role::Responder => (dh(eph, &peer_eph)?, dh(stat, &peer_eph)?, dh(eph, &self.peer_static)?),
        };
        let my_static = stat.public_key().raw();
        let my_eph = eph.public_key().raw();
        let (s_i, s_r, e_i, e_r) = match self.role {
            Role::Initiator => (my_static, self.peer_static.raw(), my_eph, peer_eph.raw()),
            Role::Responder => (self.peer_static.raw(), my_static, peer_eph.raw(), my_eph),
        };
        let mut transcript = LABEL.to_vec();
        for part in [&s_i, &s_r, &e_i, &e_r] {
            transcript.extend_from_slice(part);
        }
        let salt = sha256::digest(&transcript);
        let mut ikm = Vec::with_capacity(96);
        for part in [&dh1, &dh2, &dh3] {
            ikm.extend_from_slice(part.bytes());
        }
        let okm = hkdf::hkdf_sha256(&ikm, &salt, LABEL, OKM_LEN).ok_or(Error::Key)?;
        ikm.iter_mut().for_each(|b| *b = 0);
        let mut initiator_to_responder = [0u8; 32];
        let mut responder_to_initiator = [0u8; 32];
        let mut binding = [0u8; 16];
        initiator_to_responder.copy_from_slice(&okm[..32]);
        responder_to_initiator.copy_from_slice(&okm[32..64]);
        binding.copy_from_slice(&okm[64..80]);
        let (send_key, recv_key) = match self.role {
            Role::Initiator => (initiator_to_responder, responder_to_initiator),
            Role::Responder => (responder_to_initiator, initiator_to_responder),
        };
        Ok(Channel { send_key, recv_key, send_count: 0, recv_count: 0, binding })
    }
}

/// An established channel. It is not `Clone`: two copies would reuse nonces.
pub struct Channel {
    send_key: [u8; 32],
    recv_key: [u8; 32],
    send_count: u64,
    recv_count: u64,
    binding: [u8; 16],
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Channel(sent {}, received {})", self.send_count, self.recv_count)
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.send_key = [0; 32];
        self.recv_key = [0; 32];
    }
}

fn nonce(counter: u64) -> [u8; chacha_poly::NONCE_LEN] {
    let mut n = [0u8; chacha_poly::NONCE_LEN];
    n[4..].copy_from_slice(&counter.to_be_bytes());
    n
}

impl Channel {
    /// Seals `plaintext`, bound to `aad`. The result is the counter, the ciphertext and the tag.
    pub fn seal(&mut self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        if self.send_count >= MAX_MESSAGES {
            return Err(Error::Exhausted);
        }
        let counter = self.send_count;
        let sealed = chacha_poly::seal(&self.send_key, &nonce(counter), aad, plaintext).map_err(|_| Error::Open)?;
        self.send_count += 1;
        let mut out = counter.to_be_bytes().to_vec();
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Opens the next message. A message that fails to open, or has the wrong counter, changes nothing.
    pub fn open(&mut self, aad: &[u8], message: &[u8]) -> Result<Vec<u8>, Error> {
        if message.len() < OVERHEAD {
            return Err(Error::Malformed);
        }
        let mut raw = [0u8; COUNTER_LEN];
        raw.copy_from_slice(&message[..COUNTER_LEN]);
        let counter = u64::from_be_bytes(raw);
        if counter < self.recv_count {
            return Err(Error::Replay);
        }
        if counter > self.recv_count {
            return Err(Error::OutOfOrder);
        }
        let plain = chacha_poly::open(&self.recv_key, &nonce(counter), aad, &message[COUNTER_LEN..]).map_err(|_| Error::Open)?;
        self.recv_count += 1;
        Ok(plain)
    }

    /// Bytes both peers derive only when they saw the same keys and ephemeral keys. Compare them out of band (or have
    /// each peer seal them) to confirm there is no one in the middle.
    pub fn binding(&self) -> [u8; 16] {
        self.binding
    }

    /// How many messages this side has sealed and opened.
    pub fn counts(&self) -> (u64, u64) {
        (self.send_count, self.recv_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_STATIC: [u8; 32] = [0x11; 32];
    const B_STATIC: [u8; 32] = [0x22; 32];
    const A_EPH: [u8; 32] = [0x33; 32];
    const B_EPH: [u8; 32] = [0x44; 32];

    fn public(private: &[u8; 32]) -> [u8; 32] {
        PrivateKey::from_raw(private).unwrap().public_key().raw()
    }

    fn pair() -> (Channel, Channel) {
        let (a, a_pub) = begin(Role::Initiator, &A_STATIC, &public(&B_STATIC), &A_EPH).unwrap();
        let (b, b_pub) = begin(Role::Responder, &B_STATIC, &public(&A_STATIC), &B_EPH).unwrap();
        (a.finish(&b_pub).unwrap(), b.finish(&a_pub).unwrap())
    }

    #[test]
    fn both_directions_round_trip_and_the_bindings_match() {
        let (mut a, mut b) = pair();
        assert_eq!(a.binding(), b.binding());
        let to_b = a.seal(b"hdr", b"hello b").unwrap();
        assert_eq!(to_b.len(), b"hello b".len() + OVERHEAD);
        assert_eq!(b.open(b"hdr", &to_b).unwrap(), b"hello b");
        let to_a = b.seal(b"", b"hello a").unwrap();
        assert_eq!(a.open(b"", &to_a).unwrap(), b"hello a");
        assert_eq!(a.counts(), (1, 1));
    }

    #[test]
    fn the_two_directions_use_different_keys() {
        let (mut a, _b) = pair();
        let own = a.seal(b"", b"x").unwrap();
        // a message a peer sent cannot be turned back on the sender: a opens nothing it sealed itself
        assert_eq!(a.open(b"", &own), Err(Error::Open));
    }

    #[test]
    fn a_replayed_message_is_refused() {
        let (mut a, mut b) = pair();
        let m = a.seal(b"", b"once").unwrap();
        assert!(b.open(b"", &m).is_ok());
        assert_eq!(b.open(b"", &m), Err(Error::Replay));
    }

    #[test]
    fn a_skipped_message_is_out_of_order_and_changes_nothing() {
        let (mut a, mut b) = pair();
        let first = a.seal(b"", b"1").unwrap();
        let second = a.seal(b"", b"2").unwrap();
        assert_eq!(b.open(b"", &second), Err(Error::OutOfOrder));
        assert_eq!(b.open(b"", &first).unwrap(), b"1");
        assert_eq!(b.open(b"", &second).unwrap(), b"2");
    }

    #[test]
    fn altered_bytes_or_other_additional_data_do_not_open() {
        let (mut a, mut b) = pair();
        let m = a.seal(b"aad", b"secret").unwrap();
        let mut bad = m.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(b.open(b"aad", &bad), Err(Error::Open));
        assert_eq!(b.open(b"other", &m), Err(Error::Open));
        // the failed attempts did not move the counter
        assert_eq!(b.open(b"aad", &m).unwrap(), b"secret");
        assert_eq!(b.open(b"", &[0u8; 5]), Err(Error::Malformed));
    }

    #[test]
    fn a_peer_with_the_wrong_static_key_cannot_talk() {
        let (a, a_pub) = begin(Role::Initiator, &A_STATIC, &public(&B_STATIC), &A_EPH).unwrap();
        // an impostor holds another static key but answers as B
        let (m, m_pub) = begin(Role::Responder, &[0x99; 32], &public(&A_STATIC), &B_EPH).unwrap();
        let mut a = a.finish(&m_pub).unwrap();
        let mut m = m.finish(&a_pub).unwrap();
        assert_ne!(a.binding(), m.binding());
        let wire = a.seal(b"", b"for b only").unwrap();
        assert_eq!(m.open(b"", &wire), Err(Error::Open));
    }

    #[test]
    fn keys_of_the_wrong_size_or_low_order_are_refused() {
        assert_eq!(begin(Role::Initiator, &[1; 5], &public(&B_STATIC), &A_EPH).err(), Some(Error::Key));
        assert_eq!(begin(Role::Initiator, &A_STATIC, &[1; 31], &A_EPH).err(), Some(Error::Key));
        assert_eq!(begin(Role::Initiator, &A_STATIC, &public(&B_STATIC), &[]).err(), Some(Error::Key));
        let (a, _) = begin(Role::Initiator, &A_STATIC, &public(&B_STATIC), &A_EPH).unwrap();
        // the all-zero point has small order, so the agreement is refused
        assert_eq!(a.finish(&[0u8; 32]).err(), Some(Error::Key));
    }

    #[test]
    fn a_direction_stops_at_the_message_limit() {
        let (mut a, _b) = pair();
        a.send_count = MAX_MESSAGES;
        assert_eq!(a.seal(b"", b"x"), Err(Error::Exhausted));
    }
}

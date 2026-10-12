//! GSS-API context services (`gss_init_sec_context`, `gss_accept_sec_context`, `gss_context_time`,
//! `gss_inquire_context`, `gss_delete_sec_context`): the two-token exchange that gives an initiator and an acceptor a
//! shared security context with agreed flags and a lifetime.
//!
//! The initiator sends an initial context token: the mechanism identifier, framed by `gss_token`, wraps an inner token
//! of nine bytes: the tag `'I'`, the requested flags, and the requested lifetime, each a 32-bit big-endian integer. The
//! acceptor answers with a bare nine-byte inner token: the tag `'A'`, the granted flags and the granted lifetime. The
//! granted flags are the requested flags the acceptor offers. The granted lifetime is the shorter of the requested
//! lifetime and the acceptor's cap, and `GSS_C_INDEFINITE` (all ones) means no limit.
//!
//! No mechanism defines the inner token; this is the model's own framing, so it talks only to another instance of this
//! module. The flag values are the RFC 2744 values, the convention `gss_status` uses, because the reference page names
//! the flags and does not print their values. Only the core flags are modelled; `GSS_C_DCE_STYLE`, `GSS_C_IDENTIFY_FLAG`,
//! `GSS_C_EXTENDED_ERROR_FLAG` and `GSS_C_DELEG_POLICY_FLAG` are not.
//!
//! No credential is checked, because no mechanism exists to verify one. The release call, `gss_delete_sec_context`, is
//! replaced by drop.

use crate::gss_credential::INDEFINITE;
use crate::gss_oid::{Oid, OidSet};
use crate::gss_status::RoutineError;
use crate::gss_token;

/// A set of context flags (`OM_uint32` request and result flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Flags(u32);

impl Flags {
    /// `GSS_C_DELEG_FLAG`: the acceptor may take the initiator's credentials.
    pub const DELEG: Flags = Flags(1);
    /// `GSS_C_MUTUAL_FLAG`: both ends authenticate.
    pub const MUTUAL: Flags = Flags(2);
    /// `GSS_C_REPLAY_FLAG`: repeated messages are detected.
    pub const REPLAY: Flags = Flags(4);
    /// `GSS_C_SEQUENCE_FLAG`: out-of-sequence messages are detected.
    pub const SEQUENCE: Flags = Flags(8);
    /// `GSS_C_CONF_FLAG`: confidentiality is available for messages.
    pub const CONF: Flags = Flags(16);
    /// `GSS_C_INTEG_FLAG`: integrity is available for messages.
    pub const INTEG: Flags = Flags(32);
    /// `GSS_C_ANON_FLAG`: the initiator stays anonymous to the acceptor.
    pub const ANON: Flags = Flags(64);
    /// `GSS_C_PROT_READY_FLAG`: confidentiality and integrity are ready before the exchange completes.
    pub const PROT_READY: Flags = Flags(128);
    /// `GSS_C_TRANS_FLAG`: the context can be exported to another process on the same machine.
    pub const TRANS: Flags = Flags(256);

    /// The bits this model knows. A flag word with any other bit set is refused.
    const KNOWN: u32 = 0x1FF;

    /// No flags.
    pub fn none() -> Flags {
        Flags(0)
    }

    /// The flags for a wire value, or `None` when it sets a bit this model does not know.
    pub fn from_wire(bits: u32) -> Option<Flags> {
        if bits & !Flags::KNOWN == 0 {
            Some(Flags(bits))
        } else {
            None
        }
    }

    /// The wire value.
    pub fn bits(self) -> u32 {
        self.0
    }

    /// True when every flag in `other` is set here.
    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// The flags set in either.
    pub fn union(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    /// The flags set in both.
    pub fn intersection(self, other: Flags) -> Flags {
        Flags(self.0 & other.0)
    }
}

/// The tag of the initiator's request inner token.
const TAG_REQUEST: u8 = b'I';
/// The tag of the acceptor's answer inner token.
const TAG_ANSWER: u8 = b'A';
/// The length of a request or answer inner token: tag, flags, lifetime.
const INNER_LEN: usize = 9;

/// The lifecycle of one side of a context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// The initiator has not sent its request yet.
    Start { requested: Flags, lifetime: Option<u32> },
    /// The initiator has sent its request and waits for the answer.
    Pending { requested: Flags },
    /// The context is open: the agreed flags and the absolute expiry in seconds, or `None` for no limit.
    Established { flags: Flags, expires_at: Option<u64> },
}

/// One side of a security context: the mechanism it uses and where it is in the exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    mech: Oid,
    state: State,
}

fn wire_lifetime(lifetime: Option<u32>) -> u32 {
    lifetime.unwrap_or(INDEFINITE)
}

fn from_wire_lifetime(value: u32) -> Option<u32> {
    if value == INDEFINITE {
        None
    } else {
        Some(value)
    }
}

/// The shorter of the requested lifetime and the acceptor's cap. `None` (no limit) defers to the other side.
fn granted_lifetime(requested: Option<u32>, cap: Option<u32>) -> Option<u32> {
    match (requested, cap) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// The absolute expiry for a lifetime that starts at `now`.
fn expiry(now: u64, lifetime: Option<u32>) -> Option<u64> {
    lifetime.map(|secs| now + u64::from(secs))
}

fn encode(tag: u8, flags: Flags, lifetime: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(INNER_LEN);
    out.push(tag);
    out.extend_from_slice(&flags.bits().to_be_bytes());
    out.extend_from_slice(&lifetime.to_be_bytes());
    out
}

/// Reads a request or answer inner token. Any length, tag, or unknown flag bit is `GSS_S_DEFECTIVE_TOKEN`.
fn decode(tag: u8, bytes: &[u8]) -> Result<(Flags, u32), RoutineError> {
    if bytes.len() != INNER_LEN || bytes[0] != tag {
        return Err(RoutineError::DefectiveToken);
    }
    let flags = Flags::from_wire(u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]))
        .ok_or(RoutineError::DefectiveToken)?;
    let lifetime = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
    Ok((flags, lifetime))
}

impl Context {
    /// `gss_init_sec_context` before its first call: an initiator that will ask `mech` for `requested` flags and
    /// `lifetime` seconds (`None` for no limit).
    pub fn initiate(mech: Oid, requested: Flags, lifetime: Option<u32>) -> Context {
        Context { mech, state: State::Start { requested, lifetime } }
    }

    /// The initial context token to send to the acceptor. Only the first call succeeds; a second call, or a call after
    /// the context is open, is `GSS_S_FAILURE`.
    pub fn init_token(&mut self) -> Result<Vec<u8>, RoutineError> {
        let (requested, lifetime) = match self.state {
            State::Start { requested, lifetime } => (requested, lifetime),
            _ => return Err(RoutineError::Failure),
        };
        let inner = encode(TAG_REQUEST, requested, wire_lifetime(lifetime));
        self.state = State::Pending { requested };
        Ok(gss_token::encapsulate(&self.mech, &inner))
    }

    /// Reads the acceptor's answer and opens the context. `now` is the initiator's clock in seconds. An answer that
    /// grants a flag the initiator did not request is `GSS_S_DEFECTIVE_TOKEN`, and the context stays pending.
    pub fn complete(&mut self, answer: &[u8], now: u64) -> Result<(), RoutineError> {
        let requested = match self.state {
            State::Pending { requested } => requested,
            _ => return Err(RoutineError::Failure),
        };
        let (granted, wire) = decode(TAG_ANSWER, answer)?;
        if !requested.contains(granted) {
            return Err(RoutineError::DefectiveToken);
        }
        self.state = State::Established {
            flags: granted,
            expires_at: expiry(now, from_wire_lifetime(wire)),
        };
        Ok(())
    }

    /// `gss_accept_sec_context`: answers an initial context token and opens the acceptor's side. `supported` lists
    /// the mechanisms this acceptor can use, `offered` the flags it allows, and `max_lifetime` its cap in seconds
    /// (`None` for no cap). Returns the open context and the answer token to send back. An unsupported mechanism is
    /// `GSS_S_BAD_MECH`; a token that does not decode is `GSS_S_DEFECTIVE_TOKEN`.
    pub fn accept(
        supported: &OidSet,
        offered: Flags,
        max_lifetime: Option<u32>,
        token: &[u8],
        now: u64,
    ) -> Result<(Context, Vec<u8>), RoutineError> {
        let (mech, inner) = gss_token::decapsulate(token).map_err(|e| e.routine_error())?;
        if !supported.contains(&mech) {
            return Err(RoutineError::BadMech);
        }
        let (requested, wire) = decode(TAG_REQUEST, inner)?;
        let granted = requested.intersection(offered);
        let lifetime = granted_lifetime(from_wire_lifetime(wire), max_lifetime);
        let answer = encode(TAG_ANSWER, granted, wire_lifetime(lifetime));
        let context = Context {
            mech,
            state: State::Established { flags: granted, expires_at: expiry(now, lifetime) },
        };
        Ok((context, answer))
    }

    /// The mechanism this context uses.
    pub fn mech(&self) -> &Oid {
        &self.mech
    }

    /// The agreed flags of an open context (`gss_inquire_context`), or `None` before the exchange completes.
    pub fn flags(&self) -> Option<Flags> {
        match self.state {
            State::Established { flags, .. } => Some(flags),
            _ => None,
        }
    }

    /// True once the exchange has completed, whether or not the context has since expired.
    pub fn is_established(&self) -> bool {
        matches!(self.state, State::Established { .. })
    }

    /// `gss_context_time`: the seconds left at `now`. A context with no limit reports `GSS_C_INDEFINITE`. Once `now`
    /// reaches the expiry the context is `GSS_S_CONTEXT_EXPIRED`. A context that is not open is `GSS_S_NO_CONTEXT`.
    pub fn context_time(&self, now: u64) -> Result<u32, RoutineError> {
        match self.state {
            State::Established { expires_at: None, .. } => Ok(INDEFINITE),
            State::Established { expires_at: Some(t), .. } => {
                if now >= t {
                    Err(RoutineError::ContextExpired)
                } else {
                    Ok(u32::try_from(t - now).unwrap_or(INDEFINITE - 1))
                }
            }
            _ => Err(RoutineError::NoContext),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kerberos() -> Oid {
        Oid::parse("1.2.840.113554.1.2.2").unwrap()
    }

    fn supported() -> OidSet {
        let mut set = OidSet::new();
        set.add(kerberos());
        set
    }

    #[test]
    fn the_initial_token_has_the_worked_bytes() {
        // Flags MUTUAL | INTEG = 2 | 32 = 0x22. Lifetime 3600 = 0x0E10. Inner token: 9 bytes. The mechanism element
        // is 11 bytes, so the body is 11 + 9 = 20 = 0x14, and the whole token is 2 + 20 = 22 bytes.
        let mut ctx = Context::initiate(kerberos(), Flags::MUTUAL.union(Flags::INTEG), Some(3600));
        let token = ctx.init_token().unwrap();
        assert_eq!(
            token,
            vec![
                0x60, 0x14, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x12, 0x01, 0x02, 0x02, 0x49, 0x00, 0x00,
                0x00, 0x22, 0x00, 0x00, 0x0E, 0x10
            ]
        );
    }

    #[test]
    fn a_handshake_grants_the_shared_flags_and_the_shorter_lifetime() {
        // The initiator asks for MUTUAL | REPLAY (6) for 3600 s. The acceptor offers MUTUAL only, with a 600 s cap.
        // Granted: 6 & 2 = 2. Lifetime: min(3600, 600) = 600. Answer: 'A', 00000002, 00000258.
        let mut initiator = Context::initiate(kerberos(), Flags::MUTUAL.union(Flags::REPLAY), Some(3600));
        let token = initiator.init_token().unwrap();
        let (acceptor, answer) = Context::accept(&supported(), Flags::MUTUAL, Some(600), &token, 1000).unwrap();
        assert_eq!(answer, vec![0x41, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x02, 0x58]);
        assert_eq!(acceptor.flags(), Some(Flags::MUTUAL));
        assert_eq!(acceptor.context_time(1000), Ok(600));

        initiator.complete(&answer, 1000).unwrap();
        assert_eq!(initiator.flags(), Some(Flags::MUTUAL));
        assert_eq!(initiator.context_time(1000), Ok(600));
        assert_eq!(initiator.context_time(1599), Ok(1));
        // The expiry is 1000 + 600 = 1600, and a context is expired at that instant.
        assert_eq!(initiator.context_time(1600), Err(RoutineError::ContextExpired));
    }

    #[test]
    fn a_context_with_no_limit_reports_indefinite_and_never_expires() {
        let mut initiator = Context::initiate(kerberos(), Flags::none(), None);
        let token = initiator.init_token().unwrap();
        let (_, answer) = Context::accept(&supported(), Flags::none(), None, &token, 5).unwrap();
        assert_eq!(answer, vec![0x41, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]);
        initiator.complete(&answer, 5).unwrap();
        assert_eq!(initiator.context_time(1_000_000_000_000), Ok(INDEFINITE));
    }

    #[test]
    fn an_unsupported_mechanism_is_refused_by_the_acceptor() {
        let mut initiator = Context::initiate(kerberos(), Flags::MUTUAL, Some(60));
        let token = initiator.init_token().unwrap();
        let result = Context::accept(&OidSet::new(), Flags::MUTUAL, None, &token, 0);
        assert_eq!(result.err(), Some(RoutineError::BadMech));
    }

    #[test]
    fn the_steps_must_run_in_order() {
        let mut initiator = Context::initiate(kerberos(), Flags::MUTUAL, Some(60));
        // Completing before the request is sent is refused, and so is a context that is not open yet.
        assert_eq!(initiator.complete(&[0x41, 0, 0, 0, 0, 0, 0, 0, 60], 0), Err(RoutineError::Failure));
        assert_eq!(initiator.context_time(0), Err(RoutineError::NoContext));
        initiator.init_token().unwrap();
        // A second request is refused.
        assert_eq!(initiator.init_token(), Err(RoutineError::Failure));
    }

    #[test]
    fn an_answer_that_grants_an_unrequested_flag_is_refused_and_the_context_stays_pending() {
        let mut initiator = Context::initiate(kerberos(), Flags::MUTUAL, Some(60));
        initiator.init_token().unwrap();
        // ANON (64) was never requested.
        let bad: [u8; 9] = [0x41, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x3C];
        assert_eq!(initiator.complete(&bad, 0), Err(RoutineError::DefectiveToken));
        assert!(!initiator.is_established());
        // A good answer still completes the exchange.
        let good: [u8; 9] = [0x41, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x3C];
        initiator.complete(&good, 0).unwrap();
        assert_eq!(initiator.flags(), Some(Flags::MUTUAL));
    }

    #[test]
    fn a_request_with_an_unknown_flag_bit_is_defective() {
        // Bit 0x400 is not a flag this model knows.
        let inner: [u8; 9] = [0x49, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x3C];
        let token = gss_token::encapsulate(&kerberos(), &inner);
        let result = Context::accept(&supported(), Flags::MUTUAL, None, &token, 0);
        assert_eq!(result.err(), Some(RoutineError::DefectiveToken));
    }
}

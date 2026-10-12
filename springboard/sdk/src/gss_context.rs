//! GSS-API context services (`gss_init_sec_context`, `gss_accept_sec_context`, `gss_delete_sec_context`, and the
//! context flags): the state of one security context between an initiator and an acceptor, and the flag sets each
//! side asks for and gets back.
//!
//! The reference says that requesting a service does not guarantee it. The initiator asks for flags, and the
//! acceptor and the initiator both get back the flags that are actually engaged. This model keeps the two apart:
//! [`SecContext::requested`] is what was asked, [`SecContext::engaged`] is what the mechanism reports, and
//! [`SecContext::missing`] is the difference. A caller that needs a service sets it as *required*, and the context
//! fails the handshake when the mechanism does not engage it.
//!
//! The handshake itself belongs to a mechanism. [`Handshake`] is the trait a mechanism implements: given an input
//! token, it returns the next token to send, or the final token together with the flags and lifetime it engaged.
//! [`SecContext::step`] drives it and enforces the order of the exchange. No mechanism is registered in the shell
//! yet, so no context completes in a running app.
//!
//! Flag values are the bit values of the GSS-API specification (RFC 2744), which the reference lists by name only.

use crate::gss_name::Name;
use crate::gss_oid::Oid;
use crate::gss_status::RoutineError;
use std::fmt;

/// A set of context flags (`OM_uint32` bits such as `GSS_C_MUTUAL_FLAG`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContextFlags(u32);

/// Each named flag with its bit, in the order `names` reports them.
const NAMED: [(&str, u32); 13] = [
    ("GSS_C_DELEG_FLAG", 1),
    ("GSS_C_MUTUAL_FLAG", 2),
    ("GSS_C_REPLAY_FLAG", 4),
    ("GSS_C_SEQUENCE_FLAG", 8),
    ("GSS_C_CONF_FLAG", 16),
    ("GSS_C_INTEG_FLAG", 32),
    ("GSS_C_ANON_FLAG", 64),
    ("GSS_C_PROT_READY_FLAG", 128),
    ("GSS_C_TRANS_FLAG", 256),
    ("GSS_C_DCE_STYLE", 4096),
    ("GSS_C_IDENTIFY_FLAG", 8192),
    ("GSS_C_EXTENDED_ERROR_FLAG", 16384),
    ("GSS_C_DELEG_POLICY_FLAG", 32768),
];

/// The bits of every named flag.
const KNOWN_BITS: u32 = 1 | 2 | 4 | 8 | 16 | 32 | 64 | 128 | 256 | 4096 | 8192 | 16384 | 32768;

impl ContextFlags {
    pub const NONE: ContextFlags = ContextFlags(0);
    pub const DELEG: ContextFlags = ContextFlags(1);
    pub const MUTUAL: ContextFlags = ContextFlags(2);
    pub const REPLAY: ContextFlags = ContextFlags(4);
    pub const SEQUENCE: ContextFlags = ContextFlags(8);
    pub const CONF: ContextFlags = ContextFlags(16);
    pub const INTEG: ContextFlags = ContextFlags(32);
    pub const ANON: ContextFlags = ContextFlags(64);
    pub const PROT_READY: ContextFlags = ContextFlags(128);
    pub const TRANS: ContextFlags = ContextFlags(256);
    pub const DCE_STYLE: ContextFlags = ContextFlags(4096);
    pub const IDENTIFY: ContextFlags = ContextFlags(8192);
    pub const EXTENDED_ERROR: ContextFlags = ContextFlags(16384);
    pub const DELEG_POLICY: ContextFlags = ContextFlags(32768);

    /// The raw bits.
    pub fn bits(self) -> u32 {
        self.0
    }

    /// The flags for these bits, or `None` when a bit is not a named flag.
    pub fn from_bits(bits: u32) -> Option<ContextFlags> {
        if bits & !KNOWN_BITS == 0 {
            Some(ContextFlags(bits))
        } else {
            None
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every flag in `other` is set here.
    pub fn contains(self, other: ContextFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: ContextFlags) -> ContextFlags {
        ContextFlags(self.0 | other.0)
    }

    pub fn intersection(self, other: ContextFlags) -> ContextFlags {
        ContextFlags(self.0 & other.0)
    }

    /// The flags set here and not in `other`.
    pub fn difference(self, other: ContextFlags) -> ContextFlags {
        ContextFlags(self.0 & !other.0)
    }

    /// The reference names of the set flags, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        NAMED.iter().filter(|(_, bit)| self.0 & bit != 0).map(|(name, _)| *name).collect()
    }
}

/// Who holds the context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Acceptor,
}

/// Where the exchange stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No token has been exchanged yet.
    Start,
    /// At least one token was sent, and the handshake is not finished.
    Negotiating,
    /// The handshake finished and the required flags are engaged. The context is usable.
    Established,
    /// The mechanism refused a token, or a required flag was not engaged. Nothing more can be done with it.
    Failed,
}

/// Why a step failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextError {
    /// The context is already established or has failed.
    AlreadyFinished,
    /// The exchange needs a peer token and none was given.
    MissingToken,
    /// The initiator's first step takes no input token.
    UnexpectedToken,
    /// The mechanism refused the token.
    Mechanism(RoutineError),
    /// The handshake finished without engaging these required flags.
    RequiredFlagMissing(ContextFlags),
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::AlreadyFinished => f.write_str("security context is already finished"),
            ContextError::MissingToken => f.write_str("security context needs a token from the peer"),
            ContextError::UnexpectedToken => f.write_str("initiator's first step takes no input token"),
            ContextError::Mechanism(e) => write!(f, "mechanism refused the token: {:?}", e),
            ContextError::RequiredFlagMissing(flags) => {
                write!(f, "required context flags not engaged: {}", flags.names().join(", "))
            }
        }
    }
}

impl std::error::Error for ContextError {}

/// What a mechanism answers to one input token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Send this token and wait for the peer's answer.
    Continue(Vec<u8>),
    /// The handshake is done. `token` is an optional final token to send, `engaged` the flags in force, and
    /// `lifetime` the seconds the context lasts (`None` for an indefinite lifetime).
    Complete { token: Option<Vec<u8>>, engaged: ContextFlags, lifetime: Option<u32> },
}

/// The mechanism side of a handshake. A mechanism keeps its own key and nonce state and answers each token.
pub trait Handshake {
    /// `requested` is the flag set the initiator asked for (empty for an acceptor, which learns it from the token).
    fn step(&mut self, role: Role, requested: ContextFlags, input: Option<&[u8]>) -> Result<Step, RoutineError>;
}

/// What one successful step produced: the token to send, if any, and whether the context is now established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub token: Option<Vec<u8>>,
    pub complete: bool,
}

/// One security context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecContext {
    role: Role,
    mech: Oid,
    target: Option<Name>,
    requested: ContextFlags,
    required: ContextFlags,
    state: State,
    engaged: ContextFlags,
    lifetime: Option<u32>,
}

impl SecContext {
    /// `gss_init_sec_context` side: a context toward `target`, asking for `requested`. Each flag in `required` must be
    /// engaged by the time the handshake completes.
    pub fn initiate(target: Name, mech: Oid, requested: ContextFlags, required: ContextFlags) -> SecContext {
        SecContext {
            role: Role::Initiator,
            mech,
            target: Some(target),
            requested,
            required,
            state: State::Start,
            engaged: ContextFlags::NONE,
            lifetime: None,
        }
    }

    /// `gss_accept_sec_context` side. The acceptor asks for nothing; the initiator's token says what it wants.
    pub fn accept(mech: Oid, required: ContextFlags) -> SecContext {
        SecContext {
            role: Role::Acceptor,
            mech,
            target: None,
            requested: ContextFlags::NONE,
            required,
            state: State::Start,
            engaged: ContextFlags::NONE,
            lifetime: None,
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn mech(&self) -> &Oid {
        &self.mech
    }

    /// The name the initiator asked for. `None` on the acceptor side.
    pub fn target(&self) -> Option<&Name> {
        self.target.as_ref()
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn is_established(&self) -> bool {
        self.state == State::Established
    }

    /// The flags the initiator asked for.
    pub fn requested(&self) -> ContextFlags {
        self.requested
    }

    /// The flags the mechanism engaged. Empty until the context is established.
    pub fn engaged(&self) -> ContextFlags {
        self.engaged
    }

    /// Flags that were requested and are not engaged. Only meaningful once the context is established.
    pub fn missing(&self) -> ContextFlags {
        self.requested.difference(self.engaged)
    }

    /// Seconds the context lasts, or `None` for an indefinite lifetime. Set when the context is established.
    pub fn lifetime(&self) -> Option<u32> {
        self.lifetime
    }

    /// One exchange step. `input` is the peer's last token; the first step of an acceptor needs one and the first
    /// step of an initiator takes none. A refused token or a missing required flag moves the context to `Failed`.
    pub fn step(&mut self, mechanism: &mut dyn Handshake, input: Option<&[u8]>) -> Result<Progress, ContextError> {
        match self.state {
            State::Established | State::Failed => return Err(ContextError::AlreadyFinished),
            State::Start => {
                let needs_input = self.role == Role::Acceptor;
                if needs_input && input.is_none() {
                    return Err(ContextError::MissingToken);
                }
                if !needs_input && input.is_some() {
                    return Err(ContextError::UnexpectedToken);
                }
            }
            State::Negotiating => {
                if input.is_none() {
                    return Err(ContextError::MissingToken);
                }
            }
        }
        match mechanism.step(self.role, self.requested, input) {
            Err(e) => {
                self.state = State::Failed;
                Err(ContextError::Mechanism(e))
            }
            Ok(Step::Continue(token)) => {
                self.state = State::Negotiating;
                Ok(Progress { token: Some(token), complete: false })
            }
            Ok(Step::Complete { token, engaged, lifetime }) => {
                let missing = self.required.difference(engaged);
                if !missing.is_empty() {
                    self.state = State::Failed;
                    return Err(ContextError::RequiredFlagMissing(missing));
                }
                self.state = State::Established;
                self.engaged = engaged;
                self.lifetime = lifetime;
                Ok(Progress { token, complete: true })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A mechanism that answers from a fixed script, one entry per step.
    struct Script {
        answers: VecDeque<Result<Step, RoutineError>>,
        seen: Vec<(Role, ContextFlags, Option<Vec<u8>>)>,
    }

    impl Script {
        fn new(answers: Vec<Result<Step, RoutineError>>) -> Script {
            Script { answers: answers.into(), seen: Vec::new() }
        }
    }

    impl Handshake for Script {
        fn step(&mut self, role: Role, requested: ContextFlags, input: Option<&[u8]>) -> Result<Step, RoutineError> {
            self.seen.push((role, requested, input.map(|b| b.to_vec())));
            self.answers.pop_front().expect("script ran out")
        }
    }

    fn mech() -> Oid {
        Oid::from_arcs(&[1, 2, 840, 113554, 1, 2, 2]).unwrap()
    }

    fn target() -> Name {
        Name::import(b"host@example", crate::gss_name::NameType::HostBasedService).unwrap()
    }

    #[test]
    fn flag_bits_match_the_specification_values() {
        assert_eq!(ContextFlags::DELEG.bits(), 1);
        assert_eq!(ContextFlags::MUTUAL.bits(), 2);
        assert_eq!(ContextFlags::REPLAY.bits(), 4);
        assert_eq!(ContextFlags::INTEG.bits(), 32);
        assert_eq!(ContextFlags::DELEG_POLICY.bits(), 32768);
        assert_eq!(ContextFlags::from_bits(3), Some(ContextFlags::DELEG.union(ContextFlags::MUTUAL)));
        assert_eq!(ContextFlags::from_bits(1 << 20), None);
    }

    #[test]
    fn set_operations_and_names_follow_bit_order() {
        let asked = ContextFlags::MUTUAL.union(ContextFlags::DELEG).union(ContextFlags::REPLAY);
        let got = ContextFlags::MUTUAL.union(ContextFlags::CONF);
        assert_eq!(asked.intersection(got), ContextFlags::MUTUAL);
        assert_eq!(asked.difference(got), ContextFlags::DELEG.union(ContextFlags::REPLAY));
        assert!(asked.contains(ContextFlags::MUTUAL));
        assert!(!got.contains(ContextFlags::DELEG));
        assert_eq!(asked.names(), vec!["GSS_C_DELEG_FLAG", "GSS_C_MUTUAL_FLAG", "GSS_C_REPLAY_FLAG"]);
        assert!(ContextFlags::NONE.is_empty());
    }

    #[test]
    fn two_step_handshake_completes_and_reports_what_was_not_engaged() {
        let mut ctx = SecContext::initiate(target(), mech(), ContextFlags::MUTUAL.union(ContextFlags::REPLAY), ContextFlags::MUTUAL);
        let mut m = Script::new(vec![
            Ok(Step::Continue(b"hello".to_vec())),
            Ok(Step::Complete { token: None, engaged: ContextFlags::MUTUAL, lifetime: Some(3600) }),
        ]);
        let first = ctx.step(&mut m, None).unwrap();
        assert_eq!(first, Progress { token: Some(b"hello".to_vec()), complete: false });
        assert_eq!(ctx.state(), State::Negotiating);
        let second = ctx.step(&mut m, Some(b"reply")).unwrap();
        assert_eq!(second, Progress { token: None, complete: true });
        assert!(ctx.is_established());
        assert_eq!(ctx.lifetime(), Some(3600));
        assert_eq!(ctx.missing(), ContextFlags::REPLAY);
        assert_eq!(m.seen[0], (Role::Initiator, ContextFlags::MUTUAL.union(ContextFlags::REPLAY), None));
        assert_eq!(m.seen[1].2, Some(b"reply".to_vec()));
    }

    #[test]
    fn a_missing_required_flag_fails_the_context() {
        let mut ctx = SecContext::initiate(target(), mech(), ContextFlags::MUTUAL, ContextFlags::MUTUAL);
        let mut m = Script::new(vec![Ok(Step::Complete { token: Some(b"done".to_vec()), engaged: ContextFlags::CONF, lifetime: None })]);
        assert_eq!(ctx.step(&mut m, None), Err(ContextError::RequiredFlagMissing(ContextFlags::MUTUAL)));
        assert_eq!(ctx.state(), State::Failed);
        assert!(!ctx.is_established());
        assert_eq!(ctx.step(&mut m, None), Err(ContextError::AlreadyFinished));
    }

    #[test]
    fn the_exchange_order_is_enforced_before_the_mechanism_runs() {
        let mut init = SecContext::initiate(target(), mech(), ContextFlags::NONE, ContextFlags::NONE);
        let mut acc = SecContext::accept(mech(), ContextFlags::NONE);
        let mut m = Script::new(vec![Ok(Step::Continue(vec![1]))]);
        assert_eq!(init.step(&mut m, Some(b"x")), Err(ContextError::UnexpectedToken));
        assert_eq!(acc.step(&mut m, None), Err(ContextError::MissingToken));
        assert!(m.seen.is_empty(), "no token reached the mechanism");
        assert_eq!(acc.role(), Role::Acceptor);
        assert!(acc.target().is_none());
        assert!(init.target().is_some());
    }

    #[test]
    fn a_refused_token_is_reported_and_leaves_the_context_failed() {
        let mut ctx = SecContext::accept(mech(), ContextFlags::NONE);
        let mut m = Script::new(vec![Err(RoutineError::DefectiveToken)]);
        assert_eq!(ctx.step(&mut m, Some(b"junk")), Err(ContextError::Mechanism(RoutineError::DefectiveToken)));
        assert_eq!(ctx.state(), State::Failed);
        assert_eq!(ctx.step(&mut m, Some(b"more")), Err(ContextError::AlreadyFinished));
    }

    #[test]
    fn an_acceptor_learns_engaged_flags_from_the_completion() {
        let mut ctx = SecContext::accept(mech(), ContextFlags::INTEG);
        let mut m = Script::new(vec![Ok(Step::Complete { token: None, engaged: ContextFlags::INTEG.union(ContextFlags::ANON), lifetime: None })]);
        let p = ctx.step(&mut m, Some(b"init-token")).unwrap();
        assert!(p.complete && p.token.is_none());
        assert_eq!(ctx.engaged(), ContextFlags::INTEG.union(ContextFlags::ANON));
        assert_eq!(ctx.lifetime(), None);
        assert!(ctx.missing().is_empty());
        assert_eq!(m.seen[0].1, ContextFlags::NONE);
    }
}

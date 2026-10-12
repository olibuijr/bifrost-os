//! Access control for stored secrets and authorization rights.
//!
//! A secret can carry an [`AccessControl`]: an expression of constraints that say which kind of user presence must
//! be shown before the secret is released. The constraints are a passcode, any enrolled biometry, the biometry set
//! that was enrolled when the secret was stored, a watch, or either of the first two (user presence). They combine
//! with `and` and `or`. A [`RightStore`] keeps the authorization rights the user granted, each valid until revoked or
//! for a number of seconds. The model is pure: the caller reports what the user just presented as a [`Presence`] and
//! the model answers. It reads no hardware and no clock; the caller passes the time in seconds.

use std::collections::BTreeMap;

/// Deepest nesting of `and` and `or` in one expression.
pub const MAX_DEPTH: usize = 8;
/// Most authorization rights kept in one store.
pub const MAX_RIGHTS: usize = 256;
/// Longest right name, in bytes.
pub const MAX_RIGHT_NAME: usize = 128;

/// One condition on user presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    /// The device passcode was entered.
    DevicePasscode,
    /// Any enrolled biometry matched.
    BiometryAny,
    /// Biometry matched and the enrolled set is the one that was enrolled when the secret was stored, so a change
    /// in enrollment locks the secret.
    BiometryCurrentSet,
    /// Biometry or the passcode.
    UserPresence,
    /// A paired watch is present.
    Watch,
}

/// A constraint, or a conjunction (`and`) or disjunction (`or`) of expressions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Constraint(Constraint),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

/// What the user just presented. The caller fills it in from the device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub passcode: bool,
    pub biometry: bool,
    /// True when the enrolled biometry is still the set the secret was stored with.
    pub enrolled_set_unchanged: bool,
    pub watch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// An `and` or `or` with no operands.
    EmptyConjunction,
    /// Nested deeper than [`MAX_DEPTH`].
    TooDeep,
    EmptyName,
    NameTooLong,
    /// A right name holds only lowercase letters, digits, `.`, `-` and `_`, and starts with a letter.
    BadName,
    TooManyRights,
}

impl Constraint {
    pub fn satisfied(self, p: &Presence) -> bool {
        match self {
            Constraint::DevicePasscode => p.passcode,
            Constraint::BiometryAny => p.biometry,
            Constraint::BiometryCurrentSet => p.biometry && p.enrolled_set_unchanged,
            Constraint::UserPresence => p.passcode || p.biometry,
            Constraint::Watch => p.watch,
        }
    }
}

impl Expr {
    /// Whether the presence satisfies the expression. An `and` needs every operand, an `or` one of them.
    pub fn satisfied(&self, p: &Presence) -> bool {
        match self {
            Expr::Constraint(c) => c.satisfied(p),
            Expr::And(xs) => xs.iter().all(|x| x.satisfied(p)),
            Expr::Or(xs) => xs.iter().any(|x| x.satisfied(p)),
        }
    }

    /// Checks that no conjunction is empty and the nesting stays within [`MAX_DEPTH`].
    pub fn validate(&self) -> Result<(), Error> {
        self.validate_at(1)
    }

    fn validate_at(&self, depth: usize) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        match self {
            Expr::Constraint(_) => Ok(()),
            Expr::And(xs) | Expr::Or(xs) => {
                if xs.is_empty() {
                    return Err(Error::EmptyConjunction);
                }
                xs.iter().try_for_each(|x| x.validate_at(depth + 1))
            }
        }
    }
}

/// The access rule of one stored secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessControl {
    expr: Expr,
}

impl AccessControl {
    /// Makes an access rule from a valid expression.
    pub fn new(expr: Expr) -> Result<AccessControl, Error> {
        expr.validate()?;
        Ok(AccessControl { expr })
    }

    pub fn expr(&self) -> &Expr {
        &self.expr
    }

    /// Whether the presence releases the secret.
    pub fn allows(&self, p: &Presence) -> bool {
        self.expr.satisfied(p)
    }
}

/// Whether a right is valid at a given time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightState {
    Unknown,
    Valid,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Right {
    pub granted_at: u64,
    /// Seconds the right stays valid after it was granted. `None` keeps it until it is revoked.
    pub valid_for: Option<u64>,
}

/// The authorization rights the user granted, by name. Times are seconds since any fixed epoch the caller uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RightStore {
    rights: BTreeMap<String, Right>,
}

impl RightStore {
    pub fn new() -> RightStore {
        RightStore::default()
    }

    /// Grants a right, or renews it if it is already in the store.
    pub fn grant(&mut self, name: &str, now: u64, valid_for: Option<u64>) -> Result<(), Error> {
        check_right_name(name)?;
        if !self.rights.contains_key(name) && self.rights.len() >= MAX_RIGHTS {
            return Err(Error::TooManyRights);
        }
        self.rights.insert(name.to_string(), Right { granted_at: now, valid_for });
        Ok(())
    }

    /// The state of a right at `now`. A right with `valid_for` of zero is expired at once.
    pub fn state(&self, name: &str, now: u64) -> RightState {
        match self.rights.get(name) {
            None => RightState::Unknown,
            Some(r) if live(r, now) => RightState::Valid,
            Some(_) => RightState::Expired,
        }
    }

    pub fn is_valid(&self, name: &str, now: u64) -> bool {
        self.state(name, now) == RightState::Valid
    }

    /// Removes a right. Returns whether it was in the store.
    pub fn revoke(&mut self, name: &str) -> bool {
        self.rights.remove(name).is_some()
    }

    /// Drops every expired right and returns how many were dropped.
    pub fn prune(&mut self, now: u64) -> usize {
        let before = self.rights.len();
        self.rights.retain(|_, r| live(r, now));
        before - self.rights.len()
    }

    pub fn len(&self) -> usize {
        self.rights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rights.is_empty()
    }
}

/// Right that a privileged helper install checks first (docs/parity-matrix, theme-20). Only the gate is modelled here;
/// the install itself is not implemented.
pub const RIGHT_HELPER_INSTALL: &str = "helper.install";
/// Right that a privileged helper install checks before it changes system daemons.
pub const RIGHT_DAEMON_MODIFY: &str = "daemon.modify";

/// Whether a privileged helper may be installed at `now`: both rights must be valid in the store.
pub fn helper_install_allowed(store: &RightStore, now: u64) -> bool {
    store.is_valid(RIGHT_HELPER_INSTALL, now) && store.is_valid(RIGHT_DAEMON_MODIFY, now)
}

/// Whether a right granted at `granted_at` is still valid at `now`.
fn live(r: &Right, now: u64) -> bool {
    match r.valid_for {
        None => true,
        Some(secs) => now.saturating_sub(r.granted_at) < secs,
    }
}

fn check_right_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::EmptyName);
    }
    if name.len() > MAX_RIGHT_NAME {
        return Err(Error::NameTooLong);
    }
    let ok = name.bytes().enumerate().all(|(i, b)| {
        if i == 0 {
            b.is_ascii_lowercase()
        } else {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        }
    });
    if ok { Ok(()) } else { Err(Error::BadName) }
}

/// An operation on an asymmetric key pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOperation {
    /// Creates a signature with the private key.
    Sign,
    /// Decrypts with the private key.
    Decrypt,
    /// Performs a key agreement with the private key.
    Exchange,
    /// Encrypts with the public key.
    Encrypt,
    /// Verifies a signature with the public key.
    Verify,
    /// Exports the bytes of the public key.
    Export,
}

impl KeyOperation {
    /// Whether the operation uses the private half, the only half that needs user presence.
    pub fn uses_private_key(self) -> bool {
        matches!(self, KeyOperation::Sign | KeyOperation::Decrypt | KeyOperation::Exchange)
    }
}

/// The access rule of a key pair. The private half is gated by an [`AccessControl`]. The public half is not
/// secret, so its operations need no presence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPairPolicy {
    private: AccessControl,
}

impl KeyPairPolicy {
    /// Makes the policy for the private half from a valid expression.
    pub fn new(expr: Expr) -> Result<KeyPairPolicy, Error> {
        Ok(KeyPairPolicy { private: AccessControl::new(expr)? })
    }

    /// Whether the presence allows the operation.
    pub fn allows(&self, op: KeyOperation, p: &Presence) -> bool {
        if op.uses_private_key() {
            self.private.allows(p)
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(passcode: bool, biometry: bool, unchanged: bool, watch: bool) -> Presence {
        Presence { passcode, biometry, enrolled_set_unchanged: unchanged, watch }
    }

    #[test]
    fn user_presence_accepts_the_passcode_or_biometry() {
        let c = Constraint::UserPresence;
        assert!(c.satisfied(&presence(true, false, false, false)));
        assert!(c.satisfied(&presence(false, true, false, false)));
        assert!(!c.satisfied(&presence(false, false, true, true)), "a watch alone is not user presence");
    }

    #[test]
    fn the_current_biometry_set_fails_once_the_enrollment_changes() {
        let c = Constraint::BiometryCurrentSet;
        assert!(c.satisfied(&presence(false, true, true, false)));
        assert!(!c.satisfied(&presence(false, true, false, false)), "enrollment changed: locked");
        assert!(Constraint::BiometryAny.satisfied(&presence(false, true, false, false)), "any set still matches");
    }

    #[test]
    fn and_needs_every_operand_and_or_needs_one() {
        let both = AccessControl::new(Expr::And(vec![
            Expr::Constraint(Constraint::BiometryAny),
            Expr::Constraint(Constraint::Watch),
        ]))
        .unwrap();
        assert!(!both.allows(&presence(false, true, true, false)));
        assert!(both.allows(&presence(false, true, true, true)));

        let either = AccessControl::new(Expr::Or(vec![
            Expr::Constraint(Constraint::DevicePasscode),
            Expr::Constraint(Constraint::Watch),
        ]))
        .unwrap();
        assert!(either.allows(&presence(true, false, false, false)));
        assert!(either.allows(&presence(false, false, false, true)));
        assert!(!either.allows(&presence(false, true, true, false)));
    }

    #[test]
    fn an_empty_conjunction_and_a_deep_nesting_are_refused() {
        assert_eq!(AccessControl::new(Expr::And(vec![])), Err(Error::EmptyConjunction));
        assert_eq!(
            AccessControl::new(Expr::Or(vec![Expr::And(vec![])])),
            Err(Error::EmptyConjunction)
        );

        let mut deep = Expr::Constraint(Constraint::Watch);
        for _ in 0..MAX_DEPTH {
            deep = Expr::Or(vec![deep]);
        }
        assert_eq!(deep.validate(), Err(Error::TooDeep));
        let mut ok = Expr::Constraint(Constraint::Watch);
        for _ in 0..MAX_DEPTH - 1 {
            ok = Expr::Or(vec![ok]);
        }
        assert_eq!(ok.validate(), Ok(()));
    }

    #[test]
    fn a_right_is_valid_until_its_lifetime_ends() {
        let mut store = RightStore::new();
        store.grant("system.privilege.update", 100, Some(60)).unwrap();
        assert_eq!(store.state("system.privilege.update", 100), RightState::Valid);
        assert_eq!(store.state("system.privilege.update", 159), RightState::Valid);
        assert_eq!(store.state("system.privilege.update", 160), RightState::Expired, "the lifetime is exclusive");
        assert_eq!(store.state("never.granted", 100), RightState::Unknown);
        assert!(!store.is_valid("system.privilege.update", 500));
    }

    #[test]
    fn a_right_without_a_lifetime_lasts_until_revoked() {
        let mut store = RightStore::new();
        store.grant("system.keychain.read", 0, None).unwrap();
        assert!(store.is_valid("system.keychain.read", u64::MAX));
        assert!(store.revoke("system.keychain.read"));
        assert!(!store.revoke("system.keychain.read"));
        assert_eq!(store.state("system.keychain.read", 1), RightState::Unknown);
        assert!(store.is_empty());
    }

    #[test]
    fn prune_drops_only_the_expired_rights() {
        let mut store = RightStore::new();
        store.grant("a.short", 0, Some(10)).unwrap();
        store.grant("b.long", 0, Some(1000)).unwrap();
        store.grant("c.forever", 0, None).unwrap();
        assert_eq!(store.prune(50), 1);
        assert_eq!(store.len(), 2);
        assert_eq!(store.state("a.short", 50), RightState::Unknown);
        assert_eq!(store.state("b.long", 50), RightState::Valid);
    }

    #[test]
    fn a_zero_lifetime_is_expired_at_once_and_renewal_restarts_the_clock() {
        let mut store = RightStore::new();
        store.grant("a.zero", 5, Some(0)).unwrap();
        assert_eq!(store.state("a.zero", 5), RightState::Expired);
        store.grant("a.renew", 0, Some(10)).unwrap();
        store.grant("a.renew", 8, Some(10)).unwrap();
        assert!(store.is_valid("a.renew", 15), "renewed at 8, valid until 18");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn bad_right_names_and_a_full_store_are_refused() {
        let mut store = RightStore::new();
        assert_eq!(store.grant("", 0, None), Err(Error::EmptyName));
        assert_eq!(store.grant("Admin.Right", 0, None), Err(Error::BadName));
        assert_eq!(store.grant("1.right", 0, None), Err(Error::BadName));
        assert_eq!(store.grant("has space", 0, None), Err(Error::BadName));
        assert_eq!(store.grant(&"a".repeat(MAX_RIGHT_NAME + 1), 0, None), Err(Error::NameTooLong));
        for i in 0..MAX_RIGHTS {
            store.grant(&format!("r.n{i}"), 0, None).unwrap();
        }
        assert_eq!(store.grant("r.extra", 0, None), Err(Error::TooManyRights));
        assert_eq!(store.grant("r.n0", 1, None), Ok(()), "renewing an existing right is not a new one");
    }

    #[test]
    fn key_pair_private_operations_need_presence_public_ones_do_not() {
        let policy = KeyPairPolicy::new(Expr::Constraint(Constraint::BiometryAny)).unwrap();
        let none = Presence::default();
        let face = Presence { biometry: true, ..Presence::default() };
        assert!(!policy.allows(KeyOperation::Sign, &none));
        assert!(!policy.allows(KeyOperation::Decrypt, &none));
        assert!(!policy.allows(KeyOperation::Exchange, &none));
        assert!(policy.allows(KeyOperation::Encrypt, &none));
        assert!(policy.allows(KeyOperation::Verify, &none));
        assert!(policy.allows(KeyOperation::Export, &none));
        assert!(policy.allows(KeyOperation::Sign, &face));
    }

    #[test]
    fn key_pair_current_set_locks_when_enrollment_changes() {
        let policy = KeyPairPolicy::new(Expr::Constraint(Constraint::BiometryCurrentSet)).unwrap();
        let changed = Presence { biometry: true, enrolled_set_unchanged: false, ..Presence::default() };
        let same = Presence { biometry: true, enrolled_set_unchanged: true, ..Presence::default() };
        assert!(!policy.allows(KeyOperation::Sign, &changed));
        assert!(policy.allows(KeyOperation::Sign, &same));
        assert!(policy.allows(KeyOperation::Verify, &changed), "the public half stays usable");
    }

    #[test]
    fn key_pair_and_expression_needs_every_part() {
        let expr = Expr::And(vec![Expr::Constraint(Constraint::DevicePasscode), Expr::Constraint(Constraint::Watch)]);
        let policy = KeyPairPolicy::new(expr).unwrap();
        let passcode_only = Presence { passcode: true, ..Presence::default() };
        let both = Presence { passcode: true, watch: true, ..Presence::default() };
        assert!(!policy.allows(KeyOperation::Decrypt, &passcode_only));
        assert!(policy.allows(KeyOperation::Decrypt, &both));
    }

    #[test]
    fn helper_install_needs_both_rights() {
        let mut store = RightStore::new();
        store.grant(RIGHT_HELPER_INSTALL, 100, None).unwrap();
        assert!(!helper_install_allowed(&store, 100));
        store.grant(RIGHT_DAEMON_MODIFY, 100, None).unwrap();
        assert!(helper_install_allowed(&store, 100));
    }

    #[test]
    fn helper_install_rights_expire_on_their_own_clock() {
        let mut store = RightStore::new();
        store.grant(RIGHT_HELPER_INSTALL, 100, Some(60)).unwrap();
        store.grant(RIGHT_DAEMON_MODIFY, 100, None).unwrap();
        // Granted at 100 for 60 seconds: valid at 159, expired at 160.
        assert!(helper_install_allowed(&store, 159));
        assert!(!helper_install_allowed(&store, 160));
    }

    #[test]
    fn revoking_one_right_blocks_the_helper_install() {
        let mut store = RightStore::new();
        store.grant(RIGHT_HELPER_INSTALL, 0, None).unwrap();
        store.grant(RIGHT_DAEMON_MODIFY, 0, None).unwrap();
        assert!(store.revoke(RIGHT_DAEMON_MODIFY));
        assert!(!helper_install_allowed(&store, 1));
    }

    #[test]
    fn key_pair_rejects_an_empty_expression() {
        assert_eq!(KeyPairPolicy::new(Expr::And(Vec::new())), Err(Error::EmptyConjunction));
    }

    #[test]
    fn only_sign_decrypt_and_exchange_use_the_private_half() {
        for op in [KeyOperation::Sign, KeyOperation::Decrypt, KeyOperation::Exchange] {
            assert!(op.uses_private_key(), "{op:?}");
        }
        for op in [KeyOperation::Encrypt, KeyOperation::Verify, KeyOperation::Export] {
            assert!(!op.uses_private_key(), "{op:?}");
        }
    }
}

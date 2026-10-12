//! Smart card sign-in: which tokens are registered to which account, whether a registration survives the token being
//! removed, and whether a token must be used for sign-in at all.
//!
//! A token is registered to one account name. A registration is persistent (for a token that stays available, such
//! as a hardware security module) or not; a non-persistent registration is dropped when its token is removed. A user
//! signs in with a token only when the token is present in the [`TokenWatcher`], is registered to that account, and
//! the driver verified its secret (the outcome is passed to [`TokenSession::authenticate`]). When enforcement is on,
//! an account that has a registered token may not sign in with a password. Nothing here checks a PIN.
use crate::token_watch::{TokenError, TokenSession, TokenWatcher};
use std::collections::BTreeMap;

/// One registration: the token, the account it signs in, and whether it survives removal of the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub token_id: String,
    pub account: String,
    pub persistent: bool,
}

/// The registered tokens, keyed by token ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistrationManager {
    registrations: BTreeMap<String, Registration>,
    enforced: bool,
}

impl RegistrationManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The IDs of the registered tokens, sorted.
    pub fn registered_tokens(&self) -> Vec<&str> {
        self.registrations.keys().map(|k| k.as_str()).collect()
    }

    /// Registers a token to an account. Both names must be non-empty, and a token that is already registered is
    /// `BadParameter`: unregister it first.
    pub fn register(&mut self, token_id: &str, account: &str, persistent: bool) -> Result<(), TokenError> {
        if token_id.is_empty() || account.is_empty() || self.registrations.contains_key(token_id) {
            return Err(TokenError::BadParameter);
        }
        let reg = Registration { token_id: token_id.to_string(), account: account.to_string(), persistent };
        self.registrations.insert(token_id.to_string(), reg);
        Ok(())
    }

    /// Removes a registration and returns it. An unknown token is `TokenNotFound`.
    pub fn unregister(&mut self, token_id: &str) -> Result<Registration, TokenError> {
        self.registrations.remove(token_id).ok_or(TokenError::TokenNotFound)
    }

    /// The account a token is registered to.
    pub fn account_for(&self, token_id: &str) -> Option<&str> {
        self.registrations.get(token_id).map(|r| r.account.as_str())
    }

    /// Called when a token goes away. A non-persistent registration is dropped and `true` is returned; a persistent
    /// one stays and `false` is returned, as does a token that was never registered.
    pub fn token_removed(&mut self, token_id: &str) -> bool {
        let persistent = match self.registrations.get(token_id) {
            Some(reg) => reg.persistent,
            None => return false,
        };
        if persistent {
            return false;
        }
        self.registrations.remove(token_id);
        true
    }

    /// Turns enforcement on or off. Off is the default.
    pub fn set_enforced(&mut self, enforced: bool) {
        self.enforced = enforced;
    }

    pub fn enforced(&self) -> bool {
        self.enforced
    }

    /// `false` when enforcement is on and the account has a registered token, otherwise `true`.
    pub fn password_sign_in_allowed(&self, account: &str) -> bool {
        !(self.enforced && self.registrations.values().any(|r| r.account == account))
    }

    /// Passes the outcome of a token sign-in to the session. The token must be present in the watcher and registered
    /// to `account`. A token that is not present or not registered is `TokenNotFound`; a registration to another
    /// account is `AuthenticationFailed` and does not count against the session.
    pub fn authenticate(
        &self,
        watcher: &TokenWatcher,
        session: &mut TokenSession,
        account: &str,
        verified: bool,
    ) -> Result<(), TokenError> {
        let token_id = session.token_id().to_string();
        if watcher.token_info(&token_id).is_none() {
            return Err(TokenError::TokenNotFound);
        }
        match self.account_for(&token_id) {
            None => Err(TokenError::TokenNotFound),
            Some(registered) if registered != account => Err(TokenError::AuthenticationFailed),
            Some(_) => session.authenticate(verified),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_watch::{AuthState, TokenInfo};

    fn present(watcher: &mut TokenWatcher, id: &str) {
        let info = TokenInfo { token_id: id.to_string(), driver_name: "card".to_string(), slot_name: "reader".to_string() };
        watcher.insert(info).unwrap();
    }

    #[test]
    fn registered_tokens_are_sorted_and_duplicates_are_refused() {
        let mut m = RegistrationManager::new();
        m.register("usb-card", "olibuijr", false).unwrap();
        m.register("hsm-1", "olibuijr", true).unwrap();
        assert_eq!(m.registered_tokens(), vec!["hsm-1", "usb-card"]);
        assert_eq!(m.register("usb-card", "guest", false), Err(TokenError::BadParameter));
        assert_eq!(m.register("", "guest", false), Err(TokenError::BadParameter));
        assert_eq!(m.register("new-card", "", false), Err(TokenError::BadParameter));
        assert_eq!(m.account_for("usb-card"), Some("olibuijr"));
    }

    #[test]
    fn unregister_returns_the_registration_and_an_unknown_token_is_refused() {
        let mut m = RegistrationManager::new();
        m.register("usb-card", "olibuijr", false).unwrap();
        let removed = m.unregister("usb-card").unwrap();
        assert_eq!(removed.account, "olibuijr");
        assert!(!removed.persistent);
        assert_eq!(m.unregister("usb-card"), Err(TokenError::TokenNotFound));
    }

    #[test]
    fn removing_a_token_drops_only_a_non_persistent_registration() {
        let mut m = RegistrationManager::new();
        m.register("usb-card", "olibuijr", false).unwrap();
        m.register("hsm-1", "olibuijr", true).unwrap();
        assert!(m.token_removed("usb-card"));
        assert!(!m.token_removed("hsm-1"));
        assert!(!m.token_removed("never-seen"));
        assert_eq!(m.registered_tokens(), vec!["hsm-1"]);
    }

    #[test]
    fn sign_in_needs_the_token_present_and_registered() {
        let mut m = RegistrationManager::new();
        m.register("card-1", "olibuijr", false).unwrap();
        let mut watcher = TokenWatcher::new();
        let mut session = TokenSession::new("card-1");
        assert_eq!(m.authenticate(&watcher, &mut session, "olibuijr", true), Err(TokenError::TokenNotFound));
        assert_eq!(session.auth_state(), AuthState::Unauthenticated);
        present(&mut watcher, "card-1");
        assert_eq!(m.authenticate(&watcher, &mut session, "olibuijr", true), Ok(()));
        assert_eq!(session.auth_state(), AuthState::Authenticated);
    }

    #[test]
    fn a_token_registered_to_another_account_does_not_count_as_a_failure() {
        let mut m = RegistrationManager::new();
        m.register("card-1", "olibuijr", false).unwrap();
        let mut watcher = TokenWatcher::new();
        present(&mut watcher, "card-1");
        let mut session = TokenSession::new("card-1");
        assert_eq!(m.authenticate(&watcher, &mut session, "guest", true), Err(TokenError::AuthenticationFailed));
        assert_eq!(session.failures(), 0);
        assert_eq!(session.auth_state(), AuthState::Unauthenticated);
    }

    #[test]
    fn a_wrong_secret_counts_against_the_session_limit() {
        let mut m = RegistrationManager::new();
        m.register("card-1", "olibuijr", false).unwrap();
        let mut watcher = TokenWatcher::new();
        present(&mut watcher, "card-1");
        let mut session = TokenSession::new("card-1");
        for _ in 0..3 {
            assert_eq!(m.authenticate(&watcher, &mut session, "olibuijr", false), Err(TokenError::AuthenticationFailed));
        }
        assert_eq!(session.auth_state(), AuthState::Locked);
    }

    #[test]
    fn enforcement_blocks_passwords_only_for_accounts_with_a_token() {
        let mut m = RegistrationManager::new();
        m.register("card-1", "olibuijr", false).unwrap();
        assert!(m.password_sign_in_allowed("olibuijr"));
        m.set_enforced(true);
        assert!(m.enforced());
        assert!(!m.password_sign_in_allowed("olibuijr"));
        assert!(m.password_sign_in_allowed("guest"));
        m.set_enforced(false);
        assert!(m.password_sign_in_allowed("olibuijr"));
    }
}

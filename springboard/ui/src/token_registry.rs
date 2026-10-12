//! Registration of smart cards by token ID, and the policy that says when a card is needed to log in.
//! Pure model: the shell stores the registered IDs in its preferences; this module decides the outcome of a
//! register, unregister or login check. Card presence and the PIN are checked by the driver, which passes the
//! session to `login_allowed`.
use std::collections::BTreeSet;

use crate::token_watch::{TokenError, TokenSession};

/// Whether a login needs a registered smart card. `Optional` lets a login go through without a card, and
/// `Required` lets it through only with an authenticated session on a registered card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthPolicy {
    #[default]
    Optional,
    Required,
}

/// The registered smart card token IDs and the login policy. IDs are kept sorted and unique.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenRegistry {
    tokens: BTreeSet<String>,
    policy: AuthPolicy,
}

impl TokenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn policy(&self) -> AuthPolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: AuthPolicy) {
        self.policy = policy;
    }

    /// Registers a card. Returns `true` when the ID was new and `false` when it was already registered. An empty ID
    /// is `BadParameter`.
    pub fn register(&mut self, token_id: &str) -> Result<bool, TokenError> {
        if token_id.is_empty() {
            return Err(TokenError::BadParameter);
        }
        Ok(self.tokens.insert(token_id.to_string()))
    }

    /// Removes a card. An ID that is not registered is `TokenNotFound`.
    pub fn unregister(&mut self, token_id: &str) -> Result<(), TokenError> {
        if self.tokens.remove(token_id) {
            Ok(())
        } else {
            Err(TokenError::TokenNotFound)
        }
    }

    pub fn is_registered(&self, token_id: &str) -> bool {
        self.tokens.contains(token_id)
    }

    /// The registered IDs, sorted.
    pub fn registered(&self) -> Vec<&str> {
        self.tokens.iter().map(|t| t.as_str()).collect()
    }

    /// Decides whether a login may proceed with the given card session. Under `Optional` every login is allowed. Under
    /// `Required` the session's card must be registered (otherwise `TokenNotFound`) and the session must be
    /// authenticated (otherwise `AuthenticationNeeded`). A session with no card cannot satisfy `Required`.
    pub fn login_allowed(&self, session: Option<&TokenSession>) -> Result<(), TokenError> {
        match self.policy {
            AuthPolicy::Optional => Ok(()),
            AuthPolicy::Required => {
                let session = session.ok_or(TokenError::TokenNotFound)?;
                if !self.is_registered(session.token_id()) {
                    return Err(TokenError::TokenNotFound);
                }
                session.require()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_reports_new_ids_and_refuses_empty_ones() {
        let mut r = TokenRegistry::new();
        assert_eq!(r.register("card-b"), Ok(true));
        assert_eq!(r.register("card-a"), Ok(true));
        assert_eq!(r.register("card-a"), Ok(false));
        assert_eq!(r.register(""), Err(TokenError::BadParameter));
        assert_eq!(r.registered(), vec!["card-a", "card-b"]);
    }

    #[test]
    fn unregister_removes_a_registered_card_only() {
        let mut r = TokenRegistry::new();
        r.register("card-a").unwrap();
        assert_eq!(r.unregister("card-a"), Ok(()));
        assert!(!r.is_registered("card-a"));
        assert_eq!(r.unregister("card-a"), Err(TokenError::TokenNotFound));
    }

    #[test]
    fn the_default_policy_lets_every_login_through() {
        let r = TokenRegistry::new();
        assert_eq!(r.policy(), AuthPolicy::Optional);
        assert_eq!(r.login_allowed(None), Ok(()));
    }

    #[test]
    fn required_policy_without_a_card_is_refused() {
        let mut r = TokenRegistry::new();
        r.set_policy(AuthPolicy::Required);
        assert_eq!(r.login_allowed(None), Err(TokenError::TokenNotFound));
    }

    #[test]
    fn required_policy_needs_a_registered_and_authenticated_card() {
        let mut r = TokenRegistry::new();
        r.set_policy(AuthPolicy::Required);
        let mut session = TokenSession::new("card-a");
        // Not registered yet: the card is unknown to the policy.
        assert_eq!(r.login_allowed(Some(&session)), Err(TokenError::TokenNotFound));
        r.register("card-a").unwrap();
        // Registered but the PIN has not been checked.
        assert_eq!(r.login_allowed(Some(&session)), Err(TokenError::AuthenticationNeeded));
        session.authenticate(true).unwrap();
        assert_eq!(r.login_allowed(Some(&session)), Ok(()));
    }

    #[test]
    fn a_locked_card_session_does_not_satisfy_required() {
        let mut r = TokenRegistry::new();
        r.set_policy(AuthPolicy::Required);
        r.register("card-a").unwrap();
        let mut session = TokenSession::with_max_failures("card-a", 1);
        let _ = session.authenticate(false);
        assert_eq!(r.login_allowed(Some(&session)), Err(TokenError::AuthenticationNeeded));
    }
}

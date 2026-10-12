//! Cryptographic tokens: the watcher that tracks the tokens in the system, the error codes of the token layer, and
//! the authentication state of one token session.
//! Pure model: a driver reports tokens with `TokenWatcher::insert` and `remove`, and the model returns the event a
//! handler would receive. The driver checks the PIN or other secret and passes the outcome to
//! `TokenSession::authenticate`; no secret is checked here.
use std::collections::BTreeMap;
use std::fmt;

/// Errors of the token layer. Each variant has a fixed message, shown by `Display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    NotImplemented,
    CommunicationError,
    CorruptedData,
    CanceledByUser,
    AuthenticationFailed,
    ObjectNotFound,
    TokenNotFound,
    BadParameter,
    AuthenticationNeeded,
    InvalidatedDeviceKey,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            TokenError::NotImplemented => "not implemented",
            TokenError::CommunicationError => "communication error",
            TokenError::CorruptedData => "corrupted data",
            TokenError::CanceledByUser => "canceled by the user",
            TokenError::AuthenticationFailed => "authentication failed",
            TokenError::ObjectNotFound => "object not found",
            TokenError::TokenNotFound => "token not found",
            TokenError::BadParameter => "bad parameter",
            TokenError::AuthenticationNeeded => "authentication needed",
            TokenError::InvalidatedDeviceKey => "device key invalidated",
        };
        f.write_str(text)
    }
}

impl std::error::Error for TokenError {}

/// What the watcher knows about one token: its ID, the driver that serves it and the slot it sits in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenInfo {
    pub token_id: String,
    pub driver_name: String,
    pub slot_name: String,
}

/// The event a token change produces, carrying the token's information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Inserted(TokenInfo),
    Removed(TokenInfo),
}

/// Tracks the tokens present in the system, ordered by token ID. Token IDs are unique.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenWatcher {
    tokens: BTreeMap<String, TokenInfo>,
}

impl TokenWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// The IDs of the tokens present, sorted.
    pub fn token_ids(&self) -> Vec<&str> {
        self.tokens.keys().map(|k| k.as_str()).collect()
    }

    pub fn token_info(&self, token_id: &str) -> Option<&TokenInfo> {
        self.tokens.get(token_id)
    }

    /// Records a token that appeared. An empty or already present ID is refused.
    pub fn insert(&mut self, info: TokenInfo) -> Result<WatchEvent, TokenError> {
        if info.token_id.is_empty() || self.tokens.contains_key(&info.token_id) {
            return Err(TokenError::BadParameter);
        }
        self.tokens.insert(info.token_id.clone(), info.clone());
        Ok(WatchEvent::Inserted(info))
    }

    /// Records a token that went away and returns its last information. An unknown ID is `TokenNotFound`.
    pub fn remove(&mut self, token_id: &str) -> Result<WatchEvent, TokenError> {
        self.tokens.remove(token_id).map(WatchEvent::Removed).ok_or(TokenError::TokenNotFound)
    }
}

/// Default number of wrong attempts before a session locks.
pub const DEFAULT_MAX_FAILURES: u32 = 3;

/// The authentication state of a token session. `Locked` is reached after too many failed attempts and does not
/// clear with `logout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthState {
    Unauthenticated,
    Authenticated,
    Locked,
}

/// A session with one token. It counts failed attempts and gates work on the authentication state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenSession {
    token_id: String,
    state: AuthState,
    failures: u32,
    max_failures: u32,
}

impl TokenSession {
    pub fn new(token_id: &str) -> Self {
        Self::with_max_failures(token_id, DEFAULT_MAX_FAILURES)
    }

    pub fn with_max_failures(token_id: &str, max_failures: u32) -> Self {
        TokenSession { token_id: token_id.to_string(), state: AuthState::Unauthenticated, failures: 0, max_failures }
    }

    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    pub fn auth_state(&self) -> AuthState {
        self.state
    }

    /// Failed attempts since the last success.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Records the outcome of an attempt the driver made. A success clears the failure count. The attempt that
    /// reaches the limit locks the session, and a locked session refuses every attempt.
    pub fn authenticate(&mut self, verified: bool) -> Result<(), TokenError> {
        match self.state {
            AuthState::Locked => Err(TokenError::AuthenticationFailed),
            AuthState::Authenticated => Ok(()),
            AuthState::Unauthenticated => {
                if verified {
                    self.state = AuthState::Authenticated;
                    self.failures = 0;
                    Ok(())
                } else {
                    self.failures += 1;
                    if self.failures >= self.max_failures {
                        self.state = AuthState::Locked;
                    }
                    Err(TokenError::AuthenticationFailed)
                }
            }
        }
    }

    /// Succeeds only while authenticated, otherwise `AuthenticationNeeded`.
    pub fn require(&self) -> Result<(), TokenError> {
        if self.state == AuthState::Authenticated {
            Ok(())
        } else {
            Err(TokenError::AuthenticationNeeded)
        }
    }

    /// Ends an authenticated session. A locked session stays locked.
    pub fn logout(&mut self) {
        if self.state == AuthState::Authenticated {
            self.state = AuthState::Unauthenticated;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, slot: &str) -> TokenInfo {
        TokenInfo { token_id: id.to_string(), driver_name: "card-driver".to_string(), slot_name: slot.to_string() }
    }

    #[test]
    fn error_messages_are_fixed_text() {
        assert_eq!(TokenError::TokenNotFound.to_string(), "token not found");
        assert_eq!(TokenError::AuthenticationNeeded.to_string(), "authentication needed");
        assert_eq!(TokenError::InvalidatedDeviceKey.to_string(), "device key invalidated");
    }

    #[test]
    fn watcher_lists_tokens_sorted_and_refuses_duplicates() {
        let mut w = TokenWatcher::new();
        assert_eq!(w.insert(info("b-token", "reader B")), Ok(WatchEvent::Inserted(info("b-token", "reader B"))));
        w.insert(info("a-token", "reader A")).unwrap();
        assert_eq!(w.token_ids(), vec!["a-token", "b-token"]);
        assert_eq!(w.insert(info("a-token", "reader A")), Err(TokenError::BadParameter));
        assert_eq!(w.insert(info("", "reader A")), Err(TokenError::BadParameter));
        assert_eq!(w.token_info("b-token").unwrap().slot_name, "reader B");
    }

    #[test]
    fn removing_a_token_returns_its_last_information() {
        let mut w = TokenWatcher::new();
        w.insert(info("a-token", "reader A")).unwrap();
        assert_eq!(w.remove("a-token"), Ok(WatchEvent::Removed(info("a-token", "reader A"))));
        assert!(w.token_ids().is_empty());
        assert_eq!(w.remove("a-token"), Err(TokenError::TokenNotFound));
    }

    #[test]
    fn work_needs_authentication_first() {
        let s = TokenSession::new("a-token");
        assert_eq!(s.auth_state(), AuthState::Unauthenticated);
        assert_eq!(s.require(), Err(TokenError::AuthenticationNeeded));
    }

    #[test]
    fn a_success_after_two_failures_authenticates_and_clears_the_count() {
        let mut s = TokenSession::new("a-token");
        assert_eq!(s.authenticate(false), Err(TokenError::AuthenticationFailed));
        assert_eq!(s.authenticate(false), Err(TokenError::AuthenticationFailed));
        assert_eq!(s.failures(), 2);
        assert_eq!(s.authenticate(true), Ok(()));
        assert_eq!(s.auth_state(), AuthState::Authenticated);
        assert_eq!(s.failures(), 0);
        assert_eq!(s.require(), Ok(()));
    }

    #[test]
    fn the_third_failure_locks_the_session_and_logout_does_not_unlock_it() {
        let mut s = TokenSession::new("a-token");
        for _ in 0..3 {
            let _ = s.authenticate(false);
        }
        assert_eq!(s.auth_state(), AuthState::Locked);
        assert_eq!(s.authenticate(true), Err(TokenError::AuthenticationFailed));
        s.logout();
        assert_eq!(s.auth_state(), AuthState::Locked);
        assert_eq!(s.require(), Err(TokenError::AuthenticationNeeded));
    }

    #[test]
    fn logout_ends_an_authenticated_session() {
        let mut s = TokenSession::with_max_failures("a-token", 1);
        assert_eq!(s.authenticate(true), Ok(()));
        s.logout();
        assert_eq!(s.auth_state(), AuthState::Unauthenticated);
        assert_eq!(s.require(), Err(TokenError::AuthenticationNeeded));
        // With a limit of one, a single wrong try locks it.
        assert_eq!(s.authenticate(false), Err(TokenError::AuthenticationFailed));
        assert_eq!(s.auth_state(), AuthState::Locked);
    }
}

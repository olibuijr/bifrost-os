//! Web service tokens: the operation that turns an app's API token into a token for signing a user in on the web.
//!
//! Mirrors the reference fetch operation (`init(apiToken:)`, the API token property, and the result of the operation).
//! A `FetchWebToken` holds the API token that names the app's container and runs once, through a transport the caller
//! supplies. The transport is the only place a network call could happen, and this module has none: it is a closure
//! from the API token to the token text, or to an error. Tests pass a closure that returns a fixed value.
//!
//! The rules here are ours, because the reference page gives the parameters and the result but not the checks:
//! - the API token must not be empty and must not contain whitespace or control characters;
//! - a transport that returns an empty token is a failure, so an app never gets an empty web token;
//! - the token value is never printed: `Debug` shows its length only.
//!
//! Gaps: no retry (a transient error comes back to the caller, which decides), no expiry time for the web token, and
//! no cache of a token between runs.

use std::fmt;

use crate::record_sync::{ErrorCode, RecordError};

/// A token that lets the holder start a web sign-in for one app container.
#[derive(Clone, PartialEq, Eq)]
pub struct WebToken {
    value: String,
}

impl WebToken {
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for WebToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WebToken(<{} bytes>)", self.value.len())
    }
}

/// One fetch of a web token for an API token (the fetch operation of the reference SDK). `new` takes the API token, and `empty`
/// leaves it to be set later with `set_api_token`, as the reference operation allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchWebToken {
    api_token: Option<String>,
}

impl FetchWebToken {
    pub fn new(api_token: impl Into<String>) -> Self {
        FetchWebToken { api_token: Some(api_token.into()) }
    }

    pub fn empty() -> Self {
        FetchWebToken { api_token: None }
    }

    pub fn api_token(&self) -> Option<&str> {
        self.api_token.as_deref()
    }

    pub fn set_api_token(&mut self, api_token: impl Into<String>) {
        self.api_token = Some(api_token.into());
    }

    /// Runs the fetch. `transport` receives the API token and returns the token text the service issued, or an error
    /// the service reported. The API token is checked before the transport is called, so a bad one never leaves the
    /// app.
    pub fn run<F>(&self, transport: F) -> Result<WebToken, RecordError>
    where
        F: FnOnce(&str) -> Result<String, RecordError>,
    {
        let api_token = match self.api_token.as_deref() {
            Some(t) if !t.is_empty() => t,
            _ => return Err(RecordError::new(ErrorCode::InvalidArguments, "no API token is set for the fetch")),
        };
        if api_token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(RecordError::new(ErrorCode::InvalidArguments, "the API token contains a space or control character"));
        }
        let issued = transport(api_token)?;
        if issued.is_empty() {
            return Err(RecordError::new(ErrorCode::NotAuthenticated, "the service issued an empty web token"));
        }
        Ok(WebToken { value: issued })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_returns_the_issued_token_and_passes_the_api_token_through() {
        let op = FetchWebToken::new("api-123");
        let token = op
            .run(|api| {
                assert_eq!(api, "api-123");
                Ok("web-token-xyz".to_string())
            })
            .unwrap();
        assert_eq!(token.value(), "web-token-xyz");
    }

    #[test]
    fn missing_or_empty_api_token_fails_without_calling_the_transport() {
        let mut called = false;
        let err = FetchWebToken::empty()
            .run(|_| {
                called = true;
                Ok("never".to_string())
            })
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidArguments);
        assert!(!called);

        let err = FetchWebToken::new("").run(|_| Ok("never".to_string())).unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidArguments);
    }

    #[test]
    fn api_token_with_whitespace_is_refused_before_the_transport_runs() {
        let mut called = false;
        let err = FetchWebToken::new("api 123")
            .run(|_| {
                called = true;
                Ok("never".to_string())
            })
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidArguments);
        assert!(!called);
        assert!(FetchWebToken::new("tab\there").run(|_| Ok("x".to_string())).is_err());
    }

    #[test]
    fn empty_issued_token_is_a_failure_and_service_errors_pass_through() {
        let err = FetchWebToken::new("api").run(|_| Ok(String::new())).unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotAuthenticated);

        let err = FetchWebToken::new("api")
            .run(|_| Err(RecordError::new(ErrorCode::RequestRateLimited, "slow down").with_retry_after(30)))
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::RequestRateLimited);
        assert_eq!(err.retry_after(), Some(30));
    }

    #[test]
    fn set_api_token_replaces_the_missing_value_and_debug_hides_the_token() {
        let mut op = FetchWebToken::empty();
        assert_eq!(op.api_token(), None);
        op.set_api_token("late-api");
        assert_eq!(op.api_token(), Some("late-api"));
        let token = op.run(|_| Ok("secret-web-token".to_string())).unwrap();
        let shown = format!("{:?}", token);
        assert_eq!(shown, "WebToken(<16 bytes>)");
        assert!(!shown.contains("secret"));
    }
}

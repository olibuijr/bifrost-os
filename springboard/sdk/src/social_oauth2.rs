//! OAuth 2 tokens for social service requests (the OAuth 2 part of `SLRequest` parity, see `docs/parity/social.md`).
//!
//! A service's token endpoint answers with JSON (RFC 6749 section 5.1). [`OAuth2Token::from_response`] reads it, using
//! the caller's clock so the expiry is testable. A token is used as a bearer token (RFC 6750): [`OAuth2Token::sign`]
//! sets `Authorization: Bearer`. Once the token is inside the refresh margin, or expired, [`OAuth2Token::refresh_request`]
//! builds the form POST for the token endpoint (`grant_type=refresh_token`) and the caller sends it.
//!
//! The network is the caller's: nothing here opens a connection. The account store keeps the token the same way it
//! keeps a bearer token (see [`crate::social_account`]); the sign-in flow that obtains the first token is not in the shell.

use std::fmt;

use crate::social::{RequestMethod, ServiceRequest};
use crate::url::URLRequest;
use bifrost_core::json::{parse, Json};

/// A token this close to its expiry is refreshed before a request uses it, in seconds.
pub const REFRESH_MARGIN_SECS: u64 = 60;

/// Why a token could not be read, refreshed or used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuth2Error {
    /// The response is not JSON, or has no `access_token`.
    Malformed,
    /// The response names a token type other than `Bearer`.
    UnsupportedTokenType,
    /// The token has expired and cannot sign a request.
    Expired,
    /// The token has no refresh token, so it cannot be renewed.
    NoRefreshToken,
}

impl fmt::Display for OAuth2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OAuth2Error::Malformed => "the token response has no access token",
            OAuth2Error::UnsupportedTokenType => "the token is not a bearer token",
            OAuth2Error::Expired => "the token has expired",
            OAuth2Error::NoRefreshToken => "the token cannot be refreshed",
        })
    }
}

/// One OAuth 2 token and what is known about its life.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuth2Token {
    access_token: String,
    refresh_token: Option<String>,
    /// Unix seconds at which the access token stops working, when the service said how long it lasts.
    expires_at: Option<u64>,
}

impl fmt::Debug for OAuth2Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OAuth2Token {{ expires_at: {:?}, secrets: <hidden> }}", self.expires_at)
    }
}

impl OAuth2Token {
    /// Reads a token response. `now` is the current Unix time in seconds; `expires_in` is counted from it.
    pub fn from_response(body: &str, now: u64) -> Result<OAuth2Token, OAuth2Error> {
        let json = parse(body).map_err(|_| OAuth2Error::Malformed)?;
        let access_token = json
            .get("access_token")
            .and_then(Json::as_str)
            .filter(|t| !t.is_empty())
            .ok_or(OAuth2Error::Malformed)?
            .to_string();
        if let Some(kind) = json.get("token_type").and_then(Json::as_str) {
            if !kind.eq_ignore_ascii_case("bearer") {
                return Err(OAuth2Error::UnsupportedTokenType);
            }
        }
        let refresh_token = json
            .get("refresh_token")
            .and_then(Json::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let expires_at = json.get("expires_in").and_then(Json::as_u64).map(|s| now.saturating_add(s));
        Ok(OAuth2Token { access_token, refresh_token, expires_at })
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    /// True when the access token has run out at `now`. A token with no expiry never runs out.
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|t| now >= t)
    }

    /// True when a refresh is due at `now`: the token expires within [`REFRESH_MARGIN_SECS`] (or has expired), and a
    /// refresh token is held.
    pub fn needs_refresh(&self, now: u64) -> bool {
        self.refresh_token.is_some() && self.expires_at.is_some_and(|t| now.saturating_add(REFRESH_MARGIN_SECS) >= t)
    }

    /// The form POST for the token endpoint that exchanges the refresh token for a new token.
    pub fn refresh_request(&self, token_url: &str, client_id: &str) -> Result<ServiceRequest, OAuth2Error> {
        let refresh = self.refresh_token.as_deref().ok_or(OAuth2Error::NoRefreshToken)?;
        Ok(ServiceRequest::new(
            RequestMethod::Post,
            token_url,
            &[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", client_id)],
        ))
    }

    /// Sets `Authorization: Bearer` on the request. Refuses once the token has expired at `now`.
    pub fn sign(&self, req: &mut URLRequest, now: u64) -> Result<(), OAuth2Error> {
        if self.is_expired(now) {
            return Err(OAuth2Error::Expired);
        }
        req.set_header("Authorization", &format!("Bearer {}", self.access_token));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;

    fn token(body: &str) -> OAuth2Token {
        OAuth2Token::from_response(body, NOW).unwrap()
    }

    #[test]
    fn a_token_response_sets_the_expiry_from_expires_in() {
        let t = token(r#"{"access_token":"abc","token_type":"bearer","expires_in":3600,"refresh_token":"r1"}"#);
        assert_eq!(t.access_token(), "abc");
        assert_eq!(t.expires_at(), Some(NOW + 3600));
        assert!(!t.is_expired(NOW + 3599));
        assert!(t.is_expired(NOW + 3600));
    }

    #[test]
    fn a_token_without_expires_in_never_expires() {
        let t = token(r#"{"access_token":"abc"}"#);
        assert_eq!(t.expires_at(), None);
        assert!(!t.is_expired(u64::MAX));
        assert!(!t.needs_refresh(u64::MAX), "no refresh token and no expiry");
    }

    #[test]
    fn a_response_without_an_access_token_or_a_bearer_type_is_refused() {
        assert_eq!(OAuth2Token::from_response("not json", NOW), Err(OAuth2Error::Malformed));
        assert_eq!(OAuth2Token::from_response(r#"{"access_token":""}"#, NOW), Err(OAuth2Error::Malformed));
        assert_eq!(OAuth2Token::from_response(r#"{"token_type":"bearer"}"#, NOW), Err(OAuth2Error::Malformed));
        assert_eq!(
            OAuth2Token::from_response(r#"{"access_token":"a","token_type":"mac"}"#, NOW),
            Err(OAuth2Error::UnsupportedTokenType)
        );
    }

    #[test]
    fn a_refresh_is_due_inside_the_margin_and_needs_a_refresh_token() {
        let with_refresh = token(r#"{"access_token":"a","expires_in":300,"refresh_token":"r"}"#);
        assert!(!with_refresh.needs_refresh(NOW));
        assert!(with_refresh.needs_refresh(NOW + 300 - REFRESH_MARGIN_SECS));

        let without = token(r#"{"access_token":"a","expires_in":300}"#);
        assert!(!without.needs_refresh(NOW + 299));
    }

    #[test]
    fn the_refresh_request_is_a_form_post_to_the_token_endpoint() {
        let t = token(r#"{"access_token":"a","expires_in":300,"refresh_token":"r 1"}"#);
        let req = t.refresh_request("https://id.example.is/token", "client").unwrap();
        assert_eq!(req.method.as_str(), "POST");
        let url = req.prepared_url_request().unwrap();
        assert!(url.url.starts_with("https://id.example.is/token"));
        let body = String::from_utf8_lossy(url.body.as_deref().unwrap_or(&[])).to_string();
        assert!(body.contains("grant_type=refresh_token"), "{body}");
        assert!(body.contains("refresh_token=r+1"), "{body}");
        assert!(body.contains("client_id=client"), "{body}");
    }

    #[test]
    fn a_token_without_a_refresh_token_cannot_refresh() {
        let t = token(r#"{"access_token":"a"}"#);
        assert_eq!(t.refresh_request("https://id.example.is/token", "c").unwrap_err(), OAuth2Error::NoRefreshToken);
    }

    #[test]
    fn signing_sets_the_bearer_header_until_the_token_expires() {
        let t = token(r#"{"access_token":"tok","expires_in":60}"#);
        let mut req = ServiceRequest::new(RequestMethod::Get, "https://api.example.is/me", &[]).prepared_url_request().unwrap();
        t.sign(&mut req, NOW).unwrap();
        assert!(req.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer tok"));
        assert_eq!(t.sign(&mut req, NOW + 60), Err(OAuth2Error::Expired));
    }

    #[test]
    fn the_debug_text_hides_the_tokens() {
        let t = token(r#"{"access_token":"secret-value","refresh_token":"r-secret"}"#);
        let shown = format!("{t:?}");
        assert!(!shown.contains("secret-value") && !shown.contains("r-secret"), "{shown}");
    }
}

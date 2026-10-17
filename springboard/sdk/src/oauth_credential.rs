//! Typed OAuth credentials: the access token an app holds for a service, its refresh token, its expiry time and its
//! scope. This is the model behind `ACAccountCredential` for OAuth. An app keeps the credential as one opaque secret
//! in [`crate::keychain`]: [`OAuthCredential::encode`] writes it and [`OAuthCredential::decode`] reads it back, so
//! the keychain never sees the individual fields.
//!
//! Times are whole seconds since the Unix epoch. A credential with no expiry never expires. Renewal is the app's own
//! network call: the app asks the service for a new access token with [`OAuthCredential::can_renew`] true, then
//! stores the answer with [`OAuthCredential::renewed`], which keeps the refresh token and scope when the service does
//! not send new ones.

/// Why a credential could not be built or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialError {
    /// The access token is empty.
    EmptyAccessToken,
    /// A token or scope entry contains a control character, or a scope entry contains a space.
    InvalidField(&'static str),
    /// The stored bytes are not a credential written by [`OAuthCredential::encode`].
    Malformed,
}

/// Header line of an encoded credential. A reader rejects anything else.
pub const ENCODED_HEADER: &str = "bifrost-oauth-v1";

/// One OAuth credential for one account and service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthCredential {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
    scope: Vec<String>,
}

fn check_token(value: &str, field: &'static str) -> Result<(), CredentialError> {
    if value.chars().any(|c| c.is_control()) {
        return Err(CredentialError::InvalidField(field));
    }
    Ok(())
}

impl OAuthCredential {
    /// Builds a credential. The access token must not be empty. No field may hold a control character, and scope
    /// entries may not contain spaces (the scope is written space-separated).
    pub fn new(
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<u64>,
        scope: &[&str],
    ) -> Result<OAuthCredential, CredentialError> {
        if access_token.is_empty() {
            return Err(CredentialError::EmptyAccessToken);
        }
        check_token(access_token, "access_token")?;
        if let Some(r) = refresh_token {
            check_token(r, "refresh_token")?;
        }
        let mut entries = Vec::with_capacity(scope.len());
        for s in scope {
            check_token(s, "scope")?;
            if s.contains(' ') || s.is_empty() {
                return Err(CredentialError::InvalidField("scope"));
            }
            entries.push(s.to_string());
        }
        Ok(OAuthCredential {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.filter(|r| !r.is_empty()).map(str::to_string),
            expires_at,
            scope: entries,
        })
    }

    /// The bearer token to send with a request.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// The token that asks the service for a new access token, when the service issued one.
    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }

    /// The expiry time in Unix seconds, or `None` when the token does not expire.
    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    /// The scope the service granted.
    pub fn scope(&self) -> &[String] {
        &self.scope
    }

    /// Whether the access token has expired at `now`.
    pub fn is_expired(&self, now: u64) -> bool {
        matches!(self.expires_at, Some(at) if at <= now)
    }

    /// Whether the access token expires within `margin` seconds of `now`, or has already. An app renews at this
    /// point, so a request sent just before the deadline does not fail.
    pub fn needs_renewal(&self, now: u64, margin: u64) -> bool {
        matches!(self.expires_at, Some(at) if at <= now.saturating_add(margin))
    }

    /// Whether a refresh token is held, so the app can renew without the user signing in again.
    pub fn can_renew(&self) -> bool {
        self.refresh_token.is_some()
    }

    /// The credential after a renewal: the new access token and expiry, the refresh token (the new one if the
    /// service sent one, otherwise the one held) and the scope held.
    pub fn renewed(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<u64>,
    ) -> Result<OAuthCredential, CredentialError> {
        let scope: Vec<&str> = self.scope.iter().map(String::as_str).collect();
        let refresh = refresh_token.or(self.refresh_token.as_deref());
        OAuthCredential::new(access_token, refresh, expires_at, &scope)
    }

    /// Writes the credential as the opaque secret the keychain stores: a header line, then one line per field.
    /// Fields that are absent are written as empty lines.
    pub fn encode(&self) -> Vec<u8> {
        let expires = self.expires_at.map(|t| t.to_string()).unwrap_or_default();
        format!(
            "{ENCODED_HEADER}\n{}\n{}\n{}\n{}\n",
            self.access_token,
            self.refresh_token.as_deref().unwrap_or(""),
            expires,
            self.scope.join(" ")
        )
        .into_bytes()
    }

    /// Reads a credential written by [`OAuthCredential::encode`]. Any other bytes are [`CredentialError::Malformed`].
    pub fn decode(bytes: &[u8]) -> Result<OAuthCredential, CredentialError> {
        let text = std::str::from_utf8(bytes).map_err(|_| CredentialError::Malformed)?;
        let mut lines = text.strip_suffix('\n').ok_or(CredentialError::Malformed)?.split('\n');
        if lines.next() != Some(ENCODED_HEADER) {
            return Err(CredentialError::Malformed);
        }
        let access = lines.next().ok_or(CredentialError::Malformed)?;
        let refresh = lines.next().ok_or(CredentialError::Malformed)?;
        let expires = lines.next().ok_or(CredentialError::Malformed)?;
        let scope = lines.next().ok_or(CredentialError::Malformed)?;
        if lines.next().is_some() {
            return Err(CredentialError::Malformed);
        }
        let expires_at = if expires.is_empty() {
            None
        } else {
            Some(expires.parse::<u64>().map_err(|_| CredentialError::Malformed)?)
        };
        let scope: Vec<&str> = scope.split(' ').filter(|s| !s.is_empty()).collect();
        let refresh = if refresh.is_empty() { None } else { Some(refresh) };
        OAuthCredential::new(access, refresh, expires_at, &scope).map_err(|_| CredentialError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OAuthCredential {
        OAuthCredential::new("acc-1", Some("ref-1"), Some(1_000), &["mail", "calendar"]).unwrap()
    }

    #[test]
    fn builds_and_reads_back_every_field() {
        let c = sample();
        assert_eq!(c.access_token(), "acc-1");
        assert_eq!(c.refresh_token(), Some("ref-1"));
        assert_eq!(c.expires_at(), Some(1_000));
        assert_eq!(c.scope(), &["mail".to_string(), "calendar".to_string()]);
        assert!(c.can_renew());
    }

    #[test]
    fn rejects_an_empty_access_token_and_bad_fields() {
        assert_eq!(OAuthCredential::new("", None, None, &[]), Err(CredentialError::EmptyAccessToken));
        assert_eq!(
            OAuthCredential::new("a\nb", None, None, &[]),
            Err(CredentialError::InvalidField("access_token"))
        );
        assert_eq!(
            OAuthCredential::new("a", Some("r\t"), None, &[]),
            Err(CredentialError::InvalidField("refresh_token"))
        );
        assert_eq!(OAuthCredential::new("a", None, None, &["two words"]), Err(CredentialError::InvalidField("scope")));
        assert_eq!(OAuthCredential::new("a", None, None, &[""]), Err(CredentialError::InvalidField("scope")));
    }

    #[test]
    fn an_empty_refresh_token_is_no_refresh_token() {
        let c = OAuthCredential::new("a", Some(""), None, &[]).unwrap();
        assert_eq!(c.refresh_token(), None);
        assert!(!c.can_renew());
    }

    #[test]
    fn expiry_and_renewal_window() {
        let c = sample();
        assert!(!c.is_expired(999));
        assert!(c.is_expired(1_000));
        assert!(c.needs_renewal(900, 100));
        assert!(!c.needs_renewal(899, 100));
        let forever = OAuthCredential::new("a", None, None, &[]).unwrap();
        assert!(!forever.is_expired(u64::MAX));
        assert!(!forever.needs_renewal(u64::MAX, u64::MAX));
    }

    #[test]
    fn renewal_keeps_the_refresh_token_and_scope_when_none_is_sent() {
        let c = sample();
        let r = c.renewed("acc-2", None, Some(5_000)).unwrap();
        assert_eq!(r.access_token(), "acc-2");
        assert_eq!(r.refresh_token(), Some("ref-1"));
        assert_eq!(r.expires_at(), Some(5_000));
        assert_eq!(r.scope(), c.scope());
        let rotated = c.renewed("acc-3", Some("ref-2"), None).unwrap();
        assert_eq!(rotated.refresh_token(), Some("ref-2"));
        assert_eq!(rotated.expires_at(), None);
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let c = sample();
        let bytes = c.encode();
        assert!(bytes.starts_with(ENCODED_HEADER.as_bytes()));
        assert_eq!(OAuthCredential::decode(&bytes), Ok(c));
        let bare = OAuthCredential::new("only", None, None, &[]).unwrap();
        assert_eq!(OAuthCredential::decode(&bare.encode()), Ok(bare));
    }

    #[test]
    fn decode_rejects_other_bytes() {
        assert_eq!(OAuthCredential::decode(b"nope"), Err(CredentialError::Malformed));
        assert_eq!(OAuthCredential::decode(&[0xff, 0xfe]), Err(CredentialError::Malformed));
        let mut extra = sample().encode();
        extra.extend_from_slice(b"x\n");
        assert_eq!(OAuthCredential::decode(&extra), Err(CredentialError::Malformed));
        assert_eq!(
            OAuthCredential::decode(b"bifrost-oauth-v1\nacc\n\nnot-a-number\n\n"),
            Err(CredentialError::Malformed)
        );
        assert_eq!(OAuthCredential::decode(b"bifrost-oauth-v1\n\n\n\n\n"), Err(CredentialError::Malformed));
    }
}

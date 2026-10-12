//! `URLCredential`, `URLCredentialStorage`, `URLProtectionSpace` and HTTP Basic challenge parity for the URL loading
//! system. Builds the `Authorization` header from a user name and password, parses the first challenge of a
//! `WWW-Authenticate` value, and keeps credentials per host, port and realm.
//!
//! Pure std with its own base64 encoder, no network. `Permanent` credentials are held in memory like session ones:
//! there is no keychain link, so nothing survives a restart (a gap, see `docs/parity/w3-cfnetwork.md`). Digest and
//! client-certificate or server-trust credentials are not modelled.

use crate::url::URLRequest;
use crate::url_components::URLComponents;

/// `URLCredential.Persistence`. `None` means the credential is not kept after the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialPersistence {
    None,
    ForSession,
    Permanent,
}

/// `URLCredential` for a user name and password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct URLCredential {
    pub user: String,
    pub password: String,
    pub persistence: CredentialPersistence,
}

impl URLCredential {
    /// `init(user:password:persistence:)`.
    pub fn new(user: &str, password: &str, persistence: CredentialPersistence) -> Self {
        URLCredential { user: user.to_string(), password: password.to_string(), persistence }
    }

    /// `hasPassword`: true when the password is not empty.
    pub fn has_password(&self) -> bool {
        !self.password.is_empty()
    }

    /// The `Authorization` value for HTTP Basic (RFC 7617): `Basic ` and base64 of `user:password`.
    pub fn basic_authorization(&self) -> String {
        format!("Basic {}", base64_encode(format!("{}:{}", self.user, self.password).as_bytes()))
    }

    /// Sets the `Authorization` header on a request, replacing an earlier one.
    pub fn apply_to(&self, request: &mut URLRequest) {
        request.set_header("Authorization", &self.basic_authorization());
    }
}

/// The first challenge of a `WWW-Authenticate` value. The scheme is lower case. Later challenges are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HTTPAuthChallenge {
    pub scheme: String,
    pub realm: Option<String>,
}

/// Parses a `WWW-Authenticate` value. Returns `None` when there is no scheme.
pub fn parse_www_authenticate(value: &str) -> Option<HTTPAuthChallenge> {
    let trimmed = value.trim();
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let scheme = &trimmed[..end];
    if scheme.is_empty() {
        return None;
    }
    let mut realm = None;
    for param in split_params(&trimmed[end..]) {
        if let Some((key, val)) = param.split_once('=') {
            if key.trim().eq_ignore_ascii_case("realm") {
                realm = Some(val.trim().trim_matches('"').to_string());
            }
        }
    }
    Some(HTTPAuthChallenge { scheme: scheme.to_ascii_lowercase(), realm })
}

/// Splits challenge parameters on commas that are outside quoted strings.
fn split_params(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// `URLProtectionSpace` for HTTP authentication: the host, port and realm a credential applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct URLProtectionSpace {
    pub host: String,
    pub port: u16,
    pub realm: Option<String>,
}

impl URLProtectionSpace {
    /// The space a challenge from `url` defines. The port defaults to 443 for https and 80 for other schemes.
    pub fn for_challenge(url: &str, challenge: &HTTPAuthChallenge) -> Option<Self> {
        let parts = URLComponents::from_string(url)?;
        let host = parts.host.as_deref()?.to_ascii_lowercase();
        let https = parts.scheme.as_deref().map_or(false, |s| s.eq_ignore_ascii_case("https"));
        let port = parts.port.unwrap_or(if https { 443 } else { 80 });
        Some(URLProtectionSpace { host, port, realm: challenge.realm.clone() })
    }
}

/// `URLCredentialStorage`: credentials for protection spaces, kept in the order they were added.
#[derive(Clone, Debug, Default)]
pub struct URLCredentialStorage {
    entries: Vec<(URLProtectionSpace, URLCredential)>,
}

impl URLCredentialStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// `setCredential(_:for:)`: stores a credential for a space, replacing one already there. A credential with
    /// `CredentialPersistence::None` is not kept. Returns whether it was stored.
    pub fn set_credential(&mut self, credential: URLCredential, space: URLProtectionSpace) -> bool {
        if credential.persistence == CredentialPersistence::None {
            return false;
        }
        self.entries.retain(|(s, _)| *s != space);
        self.entries.push((space, credential));
        true
    }

    /// `credential(for:)`: the credential stored for a space.
    pub fn credential(&self, space: &URLProtectionSpace) -> Option<&URLCredential> {
        self.entries.iter().find(|(s, _)| s == space).map(|(_, c)| c)
    }

    /// `removeCredential(_:for:)`. Returns whether a credential was removed.
    pub fn remove_credential(&mut self, space: &URLProtectionSpace) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(s, _)| s != space);
        self.entries.len() != before
    }

    /// The number of stored credentials.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no credential is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (RFC 4648) with `=` padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_TABLE[((n >> 18) & 63) as usize] as char);
        out.push(BASE64_TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { BASE64_TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { BASE64_TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn basic_authorization_is_the_rfc_7617_example() {
        let c = URLCredential::new("Aladdin", "open sesame", CredentialPersistence::ForSession);
        assert_eq!(c.basic_authorization(), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert!(c.has_password());
        assert!(!URLCredential::new("u", "", CredentialPersistence::None).has_password());
    }

    #[test]
    fn apply_to_replaces_an_existing_authorization_header() {
        let mut req = URLRequest::new("https://example.com/private");
        req.set_header("Authorization", "Bearer old");
        URLCredential::new("Aladdin", "open sesame", CredentialPersistence::ForSession).apply_to(&mut req);
        let values: Vec<&str> = req.headers.iter().filter(|(k, _)| k == "Authorization").map(|(_, v)| v.as_str()).collect();
        assert_eq!(values, vec!["Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="]);
    }

    #[test]
    fn challenge_realm_may_contain_a_comma_inside_quotes() {
        let c = parse_www_authenticate(r#"Basic realm="Secure, Area", charset="UTF-8""#).unwrap();
        assert_eq!(c.scheme, "basic");
        assert_eq!(c.realm.as_deref(), Some("Secure, Area"));
        let bearer = parse_www_authenticate("Bearer").unwrap();
        assert_eq!(bearer.scheme, "bearer");
        assert_eq!(bearer.realm, None);
        assert_eq!(parse_www_authenticate("   "), None);
    }

    #[test]
    fn protection_space_uses_scheme_default_ports_and_lower_case_host() {
        let chal = parse_www_authenticate(r#"Basic realm="Area""#).unwrap();
        let s = URLProtectionSpace::for_challenge("https://Example.com/a", &chal).unwrap();
        assert_eq!(s, URLProtectionSpace { host: "example.com".into(), port: 443, realm: Some("Area".into()) });
        let p = URLProtectionSpace::for_challenge("http://example.com/a", &chal).unwrap();
        assert_eq!(p.port, 80);
        let custom = URLProtectionSpace::for_challenge("https://example.com:8443/a", &chal).unwrap();
        assert_eq!(custom.port, 8443);
    }

    #[test]
    fn storage_keeps_one_credential_per_space_and_skips_none_persistence() {
        let chal = parse_www_authenticate(r#"Basic realm="Area""#).unwrap();
        let space = URLProtectionSpace::for_challenge("https://example.com/", &chal).unwrap();
        let mut store = URLCredentialStorage::new();
        assert!(store.is_empty());
        assert!(!store.set_credential(URLCredential::new("x", "y", CredentialPersistence::None), space.clone()));
        assert!(store.set_credential(URLCredential::new("ann", "1", CredentialPersistence::ForSession), space.clone()));
        assert!(store.set_credential(URLCredential::new("bob", "2", CredentialPersistence::Permanent), space.clone()));
        assert_eq!(store.len(), 1);
        assert_eq!(store.credential(&space).map(|c| c.user.as_str()), Some("bob"));
        let other_realm = URLProtectionSpace { realm: Some("Other".into()), ..space.clone() };
        assert_eq!(store.credential(&other_realm), None);
        assert!(store.remove_credential(&space));
        assert!(!store.remove_credential(&space));
    }
}

//! `URLCredential`, `URLCredentialStorage`, `URLProtectionSpace` and HTTP Basic challenge parity for the URL loading
//! system. Builds the `Authorization` header from a user name and password, parses the first challenge of a
//! `WWW-Authenticate` value, and keeps credentials per host, port and realm.
//!
//! Pure std with its own base64 encoder, no network. An authentication challenge ([`AuthChallenge`]) is answered with
//! an [`AuthDisposition`], the same four answers the platform's challenge delegate returns. `Permanent` and
//! `Synchronizable` credentials stay in memory; [`URLCredentialStorage::to_keychain_bytes`] writes them out as bytes
//! for the caller to store as a keychain item, and [`URLCredentialStorage::from_keychain_bytes`] reads them back after a
//! restart. Nothing here talks to the keychain or to other devices, so `Synchronizable` is kept on this device only.
//! Digest and client-certificate or server-trust credentials are not modelled.

use crate::url::URLRequest;
use crate::url_components::URLComponents;

/// `URLCredential.Persistence`. `None` means the credential is not kept after the request. `ForSession` lasts until
/// the process ends. `Permanent` and `Synchronizable` are kept for the caller to write to the keychain; `Synchronizable`
/// is also meant for the user's other devices, which this model does not reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialPersistence {
    None,
    ForSession,
    Permanent,
    Synchronizable,
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

/// The answer to an authentication challenge (`URLSession.AuthChallengeDisposition`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthDisposition {
    /// `useCredential`: answer the challenge with this credential.
    UseCredential(URLCredential),
    /// `performDefaultHandling`: let the platform handle the challenge, which asks the user.
    PerformDefaultHandling,
    /// `cancelAuthenticationChallenge`: cancel the whole request.
    CancelAuthenticationChallenge,
    /// `rejectProtectionSpace`: refuse this space and offer the next one.
    RejectProtectionSpace,
}

/// An authentication challenge from a server (`URLAuthenticationChallenge`): the space it protects, how many earlier
/// attempts failed, and the credential the system proposes for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthChallenge {
    pub space: URLProtectionSpace,
    pub previous_failure_count: u32,
    pub proposed: Option<URLCredential>,
}

impl AuthChallenge {
    /// A first challenge for `space`: no failed attempt and no proposed credential.
    pub fn new(space: URLProtectionSpace) -> Self {
        AuthChallenge { space, previous_failure_count: 0, proposed: None }
    }

    /// The answer this model gives. Only a first attempt is answered with a credential: the proposed one wins, then the
    /// one stored for the space. After a failed attempt, or when there is no credential, the answer is
    /// `PerformDefaultHandling`, so a password that was refused is not sent again.
    pub fn disposition(&self, storage: &URLCredentialStorage) -> AuthDisposition {
        if self.previous_failure_count > 0 {
            return AuthDisposition::PerformDefaultHandling;
        }
        let candidate = self.proposed.clone().or_else(|| storage.credential(&self.space).cloned());
        match candidate {
            Some(credential) => AuthDisposition::UseCredential(credential),
            None => AuthDisposition::PerformDefaultHandling,
        }
    }
}

/// Version byte of the keychain layout written by [`URLCredentialStorage::to_keychain_bytes`].
const KEYCHAIN_VERSION: u8 = 1;
const TAG_PERMANENT: u8 = 2;
const TAG_SYNCHRONIZABLE: u8 = 3;

impl URLCredentialStorage {
    /// The entries that outlive the process: `Permanent` and `Synchronizable` credentials, in the order they were added.
    pub fn persistent_entries(&self) -> Vec<(&URLProtectionSpace, &URLCredential)> {
        self.entries
            .iter()
            .filter(|(_, c)| matches!(c.persistence, CredentialPersistence::Permanent | CredentialPersistence::Synchronizable))
            .map(|(s, c)| (s, c))
            .collect()
    }

    /// The persistent entries as bytes for one keychain item: a version byte, the entry count (u32 big-endian), then per
    /// entry the host (length-prefixed), the port (u16 big-endian), the realm (a flag byte, then length-prefixed when
    /// set), the user and password (length-prefixed), and a persistence tag.
    pub fn to_keychain_bytes(&self) -> Vec<u8> {
        let entries = self.persistent_entries();
        let mut out = vec![KEYCHAIN_VERSION];
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (space, credential) in entries {
            push_text(&mut out, &space.host);
            out.extend_from_slice(&space.port.to_be_bytes());
            match &space.realm {
                Some(realm) => {
                    out.push(1);
                    push_text(&mut out, realm);
                }
                None => out.push(0),
            }
            push_text(&mut out, &credential.user);
            push_text(&mut out, &credential.password);
            out.push(match credential.persistence {
                CredentialPersistence::Synchronizable => TAG_SYNCHRONIZABLE,
                _ => TAG_PERMANENT,
            });
        }
        out
    }

    /// Reads bytes written by [`URLCredentialStorage::to_keychain_bytes`]. `None` for an unknown version, a truncated
    /// entry, a tag that is not a persistent one, or trailing bytes.
    pub fn from_keychain_bytes(bytes: &[u8]) -> Option<URLCredentialStorage> {
        let mut rest = bytes;
        if take_bytes(&mut rest, 1)?[0] != KEYCHAIN_VERSION {
            return None;
        }
        let count = u32::from_be_bytes(take_bytes(&mut rest, 4)?.try_into().ok()?);
        let mut storage = URLCredentialStorage::new();
        for _ in 0..count {
            let host = take_text(&mut rest)?;
            let port = u16::from_be_bytes(take_bytes(&mut rest, 2)?.try_into().ok()?);
            let realm = match take_bytes(&mut rest, 1)?[0] {
                0 => None,
                1 => Some(take_text(&mut rest)?),
                _ => return None,
            };
            let user = take_text(&mut rest)?;
            let password = take_text(&mut rest)?;
            let persistence = match take_bytes(&mut rest, 1)?[0] {
                TAG_PERMANENT => CredentialPersistence::Permanent,
                TAG_SYNCHRONIZABLE => CredentialPersistence::Synchronizable,
                _ => return None,
            };
            storage.entries.push((URLProtectionSpace { host, port, realm }, URLCredential { user, password, persistence }));
        }
        if rest.is_empty() { Some(storage) } else { None }
    }
}

fn push_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn take_bytes<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if rest.len() < n {
        return None;
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Some(head)
}

fn take_text(rest: &mut &[u8]) -> Option<String> {
    let len = u32::from_be_bytes(take_bytes(rest, 4)?.try_into().ok()?) as usize;
    String::from_utf8(take_bytes(rest, len)?.to_vec()).ok()
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

    #[test]
    fn keychain_bytes_match_the_hand_worked_layout() {
        let space = URLProtectionSpace { host: "a".into(), port: 443, realm: None };
        let mut store = URLCredentialStorage::new();
        assert!(store.set_credential(URLCredential::new("u", "p", CredentialPersistence::Permanent), space.clone()));
        // version 1, count 1, host "a" (len 1), port 443 = 0x01BB, no realm, user "u", password "p", Permanent tag 2.
        let bytes = store.to_keychain_bytes();
        assert_eq!(bytes, vec![1, 0, 0, 0, 1, 0, 0, 0, 1, 97, 1, 187, 0, 0, 0, 0, 1, 117, 0, 0, 0, 1, 112, 2]);
        let restored = URLCredentialStorage::from_keychain_bytes(&bytes).unwrap();
        assert_eq!(restored.credential(&space), Some(&URLCredential::new("u", "p", CredentialPersistence::Permanent)));
    }

    #[test]
    fn only_permanent_and_synchronizable_entries_are_written_to_the_keychain() {
        let a = URLProtectionSpace { host: "a.example".into(), port: 80, realm: Some("R".into()) };
        let b = URLProtectionSpace { host: "b.example".into(), port: 8443, realm: None };
        let c = URLProtectionSpace { host: "c.example".into(), port: 443, realm: None };
        let mut store = URLCredentialStorage::new();
        store.set_credential(URLCredential::new("x", "1", CredentialPersistence::ForSession), a.clone());
        store.set_credential(URLCredential::new("y", "2", CredentialPersistence::Synchronizable), b.clone());
        store.set_credential(URLCredential::new("z", "3", CredentialPersistence::Permanent), c.clone());
        assert_eq!(store.persistent_entries().len(), 2);
        let restored = URLCredentialStorage::from_keychain_bytes(&store.to_keychain_bytes()).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.credential(&a), None);
        assert_eq!(restored.credential(&b).map(|cred| cred.persistence), Some(CredentialPersistence::Synchronizable));
        assert_eq!(restored.credential(&c).map(|cred| cred.user.as_str()), Some("z"));
    }

    #[test]
    fn keychain_bytes_with_a_bad_version_truncation_tag_or_trailing_byte_are_refused() {
        let space = URLProtectionSpace { host: "a".into(), port: 443, realm: None };
        let mut store = URLCredentialStorage::new();
        store.set_credential(URLCredential::new("u", "p", CredentialPersistence::Permanent), space);
        let good = store.to_keychain_bytes();
        let mut wrong_version = good.clone();
        wrong_version[0] = 2;
        assert!(URLCredentialStorage::from_keychain_bytes(&wrong_version).is_none());
        assert!(URLCredentialStorage::from_keychain_bytes(&good[..good.len() - 1]).is_none());
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(URLCredentialStorage::from_keychain_bytes(&trailing).is_none());
        let mut session_tag = good.clone();
        *session_tag.last_mut().unwrap() = 1;
        assert!(URLCredentialStorage::from_keychain_bytes(&session_tag).is_none());
        assert_eq!(URLCredentialStorage::from_keychain_bytes(&[1, 0, 0, 0, 0]).map(|s| s.len()), Some(0));
    }

    #[test]
    fn a_first_challenge_uses_the_proposed_credential_then_the_stored_one() {
        let space = URLProtectionSpace { host: "a".into(), port: 443, realm: Some("R".into()) };
        let mut store = URLCredentialStorage::new();
        store.set_credential(URLCredential::new("stored", "s", CredentialPersistence::ForSession), space.clone());
        let mut challenge = AuthChallenge::new(space.clone());
        assert_eq!(
            challenge.disposition(&store),
            AuthDisposition::UseCredential(URLCredential::new("stored", "s", CredentialPersistence::ForSession))
        );
        challenge.proposed = Some(URLCredential::new("proposed", "p", CredentialPersistence::None));
        assert_eq!(
            challenge.disposition(&store),
            AuthDisposition::UseCredential(URLCredential::new("proposed", "p", CredentialPersistence::None))
        );
    }

    #[test]
    fn a_refused_password_is_not_sent_again_and_no_credential_means_default_handling() {
        let space = URLProtectionSpace { host: "a".into(), port: 443, realm: None };
        let mut store = URLCredentialStorage::new();
        store.set_credential(URLCredential::new("stored", "s", CredentialPersistence::ForSession), space.clone());
        let mut challenge = AuthChallenge::new(space.clone());
        challenge.previous_failure_count = 1;
        assert_eq!(challenge.disposition(&store), AuthDisposition::PerformDefaultHandling);
        let empty = URLCredentialStorage::new();
        assert_eq!(AuthChallenge::new(space).disposition(&empty), AuthDisposition::PerformDefaultHandling);
    }
}

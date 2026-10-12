//! Client data for a passkey ceremony: the JSON an authenticator signs to show that a registration or a sign-in came from
//! a given origin and answers a server challenge. This is the client-data part of the public-key credential flow, the
//! part the platform writes for a passkey request (its `challenge`, `relyingPartyIdentifier` and user fields).
//!
//! Pure model: no authenticator, no key and no network. An app supplies the challenge bytes and the relying party
//! identifier, the shell supplies the origin, and [`ClientData`] checks the origin, encodes the challenge with the
//! unpadded base64url alphabet and writes the JSON in the order the specification lists its members. Creating or
//! signing a passkey is not modelled, so this does not make a passkey usable yet (see `docs/parity/w5-theme-06.md`).

use std::fmt;

/// Which ceremony the client data belongs to: a registration (`webauthn.create`) or a sign-in (`webauthn.get`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ceremony {
    Create,
    Get,
}

impl Ceremony {
    /// The `type` member of the client data.
    pub fn type_name(self) -> &'static str {
        match self {
            Ceremony::Create => "webauthn.create",
            Ceremony::Get => "webauthn.get",
        }
    }
}

/// Why client data could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasskeyError {
    /// The origin is not https (or http on localhost), or it has a path, query, fragment, user info or a bad host or port.
    InvalidOrigin,
    /// The challenge has no bytes. A challenge is random data from the server.
    EmptyChallenge,
    /// The relying party identifier is neither the origin's host nor a parent domain of it.
    RelyingPartyMismatch,
}

impl fmt::Display for PasskeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            PasskeyError::InvalidOrigin => "the origin is not https (or http on localhost), or it has a path, query or user",
            PasskeyError::EmptyChallenge => "the challenge is empty",
            PasskeyError::RelyingPartyMismatch => "the relying party identifier does not match the origin",
        };
        f.write_str(text)
    }
}

/// The client data of one ceremony. `challenge` is the base64url text of the challenge bytes; `origin` is normalised
/// to lower case with no default port written out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientData {
    pub ceremony: Ceremony,
    pub challenge: String,
    pub origin: String,
    pub cross_origin: bool,
}

impl ClientData {
    /// Checks the origin and encodes the challenge. The origin must be `https://host[:port]`, or `http://localhost[:port]`.
    /// An empty challenge is refused before the origin is looked at.
    pub fn new(ceremony: Ceremony, challenge: &[u8], origin: &str, cross_origin: bool) -> Result<ClientData, PasskeyError> {
        if challenge.is_empty() {
            return Err(PasskeyError::EmptyChallenge);
        }
        let origin = normalize_origin(origin).ok_or(PasskeyError::InvalidOrigin)?;
        Ok(ClientData { ceremony, challenge: base64url_encode(challenge), origin, cross_origin })
    }

    /// The JSON the authenticator signs, with members in the order `type`, `challenge`, `origin`, `crossOrigin`.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"type":"{}","challenge":"{}","origin":"{}","crossOrigin":{}}}"#,
            self.ceremony.type_name(),
            self.challenge,
            self.origin,
            self.cross_origin
        )
    }

    /// Checks that `rp_id` is the origin's host or a parent domain of it. The relying party identifier decides which
    /// passkeys an origin may use, so a parent domain is allowed and an unrelated or longer name is not.
    pub fn check_rp_id(&self, rp_id: &str) -> Result<(), PasskeyError> {
        let rp = rp_id.to_ascii_lowercase();
        let host = host_of(&self.origin);
        if !rp.is_empty() && (host == rp || host.ends_with(&format!(".{}", rp))) {
            Ok(())
        } else {
            Err(PasskeyError::RelyingPartyMismatch)
        }
    }
}

/// `scheme://host[:port]` in lower case, or `None`. Only https is accepted, and http only for localhost. A path, query,
/// fragment, user info, IPv6 literal, empty host or a port that is not 1 to 65535 is refused.
fn normalize_origin(origin: &str) -> Option<String> {
    let (scheme, rest) = origin.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if rest.chars().any(|c| matches!(c, '/' | '?' | '#' | '@' | '[')) {
        return None;
    }
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().ok().filter(|n| *n != 0)?;
            (h, Some(port))
        }
        None => (rest, None),
    };
    let host = host.to_ascii_lowercase();
    let host_ok = !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if !host_ok {
        return None;
    }
    let scheme_ok = match scheme.as_str() {
        "https" => true,
        "http" => host == "localhost",
        _ => false,
    };
    if !scheme_ok {
        return None;
    }
    Some(match port {
        Some(p) => format!("{}://{}:{}", scheme, host, p),
        None => format!("{}://{}", scheme, host),
    })
}

/// The host of a normalised origin.
fn host_of(origin: &str) -> &str {
    let rest = origin.split_once("://").map_or("", |(_, r)| r);
    rest.split(':').next().unwrap_or("")
}

const BASE64URL_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url (RFC 4648 section 5) without `=` padding.
fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 4 + 2) / 3);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64URL_TABLE[((n >> 18) & 63) as usize] as char);
        out.push(BASE64URL_TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64URL_TABLE[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(BASE64URL_TABLE[(n & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_the_rfc_4648_vectors_and_uses_the_url_alphabet() {
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foobar"), "Zm9vYmFy");
        // 0xfb 0xff is "+/8=" in standard base64; the URL alphabet writes "-_8".
        assert_eq!(base64url_encode(&[0xfb, 0xff]), "-_8");
        // 0xff alone is "/w==" in standard base64; here "_w".
        assert_eq!(base64url_encode(&[0xff]), "_w");
    }

    #[test]
    fn client_data_json_has_the_members_in_specification_order() {
        let data = ClientData::new(Ceremony::Get, b"fo", "https://Example.com:8443", false).unwrap();
        assert_eq!(data.origin, "https://example.com:8443");
        assert_eq!(data.to_json(), r#"{"type":"webauthn.get","challenge":"Zm8","origin":"https://example.com:8443","crossOrigin":false}"#);
    }

    #[test]
    fn registration_flags_cross_origin_and_uses_the_create_type() {
        let data = ClientData::new(Ceremony::Create, b"x", "https://a.example", true).unwrap();
        let json = data.to_json();
        assert!(json.contains(r#""type":"webauthn.create""#));
        assert!(json.ends_with(r#""crossOrigin":true}"#));
    }

    #[test]
    fn only_https_or_localhost_http_origins_are_accepted() {
        let ok = |origin: &str| ClientData::new(Ceremony::Get, b"c", origin, false).is_ok();
        assert!(ok("https://a.example"));
        assert!(ok("http://localhost:8080"));
        assert!(!ok("http://a.example"));
        assert!(!ok("https://a.example/path"));
        assert!(!ok("https://a.example?q=1"));
        assert!(!ok("https://u@a.example"));
        assert!(!ok("https://a.example:0"));
        assert!(!ok("ftp://a.example"));
        assert_eq!(
            ClientData::new(Ceremony::Get, b"", "https://a.example", false),
            Err(PasskeyError::EmptyChallenge)
        );
        assert_eq!(
            ClientData::new(Ceremony::Get, b"c", "http://a.example", false),
            Err(PasskeyError::InvalidOrigin)
        );
    }

    #[test]
    fn relying_party_identifier_may_be_the_host_or_a_parent_domain() {
        let data = ClientData::new(Ceremony::Get, b"c", "https://login.example.com", false).unwrap();
        assert_eq!(data.check_rp_id("login.example.com"), Ok(()));
        assert_eq!(data.check_rp_id("example.com"), Ok(()));
        assert_eq!(data.check_rp_id("ample.com"), Err(PasskeyError::RelyingPartyMismatch));
        assert_eq!(data.check_rp_id("evil.com"), Err(PasskeyError::RelyingPartyMismatch));
        assert_eq!(data.check_rp_id(""), Err(PasskeyError::RelyingPartyMismatch));
    }
}

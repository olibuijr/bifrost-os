//! Passkey request inputs: the client data that a public-key credential registration or assertion signs.
//!
//! An app asks for a passkey with a random challenge from the relying party, and the client data records the operation,
//! the challenge (base64url, no padding), the origin the request came from and whether it crossed origins. The JSON
//! field names and order follow the public-key credential client data of the WebAuthn specification. This module
//! builds that JSON only. It creates no key pair, talks to no authenticator and sends nothing over a socket, so an app
//! can prepare the bytes and a later authenticator step signs them.

use crate::http_auth::base64_encode;
use std::fmt;

/// The shortest challenge the specification accepts (16 random bytes).
pub const MIN_CHALLENGE_BYTES: usize = 16;

/// Why client data could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The challenge has fewer than [`MIN_CHALLENGE_BYTES`] bytes.
    ChallengeTooShort,
    /// The origin is not `http://` or `https://` followed by a host with no path, spaces or quotes.
    OriginInvalid,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::ChallengeTooShort => "the challenge is shorter than 16 bytes",
            Error::OriginInvalid => "the origin is not a scheme and host",
        };
        f.write_str(text)
    }
}

/// Which request the client data is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Registration of a new passkey for the relying party (`webauthn.create`).
    Register,
    /// Assertion with an existing passkey (`webauthn.get`).
    Authenticate,
}

impl Operation {
    /// The `type` value written into the client data.
    pub fn type_name(self) -> &'static str {
        match self {
            Operation::Register => "webauthn.create",
            Operation::Authenticate => "webauthn.get",
        }
    }
}

/// URL-safe base64 (RFC 4648 section 5) without padding, the encoding the client data uses for the challenge.
pub fn base64url(data: &[u8]) -> String {
    base64_encode(data).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

/// Client data for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientData {
    pub operation: Operation,
    pub challenge: Vec<u8>,
    pub origin: String,
    /// True when the request comes from a frame on a different origin than the top page.
    pub cross_origin: bool,
}

impl ClientData {
    /// Client data for `operation`, checked for a long enough challenge and a plain origin.
    pub fn new(operation: Operation, challenge: &[u8], origin: &str) -> Result<ClientData, Error> {
        if challenge.len() < MIN_CHALLENGE_BYTES {
            return Err(Error::ChallengeTooShort);
        }
        if !valid_origin(origin) {
            return Err(Error::OriginInvalid);
        }
        Ok(ClientData { operation, challenge: challenge.to_vec(), origin: origin.to_string(), cross_origin: false })
    }

    /// The JSON the authenticator signs: `type`, `challenge`, `origin` and `crossOrigin`, in that order.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"type\":\"{}\",\"challenge\":\"{}\",\"origin\":\"{}\",\"crossOrigin\":{}}}",
            self.operation.type_name(),
            base64url(&self.challenge),
            self.origin,
            self.cross_origin
        )
    }
}

/// `http://` or `https://`, then a non-empty host with no path, whitespace, control characters or quotes. The
/// origin is written into JSON unescaped, so characters that would need escaping are rejected here.
fn valid_origin(origin: &str) -> bool {
    let host = match origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://")) {
        Some(host) => host,
        None => return false,
    };
    !host.is_empty()
        && !host.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '"' | '\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_drops_padding_and_uses_the_url_alphabet() {
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        // 0xfb 0xff is "+/8=" in standard base64; the URL alphabet gives "-_8".
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn sixteen_zero_bytes_encode_to_twenty_two_characters() {
        // 16 bytes = 128 bits = ceil(128 / 6) = 22 characters; 0x00 x 16 gives 22 'A'.
        assert_eq!(base64url(&[0u8; 16]), "A".repeat(22));
    }

    #[test]
    fn registration_json_has_the_fields_in_specification_order() {
        let data = ClientData::new(Operation::Register, &[0u8; 16], "https://bifrost.olibuijr.com").unwrap();
        let expected = format!(
            "{{\"type\":\"webauthn.create\",\"challenge\":\"{}\",\"origin\":\"https://bifrost.olibuijr.com\",\"crossOrigin\":false}}",
            "A".repeat(22)
        );
        assert_eq!(data.to_json(), expected);
    }

    #[test]
    fn assertion_uses_the_get_type_and_cross_origin_flag() {
        let mut data = ClientData::new(Operation::Authenticate, &[0xfb, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "http://localhost:8080").unwrap();
        data.cross_origin = true;
        let json = data.to_json();
        assert!(json.starts_with("{\"type\":\"webauthn.get\",\"challenge\":\"-_8"));
        assert!(json.ends_with("\"origin\":\"http://localhost:8080\",\"crossOrigin\":true}"));
    }

    #[test]
    fn a_fifteen_byte_challenge_is_refused() {
        assert_eq!(ClientData::new(Operation::Register, &[0u8; 15], "https://a.b"), Err(Error::ChallengeTooShort));
        assert!(ClientData::new(Operation::Register, &[0u8; 16], "https://a.b").is_ok());
    }

    #[test]
    fn origins_must_be_a_scheme_and_a_host_only() {
        assert_eq!(ClientData::new(Operation::Register, &[0u8; 16], "https://"), Err(Error::OriginInvalid));
        assert_eq!(ClientData::new(Operation::Register, &[0u8; 16], "ftp://x.is"), Err(Error::OriginInvalid));
        assert_eq!(ClientData::new(Operation::Register, &[0u8; 16], "https://a.is/path"), Err(Error::OriginInvalid));
        assert_eq!(ClientData::new(Operation::Register, &[0u8; 16], "https://a\"b.is"), Err(Error::OriginInvalid));
        assert!(ClientData::new(Operation::Register, &[0u8; 16], "http://localhost:8080").is_ok());
    }
}

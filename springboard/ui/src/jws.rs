//! The structure of a signed transaction in JWS Compact Serialization (`VerificationResult`, `VerificationError`): the
//! three base64url segments of `header.payload.signature`, the checks that need no key (segment count, encoding, the
//! header's algorithm and certificate chain, the length of an ES256 signature), and the verified or unverified result
//! that a signature check produces. BifrOSt has no P-256 verifier yet, so `verify_result` takes the outcome of that
//! check from its caller; this module never reports a signature as valid on its own. Pure: no key, clock or network.

/// Why a signed value failed verification (`VerificationResult.VerificationError`), plus one BifrOSt case for an
/// algorithm other than ES256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationError {
    /// The header or payload lacks a field needed to verify the signature (the algorithm, or the certificate chain).
    MissingRequiredProperties,
    /// A segment is not valid base64url, the token does not have three segments, or the signature is not 64 bytes.
    InvalidEncoding,
    /// The signature does not match the header and payload.
    InvalidSignature,
    /// The header names an algorithm other than ES256.
    UnsupportedAlgorithm,
}

/// A signed value after verification (`VerificationResult`): `Verified` when the signature checks, `Unverified` with the
/// reason when it does not. The payload is kept in both cases; only the verified one is safe to trust.
#[derive(Clone, Debug, PartialEq)]
pub enum VerificationResult<T> {
    Verified(T),
    Unverified(T, VerificationError),
}

impl<T> VerificationResult<T> {
    /// The payload if it verified (`payloadValue`).
    pub fn payload_value(&self) -> Option<&T> {
        match self {
            VerificationResult::Verified(v) => Some(v),
            VerificationResult::Unverified(_, _) => None,
        }
    }

    /// The payload whether or not it verified (`unsafePayloadValue`). Read it only to show or log the failure.
    pub fn unchecked_payload_value(&self) -> &T {
        match self {
            VerificationResult::Verified(v) | VerificationResult::Unverified(v, _) => v,
        }
    }

    pub fn is_verified(&self) -> bool {
        matches!(self, VerificationResult::Verified(_))
    }
}

/// The result of a signature check on `payload`: verified when `signature_ok` is true, otherwise unverified with
/// `InvalidSignature`. The caller computes `signature_ok` from the ES256 check over `Compact::signing_input`.
pub fn verify_result<T>(payload: T, signature_ok: bool) -> VerificationResult<T> {
    if signature_ok {
        VerificationResult::Verified(payload)
    } else {
        VerificationResult::Unverified(payload, VerificationError::InvalidSignature)
    }
}

/// The decoded segments of a compact token, with the bytes the signature covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compact {
    /// The decoded header JSON.
    pub header: Vec<u8>,
    /// The decoded payload JSON.
    pub payload: Vec<u8>,
    /// The raw 64-byte ES256 signature (r then s).
    pub signature: Vec<u8>,
    /// The ASCII bytes of `header.payload`, the input the signature covers.
    pub signing_input: Vec<u8>,
}

/// The value of one base64url digit (RFC 4648, without padding).
fn b64url_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

/// Decodes unpadded base64url. None for a character outside the alphabet or a length of 4n + 1, which no byte string
/// produces. Leftover bits at the end are dropped.
pub fn decode_b64url(s: &str) -> Option<Vec<u8>> {
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &c in s.as_bytes() {
        acc = (acc << 6) | b64url_value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1u32 << bits) - 1;
        }
    }
    Some(out)
}

/// The value of a string field in a flat JSON object: `"key": "value"`. Escapes are not decoded, which is enough for the
/// algorithm name in a token header. None when the key is missing or its value is not a string.
pub fn json_string_field(json: &[u8], key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let pos = json.windows(needle.len()).position(|w| w == needle.as_bytes())?;
    let mut i = pos + needle.len();
    while json.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
        i += 1;
    }
    if json.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    while json.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
        i += 1;
    }
    if json.get(i) != Some(&b'"') {
        return None;
    }
    let start = i + 1;
    let end = start + json[start..].iter().position(|&c| c == b'"')?;
    String::from_utf8(json[start..end].to_vec()).ok()
}

/// Whether the JSON contains the key at all.
fn json_has_key(json: &[u8], key: &str) -> bool {
    let needle = format!("\"{key}\"");
    json.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// Splits and checks a compact token without verifying its signature. Errors, in order: a token that does not have three
/// segments, a segment that is not base64url, an empty header or payload (`MissingRequiredProperties`), a signature that
/// is not 64 bytes, an algorithm other than ES256, and a header with no certificate chain (`x5c`).
pub fn parse_compact(token: &str) -> Result<Compact, VerificationError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(VerificationError::InvalidEncoding);
    }
    let header = decode_b64url(parts[0]).ok_or(VerificationError::InvalidEncoding)?;
    let payload = decode_b64url(parts[1]).ok_or(VerificationError::InvalidEncoding)?;
    let signature = decode_b64url(parts[2]).ok_or(VerificationError::InvalidEncoding)?;
    if header.is_empty() || payload.is_empty() {
        return Err(VerificationError::MissingRequiredProperties);
    }
    if signature.len() != 64 {
        return Err(VerificationError::InvalidEncoding);
    }
    match json_string_field(&header, "alg") {
        None => return Err(VerificationError::MissingRequiredProperties),
        Some(alg) if alg != "ES256" => return Err(VerificationError::UnsupportedAlgorithm),
        Some(_) => {}
    }
    if !json_has_key(&header, "x5c") {
        return Err(VerificationError::MissingRequiredProperties);
    }
    Ok(Compact {
        header,
        payload,
        signature,
        signing_input: format!("{}.{}", parts[0], parts[1]).into_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The base64url of `{"alg":"ES256"}`.
    const ES256_HEADER: &str = "eyJhbGciOiJFUzI1NiJ9";
    /// The base64url of `{}`.
    const EMPTY_OBJECT: &str = "e30";
    /// 64 zero bytes in unpadded base64url: 86 'A' characters.
    fn zero_signature() -> String {
        "A".repeat(86)
    }

    #[test]
    fn base64url_decodes_known_bytes() {
        assert_eq!(decode_b64url("SGk"), Some(b"Hi".to_vec()));
        assert_eq!(decode_b64url("SG"), Some(b"H".to_vec()));
        assert_eq!(decode_b64url(""), Some(Vec::new()));
        assert_eq!(decode_b64url(EMPTY_OBJECT), Some(b"{}".to_vec()));
    }

    #[test]
    fn base64url_rejects_standard_symbols_and_impossible_lengths() {
        assert_eq!(decode_b64url("a+b"), None);
        assert_eq!(decode_b64url("a/b"), None);
        assert_eq!(decode_b64url("A"), None);
        assert_eq!(decode_b64url("ABCDE"), None);
    }

    #[test]
    fn json_field_reads_a_string_value() {
        assert_eq!(json_string_field(br#"{"alg":"ES256"}"#, "alg"), Some("ES256".to_string()));
        assert_eq!(json_string_field(br#"{ "alg" : "ES256" }"#, "alg"), Some("ES256".to_string()));
        assert_eq!(json_string_field(br#"{"alg":"ES256"}"#, "x5c"), None);
        assert_eq!(json_string_field(br#"{"alg":5}"#, "alg"), None);
    }

    #[test]
    fn a_token_with_two_segments_is_an_encoding_error() {
        assert_eq!(parse_compact("abc.def"), Err(VerificationError::InvalidEncoding));
        assert_eq!(parse_compact("a.b.c.d"), Err(VerificationError::InvalidEncoding));
    }

    #[test]
    fn a_signature_that_is_not_64_bytes_is_an_encoding_error() {
        // "AAAA" decodes to 3 bytes.
        let token = format!("{ES256_HEADER}.{EMPTY_OBJECT}.AAAA");
        assert_eq!(parse_compact(&token), Err(VerificationError::InvalidEncoding));
    }

    #[test]
    fn a_header_without_a_certificate_chain_is_missing_properties() {
        let token = format!("{ES256_HEADER}.{EMPTY_OBJECT}.{}", zero_signature());
        assert_eq!(parse_compact(&token), Err(VerificationError::MissingRequiredProperties));
    }

    #[test]
    fn verify_result_keeps_the_payload_either_way() {
        let ok = verify_result(5u32, true);
        assert!(ok.is_verified());
        assert_eq!(ok.payload_value(), Some(&5));

        let bad = verify_result(5u32, false);
        assert_eq!(bad, VerificationResult::Unverified(5, VerificationError::InvalidSignature));
        assert_eq!(bad.payload_value(), None);
        assert_eq!(bad.unchecked_payload_value(), &5);
    }
}

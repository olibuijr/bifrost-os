//! GSS-API token management (`gss_encapsulate_token`, `gss_decapsulate_token`): the framing that wraps a mechanism
//! token so that a peer can tell which mechanism produced it.
//!
//! An initial context token is the ASN.1 value `[APPLICATION 0] SEQUENCE { thisMech OBJECT IDENTIFIER, innerToken }`
//! (RFC 2743 section 3.1): tag 0x60, a DER length, the mechanism identifier, then the mechanism's own token with no
//! further framing. Lengths use the short form below 128 and the long form above it (0x80 | n, then n big-endian
//! octets). Decoding accepts only definite lengths, minimal long forms, and a body that exactly covers the input.
//!
//! Not here: the contents of the inner token, which belong to a mechanism, and the release calls, which drop replaces.

use crate::gss_oid::Oid;
use crate::gss_status::RoutineError;
use std::fmt;

/// The application tag that opens an initial context token (`[APPLICATION 0]`, constructed).
pub const TAG_INITIAL_CONTEXT: u8 = 0x60;
/// The universal tag of an OBJECT IDENTIFIER.
const TAG_OID: u8 = 0x06;

/// Why a token cannot be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The input ends before the value does.
    Truncated,
    /// The first tag is not the initial context token tag.
    BadTag,
    /// A length is indefinite, non-minimal, or does not match the input.
    BadLength,
    /// The mechanism field is not an object identifier, or is malformed.
    BadMechanism,
}

impl TokenError {
    /// The GSS routine error this failure reports: every token defect is `GSS_S_DEFECTIVE_TOKEN`.
    pub fn routine_error(self) -> RoutineError {
        RoutineError::DefectiveToken
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TokenError::Truncated => "token ends before its value",
            TokenError::BadTag => "token does not start with the initial context tag",
            TokenError::BadLength => "token length is indefinite, non-minimal or does not match",
            TokenError::BadMechanism => "token mechanism field is not a valid object identifier",
        })
    }
}

impl std::error::Error for TokenError {}

/// `gss_encapsulate_token`: wraps `inner` as an initial context token for `mech`.
pub fn encapsulate(mech: &Oid, inner: &[u8]) -> Vec<u8> {
    let mech_content = mech.to_der_content();
    let mut body = Vec::with_capacity(2 + mech_content.len() + inner.len());
    body.push(TAG_OID);
    push_length(&mut body, mech_content.len());
    body.extend_from_slice(&mech_content);
    body.extend_from_slice(inner);

    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(TAG_INITIAL_CONTEXT);
    push_length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

/// `gss_decapsulate_token`: returns the mechanism identifier and the inner token, borrowed from `token`.
pub fn decapsulate(token: &[u8]) -> Result<(Oid, &[u8]), TokenError> {
    let (&tag, rest) = token.split_first().ok_or(TokenError::Truncated)?;
    if tag != TAG_INITIAL_CONTEXT {
        return Err(TokenError::BadTag);
    }
    let (len, body) = read_length(rest)?;
    if body.len() < len {
        return Err(TokenError::Truncated);
    }
    if body.len() > len {
        return Err(TokenError::BadLength);
    }
    let (&oid_tag, after_tag) = body.split_first().ok_or(TokenError::Truncated)?;
    if oid_tag != TAG_OID {
        return Err(TokenError::BadMechanism);
    }
    let (oid_len, oid_body) = read_length(after_tag)?;
    if oid_body.len() < oid_len {
        return Err(TokenError::Truncated);
    }
    let (oid_content, inner) = oid_body.split_at(oid_len);
    let mech = Oid::from_der_content(oid_content).map_err(|_| TokenError::BadMechanism)?;
    Ok((mech, inner))
}

/// Appends a DER length: the short form below 128, otherwise 0x80 | n followed by the n significant octets.
fn push_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
        return;
    }
    let bytes = len.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count();
    let significant = &bytes[skip..];
    out.push(0x80 | significant.len() as u8);
    out.extend_from_slice(significant);
}

/// Reads a DER length and returns it with the bytes that follow it.
fn read_length(bytes: &[u8]) -> Result<(usize, &[u8]), TokenError> {
    let (&first, rest) = bytes.split_first().ok_or(TokenError::Truncated)?;
    if first < 0x80 {
        return Ok((first as usize, rest));
    }
    let count = (first & 0x7F) as usize;
    // Zero octets is the indefinite form, which this decoder refuses; more octets than a usize holds cannot be read.
    if count == 0 || count > std::mem::size_of::<usize>() {
        return Err(TokenError::BadLength);
    }
    if rest.len() < count {
        return Err(TokenError::Truncated);
    }
    let (digits, after) = rest.split_at(count);
    if digits[0] == 0 {
        return Err(TokenError::BadLength);
    }
    let len = digits.iter().fold(0usize, |acc, &b| (acc << 8) | b as usize);
    if len < 0x80 {
        // A value below 128 must use the short form.
        return Err(TokenError::BadLength);
    }
    Ok((len, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1.2.840.113554.1.2.2: content 2A 86 48 86 F7 12 01 02 02, so its DER element is 06 09 followed by that content.
    fn kerberos_v5() -> Oid {
        Oid::parse("1.2.840.113554.1.2.2").unwrap()
    }

    #[test]
    fn the_mechanism_identifier_has_the_worked_der_content() {
        assert_eq!(
            kerberos_v5().to_der_content(),
            vec![0x2A, 0x86, 0x48, 0x86, 0xF7, 0x12, 0x01, 0x02, 0x02]
        );
    }

    #[test]
    fn encapsulating_a_short_token_writes_the_worked_bytes() {
        // Body: OID element 11 bytes plus 2 inner bytes = 13 = 0x0D. Whole token: 2 + 13 = 15 bytes.
        let token = encapsulate(&kerberos_v5(), &[0x01, 0x02]);
        assert_eq!(
            token,
            vec![0x60, 0x0D, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x12, 0x01, 0x02, 0x02, 0x01, 0x02]
        );
    }

    #[test]
    fn an_empty_inner_token_gives_a_body_of_eleven() {
        let token = encapsulate(&kerberos_v5(), &[]);
        assert_eq!(&token[..2], &[0x60, 0x0B]);
        assert_eq!(token.len(), 13);
    }

    #[test]
    fn a_long_body_uses_the_two_octet_length_form() {
        // Body: 11 + 200 = 211 = 0xD3, which needs the long form 0x81 0xD3. Whole token: 1 + 2 + 211 = 214 bytes.
        let inner = vec![0xAA; 200];
        let token = encapsulate(&kerberos_v5(), &inner);
        assert_eq!(&token[..3], &[0x60, 0x81, 0xD3]);
        assert_eq!(token.len(), 214);
    }

    #[test]
    fn decapsulating_returns_the_mechanism_and_the_inner_token() {
        let token = encapsulate(&kerberos_v5(), &[0x01, 0x02]);
        let (mech, inner) = decapsulate(&token).unwrap();
        assert_eq!(mech.to_dotted(), "1.2.840.113554.1.2.2");
        assert_eq!(inner, &[0x01, 0x02]);
    }

    #[test]
    fn a_long_form_token_round_trips() {
        let inner = vec![0x5A; 300];
        let token = encapsulate(&kerberos_v5(), &inner);
        let (_, back) = decapsulate(&token).unwrap();
        assert_eq!(back, inner.as_slice());
    }

    #[test]
    fn a_wrong_outer_tag_is_refused() {
        let mut token = encapsulate(&kerberos_v5(), &[1]);
        token[0] = 0x61;
        assert_eq!(decapsulate(&token), Err(TokenError::BadTag));
    }

    #[test]
    fn a_cut_token_is_truncated_and_a_padded_one_is_a_bad_length() {
        let token = encapsulate(&kerberos_v5(), &[1, 2]);
        assert_eq!(decapsulate(&token[..token.len() - 1]), Err(TokenError::Truncated));
        let mut padded = token.clone();
        padded.push(0x00);
        assert_eq!(decapsulate(&padded), Err(TokenError::BadLength));
        assert_eq!(decapsulate(&[]), Err(TokenError::Truncated));
    }

    #[test]
    fn a_mechanism_field_that_is_not_an_oid_is_refused() {
        // Body: tag 0x05 (NULL) where the identifier should be; length matches, so only the mechanism check fails.
        let token = vec![0x60, 0x02, 0x05, 0x00];
        assert_eq!(decapsulate(&token), Err(TokenError::BadMechanism));
        assert_eq!(TokenError::BadMechanism.routine_error(), RoutineError::DefectiveToken);
    }

    #[test]
    fn a_non_minimal_long_length_is_refused() {
        // 0x81 0x05 encodes 5 in the long form, which the short form must carry.
        let token = vec![0x60, 0x81, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decapsulate(&token), Err(TokenError::BadLength));
    }
}

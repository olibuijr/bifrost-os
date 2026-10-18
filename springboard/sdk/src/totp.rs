//! One-time verification codes (RFC 6238 time-based codes over RFC 4226 HOTP): the six-digit code a site asks for after
//! a password.
//!
//! The shared secret is the base32 text a site shows beside its QR code (RFC 4648 alphabet). Lowercase letters, spaces,
//! hyphens and padding are accepted, since sites print the secret in groups. Codes use HMAC-SHA1 and 30-second steps,
//! which is what almost every site uses; 6 to 8 digits are accepted.

use crate::social_account::hmac_sha1;
use std::fmt;

/// Length of one code's time step, in seconds.
pub const PERIOD: u64 = 30;
/// The usual code length.
pub const DIGITS: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The secret has no base32 letters or digits at all.
    Empty,
    /// A character outside the base32 alphabet.
    BadSecret(char),
    /// A digit count outside 6 to 8.
    Digits(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Empty => write!(f, "the secret is empty"),
            Error::BadSecret(c) => write!(f, "the secret has a character outside base32: {c:?}"),
            Error::Digits(n) => write!(f, "a code has 6 to 8 digits, not {n}"),
        }
    }
}

/// Decode a base32 secret (RFC 4648) into its key bytes.
pub fn decode_base32(text: &str) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    let mut any = false;
    for c in text.chars() {
        if matches!(c, ' ' | '-' | '=') {
            continue;
        }
        let v = match c.to_ascii_uppercase() {
            u @ 'A'..='Z' => u as u32 - 'A' as u32,
            u @ '2'..='7' => u as u32 - '2' as u32 + 26,
            _ => return Err(Error::BadSecret(c)),
        };
        any = true;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    if !any {
        return Err(Error::Empty);
    }
    Ok(out)
}

/// The HOTP code (RFC 4226) for a key and a counter.
pub fn hotp(key: &[u8], counter: u64, digits: u32) -> Result<String, Error> {
    if !(6..=8).contains(&digits) {
        return Err(Error::Digits(digits));
    }
    let mac = hmac_sha1(key, &counter.to_be_bytes());
    let offset = (mac[19] & 0x0f) as usize;
    let code = ((mac[offset] as u32 & 0x7f) << 24)
        | ((mac[offset + 1] as u32) << 16)
        | ((mac[offset + 2] as u32) << 8)
        | mac[offset + 3] as u32;
    Ok(format!("{:0width$}", code % 10u32.pow(digits), width = digits as usize))
}

/// The code a base32 secret gives at a Unix time (seconds).
pub fn code_at(secret: &str, unix_time: u64, digits: u32) -> Result<String, Error> {
    code_at_with(secret, unix_time, digits, PERIOD)
}

/// The code a base32 secret gives at a Unix time (seconds) for a step of `period` seconds. A zero period counts as one second.
pub fn code_at_with(secret: &str, unix_time: u64, digits: u32, period: u64) -> Result<String, Error> {
    let key = decode_base32(secret)?;
    hotp(&key, unix_time / period.max(1), digits)
}

/// Seconds until the current code changes, for a countdown beside the code.
pub fn seconds_left(unix_time: u64) -> u64 {
    PERIOD - unix_time % PERIOD
}

/// The settings of one time-based code, as a site's `otpauth://totp/` URI gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Otp {
    /// The base32 secret, as written in the URI.
    pub secret: String,
    /// Digits in a code, 6 to 8.
    pub digits: u32,
    /// Seconds per code, at least 1.
    pub period: u64,
}

impl Otp {
    /// The code at a Unix time (seconds).
    pub fn code_at(&self, unix_time: u64) -> Result<String, Error> {
        code_at_with(&self.secret, unix_time, self.digits, self.period)
    }

    /// Seconds until this code changes.
    pub fn seconds_left(&self, unix_time: u64) -> u64 {
        self.period - unix_time % self.period
    }
}

/// Reads `otpauth://totp/LABEL?secret=...&digits=6&period=30&algorithm=SHA1`, the URI a site's QR code encodes.
/// Only time-based codes with SHA1 are computed; the label and issuer are not needed to make the code, so they are
/// ignored. `None` for another scheme or type, a missing or malformed secret, a digit count outside 6 to 8, a zero
/// period, or another algorithm.
pub fn parse_otpauth(uri: &str) -> Option<Otp> {
    let query = uri.trim().strip_prefix("otpauth://totp/")?.split_once('?')?.1;
    let mut secret = None;
    let mut digits = DIGITS;
    let mut period = PERIOD;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(value.to_string()),
            "digits" => digits = value.parse().ok()?,
            "period" => period = value.parse().ok()?,
            "algorithm" => {
                if !value.eq_ignore_ascii_case("SHA1") {
                    return None;
                }
            }
            _ => {}
        }
    }
    let secret = secret.filter(|s| !s.is_empty())?;
    if !(6..=8).contains(&digits) || period == 0 {
        return None;
    }
    decode_base32(&secret).ok()?;
    Some(Otp { secret, digits, period })
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 Appendix B: the ASCII key "12345678901234567890", SHA-1.
    const SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn decodes_base32_secrets_as_printed() {
        assert_eq!(decode_base32(SECRET).unwrap(), b"12345678901234567890");
        assert_eq!(decode_base32("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap(), b"12345678901234567890");
        assert_eq!(decode_base32("GEZDGNBVGY3TQOJQ====").unwrap(), b"1234567890");
        assert_eq!(decode_base32("   "), Err(Error::Empty));
        assert_eq!(decode_base32("GE1"), Err(Error::BadSecret('1')));
    }

    #[test]
    fn hotp_matches_the_rfc_4226_vectors() {
        let key = b"12345678901234567890";
        let codes = ["755224", "287082", "359152", "969429", "338314"];
        for (counter, want) in codes.iter().enumerate() {
            assert_eq!(hotp(key, counter as u64, 6).unwrap(), *want);
        }
    }

    #[test]
    fn totp_matches_the_rfc_6238_sha1_vectors() {
        // Eight-digit values from the RFC table; six digits are their last six.
        assert_eq!(code_at(SECRET, 59, 8).unwrap(), "94287082");
        assert_eq!(code_at(SECRET, 1_111_111_109, 8).unwrap(), "07081804");
        assert_eq!(code_at(SECRET, 1_111_111_111, 8).unwrap(), "14050471");
        assert_eq!(code_at(SECRET, 1_234_567_890, 8).unwrap(), "89005924");
        assert_eq!(code_at(SECRET, 2_000_000_000, 8).unwrap(), "69279037");
        assert_eq!(code_at(SECRET, 59, DIGITS).unwrap(), "287082");
        assert_eq!(code_at(SECRET, 1_111_111_109, DIGITS).unwrap(), "081804");
    }

    #[test]
    fn a_code_holds_for_its_whole_thirty_second_step() {
        assert_eq!(code_at(SECRET, 60, DIGITS).unwrap(), code_at(SECRET, 89, DIGITS).unwrap());
        assert_ne!(code_at(SECRET, 89, DIGITS).unwrap(), code_at(SECRET, 90, DIGITS).unwrap());
        assert_eq!(seconds_left(60), 30);
        assert_eq!(seconds_left(89), 1);
    }

    #[test]
    fn rejects_digit_counts_outside_six_to_eight() {
        assert_eq!(hotp(b"k", 0, 5), Err(Error::Digits(5)));
        assert_eq!(code_at(SECRET, 59, 9), Err(Error::Digits(9)));
    }

    #[test]
    fn parses_an_otpauth_uri_with_the_default_settings() {
        let uri = format!("otpauth://totp/Example:alice?secret={SECRET}&issuer=Example");
        let otp = parse_otpauth(&uri).unwrap();
        assert_eq!(otp, Otp { secret: SECRET.to_string(), digits: DIGITS, period: PERIOD });
        assert_eq!(otp.code_at(59), Ok("287082".to_string()));
        assert_eq!(otp.seconds_left(89), 1);
    }

    #[test]
    fn reads_digits_period_and_algorithm_from_the_uri() {
        let uri = format!("otpauth://totp/x?secret={SECRET}&digits=8&period=60&algorithm=sha1");
        let otp = parse_otpauth(&uri).unwrap();
        assert_eq!((otp.digits, otp.period), (8, 60));
        assert_eq!(otp.code_at(59), Ok("94287082".to_string()));
        assert_eq!(otp.seconds_left(60), 60);
        assert_eq!(code_at_with(SECRET, 59, 8, 60), Ok("94287082".to_string()));
    }

    #[test]
    fn refuses_what_it_cannot_compute() {
        assert_eq!(parse_otpauth("https://example.com/?secret=GEZDGNBVGY3TQOJQ"), None);
        assert_eq!(parse_otpauth("otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ&counter=0"), None);
        assert_eq!(parse_otpauth("otpauth://totp/x"), None);
        assert_eq!(parse_otpauth("otpauth://totp/x?secret="), None);
        assert_eq!(parse_otpauth("otpauth://totp/x?secret=GE1"), None);
        assert_eq!(parse_otpauth("otpauth://totp/x?secret=GEZ&digits=5"), None);
        assert_eq!(parse_otpauth("otpauth://totp/x?secret=GEZ&digits=six"), None);
        assert_eq!(parse_otpauth("otpauth://totp/x?secret=GEZ&period=0"), None);
        assert_eq!(parse_otpauth("otpauth://totp/x?secret=GEZ&algorithm=SHA256"), None);
    }
}

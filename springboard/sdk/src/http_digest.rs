//! HTTP Digest authentication (RFC 2617 and RFC 7616, algorithm MD5, quality of protection `auth`): reads a
//! `WWW-Authenticate: Digest` challenge and builds the `Authorization` header that answers it.
//!
//! The model is pure. The caller supplies the client nonce and the nonce count, so the header is the same for the same
//! inputs and no random source is needed here. Challenges that ask for another algorithm (such as `MD5-sess` or
//! `SHA-256`) or offer only `auth-int` are refused, because this model does not compute them. Digest credentials are
//! not stored by [`crate::http_auth::URLCredentialStorage`]; a caller that keeps them does so itself.

use crate::digest::insecure::md5;
use crate::http_auth::split_params;

/// A parsed Digest challenge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    /// True when the server offers the `auth` quality of protection, which is the one answered here.
    pub qop_auth: bool,
}

/// What the client supplies to answer a challenge.
#[derive(Clone, Copy, Debug)]
pub struct DigestAnswer<'a> {
    pub user: &'a str,
    pub password: &'a str,
    /// The request method, such as `GET`.
    pub method: &'a str,
    /// The request target, as it appears on the request line.
    pub uri: &'a str,
    /// The client nonce chosen for this request.
    pub cnonce: &'a str,
    /// How many times this server nonce has been used, starting at 1.
    pub nonce_count: u32,
}

/// Parses a `WWW-Authenticate` value that starts with the Digest scheme. Returns `None` for another scheme, a challenge
/// without a realm or nonce, an algorithm other than MD5, or a quality of protection list without `auth`.
pub fn parse_digest_challenge(value: &str) -> Option<DigestChallenge> {
    let trimmed = value.trim();
    let scheme = trimmed.get(..6)?;
    let params = trimmed.get(6..)?;
    if !scheme.eq_ignore_ascii_case("digest") || !params.starts_with(char::is_whitespace) {
        return None;
    }
    let mut realm = None;
    let mut nonce = None;
    let mut opaque = None;
    let mut qop = None;
    let mut algorithm = None;
    for param in split_params(params) {
        let Some((key, val)) = param.split_once('=') else { continue };
        let val = unquote(val);
        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => realm = Some(val),
            "nonce" => nonce = Some(val),
            "opaque" => opaque = Some(val),
            "qop" => qop = Some(val),
            "algorithm" => algorithm = Some(val),
            _ => {}
        }
    }
    if algorithm.is_some_and(|a| !a.eq_ignore_ascii_case("md5")) {
        return None;
    }
    let qop_auth = match qop {
        None => false,
        Some(offered) => {
            if !offered.split(',').any(|t| t.trim().eq_ignore_ascii_case("auth")) {
                return None;
            }
            true
        }
    };
    Some(DigestChallenge { realm: realm?, nonce: nonce?, opaque, qop_auth })
}

/// Strips one pair of surrounding double quotes, and the backslash escapes inside them.
fn unquote(raw: &str) -> String {
    let inner = raw.trim();
    let inner = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(inner);
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Escapes a value for a quoted-string in the header.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The request digest of RFC 2617 section 3.2.2.1: with `auth`, the nonce count and client nonce are part of it.
pub fn response_digest(challenge: &DigestChallenge, answer: &DigestAnswer) -> String {
    let ha1 = hex(&md5(format!("{}:{}:{}", answer.user, challenge.realm, answer.password).as_bytes()));
    let ha2 = hex(&md5(format!("{}:{}", answer.method, answer.uri).as_bytes()));
    let text = if challenge.qop_auth {
        format!("{ha1}:{}:{:08x}:{}:auth:{ha2}", challenge.nonce, answer.nonce_count, answer.cnonce)
    } else {
        format!("{ha1}:{}:{ha2}", challenge.nonce)
    };
    hex(&md5(text.as_bytes()))
}

/// The `Authorization` header value that answers the challenge for one request.
pub fn authorization(challenge: &DigestChallenge, answer: &DigestAnswer) -> String {
    let mut header = format!(
        "Digest username={}, realm={}, nonce={}, uri={}, algorithm=MD5, response=\"{}\"",
        quote(answer.user),
        quote(&challenge.realm),
        quote(&challenge.nonce),
        quote(answer.uri),
        response_digest(challenge, answer)
    );
    if challenge.qop_auth {
        header.push_str(&format!(", qop=auth, nc={:08x}, cnonce={}", answer.nonce_count, quote(answer.cnonce)));
    }
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque={}", quote(opaque)));
    }
    header
}

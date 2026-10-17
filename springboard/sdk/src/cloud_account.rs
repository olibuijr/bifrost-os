//! Account sign-in against the BifrOSt Cloud HTTP API: the request an app sends to sign in, the answer it reads back, and the
//! session cookie that answer grants, plus the request that ends the session.
//!
//! The protocol is the cloud service's own: `POST {base}/signin` with a JSON email and password answers `200` with the
//! account as `{"account": {...}}` and a `bc_session` cookie; a refused request answers with `{"error": "<code>"}`.
//! Everything here is pure: the requests are built and the answers are read with no socket. [`sign_in`] takes the
//! transport as a closure, so the walk is tested with canned answers; [`session_send`] is the URL session transport.
//! Each call blocks: run it on a background thread.

use crate::keychain_groups;
use crate::url::{URLError, URLRequest, URLSession};
use crate::Ctx;
use bifrost_core::json::{self, Json};
use std::fmt;
use std::sync::atomic::AtomicBool;

pub use springboard::cloud_session::Session;
use springboard::cloud_session::{ACCOUNT, GROUP, SERVICE};

/// The cookie the cloud service sets for a web session.
pub const SESSION_COOKIE: &str = "bc_session";

/// Largest email address the service accepts, in bytes.
pub const MAX_EMAIL_BYTES: usize = 254;

/// Largest answer read from the service, in bytes. Longer answers are refused as malformed.
pub const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// Why a sign-in or sign-out did not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudError {
    /// The email is not an address the service accepts.
    InvalidEmail,
    /// The password is empty.
    EmptyPassword,
    /// The base address is not an `https` URL with no spaces.
    BadBaseURL(String),
    /// The session token is empty or holds a character that cannot sit in a cookie value.
    InvalidSession,
    /// The email and password do not match an account.
    BadCredentials,
    /// The address made too many failed attempts and is locked out for a while.
    RateLimited,
    /// The service refused the request with a status and an error code (empty when the answer has none).
    Rejected { status: u16, code: String },
    /// The service answered, but the answer is not the one the protocol describes.
    MalformedAnswer(String),
    /// The request never reached the service.
    Transport(String),
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloudError::InvalidEmail => f.write_str("The email address is not valid"),
            CloudError::EmptyPassword => f.write_str("The password is empty"),
            CloudError::BadBaseURL(s) => write!(f, "The cloud address must be an https URL: {s}"),
            CloudError::InvalidSession => f.write_str("The session token is not valid"),
            CloudError::BadCredentials => f.write_str("The email or password is wrong"),
            CloudError::RateLimited => f.write_str("Too many attempts; try again later"),
            CloudError::Rejected { status, code } if code.is_empty() => write!(f, "The cloud service answered {status}"),
            CloudError::Rejected { status, code } => write!(f, "The cloud service answered {status}: {code}"),
            CloudError::MalformedAnswer(s) => write!(f, "The cloud service sent an answer that is not valid: {s}"),
            CloudError::Transport(s) => write!(f, "The cloud service could not be reached: {s}"),
        }
    }
}

impl std::error::Error for CloudError {}

/// The account the service describes (`account` in a sign-in answer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudAccount {
    pub id: String,
    pub email: String,
    pub name: String,
    pub avatar: String,
    /// Seconds since the Unix epoch when the account was made.
    pub created_at: i64,
    pub is_admin: bool,
}

/// A signed-in account and the session token its cookie carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub account: CloudAccount,
    pub session_token: String,
}

/// What a transport answers: the status, the first `Set-Cookie` header if there is one, and the body text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub set_cookie: Option<String>,
    pub body: String,
}

/// A transport: sends one request and returns its answer.
pub type Transport<'a> = dyn FnMut(&URLRequest) -> Result<Reply, String> + 'a;

/// Checks an email address the way the service does: trimmed, lowercased, at most [`MAX_EMAIL_BYTES`], one `@` with a
/// local part, a domain containing a dot, and no whitespace inside.
pub fn normalize_email(raw: &str) -> Result<String, CloudError> {
    let email = raw.trim().to_lowercase();
    let well_formed = email.len() <= MAX_EMAIL_BYTES
        && email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
        && !email.contains(char::is_whitespace);
    if well_formed { Ok(email) } else { Err(CloudError::InvalidEmail) }
}

/// The credentials of one sign-in. The password is kept out of `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct SignIn {
    email: String,
    password: String,
}

impl fmt::Debug for SignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignIn").field("email", &self.email).field("password", &"<redacted>").finish()
    }
}

impl SignIn {
    /// Checks the email and requires a password. The email is stored normalised.
    pub fn new(email: &str, password: &str) -> Result<SignIn, CloudError> {
        let email = normalize_email(email)?;
        if password.is_empty() {
            return Err(CloudError::EmptyPassword);
        }
        Ok(SignIn { email, password: password.to_string() })
    }

    /// The normalised email address.
    pub fn email(&self) -> &str {
        &self.email
    }

    /// The `POST {base}/signin` request with the email and password as a JSON object.
    pub fn request(&self, base_url: &str) -> Result<URLRequest, CloudError> {
        let body = Json::Obj(vec![("email".into(), Json::Str(self.email.clone())), ("password".into(), Json::Str(self.password.clone()))]);
        let mut req = URLRequest::post(&endpoint(base_url, "signin")?, body.encode().into_bytes());
        req.set_header("Content-Type", "application/json");
        req.set_header("Accept", "application/json");
        Ok(req)
    }
}

/// The `POST {base}/signout` request that ends the session carried by `session_token`.
pub fn sign_out_request(base_url: &str, session_token: &str) -> Result<URLRequest, CloudError> {
    let token = checked_token(session_token)?;
    let mut req = URLRequest::post(&endpoint(base_url, "signout")?, Vec::new());
    req.set_header("Cookie", &format!("{SESSION_COOKIE}={token}"));
    Ok(req)
}

/// Reads the session token from one `Set-Cookie` header. `None` when the header sets another cookie, sets an empty
/// value (a cookie that ends a session), or holds a value that is not letters and digits.
pub fn session_token(set_cookie: &str) -> Option<String> {
    let pair = set_cookie.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    if name.trim() != SESSION_COOKIE {
        return None;
    }
    checked_token(value.trim()).ok().map(str::to_string)
}

/// Reads the answer to a sign-in request. A `200` needs the account and a session cookie; any other status is mapped to
/// the matching [`CloudError`] using the `error` code in its body.
pub fn parse_sign_in(status: u16, set_cookie: Option<&str>, body: &str) -> Result<SignedIn, CloudError> {
    if body.len() > MAX_ANSWER_BYTES {
        return Err(CloudError::MalformedAnswer("the answer is too large".into()));
    }
    if status != 200 {
        return Err(failure(status, body));
    }
    let parsed = json::parse(body).map_err(CloudError::MalformedAnswer)?;
    let account = parsed.get("account").ok_or_else(|| CloudError::MalformedAnswer("no account".into()))?;
    let account = read_account(account)?;
    let session_token = set_cookie
        .and_then(session_token)
        .ok_or_else(|| CloudError::MalformedAnswer(format!("no {SESSION_COOKIE} cookie")))?;
    Ok(SignedIn { account, session_token })
}

/// Reads the answer to a sign-out request: `204` ends the session, anything else is an error.
pub fn parse_sign_out(status: u16, body: &str) -> Result<(), CloudError> {
    if status == 204 { Ok(()) } else { Err(failure(status, body)) }
}

/// Sends the sign-in request through `send` and reads the answer.
pub fn sign_in(base_url: &str, credentials: &SignIn, send: &mut Transport<'_>) -> Result<SignedIn, CloudError> {
    let req = credentials.request(base_url)?;
    let reply = send(&req).map_err(CloudError::Transport)?;
    parse_sign_in(reply.status, reply.set_cookie.as_deref(), &reply.body)
}

/// The URL session transport: blocking, reads the whole answer up to [`MAX_ANSWER_BYTES`] and keeps the first
/// `Set-Cookie` header.
pub fn session_send(req: &URLRequest) -> Result<Reply, String> {
    let cancel = AtomicBool::new(false);
    let mut body: Vec<u8> = Vec::new();
    let head = URLSession::shared()
        .bytes(
            req,
            &cancel,
            &mut |_: &crate::url::HTTPURLResponse| {},
            &mut |chunk: &[u8]| {
                body.extend_from_slice(chunk);
                body.len() <= MAX_ANSWER_BYTES
            },
        )
        .map_err(|e: URLError| e.to_string())?;
    Ok(Reply {
        status: head.status_code,
        set_cookie: head.value("Set-Cookie").map(str::to_string),
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Why the BifrOSt Account session could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// No session on this device: the user is signed out, or the stored value does not decode.
    SignedOut,
    /// The keychain refused the read (for example, this app is not granted the account group).
    Keychain(keychain_groups::Error),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::SignedOut => f.write_str("Sign in to BifrOSt Account in Settings"),
            SessionError::Keychain(e) => write!(f, "The BifrOSt Account session could not be read: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Asks the keychain for this device's BifrOSt Account session (the `is.bifrost.account` group). Answer:
/// [`decode_session`]. Only first-party apps that list the group in their manifest are granted it.
pub fn session(ctx: &Ctx) -> u32 {
    keychain_groups::get(ctx, GROUP, SERVICE, ACCOUNT)
}

/// Decodes the answer to [`session`]: the session, [`SessionError::SignedOut`] when there is no item, or the keychain's
/// refusal. The token is in the returned [`Session`] only; it is never printed.
pub fn decode_session(result: &Result<Json, String>) -> Result<Session, SessionError> {
    match keychain_groups::decode(result) {
        Ok(bytes) => Session::decode(&bytes).ok_or(SessionError::SignedOut),
        Err(keychain_groups::Error::ItemNotFound) => Err(SessionError::SignedOut),
        Err(e) => Err(SessionError::Keychain(e)),
    }
}

/// The lifetime of a `Set-Cookie` header as Unix seconds: `now` plus its `Max-Age`. `None` when the header has no
/// `Max-Age` or it is not a whole number of seconds.
pub fn cookie_expiry(set_cookie: &str, now: i64) -> Option<i64> {
    let secs: i64 = set_cookie.split(';').skip(1).map(str::trim).find_map(|attr| attr.strip_prefix("Max-Age=").or_else(|| attr.strip_prefix("max-age=")))?.parse::<i64>().ok()?;
    Some(now.saturating_add(secs))
}

#[cfg(test)]
mod session_tests {
    use super::*;

    fn answer(status: f64, hex_value: &str) -> Result<Json, String> {
        Ok(Json::Obj(vec![("status".into(), Json::Num(status)), ("value".into(), Json::Str(hex_value.into()))]))
    }

    fn hex_of(bytes: &[u8]) -> String {
        springboard::keychain::hex(bytes)
    }

    #[test]
    fn the_stored_session_decodes_from_the_group_answer() {
        let stored = Session::new("tok42", "acc9", "https://bifrost.olibuijr.com/api/cloud", Some(1_900_000_000)).unwrap();
        let got = decode_session(&answer(0.0, &hex_of(&stored.encode())));
        assert_eq!(got, Ok(stored));
    }

    #[test]
    fn no_item_is_signed_out_and_a_bad_value_is_signed_out() {
        assert_eq!(decode_session(&answer(-25300.0, "")), Err(SessionError::SignedOut));
        assert_eq!(decode_session(&answer(0.0, &hex_of(b"token=old"))), Err(SessionError::SignedOut));
    }

    #[test]
    fn a_refused_read_is_a_keychain_error_not_signed_out() {
        assert_eq!(decode_session(&answer(-25291.0, "")), Err(SessionError::Keychain(keychain_groups::Error::NotPermitted)));
        assert_eq!(decode_session(&Err("no service".into())), Err(SessionError::Keychain(keychain_groups::Error::Param)));
    }

    #[test]
    fn the_error_text_never_contains_a_token() {
        let stored = Session::new("secretvalue", "", "https://bifrost.olibuijr.com/api/cloud", None).unwrap();
        let shown = format!("{:?} {}", decode_session(&answer(0.0, &hex_of(&stored.encode()))), SessionError::SignedOut);
        assert!(!shown.contains("secretvalue"));
    }

    #[test]
    fn the_cookie_lifetime_is_read_from_max_age() {
        let header = "bc_session=abc123; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Lax";
        assert_eq!(cookie_expiry(header, 1_000), Some(1_000 + 2_592_000));
        assert_eq!(cookie_expiry("bc_session=abc123; Path=/", 1_000), None);
        assert_eq!(cookie_expiry("bc_session=abc123; Max-Age=soon", 1_000), None);
    }
}

pub(crate) fn endpoint(base_url: &str, path: &str) -> Result<String, CloudError> {
    let base = base_url.trim_end_matches('/');
    let valid = base.strip_prefix("https://").is_some_and(|host| !host.is_empty() && !base.contains(char::is_whitespace) && !host.starts_with('/'));
    if valid { Ok(format!("{base}/{path}")) } else { Err(CloudError::BadBaseURL(base_url.to_string())) }
}

pub(crate) fn checked_token(token: &str) -> Result<&str, CloudError> {
    if !token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric()) { Ok(token) } else { Err(CloudError::InvalidSession) }
}

pub(crate) fn failure(status: u16, body: &str) -> CloudError {
    let code = json::parse(body).ok().and_then(|j| j.str_field("error")).unwrap_or_default();
    let known = match (status, code.as_str()) {
        (401, "bad_credentials") => Some(CloudError::BadCredentials),
        (429, "rate_limited") => Some(CloudError::RateLimited),
        (400, "bad_email") => Some(CloudError::InvalidEmail),
        _ => None,
    };
    known.unwrap_or(CloudError::Rejected { status, code })
}

fn read_account(json: &Json) -> Result<CloudAccount, CloudError> {
    let text = |key: &str| -> Result<String, CloudError> {
        json.str_field(key).ok_or_else(|| CloudError::MalformedAnswer(format!("account has no {key}")))
    };
    let created_at = json
        .get("created_at")
        .and_then(Json::number)
        .filter(|n| n.is_finite() && n.fract() == 0.0)
        .ok_or_else(|| CloudError::MalformedAnswer("account has no created_at".into()))?;
    let is_admin = match json.get("is_admin") {
        Some(Json::Bool(b)) => *b,
        _ => return Err(CloudError::MalformedAnswer("account has no is_admin".into())),
    };
    let id = text("id")?;
    if id.is_empty() {
        return Err(CloudError::MalformedAnswer("account id is empty".into()));
    }
    Ok(CloudAccount {
        id,
        email: text("email")?,
        name: text("name")?,
        avatar: text("avatar")?,
        created_at: created_at as i64,
        is_admin,
    })
}

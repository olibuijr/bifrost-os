//! The BifrOSt Cloud account protocol, pure: the sign-in body, the reading of the sign-in and sign-out answers, the
//! session cookie that the answer grants, and the cookie's lifetime. No socket and no URL type, so the shell's Settings
//! page and the SDK (`bifrost_app::cloud_account`, which builds the requests and sends them) share one reading of it.
//!
//! The protocol is the cloud service's own: `POST {base}/signin` with a JSON email and password answers `200` with the
//! account as `{"account": {...}}` and a `bc_session` cookie; a refused request answers with `{"error": "<code>"}`.
//! Sign-out is `POST {base}/signout` with the cookie and answers `204`. The base is `https://bifrost.olibuijr.com/api/cloud/v1`.

use bifrost_core::json::{self, Json};

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

impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// Checks an email address the way the service does: trimmed, lowercased, at most [`MAX_EMAIL_BYTES`], one `@` with a
/// local part, a domain containing a dot, and no whitespace inside.
pub fn normalize_email(raw: &str) -> Result<String, CloudError> {
    let email = raw.trim().to_lowercase();
    let well_formed = email.len() <= MAX_EMAIL_BYTES
        && email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
        && !email.contains(char::is_whitespace);
    if well_formed { Ok(email) } else { Err(CloudError::InvalidEmail) }
}

/// The JSON body of a sign-in request: `{"email": .., "password": ..}`. The caller checks the email and the password.
pub fn sign_in_body(email: &str, password: &str) -> Vec<u8> {
    Json::Obj(vec![("email".into(), Json::Str(email.to_string())), ("password".into(), Json::Str(password.to_string()))]).encode().into_bytes()
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

/// The lifetime of a `Set-Cookie` header as Unix seconds: `now` plus its `Max-Age`. `None` when the header has no
/// `Max-Age` or it is not a whole number of seconds.
pub fn cookie_expiry(set_cookie: &str, now: i64) -> Option<i64> {
    let secs: i64 = set_cookie.split(';').skip(1).map(str::trim).find_map(|attr| attr.strip_prefix("Max-Age=").or_else(|| attr.strip_prefix("max-age=")))?.parse::<i64>().ok()?;
    Some(now.saturating_add(secs))
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

/// `{base}/{path}` for an `https` base; the trailing slash of the base is dropped.
pub fn endpoint(base_url: &str, path: &str) -> Result<String, CloudError> {
    let base = base_url.trim_end_matches('/');
    let valid = base.strip_prefix("https://").is_some_and(|host| !host.is_empty() && !base.contains(char::is_whitespace) && !host.starts_with('/'));
    if valid { Ok(format!("{base}/{path}")) } else { Err(CloudError::BadBaseURL(base_url.to_string())) }
}

/// The token if it is not empty and holds only letters and digits (what a cookie value may carry here).
pub fn checked_token(token: &str) -> Result<&str, CloudError> {
    if !token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric()) { Ok(token) } else { Err(CloudError::InvalidSession) }
}

/// The error for a refused request: the known codes map to their own variants, everything else is `Rejected`.
pub fn failure(status: u16, body: &str) -> CloudError {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn account_json(extra: &str) -> String {
        format!(r#"{{"account":{{"id":"acc1","email":"ada@example.com","name":"Ada","avatar":"","created_at":1700000000,"is_admin":false{extra}}}}}"#)
    }

    #[test]
    fn a_sign_in_answer_gives_the_account_and_the_cookie_token() {
        let got = parse_sign_in(200, Some("bc_session=tok123; Path=/; Max-Age=60; HttpOnly"), &account_json("")).unwrap();
        assert_eq!(got.session_token, "tok123");
        assert_eq!(got.account.email, "ada@example.com");
        assert_eq!(got.account.created_at, 1_700_000_000);
    }

    #[test]
    fn refusals_map_to_their_codes_and_a_missing_cookie_is_malformed() {
        assert_eq!(parse_sign_in(401, None, r#"{"error":"bad_credentials"}"#), Err(CloudError::BadCredentials));
        assert_eq!(parse_sign_in(429, None, r#"{"error":"rate_limited"}"#), Err(CloudError::RateLimited));
        assert_eq!(parse_sign_in(500, None, "x"), Err(CloudError::Rejected { status: 500, code: String::new() }));
        assert!(matches!(parse_sign_in(200, None, &account_json("")), Err(CloudError::MalformedAnswer(_))));
        assert!(matches!(parse_sign_in(200, Some("other=1"), &account_json("")), Err(CloudError::MalformedAnswer(_))));
    }

    #[test]
    fn the_sign_in_body_and_the_emails_are_checked() {
        let body = String::from_utf8(sign_in_body("ada@example.com", "pw")).unwrap();
        assert!(body.contains("\"email\"") && body.contains("\"password\""));
        assert_eq!(normalize_email(" Ada@Example.com "), Ok("ada@example.com".to_string()));
        assert_eq!(normalize_email("nope"), Err(CloudError::InvalidEmail));
    }

    #[test]
    fn sign_out_needs_204_and_the_endpoint_needs_https() {
        assert_eq!(parse_sign_out(204, ""), Ok(()));
        assert!(parse_sign_out(200, "").is_err());
        assert_eq!(endpoint("https://h.example/api/", "signin"), Ok("https://h.example/api/signin".to_string()));
        assert!(matches!(endpoint("http://h.example", "signin"), Err(CloudError::BadBaseURL(_))));
        assert_eq!(checked_token("a b"), Err(CloudError::InvalidSession));
    }

    #[test]
    fn the_cookie_lifetime_is_read_from_max_age() {
        let header = "bc_session=abc123; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Lax";
        assert_eq!(cookie_expiry(header, 1_000), Some(1_000 + 2_592_000));
        assert_eq!(cookie_expiry("bc_session=abc123; Path=/", 1_000), None);
        assert_eq!(cookie_expiry("bc_session=abc123; Max-Age=soon", 1_000), None);
    }
}

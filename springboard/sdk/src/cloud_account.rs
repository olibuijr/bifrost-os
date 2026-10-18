//! Account sign-in against the BifrOSt Cloud HTTP API, for apps: the requests an app sends to sign in and to end the
//! session, the URL transport that sends them, and the keychain read of the device's session.
//!
//! The protocol itself (the bodies, the answers, the cookie) is `springboard::cloud_account`, shared with the shell's
//! Settings page; this module re-exports it. Everything that builds a request is pure, so it is tested with no socket.
//! [`sign_in`] takes the transport as a closure, so the walk is tested with canned answers; [`session_send`] is the URL
//! session transport. Each call blocks: run it on a background thread.

use crate::keychain_groups;
use crate::url::{URLError, URLRequest, URLSession};
use crate::Ctx;
use bifrost_core::json::Json;
use std::fmt;
use std::sync::atomic::AtomicBool;

pub use springboard::cloud_account::{
    cookie_expiry, normalize_email, parse_sign_in, parse_sign_out, session_token, CloudAccount, CloudError, Reply, SignedIn, MAX_ANSWER_BYTES,
    MAX_EMAIL_BYTES, SESSION_COOKIE,
};
pub(crate) use springboard::cloud_account::{checked_token, endpoint, failure};
pub use springboard::cloud_session::Session;
use springboard::cloud_session::{ACCOUNT, GROUP, SERVICE};

/// A transport: sends one request and returns its answer.
pub type Transport<'a> = dyn FnMut(&URLRequest) -> Result<Reply, String> + 'a;

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
        let mut req = URLRequest::post(&endpoint(base_url, "signin")?, springboard::cloud_account::sign_in_body(&self.email, &self.password));
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
        let stored = Session::new("tok42", "acc9", "https://bifrost.olibuijr.com/api/cloud/v1", Some(1_900_000_000)).unwrap();
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
        let stored = Session::new("secretvalue", "", "https://bifrost.olibuijr.com/api/cloud/v1", None).unwrap();
        let shown = format!("{:?} {}", decode_session(&answer(0.0, &hex_of(&stored.encode()))), SessionError::SignedOut);
        assert!(!shown.contains("secretvalue"));
    }

    #[test]
    fn the_sign_out_request_carries_the_cookie_and_needs_a_clean_token() {
        let req = sign_out_request("https://bifrost.olibuijr.com/api/cloud/v1", "tok42").unwrap();
        assert!(req.url.ends_with("/signout"));
        assert!(sign_out_request("https://h.example", "bad token").is_err());
    }
}

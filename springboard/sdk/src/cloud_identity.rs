//! Cloud identity: the signed-in account of the BifrOSt cloud service as an app sees it (the user record name, the
//! display name and the sign-up address).
//!
//! The service answers `GET /api/cloud/v1/me` with `{"account": {...}, "session_id": "..."}` for a valid session and
//! with HTTP 401 otherwise. This module reads that answer: [`identity_from_me`] maps the status and the body to a
//! [`UserIdentity`] or a typed [`Error`]. It makes no request itself; the caller performs the request and passes the
//! status and body in, so the model stays headless. The service has no lookup-by-address route, so an app can learn only
//! the identity of the signed-in user, never another person's.

use bifrost_core::json::{self, Json};
use std::fmt;

/// The signed-in user as the cloud service describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserIdentity {
    /// The account id the service keeps for the user. It is never empty.
    pub user_record_name: String,
    /// The display name. Empty when the user has not set one.
    pub display_name: String,
    /// The address the user signed up with. It contains an `@`.
    pub lookup_email: String,
    pub is_admin: bool,
}

/// Why no identity could be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The service answered 401: there is no valid session.
    NotSignedIn,
    /// The service answered with another status.
    Unexpected { status: u16 },
    /// The answer is not the expected JSON, or a field is missing or empty. The text says which.
    Malformed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotSignedIn => f.write_str("no account is signed in to the cloud service"),
            Error::Unexpected { status } => write!(f, "the cloud service answered with status {status}"),
            Error::Malformed(reason) => write!(f, "the cloud identity answer is malformed: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// Reads the signed-in identity from the answer to `GET /me`: the HTTP status and the body.
pub fn identity_from_me(status: u16, body: &str) -> Result<UserIdentity, Error> {
    match status {
        200 => {}
        401 => return Err(Error::NotSignedIn),
        other => return Err(Error::Unexpected { status: other }),
    }
    let doc = json::parse(body).map_err(Error::Malformed)?;
    let account = doc.get("account").ok_or_else(|| Error::Malformed("no account object".into()))?;
    let text = |key: &str| -> Result<String, Error> {
        account
            .get(key)
            .and_then(Json::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Malformed(format!("account has no string field {key}")))
    };
    let user_record_name = text("id")?;
    let lookup_email = text("email")?;
    let display_name = text("name")?;
    if user_record_name.is_empty() {
        return Err(Error::Malformed("account id is empty".into()));
    }
    if !lookup_email.contains('@') {
        return Err(Error::Malformed("account email has no @".into()));
    }
    let is_admin = match account.get("is_admin") {
        Some(Json::Bool(admin)) => *admin,
        _ => return Err(Error::Malformed("account has no boolean is_admin".into())),
    };
    Ok(UserIdentity { user_record_name, display_name, lookup_email, is_admin })
}

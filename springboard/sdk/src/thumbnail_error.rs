//! Errors from thumbnail generation: a domain, a code for each kind of failure and a message for the log.
//!
//! The thumbnails service answers a failed picture with a message (see [`crate::thumbnails::State::Failed`]) and a
//! request that is refused gives a [`RequestError`]. [`ThumbnailError::from_state`] and
//! [`ThumbnailError::from_request`] turn those into one value with a stable code, so an app can tell a missing file from
//! a type that cannot be drawn. The codes are numbered from 1 and are part of the wire contract with the shell.

use crate::thumbnails::{RequestError, State};

/// The error domain every thumbnail error carries.
pub const DOMAIN: &str = "bifrost.thumbnails";

/// Why a thumbnail could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbnailErrorCode {
    /// The request had a bad path or size and was never sent.
    BadRequest = 1,
    /// The file does not exist or cannot be read.
    NoSuchFile = 2,
    /// The file's type has no picture and no icon could be drawn.
    Unsupported = 3,
    /// The request was cancelled before a picture arrived.
    Cancelled = 4,
    /// Any other failure while drawing.
    Failed = 5,
}

impl ThumbnailErrorCode {
    /// The numeric code (1 to 5), as a log field or a service reply carries it.
    pub fn raw(self) -> i64 {
        self as i64
    }

    /// The code for a numeric value, or None for a value that is not one of the codes.
    pub fn from_raw(raw: i64) -> Option<Self> {
        match raw {
            1 => Some(ThumbnailErrorCode::BadRequest),
            2 => Some(ThumbnailErrorCode::NoSuchFile),
            3 => Some(ThumbnailErrorCode::Unsupported),
            4 => Some(ThumbnailErrorCode::Cancelled),
            5 => Some(ThumbnailErrorCode::Failed),
            _ => None,
        }
    }

    /// A short name for the code, for logs.
    pub fn name(self) -> &'static str {
        match self {
            ThumbnailErrorCode::BadRequest => "bad-request",
            ThumbnailErrorCode::NoSuchFile => "no-such-file",
            ThumbnailErrorCode::Unsupported => "unsupported",
            ThumbnailErrorCode::Cancelled => "cancelled",
            ThumbnailErrorCode::Failed => "failed",
        }
    }
}

/// A thumbnail failure: its code and the message the shell gave, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailError {
    pub code: ThumbnailErrorCode,
    pub message: String,
}

impl ThumbnailError {
    pub fn new(code: ThumbnailErrorCode, message: impl Into<String>) -> Self {
        ThumbnailError { code, message: message.into() }
    }

    /// The error domain, the same for every thumbnail error.
    pub fn domain(&self) -> &'static str {
        DOMAIN
    }

    /// The failure a reply reports. None for a state that is not a failure.
    pub fn from_state(state: &State) -> Option<Self> {
        match state {
            State::Failed(message) => Some(ThumbnailError::new(classify(message), message.clone())),
            _ => None,
        }
    }

    /// The failure for a request the shell refused.
    pub fn from_request(error: &RequestError) -> Self {
        ThumbnailError::new(ThumbnailErrorCode::BadRequest, error.to_string())
    }
}

impl std::fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}: {}", DOMAIN, self.code.name())
        } else {
            write!(f, "{}: {} ({})", DOMAIN, self.code.name(), self.message)
        }
    }
}

impl std::error::Error for ThumbnailError {}

/// Sorts a shell failure message into a code. The shell words its messages for people, so this matches the common
/// phrases and falls back to [`ThumbnailErrorCode::Failed`].
pub fn classify(message: &str) -> ThumbnailErrorCode {
    let m = message.to_ascii_lowercase();
    if m.contains("cancel") {
        ThumbnailErrorCode::Cancelled
    } else if m.contains("no such file") || m.contains("not found") || m.contains("missing") || m.contains("gone") {
        ThumbnailErrorCode::NoSuchFile
    } else if m.contains("unsupported") || m.contains("cannot be drawn") || m.contains("no picture") {
        ThumbnailErrorCode::Unsupported
    } else {
        ThumbnailErrorCode::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_through_their_numbers() {
        for code in [
            ThumbnailErrorCode::BadRequest,
            ThumbnailErrorCode::NoSuchFile,
            ThumbnailErrorCode::Unsupported,
            ThumbnailErrorCode::Cancelled,
            ThumbnailErrorCode::Failed,
        ] {
            assert_eq!(ThumbnailErrorCode::from_raw(code.raw()), Some(code));
        }
        assert_eq!(ThumbnailErrorCode::BadRequest.raw(), 1);
        assert_eq!(ThumbnailErrorCode::Failed.raw(), 5);
        assert_eq!(ThumbnailErrorCode::from_raw(0), None);
        assert_eq!(ThumbnailErrorCode::from_raw(6), None);
    }

    #[test]
    fn the_shell_message_picks_the_code() {
        assert_eq!(classify("file not found: /a/b.pdf"), ThumbnailErrorCode::NoSuchFile);
        assert_eq!(classify("The file is gone"), ThumbnailErrorCode::NoSuchFile);
        assert_eq!(classify("type has no picture"), ThumbnailErrorCode::Unsupported);
        assert_eq!(classify("Cancelled by the caller"), ThumbnailErrorCode::Cancelled);
        assert_eq!(classify("decoder crashed"), ThumbnailErrorCode::Failed);
        assert_eq!(classify(""), ThumbnailErrorCode::Failed);
    }

    #[test]
    fn a_failed_state_becomes_an_error_and_other_states_do_not() {
        let failed = State::Failed("no such file".into());
        let e = ThumbnailError::from_state(&failed).unwrap();
        assert_eq!(e.code, ThumbnailErrorCode::NoSuchFile);
        assert_eq!(e.message, "no such file");
        assert_eq!(e.domain(), DOMAIN);
        assert_eq!(ThumbnailError::from_state(&State::Pending), None);
        assert_eq!(ThumbnailError::from_state(&State::Icon { type_id: "public.folder".into(), badge: None }), None);
    }

    #[test]
    fn a_refused_request_is_a_bad_request() {
        let e = ThumbnailError::from_request(&RequestError::BadSize);
        assert_eq!(e.code, ThumbnailErrorCode::BadRequest);
        assert!(e.message.contains("size"));
    }

    #[test]
    fn display_names_the_domain_and_the_code() {
        let e = ThumbnailError::new(ThumbnailErrorCode::Unsupported, "no picture");
        assert_eq!(e.to_string(), "bifrost.thumbnails: unsupported (no picture)");
        let bare = ThumbnailError::new(ThumbnailErrorCode::Cancelled, "");
        assert_eq!(bare.to_string(), "bifrost.thumbnails: cancelled");
    }
}

//! The Anthropic Messages client shared by the Assistant app (`apps/assistant-desktop`, streaming with tools) and the
//! shell (`src/shell/assistant.rs`, one intent call on a worker thread). This crate holds what both need and nothing
//! that is app-specific: the wire format ([`wire`]), the stream reader ([`sse`]), the intent protocol ([`intents`]) and
//! a blocking transport ([`transport`]). It depends on `bifrost-core` and TLS crates only, so the shell may depend on it.

pub mod intents;
pub mod sse;
pub mod transport;
pub mod wire;

use intents::{Catalogue, Reply};
use std::fmt;

/// The keychain item that holds the Anthropic API key: the service and the account of one generic password. The
/// caller reads the key (the shell's keychain, or the app's keychain request) and passes it in.
pub const KEY_SERVICE: &str = "is.olibuijr.assistant";
pub const KEY_ACCOUNT: &str = "anthropic-api-key";

/// What the caller needs to reach the backend: the API key read from the keychain item, and the model id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub api_key: String,
    pub model: String,
}

/// Why a request produced no reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No backend is chosen, so nothing was sent.
    NoBackend,
    /// The network or the service could not be reached.
    Unreachable(String),
    /// The service answered with an HTTP error.
    Http { status: u16, message: String },
    /// The reply was not a usable answer or action.
    Reply(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoBackend => write!(f, "No assistant backend is chosen."),
            Error::Unreachable(e) => f.write_str(e),
            Error::Http { message, .. } => f.write_str(message),
            Error::Reply(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

/// One blocking intent request: sends `messages` with the catalogue, then parses the reply against it. Call it from a
/// worker thread; it waits for the whole reply.
pub fn ask(connection: &Connection, messages: &[(bool, String)], catalogue: &Catalogue) -> Result<Reply, Error> {
    let body = intents::intent_request_body(&connection.model, catalogue, messages);
    let (status, text) = transport::post_messages(&connection.api_key, &body)?;
    if status != 200 {
        return Err(Error::Http { status, message: wire::error_message(status, &text) });
    }
    let answer = intents::reply_text(&text).map_err(Error::Reply)?;
    intents::parse_reply(&answer, catalogue).map_err(Error::Reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_http_error_reads_as_its_message() {
        let e = Error::Http { status: 401, message: wire::error_message(401, "{\"error\":{\"message\":\"invalid x-api-key\"}}") };
        assert_eq!(e.to_string(), "invalid x-api-key (HTTP 401). Check the API key in Settings.");
    }

    #[test]
    fn an_offline_error_is_an_availability_failure() {
        let e = Error::Unreachable("Cannot reach api.anthropic.com: no route".into());
        assert!(wire::is_unreachable(&e.to_string()));
    }
}

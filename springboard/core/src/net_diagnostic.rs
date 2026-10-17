//! Connection diagnosis: the app-facing model for explaining why a network request failed.
//!
//! The caller records what it saw at each stage of a request: the link, the address, the name lookup,
//! the connection, the secure handshake and the server's status. `diagnose` returns the first stage that
//! failed, with a short title and the next step for the user. The model is pure std and probes nothing
//! itself, so the app or the shell supplies the observations. For a plain (non-secure) address, the
//! caller sets `secure` to true, since there is no handshake to fail.

use std::fmt;

/// The stages of a request, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Link,
    Address,
    Name,
    Connection,
    Security,
    Server,
    Ok,
}

impl Stage {
    pub fn is_failure(self) -> bool {
        self != Stage::Ok
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Stage::Link => "link",
            Stage::Address => "address",
            Stage::Name => "name",
            Stage::Connection => "connection",
            Stage::Security => "security",
            Stage::Server => "server",
            Stage::Ok => "ok",
        };
        write!(f, "{text}")
    }
}

/// What the caller saw at each stage. `http_status` is `None` when no status line arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub link_up: bool,
    pub has_address: bool,
    pub name_resolved: bool,
    pub connected: bool,
    pub secure: bool,
    pub http_status: Option<u16>,
}

/// The first failing stage, with the text to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosis {
    pub stage: Stage,
    pub title: &'static str,
    pub advice: &'static str,
}

/// Returns the first stage that failed. A status of 500 or more is a server failure. Any lower status,
/// or no status yet, counts as the connection working.
pub fn diagnose(obs: &Observation) -> Diagnosis {
    let (stage, title, advice) = if !obs.link_up {
        (
            Stage::Link,
            "There is no network connection",
            "Check that Wi-Fi or the cable is connected.",
        )
    } else if !obs.has_address {
        (
            Stage::Address,
            "This computer has no IP address",
            "Wait for the network to assign an address, or reconnect.",
        )
    } else if !obs.name_resolved {
        (
            Stage::Name,
            "The site name could not be found",
            "Check the address for typos, or try again later.",
        )
    } else if !obs.connected {
        (
            Stage::Connection,
            "The server did not accept the connection",
            "The server may be offline, or a firewall may block it.",
        )
    } else if !obs.secure {
        (
            Stage::Security,
            "The secure connection could not be verified",
            "The site's certificate is not trusted, or it has expired.",
        )
    } else if matches!(obs.http_status, Some(status) if status >= 500) {
        (
            Stage::Server,
            "The server reported an error",
            "The site has a problem. Try again later.",
        )
    } else {
        (Stage::Ok, "The connection works", "")
    };
    Diagnosis { stage, title, advice }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn working() -> Observation {
        Observation {
            link_up: true,
            has_address: true,
            name_resolved: true,
            connected: true,
            secure: true,
            http_status: Some(200),
        }
    }

    #[test]
    fn a_working_request_reports_ok() {
        let d = diagnose(&working());
        assert_eq!(d.stage, Stage::Ok);
        assert!(!d.stage.is_failure());
        assert_eq!(d.title, "The connection works");
    }

    #[test]
    fn the_link_is_checked_first() {
        let mut obs = working();
        obs.link_up = false;
        obs.connected = false;
        assert_eq!(diagnose(&obs).stage, Stage::Link);
    }

    #[test]
    fn a_missing_address_is_reported_before_the_name() {
        let mut obs = working();
        obs.has_address = false;
        obs.name_resolved = false;
        let d = diagnose(&obs);
        assert_eq!(d.stage, Stage::Address);
        assert_eq!(d.title, "This computer has no IP address");
    }

    #[test]
    fn each_later_failure_maps_to_its_stage() {
        let mut name = working();
        name.name_resolved = false;
        assert_eq!(diagnose(&name).stage, Stage::Name);

        let mut conn = working();
        conn.connected = false;
        assert_eq!(diagnose(&conn).stage, Stage::Connection);

        let mut tls = working();
        tls.secure = false;
        assert_eq!(diagnose(&tls).stage, Stage::Security);
    }

    #[test]
    fn a_server_error_is_only_500_or_more() {
        let mut bad = working();
        bad.http_status = Some(503);
        let d = diagnose(&bad);
        assert_eq!(d.stage, Stage::Server);
        assert!(d.stage.is_failure());

        let mut missing = working();
        missing.http_status = Some(404);
        assert_eq!(diagnose(&missing).stage, Stage::Ok, "a 404 is the server answering, not a failure here");

        let mut none = working();
        none.http_status = None;
        assert_eq!(diagnose(&none).stage, Stage::Ok);
    }

    #[test]
    fn stages_print_their_lowercase_names() {
        assert_eq!(Stage::Connection.to_string(), "connection");
        assert_eq!(Stage::Ok.to_string(), "ok");
    }
}

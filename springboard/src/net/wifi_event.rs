//! wpa_supplicant's unsolicited events, reduced to the few that explain why
//! a join failed, and the short texts the Wi-Fi page shows for them.

use super::wpa;

/// Why an attempt to join was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// the pre-shared key did not match
    WrongPassword,
    /// 802.1X or another authentication step was rejected
    AuthFailed,
}

impl Failure {
    /// The row subtitle for this failure.
    pub fn note(self) -> &'static str {
        match self {
            Failure::WrongPassword => "Incorrect Password",
            Failure::AuthFailed => "Authentication Failed",
        }
    }
}

/// Shown while the radio is associated but DHCP gave no address.
pub const NO_INTERNET: &str = "No Internet Connection";

/// A failure and, when the event names it, the network it concerns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub ssid: Option<String>,
    pub failure: Failure,
}

/// The `ssid="..."` value of an event line, decoded. The name may hold
/// spaces, so it ends at the closing quote before the next field.
fn event_ssid(line: &str) -> Option<String> {
    let start = line.find("ssid=\"")? + 6;
    let rest = &line[start..];
    let end = rest.find("\" auth_failures=").or_else(|| rest.rfind('"'))?;
    Some(wpa::unescape(&rest[..end]))
}

fn event_reason(line: &str) -> Option<&str> {
    let start = line.find("reason=")? + 7;
    line[start..].split_whitespace().next()
}

/// Reads one event line (with its `<level>` prefix); None for events that
/// do not explain a failed join.
pub fn parse(line: &str) -> Option<Event> {
    if line.contains("CTRL-EVENT-SSID-TEMP-DISABLED") {
        let failure = match event_reason(line)? {
            "WRONG_KEY" => Failure::WrongPassword,
            "AUTH_FAILED" => Failure::AuthFailed,
            _ => return None,
        };
        return Some(Event { ssid: event_ssid(line), failure });
    }
    if line.contains("4-Way Handshake failed") && line.contains("pre-shared key may be incorrect") {
        return Some(Event { ssid: None, failure: Failure::WrongPassword });
    }
    if line.contains("CTRL-EVENT-EAP-FAILURE") {
        return Some(Event { ssid: None, failure: Failure::AuthFailed });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_key_names_the_network() {
        let l = "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=1 ssid=\"My Home WIFI\" auth_failures=1 duration=10 reason=WRONG_KEY";
        let e = parse(l).unwrap();
        assert_eq!(e, Event { ssid: Some("My Home WIFI".into()), failure: Failure::WrongPassword });
        assert_eq!(e.failure.note(), "Incorrect Password");
    }

    #[test]
    fn eap_rejection_is_an_authentication_failure() {
        let l = "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid=\"Corp\" auth_failures=2 duration=20 reason=AUTH_FAILED";
        assert_eq!(parse(l).unwrap().failure, Failure::AuthFailed);
        assert_eq!(parse("<3>CTRL-EVENT-EAP-FAILURE EAP authentication failed"), Some(Event { ssid: None, failure: Failure::AuthFailed }));
    }

    #[test]
    fn handshake_failure_has_no_ssid() {
        let l = "<3>WPA: 4-Way Handshake failed - pre-shared key may be incorrect";
        assert_eq!(parse(l), Some(Event { ssid: None, failure: Failure::WrongPassword }));
    }

    #[test]
    fn unrelated_events_and_reasons_are_ignored() {
        assert_eq!(parse("<3>CTRL-EVENT-CONNECTED - Connection to aa:bb completed"), None);
        assert_eq!(parse("<3>CTRL-EVENT-SSID-TEMP-DISABLED id=1 ssid=\"X\" auth_failures=1 duration=10 reason=CONN_FAILED"), None);
        assert_eq!(parse("<3>CTRL-EVENT-SCAN-RESULTS"), None);
    }

    #[test]
    fn escaped_ssids_are_decoded() {
        let l = "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=1 ssid=\"Caf\\xc3\\xa9\" auth_failures=1 duration=10 reason=WRONG_KEY";
        assert_eq!(parse(l).unwrap().ssid.as_deref(), Some("Café"));
    }
}

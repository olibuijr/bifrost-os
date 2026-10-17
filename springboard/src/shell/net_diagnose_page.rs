//! Settings > Network > Diagnose: runs `bifrost_core::net_probe` (link, address, name, connection) off the draw thread
//! and shows each step and the diagnosis. The host asked about is the update server the system already talks to, so
//! nothing new is contacted and nothing is sent to a diagnostics service.

use super::list::{Group, Item, Tint};
use super::pages::Action;
use bifrost_core::net_diagnostic::Stage;
use bifrost_core::net_probe::Report;

/// The host and port of a URL such as `https://host[:port]/path`. https defaults to 443, anything else to 80.
pub fn target_of(url: &str) -> (String, u16) {
    let (secure, rest) = match url.strip_prefix("https://") {
        Some(r) => (true, r),
        None => (false, url.strip_prefix("http://").unwrap_or(url)),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let default = if secure { 443 } else { 80 };
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => (host.to_string(), port.parse().unwrap_or(default)),
        _ => (authority.to_string(), default),
    }
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Link => "Link",
        Stage::Address => "Address",
        Stage::Name => "Name Lookup",
        Stage::Connection => "Connection",
        Stage::Security => "Security",
        Stage::Server => "Server",
        Stage::Ok => "Result",
    }
}

/// The groups added to the bottom of the Network page: the Diagnose button, then (once a run has finished) the steps
/// and the diagnosis. While a run is going the button says so and is not tappable.
pub fn groups(running: bool, report: Option<&Report>, host: &str) -> Vec<Group> {
    let button = if running { Item::new("Diagnosing…").tint(Tint::Dim) } else { Item::new("Diagnose").tint(Tint::Blue).action(Action::NetDiagnose) };
    let mut out = vec![Group::new(vec![button])
        .header("DIAGNOSE")
        .footer(&format!("Checks the link, the address, the name lookup and a connection to {host} from this computer. Nothing is sent to a diagnostics service."))];
    if let (false, Some(r)) = (running, report) {
        let steps = r
            .steps
            .iter()
            .map(|s| {
                let state = match s.passed {
                    Some(true) => "OK",
                    Some(false) => "Failed",
                    None => "Skipped",
                };
                Item::new(stage_name(s.stage)).value(if s.passed.is_none() { state.to_string() } else { format!("{state}: {}", s.detail) })
            })
            .collect();
        out.push(Group::new(steps).header("STEPS"));
        out.push(Group::new(vec![Item::new(r.diagnosis.title)]).header("DIAGNOSIS").footer(r.diagnosis.advice));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_comes_from_the_update_url() {
        assert_eq!(target_of("https://bifrost.olibuijr.com/ota/springboard/latest.json"), ("bifrost.olibuijr.com".into(), 443));
        assert_eq!(target_of("http://10.99.0.1:8080/x"), ("10.99.0.1".into(), 8080));
        assert_eq!(target_of("http://lab/x"), ("lab".into(), 80));
    }

    #[test]
    fn no_report_shows_only_the_button_and_a_run_hides_the_old_report() {
        assert_eq!(groups(false, None, "h").len(), 1);
        assert_eq!(groups(true, None, "h")[0].items[0].label, "Diagnosing…");
    }
}

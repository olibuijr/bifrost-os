//! Network diagnostics that look: the probes that fill a [`net_diagnostic::Observation`](crate::net_diagnostic::Observation)
//! from this computer, run them in order and stop at the first stage that fails.
//!
//! [`run`] checks the link (any interface other than loopback is up), the address (a global address is assigned), the
//! name (the host resolves) and the connection (a TCP connection opens), then hands the observation to
//! [`net_diagnostic::diagnose`](crate::net_diagnostic::diagnose) for the words. The secure handshake and the server's
//! answer are known only to the caller that made the request, so the [`Target`] carries them. A stage after the first
//! failure is not probed: its [`Step`] says it was skipped. Every probe is local to this computer's own network stack
//! and the host asked about; nothing is sent to a diagnostics service.
//!
//! The probes sit behind [`Probe`] so the order and the text are tested without a network. [`SystemProbe`] reads
//! `/sys/class/net/*/operstate`, runs `ip -o addr show scope global`, and resolves and connects with the standard
//! library, each with a time limit so a dead resolver cannot hang the caller.

use crate::net_diagnostic::{diagnose, Diagnosis, Observation, Stage};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::Duration;

/// What the diagnosis is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target<'a> {
    pub host: &'a str,
    pub port: u16,
    /// The result of the secure handshake as the caller saw it; true for a plain address.
    pub secure: bool,
    /// The status line the server answered with, if one arrived.
    pub http_status: Option<u16>,
}

impl<'a> Target<'a> {
    /// A target whose secure stage and server stage have nothing to report.
    pub fn new(host: &'a str, port: u16) -> Target<'a> {
        Target { host, port, secure: true, http_status: None }
    }
}

/// The local checks. Each may be replaced in tests.
pub trait Probe {
    /// Interface name and its `operstate` text, for every interface.
    fn link_states(&self) -> Vec<(String, String)>;
    /// The output of `ip -o addr show scope global`.
    fn global_addresses(&self) -> String;
    /// The addresses `host` resolves to; empty when it does not.
    fn resolve(&self, host: &str, port: u16) -> Vec<SocketAddr>;
    /// True when a TCP connection to the address opens.
    fn connect(&self, addr: SocketAddr) -> bool;
}

/// One stage as it was probed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub stage: Stage,
    /// None when the stage was skipped because an earlier one failed.
    pub passed: Option<bool>,
    pub detail: String,
}

/// The whole result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub observation: Observation,
    pub diagnosis: Diagnosis,
    pub steps: Vec<Step>,
}

impl Report {
    /// One line per stage for a log or a details pane: `ok`, `failed` or `skipped`, the stage, then what was seen.
    pub fn lines(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|s| {
                let state = match s.passed {
                    Some(true) => "ok",
                    Some(false) => "failed",
                    None => "skipped",
                };
                format!("{state}: {} ({})", s.stage, s.detail)
            })
            .collect()
    }
}

/// True when some interface other than loopback is `up`. An interface whose state is `unknown` (a tunnel or a bridge
/// that does not report) counts as up only when no other interface reports.
pub fn link_up_from(states: &[(String, String)]) -> bool {
    let real = || states.iter().filter(|(name, _)| name != "lo");
    real().any(|(_, state)| state.trim() == "up") || (real().all(|(_, state)| state.trim() != "down") && real().any(|(_, state)| state.trim() == "unknown"))
}

/// True when the output of `ip -o addr show scope global` holds an address line.
pub fn has_global_address(ip_output: &str) -> bool {
    ip_output.lines().any(|l| {
        let mut words = l.split_whitespace();
        words.any(|w| w == "inet" || w == "inet6")
    })
}

/// Runs the probes for `target` in order and diagnoses the first failure.
pub fn run(probe: &dyn Probe, target: &Target) -> Report {
    let mut steps: Vec<Step> = Vec::new();
    let mut obs = Observation { link_up: false, has_address: false, name_resolved: false, connected: false, secure: target.secure, http_status: target.http_status };
    let mut alive = true;
    let push = |steps: &mut Vec<Step>, stage: Stage, alive: &mut bool, passed: bool, detail: String| {
        steps.push(Step { stage, passed: Some(passed), detail });
        if !passed {
            *alive = false;
        }
    };
    let skip = |steps: &mut Vec<Step>, stage: Stage| steps.push(Step { stage, passed: None, detail: "not probed".into() });

    // link
    let states = probe.link_states();
    obs.link_up = link_up_from(&states);
    let up: Vec<&str> = states.iter().filter(|(n, s)| n != "lo" && s.trim() == "up").map(|(n, _)| n.as_str()).collect();
    let detail = match (obs.link_up, up.is_empty()) {
        (true, false) => format!("up: {}", up.join(", ")),
        (true, true) => "a virtual interface reports no state".to_string(),
        (false, _) => "no interface is up".to_string(),
    };
    push(&mut steps, Stage::Link, &mut alive, obs.link_up, detail);

    // address
    if alive {
        obs.has_address = has_global_address(&probe.global_addresses());
        let detail = if obs.has_address { "a global address is assigned" } else { "no global address" };
        push(&mut steps, Stage::Address, &mut alive, obs.has_address, detail.to_string());
    } else {
        skip(&mut steps, Stage::Address);
    }

    // name
    let mut addrs: Vec<SocketAddr> = Vec::new();
    if alive {
        addrs = match target.host.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, target.port)],
            Err(_) => probe.resolve(target.host, target.port),
        };
        obs.name_resolved = !addrs.is_empty();
        let detail = if obs.name_resolved { format!("{} address(es) for {}", addrs.len(), target.host) } else { format!("{} does not resolve", target.host) };
        push(&mut steps, Stage::Name, &mut alive, obs.name_resolved, detail);
    } else {
        skip(&mut steps, Stage::Name);
    }

    // connection: the first few addresses are tried, the first that opens is enough
    if alive {
        let tried = addrs.iter().take(3).copied();
        let mut last = None;
        for addr in tried {
            last = Some(addr);
            if probe.connect(addr) {
                obs.connected = true;
                break;
            }
        }
        let detail = match (obs.connected, last) {
            (true, Some(a)) => format!("connected to {a}"),
            (false, Some(a)) => format!("{a} did not accept the connection"),
            _ => "no address to connect to".to_string(),
        };
        push(&mut steps, Stage::Connection, &mut alive, obs.connected, detail);
    } else {
        skip(&mut steps, Stage::Connection);
    }

    // security and server: the caller's own observations, judged only when the connection opened
    if alive {
        let detail = if obs.secure { "handshake accepted" } else { "handshake refused" };
        push(&mut steps, Stage::Security, &mut alive, obs.secure, detail.to_string());
    } else {
        skip(&mut steps, Stage::Security);
    }
    if alive {
        let failed = matches!(obs.http_status, Some(s) if s >= 500);
        let detail = match obs.http_status {
            Some(s) => format!("status {s}"),
            None => "no status yet".to_string(),
        };
        push(&mut steps, Stage::Server, &mut alive, !failed, detail);
    } else {
        skip(&mut steps, Stage::Server);
    }

    let diagnosis = diagnose(&obs);
    Report { observation: obs, diagnosis, steps }
}

/// The probes on this computer.
#[derive(Clone, Copy, Debug)]
pub struct SystemProbe {
    /// The longest a resolve or a connect may take.
    pub timeout: Duration,
}

impl Default for SystemProbe {
    fn default() -> Self {
        SystemProbe { timeout: Duration::from_secs(5) }
    }
}

impl Probe for SystemProbe {
    fn link_states(&self) -> Vec<(String, String)> {
        let Ok(dir) = std::fs::read_dir("/sys/class/net") else { return Vec::new() };
        dir.filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let state = std::fs::read_to_string(e.path().join("operstate")).ok()?;
                Some((name, state.trim().to_string()))
            })
            .collect()
    }

    fn global_addresses(&self) -> String {
        std::process::Command::new("ip")
            .args(["-o", "addr", "show", "scope", "global"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    fn resolve(&self, host: &str, port: u16) -> Vec<SocketAddr> {
        let (tx, rx) = mpsc::channel();
        let host = host.to_string();
        // the system resolver has no time limit of its own, so it runs on a thread that is abandoned when it is late
        std::thread::spawn(move || {
            let found: Vec<SocketAddr> = (host.as_str(), port).to_socket_addrs().map(|i| i.collect::<Vec<SocketAddr>>()).unwrap_or_default();
            let _ = tx.send(found);
        });
        rx.recv_timeout(self.timeout).unwrap_or_default()
    }

    fn connect(&self, addr: SocketAddr) -> bool {
        TcpStream::connect_timeout(&addr, self.timeout).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::net::Ipv4Addr;

    struct Fake {
        links: Vec<(&'static str, &'static str)>,
        addresses: &'static str,
        resolves: bool,
        accepts: bool,
        asked: RefCell<Vec<String>>,
    }

    impl Fake {
        fn working() -> Fake {
            Fake {
                links: vec![("lo", "unknown"), ("wlan0", "up")],
                addresses: "3: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global dynamic wlan0\n",
                resolves: true,
                accepts: true,
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl Probe for Fake {
        fn link_states(&self) -> Vec<(String, String)> {
            self.asked.borrow_mut().push("link".into());
            self.links.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        }
        fn global_addresses(&self) -> String {
            self.asked.borrow_mut().push("address".into());
            self.addresses.to_string()
        }
        fn resolve(&self, _host: &str, port: u16) -> Vec<SocketAddr> {
            self.asked.borrow_mut().push("resolve".into());
            if self.resolves {
                vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)), port)]
            } else {
                Vec::new()
            }
        }
        fn connect(&self, _addr: SocketAddr) -> bool {
            self.asked.borrow_mut().push("connect".into());
            self.accepts
        }
    }

    #[test]
    fn a_working_network_passes_every_stage() {
        let fake = Fake::working();
        let report = run(&fake, &Target::new("example.org", 443));
        assert_eq!(report.diagnosis.stage, Stage::Ok);
        assert!(report.steps.iter().all(|s| s.passed == Some(true)));
        assert_eq!(report.steps.len(), 6);
        assert_eq!(report.lines()[0], "ok: link (up: wlan0)");
    }

    #[test]
    fn the_first_failure_stops_the_probing_and_later_stages_are_skipped() {
        let mut fake = Fake::working();
        fake.resolves = false;
        let report = run(&fake, &Target::new("nowhere.invalid", 443));
        assert_eq!(report.diagnosis.stage, Stage::Name);
        assert_eq!(*fake.asked.borrow(), vec!["link", "address", "resolve"]);
        let states: Vec<Option<bool>> = report.steps.iter().map(|s| s.passed).collect();
        assert_eq!(states, vec![Some(true), Some(true), Some(false), None, None, None]);
        assert!(report.lines()[3].starts_with("skipped: connection"));
    }

    #[test]
    fn no_interface_up_means_no_link_and_nothing_else_is_asked() {
        let mut fake = Fake::working();
        fake.links = vec![("lo", "unknown"), ("wlan0", "down"), ("eth0", "down")];
        let report = run(&fake, &Target::new("example.org", 443));
        assert_eq!(report.diagnosis.stage, Stage::Link);
        assert_eq!(*fake.asked.borrow(), vec!["link"]);
    }

    #[test]
    fn no_address_is_the_address_stage() {
        let mut fake = Fake::working();
        fake.addresses = "";
        assert_eq!(run(&fake, &Target::new("example.org", 443)).diagnosis.stage, Stage::Address);
    }

    #[test]
    fn a_refused_connection_is_the_connection_stage_and_names_the_address() {
        let mut fake = Fake::working();
        fake.accepts = false;
        let report = run(&fake, &Target::new("example.org", 443));
        assert_eq!(report.diagnosis.stage, Stage::Connection);
        assert!(report.lines()[3].contains("did not accept the connection"));
    }

    #[test]
    fn an_address_literal_is_not_resolved() {
        let fake = Fake::working();
        let report = run(&fake, &Target::new("203.0.113.9", 80));
        assert_eq!(report.diagnosis.stage, Stage::Ok);
        assert!(!fake.asked.borrow().contains(&"resolve".to_string()));
    }

    #[test]
    fn the_callers_handshake_and_status_decide_the_last_two_stages() {
        let fake = Fake::working();
        let mut target = Target::new("example.org", 443);
        target.secure = false;
        assert_eq!(run(&fake, &target).diagnosis.stage, Stage::Security);
        let mut target = Target::new("example.org", 443);
        target.http_status = Some(503);
        let report = run(&fake, &target);
        assert_eq!(report.diagnosis.stage, Stage::Server);
        assert_eq!(report.lines()[5], "failed: server (status 503)");
    }

    #[test]
    fn link_state_rules() {
        let s = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        assert!(link_up_from(&s(&[("lo", "unknown"), ("eth0", "up")])));
        assert!(!link_up_from(&s(&[("lo", "unknown")])));
        assert!(!link_up_from(&s(&[("lo", "unknown"), ("eth0", "down")])));
        // a tunnel that does not report counts only when nothing is down
        assert!(link_up_from(&s(&[("tun0", "unknown")])));
        assert!(!link_up_from(&s(&[("tun0", "unknown"), ("eth0", "down")])));
        assert!(!link_up_from(&[]));
    }

    #[test]
    fn an_address_line_is_found_in_the_ip_output() {
        assert!(has_global_address("2: eth0    inet6 2001:db8::5/64 scope global\n"));
        assert!(has_global_address("2: eth0    inet 10.0.0.2/24 scope global eth0\n"));
        assert!(!has_global_address(""));
        assert!(!has_global_address("2: eth0 link/ether aa:bb\n"));
    }
}

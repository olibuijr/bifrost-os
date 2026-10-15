//! Network privacy attribution and the local-network permission: which app made a connection, whether a
//! destination is on the local network (and so needs the permission), and the per-app record of connections for the
//! privacy report. Pure std over address text; the shell's network service feeds it.

use std::collections::BTreeMap;

/// True for private, link-local, loopback and unique-local addresses (IPv4 or IPv6) and `.local` names.
pub fn is_local_destination(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if h.ends_with(".local") || h == "localhost" {
        return true;
    }
    if let Some(v4) = parse_v4(&h) {
        return matches!(v4, [10, ..] | [127, ..] | [169, 254, ..] | [192, 168, ..]) || (v4[0] == 172 && (16..=31).contains(&v4[1]));
    }
    h == "::1" || h.starts_with("fe80:") || h.starts_with("fc") && h.contains(':') || h.starts_with("fd") && h.contains(':')
}

fn parse_v4(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse().ok()?;
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Ask the person once: the app has not been answered yet.
    Ask,
    Deny,
}

/// The per-app answers to "find and connect to devices on your local network".
#[derive(Clone, Debug, Default)]
pub struct LocalNetwork {
    answers: BTreeMap<String, bool>,
}

impl LocalNetwork {
    pub fn answer(&mut self, app: &str, allow: bool) {
        self.answers.insert(app.to_string(), allow);
    }

    /// What to do for `app` connecting to `host`: external hosts are always allowed here.
    pub fn check(&self, app: &str, host: &str) -> Decision {
        if !is_local_destination(host) {
            return Decision::Allow;
        }
        match self.answers.get(app) {
            Some(true) => Decision::Allow,
            Some(false) => Decision::Deny,
            None => Decision::Ask,
        }
    }

    pub fn reset(&mut self, app: &str) {
        self.answers.remove(app);
    }
}

/// Connections per app and destination, for the privacy report.
#[derive(Clone, Debug, Default)]
pub struct Attribution {
    counts: BTreeMap<(String, String), u64>,
}

impl Attribution {
    pub fn record(&mut self, app: &str, host: &str) {
        *self.counts.entry((app.to_string(), host.to_ascii_lowercase())).or_insert(0) += 1;
    }

    /// The most contacted hosts of an app, most first (ties by name).
    pub fn top_hosts(&self, app: &str, limit: usize) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = self.counts.iter().filter(|((a, _), _)| a == app).map(|((_, h), n)| (h.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(limit);
        v
    }

    /// The apps that contacted `host`.
    pub fn apps_for(&self, host: &str) -> Vec<String> {
        let host = host.to_ascii_lowercase();
        self.counts.keys().filter(|(_, h)| *h == host).map(|(a, _)| a.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses() {
        for h in ["192.168.1.5", "10.0.0.1", "172.16.4.4", "172.31.255.1", "127.0.0.1", "169.254.1.1", "printer.local", "[::1]", "fe80::1", "fd12::5", "localhost"] {
            assert!(is_local_destination(h), "{h}");
        }
        for h in ["8.8.8.8", "172.32.0.1", "172.15.0.1", "example.com", "2001:db8::1", "192.169.0.1"] {
            assert!(!is_local_destination(h), "{h}");
        }
    }

    #[test]
    fn permission_flow() {
        let mut n = LocalNetwork::default();
        assert_eq!(n.check("app", "example.com"), Decision::Allow);
        assert_eq!(n.check("app", "192.168.1.9"), Decision::Ask);
        n.answer("app", false);
        assert_eq!(n.check("app", "192.168.1.9"), Decision::Deny);
        n.answer("app", true);
        assert_eq!(n.check("app", "192.168.1.9"), Decision::Allow);
        assert_eq!(n.check("other", "192.168.1.9"), Decision::Ask);
        n.reset("app");
        assert_eq!(n.check("app", "10.1.1.1"), Decision::Ask);
    }

    #[test]
    fn attribution_report() {
        let mut a = Attribution::default();
        a.record("browser", "Example.com");
        a.record("browser", "example.com");
        a.record("browser", "cdn.net");
        a.record("mail", "example.com");
        assert_eq!(a.top_hosts("browser", 5), vec![("example.com".to_string(), 2), ("cdn.net".to_string(), 1)]);
        assert_eq!(a.top_hosts("browser", 1).len(), 1);
        assert_eq!(a.apps_for("EXAMPLE.com"), vec!["browser".to_string(), "mail".to_string()]);
        assert!(a.top_hosts("none", 3).is_empty());
    }
}

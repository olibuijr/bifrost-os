//! Network statistics per socket and per process: the Activity Monitor network readings below the per-interface
//! counters of `/proc/net/dev` (`sample::parse_netdev`). The kernel's socket diagnostics, read through `ss -tinpH`, give
//! each TCP socket's byte counters and its owning processes; [`parse_ss`] reads that text and [`per_process`] sums it.
//! Reading runs `ss` once ([`read_sockets`]); parsing and summing are pure, so they are tested without a network.
//!
//! Counters are cumulative since the socket opened, so a rate is the difference of two readings over the interval
//! ([`rate`]). A socket that more than one process holds is counted for each holder, as the kernel cannot split it.

use std::collections::BTreeMap;
use std::process::Command;

/// One TCP socket with the processes that hold it and its byte counters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocketStat {
    pub state: String,
    pub local: String,
    pub peer: String,
    /// `(process name, pid)` for every holder; empty when the socket belongs to another user.
    pub owners: Vec<(String, u32)>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

/// One process's network total over its sockets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcNet {
    pub pid: u32,
    pub name: String,
    pub sockets: u32,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

/// The processes in an `ss` `users:((...))` field: `users:(("name",pid=12,fd=3),("other",pid=15,fd=9))`.
fn parse_users(field: &str) -> Vec<(String, u32)> {
    let Some(inner) = field.strip_prefix("users:(") else { return Vec::new() };
    let mut out = Vec::new();
    for part in inner.split("),(") {
        let part = part.trim_matches(|c| c == '(' || c == ')');
        let Some(rest) = part.strip_prefix('"') else { continue };
        let Some(end) = rest.find('"') else { continue };
        let name = rest[..end].to_string();
        let pid = rest[end + 1..]
            .split(',')
            .find_map(|f| f.strip_prefix("pid="))
            .and_then(|p| p.parse::<u32>().ok());
        if let Some(pid) = pid {
            out.push((name, pid));
        }
    }
    out
}

/// The number after `key:` in the info line of a socket (`bytes_sent:1520`), 0 when the key is absent.
fn counter(info: &str, key: &str) -> u64 {
    info.split_whitespace()
        .find_map(|w| w.strip_prefix(key))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Parses the output of `ss -tinpH`: a socket line (state, queues, local, peer, owners) followed by an indented line
/// of kernel counters. A socket whose info line lacks the counters (a listener) reads as zero bytes. Lines that are not
/// a socket line are skipped.
pub fn parse_ss(text: &str) -> Vec<SocketStat> {
    let mut out: Vec<SocketStat> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(last) = out.last_mut() {
                last.bytes_sent = last.bytes_sent.max(counter(line, "bytes_sent:"));
                last.bytes_received = last.bytes_received.max(counter(line, "bytes_received:"));
            }
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        out.push(SocketStat {
            state: fields[0].to_string(),
            local: fields[3].to_string(),
            peer: fields[4].to_string(),
            owners: fields.get(5).map(|f| parse_users(f)).unwrap_or_default(),
            bytes_sent: 0,
            bytes_received: 0,
        });
    }
    out
}

/// Sums the sockets per process, busiest first (received plus sent, then pid). Sockets with no known owner are left
/// out: they belong to another user and cannot be attributed.
pub fn per_process(sockets: &[SocketStat]) -> Vec<ProcNet> {
    let mut by_pid: BTreeMap<u32, ProcNet> = BTreeMap::new();
    for s in sockets {
        for (name, pid) in &s.owners {
            let p = by_pid.entry(*pid).or_insert_with(|| ProcNet { pid: *pid, name: name.clone(), ..ProcNet::default() });
            p.sockets += 1;
            p.bytes_sent += s.bytes_sent;
            p.bytes_received += s.bytes_received;
        }
    }
    let mut v: Vec<ProcNet> = by_pid.into_values().collect();
    v.sort_by(|a, b| (b.bytes_sent + b.bytes_received).cmp(&(a.bytes_sent + a.bytes_received)).then(a.pid.cmp(&b.pid)));
    v
}

/// Bytes per second between two cumulative readings `ms` apart. A counter that went backwards (a socket closed and
/// the total fell) reads as 0, and a zero interval is 0.
pub fn rate(before: u64, now: u64, ms: u64) -> u64 {
    if ms == 0 || now < before {
        return 0;
    }
    (now - before).saturating_mul(1000) / ms
}

/// Runs `ss` and parses it. Empty when `ss` is not installed or fails.
pub fn read_sockets() -> Vec<SocketStat> {
    match Command::new("ss").args(["-tinpH"]).output() {
        Ok(o) if o.status.success() => parse_ss(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: &str = "\
ESTAB 0      0      192.168.1.2:43210 93.184.216.34:443 users:((\"firefox\",pid=1234,fd=50))
\t cubic wscale:7,7 rto:204 rtt:3.1/1.5 bytes_sent:1520 bytes_acked:1521 bytes_received:35000 segs_out:20
ESTAB 0      0      192.168.1.2:43212 93.184.216.35:443 users:((\"firefox\",pid=1234,fd=51),(\"helper\",pid=1300,fd=7))
\t cubic bytes_sent:500 bytes_received:1000
LISTEN 0     128    0.0.0.0:22 0.0.0.0:*
\t cubic
ESTAB 0      0      192.168.1.2:50000 10.0.0.9:80
\t cubic bytes_sent:99 bytes_received:99
";

    #[test]
    fn sockets_carry_their_counters_and_owners() {
        let s = parse_ss(SS);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].state, "ESTAB");
        assert_eq!(s[0].peer, "93.184.216.34:443");
        assert_eq!((s[0].bytes_sent, s[0].bytes_received), (1520, 35000));
        assert_eq!(s[0].owners, vec![("firefox".to_string(), 1234)]);
        assert_eq!(s[1].owners, vec![("firefox".to_string(), 1234), ("helper".to_string(), 1300)]);
        assert_eq!((s[2].bytes_sent, s[2].bytes_received), (0, 0), "a listener has no counters");
        assert!(s[3].owners.is_empty(), "another user's socket has no owner");
    }

    #[test]
    fn processes_are_summed_busiest_first_and_unowned_sockets_are_left_out() {
        let p = per_process(&parse_ss(SS));
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].pid, p[0].sockets, p[0].bytes_sent, p[0].bytes_received), (1234, 2, 2020, 36000));
        assert_eq!((p[1].pid, p[1].name.as_str(), p[1].bytes_received), (1300, "helper", 1000));
    }

    #[test]
    fn text_that_is_not_ss_output_gives_nothing() {
        assert!(parse_ss("").is_empty());
        assert!(parse_ss("Netid State\nfoo bar\n").is_empty());
        assert!(per_process(&[]).is_empty());
    }

    #[test]
    fn a_rate_is_bytes_per_second_and_never_negative() {
        assert_eq!(rate(1000, 3000, 2000), 1000);
        assert_eq!(rate(3000, 1000, 1000), 0, "the socket closed and the total fell");
        assert_eq!(rate(0, 10, 0), 0);
    }
}

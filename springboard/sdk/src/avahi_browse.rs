//! Bonjour browsing through Avahi: the receiving half of `avahi_publish`.
//!
//! `network_browser::Browser` keeps the instances of one service type but sends no query. The installer enables
//! `avahi-daemon`, which does the multicast queries and keeps the cache for the whole machine, so [`browse`] asks it with
//! `avahi-browse -p -r -t TYPE` and feeds the answer to a `Browser`. Nothing leaves the local link, and no third party
//! is asked. The output is parsed by [`parse_line`] and applied by [`feed`], both pure, so the wire form is tested
//! without the daemon.
//!
//! Each line is `+;iface;proto;name;type;domain` (an instance appeared), `-;...` (it went away) or, after
//! resolving, `=;iface;proto;name;type;domain;host;address;port;"key=value" "key"`. Names and TXT strings carry
//! `\DDD` decimal escapes for spaces and other special bytes.

use bifrost_core::dns_browse::{Browser, ServiceType};
use bifrost_core::net_connection::Transport;
use bifrost_core::txt_record::TxtRecord;
use std::io;
use std::process::Command;

/// The program that browses through the Avahi daemon.
pub const BROWSER: &str = "avahi-browse";

/// What one line of output says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Added { name: String, kind: String, domain: String, ipv6: bool },
    Removed { name: String, kind: String, domain: String, ipv6: bool },
    Resolved { name: String, kind: String, domain: String, host: String, address: String, port: u16, txt: TxtRecord, ipv6: bool },
}

/// The arguments for one browse of `kind` in `domain` (an empty domain browses `.local`).
pub fn browse_args(kind: &ServiceType, domain: &str) -> Vec<String> {
    let proto = match kind.transport() {
        Transport::Tcp => "_tcp",
        Transport::Udp => "_udp",
    };
    let mut args = vec!["-p".to_string(), "-r".to_string(), "-t".to_string()];
    let domain = domain.trim_end_matches('.');
    if !domain.is_empty() && domain != "local" {
        args.push("-d".to_string());
        args.push(domain.to_string());
    }
    args.push(format!("_{}.{}", kind.name(), proto));
    args
}

/// Decodes the `\DDD` (three decimal digits) and `\c` escapes of the parsable output.
pub fn unescape(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            if i + 4 <= b.len() && b[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
                let n: u32 = text[i + 1..i + 4].parse().unwrap_or(63);
                out.push(n.min(255) as u8);
                i += 4;
                continue;
            }
            out.push(b[i + 1]);
            i += 2;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads the quoted TXT strings of a resolved line.
fn parse_txt(field: &str) -> TxtRecord {
    let mut txt = TxtRecord::new();
    let mut rest = field.trim();
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        // the closing quote is the first one not preceded by a backslash
        let mut end = None;
        let ab = after.as_bytes();
        let mut j = 0;
        while j < ab.len() {
            if ab[j] == b'\\' {
                j += 2;
                continue;
            }
            if ab[j] == b'"' {
                end = Some(j);
                break;
            }
            j += 1;
        }
        let Some(end) = end else { break };
        let entry = unescape(&after[..end]);
        match entry.split_once('=') {
            Some((key, value)) => {
                let _ = txt.set(key, Some(value.as_bytes()));
            }
            None if !entry.is_empty() => {
                let _ = txt.set(&entry, None);
            }
            None => {}
        }
        rest = &after[end + 1..];
    }
    txt
}

/// Parses one line of `avahi-browse -p` output. Lines of other kinds (`+` for a browser startup message, blanks) are
/// None.
pub fn parse_line(line: &str) -> Option<Line> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut fields = line.splitn(10, ';');
    let tag = fields.next()?;
    let _interface = fields.next()?;
    let proto = fields.next()?;
    let name = unescape(fields.next()?);
    let kind = fields.next()?.to_string();
    let domain = fields.next()?.to_string();
    let ipv6 = proto.eq_ignore_ascii_case("IPv6");
    match tag {
        "+" => Some(Line::Added { name, kind, domain, ipv6 }),
        "-" => Some(Line::Removed { name, kind, domain, ipv6 }),
        "=" => {
            let host = fields.next()?.to_string();
            let address = fields.next()?.to_string();
            let port: u16 = fields.next()?.parse().ok()?;
            let txt = parse_txt(fields.next().unwrap_or(""));
            Some(Line::Resolved { name, kind, domain, host, address, port, txt, ipv6 })
        }
        _ => None,
    }
}

/// Applies the output of a browse to `browser`. Only lines for the browser's own service type count. A resolved line
/// stores the instance's address (a hostname is only as good as the resolver), preferring an IPv4 address, and never
/// replaces an address already stored by a link-local IPv6 one. Returns how many instances the browser holds.
pub fn feed(browser: &mut Browser, output: &str) -> usize {
    let wanted = format!(
        "_{}.{}",
        browser.kind().name(),
        match browser.kind().transport() {
            Transport::Tcp => "_tcp",
            Transport::Udp => "_udp",
        }
    );
    for line in output.lines().filter_map(parse_line) {
        match line {
            Line::Added { name, kind, .. } if kind.eq_ignore_ascii_case(&wanted) => {
                let _ = browser.browse_reply(true, &name);
            }
            Line::Removed { name, kind, .. } if kind.eq_ignore_ascii_case(&wanted) => {
                let _ = browser.browse_reply(false, &name);
            }
            Line::Resolved { name, kind, host, address, port, txt, ipv6, .. } if kind.eq_ignore_ascii_case(&wanted) => {
                let _ = browser.browse_reply(true, &name);
                let already = browser.instance(&name).map_or(false, |i| i.is_resolved());
                let link_local = address.to_ascii_lowercase().starts_with("fe80");
                if already && (ipv6 || link_local) {
                    continue;
                }
                let target = if address.is_empty() || (ipv6 && link_local) { host } else { address };
                browser.resolve_reply(&name, &target, port, txt);
            }
            _ => {}
        }
    }
    browser.instances().len()
}

/// Browses `.local` (or `domain`) for `kind` and returns a browser holding what the daemon knows. Fails when
/// `avahi-browse` is not installed or the daemon is not running.
pub fn browse(kind: ServiceType, domain: &str) -> io::Result<Browser> {
    let output = Command::new(BROWSER).args(browse_args(&kind, domain)).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("{BROWSER} failed: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    let mut browser = Browser::new(kind, domain);
    feed(&mut browser, &String::from_utf8_lossy(&output.stdout));
    Ok(browser)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
+;eth0;IPv4;Living\\032Room\\032Speaker;_airplay._tcp;local
+;eth0;IPv6;Living\\032Room\\032Speaker;_airplay._tcp;local
+;eth0;IPv4;Printer;_ipp._tcp;local
=;eth0;IPv4;Living\\032Room\\032Speaker;_airplay._tcp;local;speaker.local;192.168.1.20;7000;\"model=S1\" \"flag\" \"note=a;b\"
=;eth0;IPv6;Living\\032Room\\032Speaker;_airplay._tcp;local;speaker.local;fe80::1;7000;\"model=S1\"
-;eth0;IPv4;Gone;_airplay._tcp;local
";

    fn airplay() -> ServiceType {
        ServiceType::parse("_airplay._tcp").unwrap()
    }

    #[test]
    fn the_arguments_name_the_type_and_a_non_local_domain() {
        assert_eq!(browse_args(&airplay(), ""), vec!["-p", "-r", "-t", "_airplay._tcp"]);
        assert_eq!(browse_args(&airplay(), "example.net."), vec!["-p", "-r", "-t", "-d", "example.net", "_airplay._tcp"]);
        let udp = ServiceType::parse("_sip._udp").unwrap();
        assert_eq!(browse_args(&udp, "local").last().unwrap(), "_sip._udp");
    }

    #[test]
    fn escapes_decode_to_bytes() {
        assert_eq!(unescape("Living\\032Room"), "Living Room");
        assert_eq!(unescape("a\\.b\\\\c"), "a.b\\c");
        assert_eq!(unescape("Caf\\195\\169"), "Café");
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("end\\"), "end\\");
    }

    #[test]
    fn a_browse_line_parses() {
        let l = parse_line("+;eth0;IPv4;My\\032Box;_ipp._tcp;local").unwrap();
        assert_eq!(l, Line::Added { name: "My Box".into(), kind: "_ipp._tcp".into(), domain: "local".into(), ipv6: false });
        assert!(matches!(parse_line("-;eth0;IPv6;x;_ipp._tcp;local"), Some(Line::Removed { ipv6: true, .. })));
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("garbage"), None);
        assert_eq!(parse_line("?;a;b;c;d;e"), None);
    }

    #[test]
    fn a_resolved_line_carries_host_address_port_and_txt() {
        let l = parse_line("=;eth0;IPv4;S;_airplay._tcp;local;speaker.local;192.168.1.20;7000;\"model=S1\" \"flag\" \"note=a;b\"").unwrap();
        let Line::Resolved { host, address, port, txt, .. } = l else { panic!("not resolved") };
        assert_eq!((host.as_str(), address.as_str(), port), ("speaker.local", "192.168.1.20", 7000));
        assert_eq!(txt.len(), 3);
        assert!(matches!(txt.get("model"), bifrost_core::txt_record::Presence::Value(v) if v == b"S1"));
        assert!(matches!(txt.get("flag"), bifrost_core::txt_record::Presence::NoValue));
        assert!(matches!(txt.get("note"), bifrost_core::txt_record::Presence::Value(v) if v == b"a;b"));
        assert_eq!(parse_line("=;eth0;IPv4;S;_a._tcp;local;h;1.2.3.4;notaport;"), None);
    }

    #[test]
    fn feed_fills_a_browser_with_the_instances_of_its_own_type() {
        let mut b = Browser::new(airplay(), "local");
        assert_eq!(feed(&mut b, OUTPUT), 1);
        let speaker = b.instance("Living Room Speaker").unwrap();
        assert_eq!(speaker.host.as_deref(), Some("192.168.1.20"));
        assert_eq!(speaker.port, Some(7000));
        assert_eq!(speaker.txt.len(), 3);
        // the printer is another type and the removed one never resolved
        assert!(b.instance("Printer").is_none());
        assert!(b.instance("Gone").is_none());
    }

    #[test]
    fn a_link_local_address_does_not_replace_an_ipv4_one() {
        let mut b = Browser::new(airplay(), "local");
        let out = "=;e;IPv6;S;_airplay._tcp;local;s.local;fe80::1;7000;\"a=1\"\n=;e;IPv4;S;_airplay._tcp;local;s.local;10.0.0.4;7000;\"a=1\"\n=;e;IPv6;S;_airplay._tcp;local;s.local;fe80::2;7000;\"a=1\"\n";
        feed(&mut b, out);
        // the first (IPv6 link-local) line stores the hostname; the IPv4 line then sets the address, and a later
        // link-local line does not undo it
        assert_eq!(b.instance("S").unwrap().host.as_deref(), Some("10.0.0.4"));
    }

    #[test]
    fn a_removal_after_an_add_drops_the_instance() {
        let mut b = Browser::new(airplay(), "local");
        feed(&mut b, "+;e;IPv4;Tmp;_airplay._tcp;local\n-;e;IPv4;Tmp;_airplay._tcp;local\n");
        assert!(b.instances().is_empty());
    }
}

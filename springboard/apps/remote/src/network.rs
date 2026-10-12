//! Network reachability for the Remote window: reads the link and address state with `ip -j` and folds it into
//! the SDK's network path model (`bifrost_app::network_path`). The model decides satisfied, requires connection
//! or unsatisfied, and which link carries traffic; this file only parses the tool output and words the result.

use bifrost_app::network_path::{IfaceKind, Interface, Path, Status};
use serde_json::Value;
use std::process::Command;

/// Reads the current path. A missing or failing `ip` gives an offline path, never a panic.
pub fn read() -> Path {
    let links = run(&["-j", "link", "show"]);
    let v4 = run(&["-j", "-4", "address", "show", "scope", "global"]);
    let v6 = run(&["-j", "-6", "address", "show", "scope", "global"]);
    parse(&links, &v4, &v6)
}

fn run(args: &[&str]) -> String {
    Command::new("ip").args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn array(text: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(text).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// Kind of a link from its name and link type. Unknown names count as `Other`.
fn kind(name: &str, link_type: &str) -> IfaceKind {
    if link_type == "loopback" || name == "lo" {
        IfaceKind::Loopback
    } else if name.starts_with("wl") {
        IfaceKind::Wifi
    } else if name.starts_with("ww") {
        IfaceKind::Cellular
    } else if name.starts_with("en") || name.starts_with("eth") {
        IfaceKind::Wired
    } else {
        IfaceKind::Other
    }
}

/// Builds the path from the three `ip -j` outputs. A non-loopback link counts when it is up. Loopback is not a
/// route, so it is left out. An address family counts when some global address of that family is listed.
pub fn parse(links: &str, v4: &str, v6: &str) -> Path {
    let mut interfaces = Vec::new();
    for l in array(links) {
        let name = l["ifname"].as_str().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let link_type = l["link_type"].as_str().unwrap_or("");
        let up = matches!(l["operstate"].as_str(), Some("UP") | Some("UNKNOWN"));
        let k = kind(name, link_type);
        if k != IfaceKind::Loopback && up {
            interfaces.push(Interface { name: name.to_string(), kind: k });
        }
    }
    Path::new(interfaces, has_address(v4), has_address(v6))
}

fn has_address(text: &str) -> bool {
    array(text).iter().any(|i| i["addr_info"].as_array().map_or(false, |a| !a.is_empty()))
}

/// The line the window shows for a path, in plain words.
pub fn status_text(p: &Path) -> String {
    match p.status {
        Status::Unsatisfied => "No network connection".to_string(),
        Status::RequiresConnection => "Getting an address\u{2026}".to_string(),
        Status::Satisfied => {
            let by = match p.interfaces.first().map(|i| i.kind) {
                Some(IfaceKind::Wired) => "Connected by Ethernet",
                Some(IfaceKind::Wifi) => "Connected by Wi-Fi",
                Some(IfaceKind::Cellular) => "Connected by mobile data",
                _ => "Connected",
            };
            if p.expensive {
                format!("{by} (metered)")
            } else {
                by.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINKS: &str = r#"[
        {"ifname":"lo","link_type":"loopback","operstate":"UNKNOWN"},
        {"ifname":"eth0","link_type":"ether","operstate":"UP"},
        {"ifname":"wlan0","link_type":"ether","operstate":"DOWN"}
    ]"#;
    const V4: &str = r#"[{"ifname":"eth0","addr_info":[{"local":"192.168.1.20"}]}]"#;
    const NONE: &str = "[]";

    #[test]
    fn wired_link_with_an_address_is_connected_by_ethernet() {
        let p = parse(LINKS, V4, NONE);
        assert_eq!(p.status, Status::Satisfied);
        assert_eq!(p.interfaces.len(), 1, "the down Wi-Fi link is left out");
        assert_eq!(p.interfaces[0].name, "eth0");
        assert_eq!(status_text(&p), "Connected by Ethernet");
    }

    #[test]
    fn no_links_is_no_connection() {
        let p = parse("[]", NONE, NONE);
        assert_eq!(p.status, Status::Unsatisfied);
        assert_eq!(status_text(&p), "No network connection");
    }

    #[test]
    fn up_link_without_an_address_is_getting_one() {
        let p = parse(LINKS, NONE, NONE);
        assert_eq!(p.status, Status::RequiresConnection);
        assert_eq!(status_text(&p), "Getting an address\u{2026}");
    }

    #[test]
    fn wifi_is_named_and_cellular_primary_is_metered() {
        let wifi = r#"[{"ifname":"wlp3s0","link_type":"ether","operstate":"UP"}]"#;
        let p = parse(wifi, V4, NONE);
        assert_eq!(status_text(&p), "Connected by Wi-Fi");
        let cell = r#"[{"ifname":"wwan0","link_type":"none","operstate":"UP"}]"#;
        let c = parse(cell, V4, NONE);
        assert_eq!(status_text(&c), "Connected by mobile data (metered)");
    }

    #[test]
    fn unreadable_output_is_offline_not_a_panic() {
        let p = parse("not json", "also not json", "");
        assert_eq!(p.status, Status::Unsatisfied);
        assert!(p.interfaces.is_empty());
    }
}

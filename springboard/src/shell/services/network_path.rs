//! The `network` service: the system's connectivity path for apps, built from the shell's desktop network state
//! (the `netd` snapshot in `shell::desktop_network`) with the pure model in `bifrost_core::net_path`.
//!
//! Methods: `path` returns the path now (`{status, interfaces, expensive, ipv4, ipv6}`). Each shell tick the path
//! is compared with the last one, and a real change is pushed to every running app as the `pathUpdate` event with
//! the same body. The wire shape is pinned by the tests here and read back by `bifrost_app::network_path`.
//!
//! Only Wi-Fi and wired interfaces come from netd today, so `expensive` is always false on PC. The model's
//! `constrained` flag (Low Data Mode) has no source yet and is not reported.
use super::{Event, SystemService};
use crate::shell::desktop_network::{Interface as NetInterface, Snapshot};
use crate::shell::Shell;
use bifrost_core::json::Json;
use bifrost_core::net_path::{IfaceKind, Interface, Path, PathMonitor, Status};

/// The event a change of path is pushed as.
pub const PATH_EVENT: &str = "pathUpdate";

pub struct NetworkPath {
    monitor: PathMonitor,
}

impl NetworkPath {
    pub fn new() -> Self {
        NetworkPath { monitor: PathMonitor::new() }
    }
}

impl Default for NetworkPath {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemService for NetworkPath {
    fn name(&self) -> &'static str {
        "network"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "path" => Ok(path_to_json(&live_path(shell.info.desktop_network.as_ref()))),
            _ => Err(format!("network has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let path = live_path(shell.info.desktop_network.as_ref());
        match self.monitor.update(path) {
            Some(changed) => vec![Event { app: None, event: PATH_EVENT.into(), body: path_to_json(changed) }],
            None => Vec::new(),
        }
    }
}

/// The path the desktop network state describes. No snapshot (no desktop network) is offline.
/// Only connected interfaces count as links, and an address family is up when a connected interface has one.
pub fn live_path(net: Option<&Snapshot>) -> Path {
    let Some(net) = net else { return Path::offline() };
    let up: Vec<&NetInterface> = net.interfaces.iter().filter(|i| i.connected()).collect();
    let interfaces = up.iter().map(|i| Interface { name: i.name.clone(), kind: iface_kind(&i.kind) }).collect();
    let ipv4 = up.iter().any(|i| !i.ipv4.is_empty());
    let ipv6 = up.iter().any(|i| !i.ipv6.is_empty());
    Path::new(interfaces, ipv4, ipv6)
}

/// Maps the interface type netd reports (`nmcli` TYPE) to the model's kind.
pub fn iface_kind(kind: &str) -> IfaceKind {
    match kind {
        "ethernet" => IfaceKind::Wired,
        "wifi" => IfaceKind::Wifi,
        "gsm" | "cdma" | "wwan" => IfaceKind::Cellular,
        "loopback" => IfaceKind::Loopback,
        _ => IfaceKind::Other,
    }
}

/// The wire name of an interface kind.
pub fn kind_name(kind: IfaceKind) -> &'static str {
    match kind {
        IfaceKind::Wired => "wired",
        IfaceKind::Wifi => "wifi",
        IfaceKind::Cellular => "cellular",
        IfaceKind::Loopback => "loopback",
        IfaceKind::Other => "other",
    }
}

/// The wire name of a path status.
pub fn status_name(status: Status) -> &'static str {
    match status {
        Status::Satisfied => "satisfied",
        Status::Unsatisfied => "unsatisfied",
        Status::RequiresConnection => "requiresConnection",
    }
}

/// The wire form of a path: `{"status", "interfaces": [{"name", "kind"}], "expensive", "ipv4", "ipv6"}`.
pub fn path_to_json(path: &Path) -> Json {
    let interfaces = path
        .interfaces
        .iter()
        .map(|i| Json::Obj(vec![("name".into(), Json::Str(i.name.clone())), ("kind".into(), Json::Str(kind_name(i.kind).into()))]))
        .collect();
    Json::Obj(vec![
        ("status".into(), Json::Str(status_name(path.status).into())),
        ("interfaces".into(), Json::Arr(interfaces)),
        ("expensive".into(), Json::Bool(path.expensive)),
        ("ipv4".into(), Json::Bool(path.ipv4)),
        ("ipv6".into(), Json::Bool(path.ipv6)),
    ])
}

/// Reads the wire form back into a path. Unknown names and missing fields are errors, never a guessed path.
pub fn path_from_json(body: &Json) -> Result<Path, String> {
    let status = match body.get("status").and_then(Json::as_str) {
        Some("satisfied") => Status::Satisfied,
        Some("unsatisfied") => Status::Unsatisfied,
        Some("requiresConnection") => Status::RequiresConnection,
        other => return Err(format!("unknown path status {other:?}")),
    };
    let mut interfaces = Vec::new();
    for item in body.get("interfaces").and_then(Json::as_array).ok_or("path has no interfaces")? {
        let name = item.get("name").and_then(Json::as_str).ok_or("interface has no name")?;
        let kind = match item.get("kind").and_then(Json::as_str) {
            Some("wired") => IfaceKind::Wired,
            Some("wifi") => IfaceKind::Wifi,
            Some("cellular") => IfaceKind::Cellular,
            Some("loopback") => IfaceKind::Loopback,
            Some("other") => IfaceKind::Other,
            other => return Err(format!("unknown interface kind {other:?}")),
        };
        interfaces.push(Interface { name: name.to_string(), kind });
    }
    let flag = |key: &str| body.get(key).and_then(|v| if let Json::Bool(b) = v { Some(*b) } else { None }).ok_or(format!("path has no {key}"));
    Ok(Path { status, interfaces, expensive: flag("expensive")?, ipv4: flag("ipv4")?, ipv6: flag("ipv6")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(kind: &str, status: &str, ipv4: bool) -> NetInterface {
        NetInterface {
            name: format!("{kind}0"),
            kind: kind.into(),
            status: status.into(),
            ipv4: if ipv4 { vec!["192.168.1.5/24".into()] } else { Vec::new() },
            ..Default::default()
        }
    }

    fn snap(interfaces: Vec<NetInterface>) -> Snapshot {
        Snapshot { interfaces, ..Default::default() }
    }

    #[test]
    fn no_desktop_network_is_offline() {
        let p = live_path(None);
        assert_eq!(p.status, Status::Unsatisfied);
        assert!(p.interfaces.is_empty());
    }

    #[test]
    fn disconnected_interfaces_do_not_count_as_links() {
        let p = live_path(Some(&snap(vec![net("wifi", "30 (disconnected)", false)])));
        assert_eq!(p.status, Status::Unsatisfied);
        assert!(p.interfaces.is_empty());
    }

    #[test]
    fn connected_wired_with_address_is_satisfied_and_not_expensive() {
        let p = live_path(Some(&snap(vec![net("wifi", "100 (connected)", true), net("ethernet", "100 (connected)", true)])));
        assert_eq!(p.status, Status::Satisfied);
        assert_eq!(p.interfaces[0].kind, IfaceKind::Wired);
        assert_eq!(p.interfaces[1].kind, IfaceKind::Wifi);
        assert!(p.ipv4 && !p.ipv6);
        assert!(!p.expensive);
    }

    #[test]
    fn link_without_address_requires_connection() {
        let p = live_path(Some(&snap(vec![net("wifi", "100 (connected)", false)])));
        assert_eq!(p.status, Status::RequiresConnection);
    }

    #[test]
    fn interface_kinds_map_from_netd_types() {
        assert_eq!(iface_kind("ethernet"), IfaceKind::Wired);
        assert_eq!(iface_kind("wifi"), IfaceKind::Wifi);
        assert_eq!(iface_kind("gsm"), IfaceKind::Cellular);
        assert_eq!(iface_kind("bridge"), IfaceKind::Other);
    }

    #[test]
    fn path_json_has_the_documented_shape() {
        let p = live_path(Some(&snap(vec![net("ethernet", "100 (connected)", true)])));
        let text = path_to_json(&p).encode();
        assert_eq!(text, r#"{"status":"satisfied","interfaces":[{"name":"ethernet0","kind":"wired"}],"expensive":false,"ipv4":true,"ipv6":false}"#);
    }

    #[test]
    fn path_json_round_trips() {
        let p = Path::new(vec![Interface { name: "wwan0".into(), kind: IfaceKind::Cellular }], true, true);
        assert_eq!(path_from_json(&path_to_json(&p)), Ok(p));
    }

    #[test]
    fn bad_wire_forms_are_errors() {
        assert!(path_from_json(&Json::Null).is_err());
        let bad_status = bifrost_core::json::parse(r#"{"status":"maybe","interfaces":[],"expensive":false,"ipv4":false,"ipv6":false}"#).unwrap();
        assert!(path_from_json(&bad_status).is_err());
        let no_flag = bifrost_core::json::parse(r#"{"status":"satisfied","interfaces":[],"expensive":false,"ipv4":true}"#).unwrap();
        assert!(path_from_json(&no_flag).is_err());
    }
}

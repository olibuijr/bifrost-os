//! The `wifi` service: the CoreWLAN surface for apps (`CWWiFiClient`, `CWInterface`), built on the shell's desktop
//! network state (the `nmcli` snapshot in `shell::desktop_network`). Every change goes to the same worker that the
//! Wi-Fi page uses, so an app and the menu bar never disagree about the network.
//!
//! Methods (every one takes an optional `interface`, the wireless interface name; the first Wi-Fi interface is
//! used when it is absent):
//! - `interfaces` returns `[{name, powered, connected, ssid, address}]`, one per Wi-Fi interface.
//! - `networks` returns the visible networks `[{ssid, bars, secure, saved}]`, as the last scan saw them.
//! - `power` `{on}` turns the radio on or off.
//! - `scan` starts a rescan. Results arrive in the next `networks` read.
//! - `join` `{ssid, password?}` joins a network. An absent or empty password joins an open network.
//! - `forget` `{ssid}` removes a saved network. Only a saved network can be forgotten.
//!
//! Each change returns `{queued: true}` at once. The outcome shows in `interfaces` and `networks` when netd has
//! acted, so an app reads the state again rather than trusting the reply.
//!
//! The wire form is read back by `bifrost_app::wlan_service`, so the two sides share the names in this file.
use super::{Event, SystemService};
use crate::shell::desktop_network::{self, Request, Snapshot};
use crate::shell::{Shell, WifiAction};
use bifrost_core::json::Json;

/// The longest SSID an 802.11 network can carry, in bytes.
pub const MAX_SSID_BYTES: usize = 32;
/// The shortest and longest WPA passphrase, in bytes. A 64-digit hex key is also accepted (see [`check_password`]).
pub const MIN_PASSPHRASE: usize = 8;
pub const MAX_PASSPHRASE: usize = 63;

/// Why a request was refused. The message goes back to the app as the service error.
fn wifi_error(message: impl Into<String>) -> String {
    message.into()
}

/// Which wireless interface a request names: the given one when it exists and is Wi-Fi, otherwise the first Wi-Fi
/// interface. No Wi-Fi interface is an error.
pub fn pick_interface(net: Option<&Snapshot>, requested: Option<&str>) -> Result<String, String> {
    let wifi: Vec<&str> = net
        .map(|n| n.interfaces.iter().filter(|i| i.kind == "wifi").map(|i| i.name.as_str()).collect())
        .unwrap_or_default();
    match requested {
        Some(name) => {
            if wifi.contains(&name) {
                Ok(name.to_string())
            } else {
                Err(wifi_error(format!("no Wi-Fi interface named {name}")))
            }
        }
        None => wifi.first().map(|s| s.to_string()).ok_or_else(|| wifi_error("this machine has no Wi-Fi interface")),
    }
}

/// The wire form of the Wi-Fi interfaces.
pub fn interfaces_json(net: Option<&Snapshot>) -> Json {
    let Some(net) = net else { return Json::Arr(Vec::new()) };
    let powered = net.wifi_on;
    let items = net
        .interfaces
        .iter()
        .filter(|i| i.kind == "wifi")
        .map(|i| {
            let connected = i.connected();
            let ssid = if connected && !i.connection.is_empty() && i.connection != "--" {
                Json::Str(i.connection.clone())
            } else {
                Json::Null
            };
            let address = i.ipv4.first().cloned().unwrap_or_default();
            Json::Obj(vec![
                ("name".into(), Json::Str(i.name.clone())),
                ("powered".into(), Json::Bool(powered)),
                ("connected".into(), Json::Bool(connected)),
                ("ssid".into(), ssid),
                ("address".into(), Json::Str(address)),
            ])
        })
        .collect();
    Json::Arr(items)
}

/// The wire form of the visible networks from the last scan.
pub fn networks_json(net: Option<&Snapshot>) -> Json {
    let Some(net) = net else { return Json::Arr(Vec::new()) };
    let items = net
        .networks
        .iter()
        .map(|n| {
            Json::Obj(vec![
                ("ssid".into(), Json::Str(n.ssid.clone())),
                ("bars".into(), Json::Num(f64::from(n.bars))),
                ("secure".into(), Json::Bool(n.secure)),
                ("saved".into(), Json::Bool(n.saved)),
            ])
        })
        .collect();
    Json::Arr(items)
}

/// Reads an SSID: non-empty and no longer than an 802.11 SSID.
pub fn check_ssid(ssid: &str) -> Result<String, String> {
    if ssid.is_empty() {
        return Err(wifi_error("the network name is empty"));
    }
    if ssid.len() > MAX_SSID_BYTES {
        return Err(wifi_error(format!("the network name is longer than {MAX_SSID_BYTES} bytes")));
    }
    Ok(ssid.to_string())
}

/// Reads a password for a WPA network: a passphrase of 8 to 63 bytes, or a 64-digit hex key. An empty value
/// means an open network and returns None.
pub fn check_password(password: &str) -> Result<Option<String>, String> {
    if password.is_empty() {
        return Ok(None);
    }
    let hex_key = password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit());
    if hex_key || (MIN_PASSPHRASE..=MAX_PASSPHRASE).contains(&password.len()) {
        Ok(Some(password.to_string()))
    } else {
        Err(wifi_error(format!("a password is {MIN_PASSPHRASE} to {MAX_PASSPHRASE} characters, or a 64-digit hex key")))
    }
}

/// Whether `ssid` is a saved network in the snapshot.
pub fn is_saved(net: Option<&Snapshot>, ssid: &str) -> bool {
    net.is_some_and(|n| n.saved.iter().any(|s| s.ssid == ssid))
}

/// A JSON boolean, when the field is one.
fn bool_of(value: Option<&Json>) -> Option<bool> {
    match value {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// The `interface` a request names, if it names one.
fn interface_param(params: &Json) -> Result<Option<&str>, String> {
    match params.get("interface") {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| wifi_error("interface must be a name")),
    }
}

fn ssid_param(params: &Json) -> Result<String, String> {
    let ssid = params.get("ssid").and_then(Json::as_str).ok_or_else(|| wifi_error("expected an ssid"))?;
    check_ssid(ssid)
}

fn queued() -> Json {
    Json::Obj(vec![("queued".into(), Json::Bool(true))])
}

pub struct WifiService;

impl WifiService {
    pub fn new() -> Self {
        WifiService
    }
}

impl Default for WifiService {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemService for WifiService {
    fn name(&self) -> &'static str {
        "wifi"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let net = shell.info.desktop_network.as_ref();
        match method {
            "interfaces" => Ok(interfaces_json(net)),
            "networks" => Ok(networks_json(net)),
            "power" => {
                pick_interface(net, interface_param(params)?)?;
                let on = bool_of(params.get("on")).ok_or_else(|| wifi_error("expected on: true or false"))?;
                desktop_network::send(Request::Wifi(WifiAction::Enable(on)));
                Ok(queued())
            }
            "scan" => {
                pick_interface(net, interface_param(params)?)?;
                desktop_network::send(Request::Wifi(WifiAction::Scan));
                Ok(queued())
            }
            "join" => {
                pick_interface(net, interface_param(params)?)?;
                let ssid = ssid_param(params)?;
                let password = params.get("password").and_then(Json::as_str).unwrap_or("");
                let psk = check_password(password)?;
                desktop_network::send(Request::Wifi(WifiAction::Join { ssid, psk }));
                Ok(queued())
            }
            "forget" => {
                pick_interface(net, interface_param(params)?)?;
                let ssid = ssid_param(params)?;
                if !is_saved(net, &ssid) {
                    return Err(wifi_error(format!("{ssid} is not a saved network")));
                }
                desktop_network::send(Request::Wifi(WifiAction::Forget(ssid)));
                Ok(queued())
            }
            _ => Err(format!("wifi has no method {method}")),
        }
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::desktop_network::Interface;
    use crate::shell::{Network, SavedNetwork};

    fn wifi(name: &str, connection: &str, ip: &str) -> Interface {
        Interface {
            name: name.into(),
            kind: "wifi".into(),
            status: "100 (connected)".into(),
            connection: connection.into(),
            ipv4: if ip.is_empty() { Vec::new() } else { vec![ip.into()] },
            ..Interface::default()
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            interfaces: vec![
                Interface { name: "eth0".into(), kind: "ethernet".into(), ..Interface::default() },
                wifi("wlan0", "Den", "192.168.1.9"),
                wifi("wlan1", "--", ""),
            ],
            wifi_available: true,
            wifi_on: true,
            networks: vec![Network { ssid: "Den".into(), bars: 3, secure: true, saved: true }],
            saved: vec![SavedNetwork { ssid: "Den".into(), auto_join: true, note: String::new(), security: None }],
            ..Snapshot::default()
        }
    }

    #[test]
    fn the_first_wifi_interface_is_the_default() {
        assert_eq!(pick_interface(Some(&snapshot()), None).unwrap(), "wlan0");
    }

    #[test]
    fn a_named_interface_must_be_a_wifi_interface() {
        assert_eq!(pick_interface(Some(&snapshot()), Some("wlan1")).unwrap(), "wlan1");
        assert!(pick_interface(Some(&snapshot()), Some("eth0")).is_err());
        assert!(pick_interface(Some(&snapshot()), Some("wlan9")).is_err());
    }

    #[test]
    fn no_snapshot_or_no_wifi_is_an_error() {
        assert!(pick_interface(None, None).is_err());
        let wired = Snapshot { interfaces: vec![Interface { name: "eth0".into(), kind: "ethernet".into(), ..Interface::default() }], ..Snapshot::default() };
        assert!(pick_interface(Some(&wired), None).is_err());
    }

    #[test]
    fn interfaces_carry_name_power_connection_and_address() {
        let json = interfaces_json(Some(&snapshot()));
        let items = json.as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("name").and_then(Json::as_str), Some("wlan0"));
        assert_eq!(bool_of(items[0].get("powered")), Some(true));
        assert_eq!(bool_of(items[0].get("connected")), Some(true));
        assert_eq!(items[0].get("ssid").and_then(Json::as_str), Some("Den"));
        assert_eq!(items[0].get("address").and_then(Json::as_str), Some("192.168.1.9"));
        assert_eq!(bool_of(items[1].get("connected")), Some(true));
        assert!(matches!(items[1].get("ssid"), Some(Json::Null)));
    }

    #[test]
    fn networks_carry_bars_security_and_saved_flag() {
        let json = networks_json(Some(&snapshot()));
        let item = &json.as_array().unwrap()[0];
        assert_eq!(item.get("ssid").and_then(Json::as_str), Some("Den"));
        assert_eq!(item.get("bars").and_then(Json::number), Some(3.0));
        assert_eq!(bool_of(item.get("secure")), Some(true));
        assert_eq!(bool_of(item.get("saved")), Some(true));
    }

    #[test]
    fn ssids_are_checked_for_length() {
        assert!(check_ssid("").is_err());
        assert!(check_ssid(&"x".repeat(32)).is_ok());
        assert!(check_ssid(&"x".repeat(33)).is_err());
    }

    #[test]
    fn passwords_are_passphrases_or_hex_keys_and_empty_means_open() {
        assert_eq!(check_password("").unwrap(), None);
        assert!(check_password("short").is_err());
        assert!(check_password("correct horse").is_ok());
        assert!(check_password(&"a".repeat(63)).is_ok());
        assert!(check_password(&"a".repeat(64)).is_ok());
        assert!(check_password(&"g".repeat(64)).is_ok());
        assert!(check_password(&"a".repeat(65)).is_err());
    }

    #[test]
    fn only_saved_networks_can_be_forgotten() {
        assert!(is_saved(Some(&snapshot()), "Den"));
        assert!(!is_saved(Some(&snapshot()), "Office"));
        assert!(!is_saved(None, "Den"));
    }
}

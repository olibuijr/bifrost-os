//! Wi-Fi for apps: the `CWWiFiClient` and `CWInterface` calls, served by the shell's `wifi` service.
//!
//! Each request function sends a service call and returns its request id. The reply arrives as
//! `App::service_reply` with that id (or through `service::take_reply`), and the matching `decode_*` function reads it.
//!
//! ```ignore
//! // in an app, after asking once with request_interfaces(ctx):
//! fn service_reply(&mut self, _ctx: &Ctx, id: u32, result: &Result<Json, String>) -> bool {
//!     if id == self.interfaces_request {
//!         self.interfaces = wlan_service::decode_interfaces(result.clone()).unwrap_or_default();
//!         return true;
//!     }
//!     false
//! }
//! ```
//!
//! Changes (`request_power`, `request_scan`, `request_join`, `request_forget`) reply `{queued}` at once. The outcome
//! shows in the next interface and network reads, so an app reads the state again rather than assuming the change
//! worked. Every request that names an interface is refused by the shell when the name is not a Wi-Fi interface.
//! The wire form is defined by `springboard::shell::services::wifi`.
//!
//! Not part of this API: setting an interface's channel or mode, access-point and IBSS configuration, and the Wi-Fi
//! keychain, which the shell keeps for itself.

use crate::runtime::Ctx;
use bifrost_core::json::Json;

/// The shell service an app calls.
pub const SERVICE: &str = "wifi";

/// One Wi-Fi interface, as `CWInterface` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiInterface {
    /// The interface name, such as `wlan0`.
    pub name: String,
    /// The radio is on.
    pub powered: bool,
    /// The interface is associated with a network.
    pub connected: bool,
    /// The network it is associated with, when connected.
    pub ssid: Option<String>,
    /// Its IPv4 address, or an empty string.
    pub address: String,
    /// The interface type as the radio reports it: `managed`, `AP` (a hotspot) or `IBSS` (ad-hoc). None when unknown.
    pub mode: Option<String>,
    /// The channel the radio is on, when it is on one.
    pub channel: Option<u16>,
    /// The centre frequency of that channel in MHz.
    pub frequency_mhz: Option<u32>,
    /// The channel width in MHz.
    pub width_mhz: Option<u16>,
    /// The transmit power in dBm.
    pub tx_power_dbm: Option<f64>,
}

/// One network from the last scan, as `CWNetwork` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// Signal strength in bars, 1 to 3.
    pub bars: u8,
    pub secure: bool,
    pub saved: bool,
}

/// Asks for the Wi-Fi interfaces. Reply: [`decode_interfaces`].
pub fn request_interfaces(ctx: &Ctx) -> u32 {
    crate::service::call(ctx, SERVICE, "interfaces", &Json::Null)
}

/// Asks for the networks from the last scan. Reply: [`decode_networks`].
pub fn request_networks(ctx: &Ctx) -> u32 {
    crate::service::call(ctx, SERVICE, "networks", &Json::Null)
}

/// Turns the radio of `interface` (or the first Wi-Fi interface) on or off. Reply: [`decode_queued`].
pub fn request_power(ctx: &Ctx, interface: Option<&str>, on: bool) -> u32 {
    crate::service::call(ctx, SERVICE, "power", &power_params(interface, on))
}

/// Starts a scan. Reply: [`decode_queued`].
pub fn request_scan(ctx: &Ctx, interface: Option<&str>) -> u32 {
    crate::service::call(ctx, SERVICE, "scan", &interface_params(interface))
}

/// Joins `ssid`. An empty or absent password joins an open network. Reply: [`decode_queued`].
pub fn request_join(ctx: &Ctx, interface: Option<&str>, ssid: &str, password: Option<&str>) -> u32 {
    crate::service::call(ctx, SERVICE, "join", &join_params(interface, ssid, password))
}

/// Removes a saved network. Only a saved network is accepted. Reply: [`decode_queued`].
pub fn request_forget(ctx: &Ctx, interface: Option<&str>, ssid: &str) -> u32 {
    crate::service::call(ctx, SERVICE, "forget", &forget_params(interface, ssid))
}

/// The request body for `power`.
pub fn power_params(interface: Option<&str>, on: bool) -> Json {
    let mut fields = interface_fields(interface);
    fields.push(("on".into(), Json::Bool(on)));
    Json::Obj(fields)
}

/// The request body for `scan`.
pub fn interface_params(interface: Option<&str>) -> Json {
    Json::Obj(interface_fields(interface))
}

/// The request body for `join`.
pub fn join_params(interface: Option<&str>, ssid: &str, password: Option<&str>) -> Json {
    let mut fields = interface_fields(interface);
    fields.push(("ssid".into(), Json::Str(ssid.to_string())));
    if let Some(password) = password {
        fields.push(("password".into(), Json::Str(password.to_string())));
    }
    Json::Obj(fields)
}

/// The request body for `forget`.
pub fn forget_params(interface: Option<&str>, ssid: &str) -> Json {
    let mut fields = interface_fields(interface);
    fields.push(("ssid".into(), Json::Str(ssid.to_string())));
    Json::Obj(fields)
}

fn interface_fields(interface: Option<&str>) -> Vec<(String, Json)> {
    match interface {
        Some(name) => vec![("interface".into(), Json::Str(name.to_string()))],
        None => Vec::new(),
    }
}

/// Reads an interface list reply. A service error or a malformed item is an `Err`.
pub fn decode_interfaces(reply: Result<Json, String>) -> Result<Vec<WifiInterface>, String> {
    let reply = reply?;
    let items = reply.as_array().ok_or("interfaces reply is not a list")?;
    items.iter().map(interface_from_json).collect()
}

/// Reads a network list reply.
pub fn decode_networks(reply: Result<Json, String>) -> Result<Vec<WifiNetwork>, String> {
    let reply = reply?;
    let items = reply.as_array().ok_or("networks reply is not a list")?;
    items.iter().map(network_from_json).collect()
}

/// Reads a change reply: `Ok(())` when the shell queued it.
pub fn decode_queued(reply: Result<Json, String>) -> Result<(), String> {
    match reply?.get("queued").and_then(bool_of) {
        Some(true) => Ok(()),
        _ => Err("the change was not queued".into()),
    }
}

fn bool_of(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

fn field<'a>(item: &'a Json, key: &str) -> Result<&'a Json, String> {
    item.get(key).ok_or_else(|| format!("wifi reply has no {key}"))
}

fn interface_from_json(item: &Json) -> Result<WifiInterface, String> {
    let name = field(item, "name")?.as_str().ok_or("wifi interface name is not text")?.to_string();
    let powered = bool_of(field(item, "powered")?).ok_or("wifi powered is not a boolean")?;
    let connected = bool_of(field(item, "connected")?).ok_or("wifi connected is not a boolean")?;
    let ssid = match field(item, "ssid")? {
        Json::Null => None,
        other => Some(other.as_str().ok_or("wifi ssid is not text")?.to_string()),
    };
    let address = field(item, "address")?.as_str().ok_or("wifi address is not text")?.to_string();
    // the radio detail is optional: a shell that does not send it, or a null, reads as unknown
    let mode = match item.get("mode") {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let number = |key: &str| item.get(key).and_then(Json::number);
    Ok(WifiInterface {
        name,
        powered,
        connected,
        ssid,
        address,
        mode,
        channel: number("channel").map(|n| n as u16),
        frequency_mhz: number("frequency_mhz").map(|n| n as u32),
        width_mhz: number("width_mhz").map(|n| n as u16),
        tx_power_dbm: number("tx_power_dbm"),
    })
}

fn network_from_json(item: &Json) -> Result<WifiNetwork, String> {
    let ssid = field(item, "ssid")?.as_str().ok_or("network ssid is not text")?.to_string();
    let bars = field(item, "bars")?.number().ok_or("network bars is not a number")?;
    let secure = bool_of(field(item, "secure")?).ok_or("network secure is not a boolean")?;
    let saved = bool_of(field(item, "saved")?).ok_or("network saved is not a boolean")?;
    Ok(WifiNetwork { ssid, bars: bars.clamp(0.0, 255.0) as u8, secure, saved })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_carries_the_interface_ssid_and_password_only_when_given() {
        let open = join_params(None, "Cafe", None);
        assert_eq!(open.get("ssid").and_then(Json::as_str), Some("Cafe"));
        assert!(open.get("interface").is_none());
        assert!(open.get("password").is_none());
        let wpa = join_params(Some("wlan0"), "Den", Some("correct horse"));
        assert_eq!(wpa.get("interface").and_then(Json::as_str), Some("wlan0"));
        assert_eq!(wpa.get("password").and_then(Json::as_str), Some("correct horse"));
    }

    #[test]
    fn power_carries_the_switch() {
        let body = power_params(None, false);
        assert_eq!(body.get("on").and_then(bool_of), Some(false));
    }

    #[test]
    fn interfaces_decode_with_and_without_a_connection() {
        let reply = Json::Arr(vec![
            Json::Obj(vec![
                ("name".into(), Json::Str("wlan0".into())),
                ("powered".into(), Json::Bool(true)),
                ("connected".into(), Json::Bool(true)),
                ("ssid".into(), Json::Str("Den".into())),
                ("address".into(), Json::Str("192.168.1.9".into())),
            ]),
            Json::Obj(vec![
                ("name".into(), Json::Str("wlan1".into())),
                ("powered".into(), Json::Bool(false)),
                ("connected".into(), Json::Bool(false)),
                ("ssid".into(), Json::Null),
                ("address".into(), Json::Str(String::new())),
            ]),
        ]);
        let list = decode_interfaces(Ok(reply)).unwrap();
        assert_eq!(list[0].ssid.as_deref(), Some("Den"));
        assert_eq!(list[1].ssid, None);
        assert!(!list[1].powered);
    }

    #[test]
    fn an_interface_reads_its_radio_detail_and_treats_nulls_as_unknown() {
        let reply = Json::Arr(vec![Json::Obj(vec![
            ("name".into(), Json::Str("wlan0".into())),
            ("powered".into(), Json::Bool(true)),
            ("connected".into(), Json::Bool(true)),
            ("ssid".into(), Json::Str("Den".into())),
            ("address".into(), Json::Str("192.168.1.9".into())),
            ("mode".into(), Json::Str("AP".into())),
            ("channel".into(), Json::Num(6.0)),
            ("frequency_mhz".into(), Json::Num(2437.0)),
            ("width_mhz".into(), Json::Num(20.0)),
            ("tx_power_dbm".into(), Json::Num(17.5)),
        ])]);
        let items = decode_interfaces(Ok(reply)).unwrap();
        assert_eq!(items[0].mode.as_deref(), Some("AP"));
        assert_eq!(items[0].channel, Some(6));
        assert_eq!(items[0].frequency_mhz, Some(2437));
        assert_eq!(items[0].width_mhz, Some(20));
        assert_eq!(items[0].tx_power_dbm, Some(17.5));
        let bare = Json::Arr(vec![Json::Obj(vec![
            ("name".into(), Json::Str("wlan1".into())),
            ("powered".into(), Json::Bool(false)),
            ("connected".into(), Json::Bool(false)),
            ("ssid".into(), Json::Null),
            ("address".into(), Json::Str(String::new())),
            ("channel".into(), Json::Null),
        ])]);
        let items = decode_interfaces(Ok(bare)).unwrap();
        assert_eq!(items[0].mode, None);
        assert_eq!(items[0].channel, None);
    }

    #[test]
    fn a_malformed_interface_is_an_error() {
        let reply = Json::Arr(vec![Json::Obj(vec![("name".into(), Json::Str("wlan0".into()))])]);
        assert!(decode_interfaces(Ok(reply)).is_err());
        assert!(decode_interfaces(Err("no such service".into())).is_err());
    }

    #[test]
    fn networks_decode_bars_and_flags() {
        let reply = Json::Arr(vec![Json::Obj(vec![
            ("ssid".into(), Json::Str("Den".into())),
            ("bars".into(), Json::Num(3.0)),
            ("secure".into(), Json::Bool(true)),
            ("saved".into(), Json::Bool(false)),
        ])]);
        let list = decode_networks(Ok(reply)).unwrap();
        assert_eq!(list, vec![WifiNetwork { ssid: "Den".into(), bars: 3, secure: true, saved: false }]);
    }

    #[test]
    fn a_change_is_queued_only_when_the_reply_says_so() {
        assert!(decode_queued(Ok(Json::Obj(vec![("queued".into(), Json::Bool(true))]))).is_ok());
        assert!(decode_queued(Ok(Json::Null)).is_err());
        assert!(decode_queued(Err("no Wi-Fi interface".into())).is_err());
    }
}

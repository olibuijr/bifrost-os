//! Publishes the link state into the system configuration store (the `State:/Network/...` keys configd keeps for
//! `SCDynamicStore`): the address and router of the interface the link runs on, and the Wi-Fi network name. The
//! Wi-Fi manager calls [`apply`] after each tick, so the store follows the link. Keys that no longer apply are
//! removed; a value that has not changed is not written, so the store's revision only moves on a real change.
//!
//! Gap: the store lives in this process. Readers in other processes need a published copy, not built yet.

use bifrost_core::system_config_store::SystemConfigStore;
use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};

static STORE: OnceLock<Mutex<SystemConfigStore>> = OnceLock::new();

/// The shared store the shell publishes into.
pub fn store() -> &'static Mutex<SystemConfigStore> {
    STORE.get_or_init(|| Mutex::new(SystemConfigStore::new()))
}

/// The `(key, value)` pairs the link describes on `iface`. Nothing is described while `iface` is empty or has no
/// address: a link that is down publishes no state.
pub fn desired(iface: &str, ip: Option<Ipv4Addr>, router: Option<Ipv4Addr>, ssid: Option<&str>) -> Vec<(String, String)> {
    let Some(ip) = ip.filter(|_| !iface.is_empty()) else {
        return Vec::new();
    };
    let mut out = vec![
        (format!("State:/Network/Interface/{iface}/IPv4"), ip.to_string()),
        ("State:/Network/Global/IPv4/PrimaryInterface".to_string(), iface.to_string()),
    ];
    if let Some(router) = router {
        out.push(("State:/Network/Global/IPv4/Router".to_string(), router.to_string()));
    }
    if let Some(ssid) = ssid {
        out.push((format!("State:/Network/Interface/{iface}/AirPort/SSID"), ssid.to_string()));
    }
    out
}

/// Makes the `State:/Network/` keys of `store` equal `desired`: stale keys are removed, the rest are set (an
/// unchanged value costs nothing).
pub fn apply(store: &mut SystemConfigStore, desired: &[(String, String)]) {
    let stale: Vec<String> = store
        .keys("State:/Network/")
        .into_iter()
        .filter(|k| !desired.iter().any(|(d, _)| d == k))
        .map(str::to_string)
        .collect();
    for key in stale {
        store.remove(&key);
    }
    for (key, value) in desired {
        let _ = store.set(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Option<Ipv4Addr> {
        s.parse().ok()
    }

    #[test]
    fn a_linked_interface_describes_its_address_router_and_network() {
        let d = desired("wlan0", ip("192.168.1.20"), ip("192.168.1.1"), Some("Home"));
        assert_eq!(d.len(), 4);
        assert!(d.contains(&("State:/Network/Interface/wlan0/IPv4".to_string(), "192.168.1.20".to_string())));
        assert!(d.contains(&("State:/Network/Global/IPv4/PrimaryInterface".to_string(), "wlan0".to_string())));
        assert!(d.contains(&("State:/Network/Global/IPv4/Router".to_string(), "192.168.1.1".to_string())));
        assert!(d.contains(&("State:/Network/Interface/wlan0/AirPort/SSID".to_string(), "Home".to_string())));
    }

    #[test]
    fn nothing_is_described_without_an_interface_or_an_address() {
        assert!(desired("", ip("10.0.0.5"), None, None).is_empty());
        assert!(desired("eth0", None, ip("10.0.0.1"), None).is_empty(), "a router without an address is not published");
    }

    #[test]
    fn apply_sets_keys_and_is_idempotent() {
        let mut s = SystemConfigStore::new();
        let d = desired("eth0", ip("10.0.0.5"), ip("10.0.0.1"), None);
        apply(&mut s, &d);
        assert_eq!(s.get("State:/Network/Global/IPv4/Router"), Some("10.0.0.1"));
        let rev = s.revision();
        apply(&mut s, &d);
        assert_eq!(s.revision(), rev, "the same state is not a change");
    }

    #[test]
    fn apply_removes_keys_the_link_no_longer_has() {
        let mut s = SystemConfigStore::new();
        apply(&mut s, &desired("wlan0", ip("192.168.1.20"), ip("192.168.1.1"), Some("Home")));
        apply(&mut s, &[]);
        assert!(s.keys("State:/Network/").is_empty(), "a link that is down publishes nothing");
        assert!(s.keys("").is_empty());
    }

    #[test]
    fn apply_leaves_keys_outside_the_network_scope_alone() {
        let mut s = SystemConfigStore::new();
        s.set("Setup:/Network/Service/wifi0/Proxies", "p").unwrap();
        apply(&mut s, &[]);
        assert_eq!(s.get("Setup:/Network/Service/wifi0/Proxies"), Some("p"));
    }
}

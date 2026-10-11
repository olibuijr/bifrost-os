//! Saved Wi-Fi networks as data: the security types the shell can join,
//! their wpa_supplicant fields, and parsing of springboard.conf back into
//! networks. Pure functions only; `wifi` applies them to the supplicant.
//! The list is kept in priority order (first = joined first) and written to
//! the config as `priority=` lines, so the order survives restarts.

use super::wpa;
use std::fmt;

/// The 802.1X outer method; the inner one is always MSCHAPv2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eap {
    Peap,
    Ttls,
}

impl Eap {
    pub const ALL: [Eap; 2] = [Eap::Peap, Eap::Ttls];

    pub fn label(self) -> &'static str {
        match self {
            Eap::Peap => "PEAP",
            Eap::Ttls => "TTLS",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    Open,
    /// WPA2 Personal (PSK)
    Wpa2,
    /// WPA3 Personal (SAE), protected management frames required
    Wpa3,
    /// transition mode: PSK or SAE, whichever the access point offers
    Wpa2Wpa3,
    Enterprise(Eap),
}

impl Security {
    /// What the "Other Network" sheet cycles through.
    pub const CHOICES: [Security; 5] = [Security::Open, Security::Wpa2, Security::Wpa3, Security::Wpa2Wpa3, Security::Enterprise(Eap::Peap)];

    pub fn label(self) -> &'static str {
        match self {
            Security::Open => "None",
            Security::Wpa2 => "WPA2 Personal",
            Security::Wpa3 => "WPA3 Personal",
            Security::Wpa2Wpa3 => "WPA2/WPA3 Personal",
            Security::Enterprise(_) => "WPA2 Enterprise",
        }
    }

    pub fn is_enterprise(self) -> bool {
        matches!(self, Security::Enterprise(_))
    }

    /// Personal security takes one shared password.
    pub fn is_personal(self) -> bool {
        matches!(self, Security::Wpa2 | Security::Wpa3 | Security::Wpa2Wpa3)
    }

    /// Shortest and longest password in chars; WPA2 passphrases are 8..=63,
    /// SAE takes anything.
    pub fn password_len(self) -> (usize, usize) {
        match self {
            Security::Wpa3 => (1, 128),
            Security::Wpa2 | Security::Wpa2Wpa3 => (8, 63),
            _ => (0, 128),
        }
    }

    /// The next entry of `CHOICES`, wrapping.
    pub fn next(self) -> Security {
        let i = Security::CHOICES.iter().position(|c| c.label() == self.label()).unwrap_or(0);
        Security::CHOICES[(i + 1) % Security::CHOICES.len()]
    }
}

/// One saved network. `password` is the passphrase of a personal network or
/// the account password of an enterprise one.
#[derive(Clone, PartialEq)]
pub struct NetworkConfig {
    pub ssid: String,
    pub security: Security,
    pub password: String,
    pub hidden: bool,
    pub identity: String,
    pub anonymous: String,
    pub auto_join: bool,
}

/// Secrets and account names stay out of logs and test failures.
impl fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkConfig")
            .field("ssid", &self.ssid)
            .field("security", &self.security)
            .field("hidden", &self.hidden)
            .field("auto_join", &self.auto_join)
            .field("credentials", &if self.password.is_empty() && self.identity.is_empty() { "none" } else { "<redacted>" })
            .finish()
    }
}

impl NetworkConfig {
    pub fn new(ssid: &str, security: Security) -> Self {
        NetworkConfig { ssid: ssid.into(), security, password: String::new(), hidden: false, identity: String::new(), anonymous: String::new(), auto_join: true }
    }

    /// A network joined from the scan list: open, or secured with a passphrase.
    pub fn personal(ssid: &str, psk: Option<&str>) -> Self {
        match psk {
            Some(p) => NetworkConfig { password: p.into(), ..NetworkConfig::new(ssid, Security::Wpa2Wpa3) },
            None => NetworkConfig::new(ssid, Security::Open),
        }
    }

    /// Why this cannot be joined yet; None when it is complete.
    pub fn problem(&self) -> Option<&'static str> {
        if self.ssid.is_empty() || self.ssid.len() > 32 {
            return Some("Enter the network name");
        }
        let n = self.password.chars().count();
        let (min, max) = self.security.password_len();
        if self.security.is_personal() && !(min..=max).contains(&n) {
            return Some("Enter the password");
        }
        if self.security.is_enterprise() && (self.identity.is_empty() || self.password.is_empty()) {
            return Some("Enter the username and password");
        }
        None
    }

    /// The wpa_supplicant fields, in order, for the file (`key=value`) and
    /// for `SET_NETWORK <id> key value`. `priority` is higher for networks
    /// joined first. Enterprise networks set no `ca_cert`, so the server is
    /// not validated.
    pub fn fields(&self, priority: i32) -> Vec<(&'static str, String)> {
        let mut f = vec![("ssid", wpa::quoted(&self.ssid))];
        if self.hidden {
            f.push(("scan_ssid", "1".into()));
        }
        match self.security {
            Security::Open => f.push(("key_mgmt", "NONE".into())),
            Security::Wpa2 => {
                f.push(("key_mgmt", "WPA-PSK".into()));
                f.push(("psk", psk_value(&self.password)));
            }
            Security::Wpa3 => {
                f.push(("key_mgmt", "SAE".into()));
                f.push(("sae_password", wpa::quoted(&self.password)));
                f.push(("ieee80211w", "2".into()));
            }
            Security::Wpa2Wpa3 => {
                f.push(("key_mgmt", "WPA-PSK SAE".into()));
                f.push(("psk", psk_value(&self.password)));
                f.push(("ieee80211w", "1".into()));
            }
            Security::Enterprise(eap) => {
                f.push(("key_mgmt", "WPA-EAP".into()));
                f.push(("eap", eap.label().into()));
                f.push(("identity", wpa::quoted(&self.identity)));
                if !self.anonymous.is_empty() {
                    f.push(("anonymous_identity", wpa::quoted(&self.anonymous)));
                }
                f.push(("password", wpa::quoted(&self.password)));
                f.push(("phase2", "\"auth=MSCHAPV2\"".into()));
                f.push(("ieee80211w", "1".into()));
            }
        }
        f.push(("priority", priority.to_string()));
        if !self.auto_join {
            f.push(("disabled", "1".into()));
        }
        f
    }

    /// The `network={...}` block of the config file.
    pub fn block(&self, priority: i32) -> String {
        let mut b = String::from("\nnetwork={\n");
        for (k, v) in self.fields(priority) {
            b += &format!("\t{k}={v}\n");
        }
        b + "}\n"
    }
}

/// A 64-digit hex key is a raw PSK and is written unquoted, as wpa_supplicant
/// itself saves it.
fn psk_value(p: &str) -> String {
    if p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit()) {
        p.to_string()
    } else {
        wpa::quoted(p)
    }
}

/// Priorities for a list in joining order: the first gets the highest.
pub fn priority_of(index: usize, len: usize) -> i32 {
    (len - index) as i32
}

/// springboard.conf for `ctrl_dir` with the given networks, first = highest priority.
/// `wowlan_triggers=any` keeps the chip associated through suspend and lets
/// incoming traffic wake the host (see bare::sleep).
pub fn conf(ctrl_dir: &str, networks: &[NetworkConfig]) -> String {
    let mut c = format!("ctrl_interface={ctrl_dir}\nupdate_config=1\nap_scan=1\npmf=1\nwowlan_triggers=any\n");
    for (i, n) in networks.iter().enumerate() {
        c += &n.block(priority_of(i, networks.len()));
    }
    c
}

/// A quoted string, or hex bytes, as wpa_supplicant writes ssid, identity and password.
fn decode(v: &str) -> String {
    if let Some(q) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return q.to_string();
    }
    if !v.is_empty() && v.len() % 2 == 0 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        let bytes: Vec<u8> = (0..v.len()).step_by(2).filter_map(|i| u8::from_str_radix(&v[i..i + 2], 16).ok()).collect();
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    v.to_string()
}

/// A passphrase keeps an unquoted 64-digit hex key verbatim.
fn psk_text(v: &str) -> String {
    v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v).to_string()
}

fn from_fields(f: &[(String, String)]) -> Option<(i32, NetworkConfig)> {
    let get = |k: &str| f.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let ssid = decode(get("ssid")?);
    let mgmt = get("key_mgmt").unwrap_or("");
    let has = |m: &str| mgmt.split_whitespace().any(|t| t == m);
    let eap = if get("eap").is_some_and(|e| e.to_uppercase().contains("TTLS")) { Eap::Ttls } else { Eap::Peap };
    let (security, password) = if has("WPA-EAP") {
        (Security::Enterprise(eap), get("password").map(decode).unwrap_or_default())
    } else if has("SAE") && has("WPA-PSK") {
        (Security::Wpa2Wpa3, get("psk").or(get("sae_password")).map(psk_text).unwrap_or_default())
    } else if has("SAE") {
        (Security::Wpa3, get("sae_password").or(get("psk")).map(psk_text).unwrap_or_default())
    } else if has("WPA-PSK") || (mgmt.is_empty() && get("psk").is_some()) {
        (Security::Wpa2, get("psk").map(psk_text).unwrap_or_default())
    } else {
        (Security::Open, String::new())
    };
    let cfg = NetworkConfig {
        ssid,
        security,
        password,
        hidden: get("scan_ssid") == Some("1"),
        identity: get("identity").map(decode).unwrap_or_default(),
        anonymous: get("anonymous_identity").map(decode).unwrap_or_default(),
        auto_join: get("disabled") != Some("1"),
    };
    Some((get("priority").and_then(|p| p.parse().ok()).unwrap_or(0), cfg))
}

/// The networks of a config file, highest priority first; files without
/// `priority=` (older ones) keep their order.
pub fn parse_conf(text: &str) -> Vec<NetworkConfig> {
    let mut found: Vec<(i32, NetworkConfig)> = Vec::new();
    let mut block: Option<Vec<(String, String)>> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with("network={") {
            block = Some(Vec::new());
        } else if line == "}" {
            if let Some(f) = block.take() {
                found.extend(from_fields(&f));
            }
        } else if let (Some(b), Some((k, v))) = (&mut block, line.split_once('=')) {
            b.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    found.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
    found.into_iter().map(|(_, n)| n).collect()
}

/// Adds `cfg` as the first network; one with the same name is replaced.
pub fn upsert(nets: &mut Vec<NetworkConfig>, cfg: NetworkConfig) {
    nets.retain(|n| n.ssid != cfg.ssid);
    nets.insert(0, cfg);
}

/// Swaps `ssid` with its neighbour above (`up`) or below; false when it
/// is unknown or already at that end.
pub fn move_network(nets: &mut [NetworkConfig], ssid: &str, up: bool) -> bool {
    let Some(i) = nets.iter().position(|n| n.ssid == ssid) else { return false };
    let j = if up { i.checked_sub(1) } else { (i + 1 < nets.len()).then_some(i + 1) };
    match j {
        Some(j) => {
            nets.swap(i, j);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enterprise() -> NetworkConfig {
        NetworkConfig { identity: "alice@corp".into(), anonymous: "anon@corp".into(), password: "pa\"ss".into(), hidden: true, ..NetworkConfig::new("Corp", Security::Enterprise(Eap::Ttls)) }
    }

    #[test]
    fn open_and_personal_blocks() {
        let open = NetworkConfig::new("Cafe", Security::Open).block(2);
        assert!(open.contains("\tssid=\"Cafe\"\n\tkey_mgmt=NONE\n\tpriority=2\n"));
        assert!(!open.contains("scan_ssid"));
        let wpa2 = NetworkConfig { password: "secret12".into(), ..NetworkConfig::new("Home", Security::Wpa2) }.block(1);
        assert!(wpa2.contains("key_mgmt=WPA-PSK\n\tpsk=\"secret12\"\n"));
        let wpa3 = NetworkConfig { password: "secret12".into(), ..NetworkConfig::new("Home", Security::Wpa3) }.block(1);
        assert!(wpa3.contains("key_mgmt=SAE\n\tsae_password=\"secret12\"\n\tieee80211w=2\n"));
        let mixed = NetworkConfig::personal("Home", Some("secret12")).block(1);
        assert!(mixed.contains("key_mgmt=WPA-PSK SAE\n\tpsk=\"secret12\"\n\tieee80211w=1\n"));
    }

    #[test]
    fn hidden_and_disabled_flags() {
        let n = NetworkConfig { hidden: true, auto_join: false, ..NetworkConfig::new("Ghost", Security::Open) };
        let b = n.block(1);
        assert!(b.contains("\tscan_ssid=1\n"));
        assert!(b.ends_with("\tdisabled=1\n}\n"));
    }

    #[test]
    fn enterprise_block_validates_no_server() {
        let b = enterprise().block(3);
        assert!(b.contains("key_mgmt=WPA-EAP\n\teap=TTLS\n\tidentity=\"alice@corp\"\n\tanonymous_identity=\"anon@corp\"\n"));
        assert!(b.contains("phase2=\"auth=MSCHAPV2\""));
        assert!(!b.contains("ca_cert"), "the server certificate is deliberately not checked");
        assert!(b.contains("password=7061227373"), "a quote in the password is written as hex");
        let plain = NetworkConfig { anonymous: String::new(), ..enterprise() }.block(3);
        assert!(!plain.contains("anonymous_identity"));
    }

    #[test]
    fn conf_round_trips_in_priority_order() {
        let nets = vec![
            NetworkConfig::personal("First", Some("secret12")),
            enterprise(),
            NetworkConfig { auto_join: false, ..NetworkConfig::new("Cafe", Security::Open) },
            NetworkConfig { password: "sae-pass".into(), ..NetworkConfig::new("Three", Security::Wpa3) },
            NetworkConfig { password: "secret12".into(), ..NetworkConfig::new("Home Wi-Fi é", Security::Wpa2) },
        ];
        let text = conf("/d/wpa", &nets);
        assert!(text.starts_with("ctrl_interface=/d/wpa\n"));
        assert!(text.contains("\nwowlan_triggers=any\n"));
        assert_eq!(parse_conf(&text), nets);
    }

    #[test]
    fn parses_a_conf_saved_by_the_supplicant_itself() {
        let text = "ctrl_interface=/d\nupdate_config=1\n\nnetwork={\n\tssid=\"Old\"\n\tpsk=\"secret12\"\n\tkey_mgmt=WPA-PSK\n\tid_str=\"x\"\n}\n\nnetwork={\n\tssid=4361666520\n\tkey_mgmt=NONE\n}\n\nnetwork={\n\tssid=\"Raw\"\n\tpsk=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n}\n";
        let n = parse_conf(text);
        assert_eq!(n.len(), 3);
        assert_eq!((n[0].ssid.as_str(), n[0].security, n[0].password.as_str()), ("Old", Security::Wpa2, "secret12"));
        assert_eq!((n[1].ssid.as_str(), n[1].security), ("Cafe ", Security::Open));
        assert_eq!((n[2].security, n[2].password.len()), (Security::Wpa2, 64));
        assert!(n[2].block(1).contains("psk=0123456789abcdef"), "a raw key stays unquoted");
        assert!(n.iter().all(|c| c.auto_join && !c.hidden));
    }

    #[test]
    fn debug_never_shows_secrets() {
        let shown = format!("{:?}", enterprise());
        assert!(!shown.contains("alice") && !shown.contains("pa\\\"ss") && !shown.contains("anon@corp"));
        assert!(shown.contains("Corp"));
    }

    #[test]
    fn completeness_rules() {
        assert!(NetworkConfig::new("", Security::Open).problem().is_some());
        assert!(NetworkConfig::new(&"x".repeat(33), Security::Open).problem().is_some());
        assert_eq!(NetworkConfig::new("Cafe", Security::Open).problem(), None);
        assert!(NetworkConfig::personal("Home", Some("short")).problem().is_some());
        assert_eq!(NetworkConfig::personal("Home", Some("longenough")).problem(), None);
        assert_eq!(NetworkConfig { password: "x".into(), ..NetworkConfig::new("Home", Security::Wpa3) }.problem(), None);
        assert!(NetworkConfig::new("Corp", Security::Enterprise(Eap::Peap)).problem().is_some());
        assert_eq!(enterprise().problem(), None);
    }

    #[test]
    fn security_cycles_through_the_choices() {
        let mut s = Security::Open;
        let mut seen = vec![s.label()];
        for _ in 0..4 {
            s = s.next();
            seen.push(s.label());
        }
        assert_eq!(seen, ["None", "WPA2 Personal", "WPA3 Personal", "WPA2/WPA3 Personal", "WPA2 Enterprise"]);
        assert_eq!(s.next(), Security::Open);
        assert_eq!(Security::Enterprise(Eap::Ttls).next(), Security::Open);
    }

    #[test]
    fn upsert_puts_the_newest_first_and_replaces_by_name() {
        let mut nets = vec![NetworkConfig::new("A", Security::Open), NetworkConfig::new("B", Security::Open)];
        upsert(&mut nets, NetworkConfig::personal("B", Some("secret12")));
        assert_eq!(nets.iter().map(|n| n.ssid.as_str()).collect::<Vec<_>>(), ["B", "A"]);
        assert_eq!(nets[0].security, Security::Wpa2Wpa3);
    }

    #[test]
    fn moving_swaps_neighbours_and_stops_at_the_ends() {
        let mut nets: Vec<NetworkConfig> = ["A", "B", "C"].iter().map(|s| NetworkConfig::new(s, Security::Open)).collect();
        assert!(move_network(&mut nets, "C", true));
        assert!(!move_network(&mut nets, "A", true));
        assert!(!move_network(&mut nets, "B", false));
        assert!(!move_network(&mut nets, "Z", true));
        assert_eq!(nets.iter().map(|n| n.ssid.as_str()).collect::<Vec<_>>(), ["A", "C", "B"]);
        let text = conf("/d", &nets);
        assert_eq!(parse_conf(&text).iter().map(|n| n.ssid.clone()).collect::<Vec<_>>(), ["A", "C", "B"]);
    }
}

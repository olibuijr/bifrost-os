//! Hotspot configuration for apps: the Wi-Fi network an app asks the system to remember, the checks the system makes
//! before it accepts one, and the list of networks the app has added.
//!
//! This is a pure model with no radio access. The system layer shows the consent prompt, writes an accepted
//! configuration into the supplicant and joins the network. `HotspotManager::apply` takes the user's answer as a flag,
//! so the model needs no UI. Each manager holds only the configurations its app added, so `remove_configuration_for_ssid`
//! cannot delete a network another app or the user set up.

use std::fmt;

/// Why a hotspot request was refused. The variants follow the hotspot configuration error values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotspotError {
    /// The network name is empty or longer than 32 octets.
    InvalidSsid,
    /// The WPA/WPA2 passphrase is not 8 to 63 printable ASCII characters or 64 hexadecimal digits.
    InvalidWpaPassphrase,
    /// The WEP key is not 5 or 13 printable ASCII characters or 10 or 26 hexadecimal digits.
    InvalidWepPassphrase,
    /// The user refused the network change at the consent prompt.
    UserDenied,
    /// Another network change has not finished yet.
    Pending,
}

impl fmt::Display for HotspotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HotspotError::InvalidSsid => write!(f, "The network name is not valid"),
            HotspotError::InvalidWpaPassphrase => write!(f, "The WPA password is not valid"),
            HotspotError::InvalidWepPassphrase => write!(f, "The WEP password is not valid"),
            HotspotError::UserDenied => write!(f, "The user refused the network change"),
            HotspotError::Pending => write!(f, "The network change has not completed"),
        }
    }
}

impl std::error::Error for HotspotError {}

/// The security of a hotspot configuration: an open network, a WEP network or a WPA/WPA2 personal network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotspotSecurity {
    Open,
    Wep,
    WpaPersonal,
}

/// The settings for one Wi-Fi network. Build it with `open` or `with_passphrase`, then pass it to `HotspotManager::apply`.
#[derive(Clone, PartialEq, Eq)]
pub struct HotspotConfiguration {
    ssid: String,
    security: HotspotSecurity,
    passphrase: String,
    /// The network does not broadcast its name; the join probes for it.
    pub hidden: bool,
    /// The configuration lasts only while the app that added it runs.
    pub join_once: bool,
    /// Days the system keeps the configuration. `None` keeps it until it is removed.
    pub lifetime_days: Option<u32>,
}

// The passphrase is a secret, so it stays out of logs and test failures.
impl fmt::Debug for HotspotConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HotspotConfiguration")
            .field("ssid", &self.ssid)
            .field("security", &self.security)
            .field("hidden", &self.hidden)
            .field("join_once", &self.join_once)
            .field("lifetime_days", &self.lifetime_days)
            .field("passphrase", &if self.passphrase.is_empty() { "none" } else { "<redacted>" })
            .finish()
    }
}

impl HotspotConfiguration {
    /// A configuration for an open network, identified by its SSID.
    pub fn open(ssid: &str) -> Self {
        HotspotConfiguration {
            ssid: ssid.to_string(),
            security: HotspotSecurity::Open,
            passphrase: String::new(),
            hidden: false,
            join_once: false,
            lifetime_days: None,
        }
    }

    /// A configuration for a protected network: WEP when `is_wep`, otherwise WPA/WPA2 personal.
    pub fn with_passphrase(ssid: &str, passphrase: &str, is_wep: bool) -> Self {
        let security = if is_wep { HotspotSecurity::Wep } else { HotspotSecurity::WpaPersonal };
        HotspotConfiguration { security, passphrase: passphrase.to_string(), ..HotspotConfiguration::open(ssid) }
    }

    /// The network name this configuration is identified by.
    pub fn ssid(&self) -> &str {
        &self.ssid
    }

    /// The security type of the network.
    pub fn security(&self) -> HotspotSecurity {
        self.security
    }

    /// Checks the name and the passphrase the way the system does before it accepts a configuration.
    pub fn validate(&self) -> Result<(), HotspotError> {
        // An SSID is 1 to 32 octets. `len` counts bytes, which are the octets.
        if self.ssid.is_empty() || self.ssid.len() > 32 {
            return Err(HotspotError::InvalidSsid);
        }
        match self.security {
            HotspotSecurity::Open => Ok(()),
            HotspotSecurity::Wep if wep_key_ok(&self.passphrase) => Ok(()),
            HotspotSecurity::Wep => Err(HotspotError::InvalidWepPassphrase),
            HotspotSecurity::WpaPersonal if wpa_passphrase_ok(&self.passphrase) => Ok(()),
            HotspotSecurity::WpaPersonal => Err(HotspotError::InvalidWpaPassphrase),
        }
    }
}

/// A WPA/WPA2 passphrase is 8 to 63 printable ASCII characters, or a 64-digit hexadecimal pre-shared key.
fn wpa_passphrase_ok(p: &str) -> bool {
    if p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    (8..=63).contains(&p.len()) && p.bytes().all(|b| (b' '..=b'~').contains(&b))
}

/// A WEP key is 5 or 13 printable ASCII characters, or 10 or 26 hexadecimal digits.
fn wep_key_ok(k: &str) -> bool {
    let ascii = k.bytes().all(|b| (b' '..=b'~').contains(&b));
    let hex = k.bytes().all(|b| b.is_ascii_hexdigit());
    match k.len() {
        5 | 13 => ascii,
        10 | 26 => hex,
        _ => false,
    }
}

/// The networks one app has added. Adding a configuration for a name that is already here replaces it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotspotManager {
    configs: Vec<HotspotConfiguration>,
}

impl HotspotManager {
    /// An empty manager. An app holds one for its own configurations.
    pub fn new() -> Self {
        HotspotManager::default()
    }

    /// Adds or updates a configuration. The configuration is checked first, then the user's answer to the consent
    /// prompt (`allowed_by_user`) is applied. The model never asks the user itself, and it does not join the network;
    /// the system layer does that after an `Ok`.
    pub fn apply(&mut self, config: HotspotConfiguration, allowed_by_user: bool) -> Result<(), HotspotError> {
        config.validate()?;
        if !allowed_by_user {
            return Err(HotspotError::UserDenied);
        }
        match self.configs.iter().position(|c| c.ssid == config.ssid) {
            Some(i) => self.configs[i] = config,
            None => self.configs.push(config),
        }
        Ok(())
    }

    /// The names of the networks this manager holds, in the order they were first added.
    pub fn configured_ssids(&self) -> Vec<String> {
        self.configs.iter().map(|c| c.ssid.clone()).collect()
    }

    /// The configuration for `ssid`, when this manager holds one.
    pub fn configuration(&self, ssid: &str) -> Option<&HotspotConfiguration> {
        self.configs.iter().find(|c| c.ssid == ssid)
    }

    /// Removes the configuration for `ssid`. Returns false when this manager does not hold one.
    pub fn remove_configuration_for_ssid(&mut self, ssid: &str) -> bool {
        match self.configs.iter().position(|c| c.ssid == ssid) {
            Some(i) => {
                self.configs.remove(i);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssid_of(len: usize) -> String {
        "a".repeat(len)
    }

    #[test]
    fn ssid_is_one_to_thirty_two_octets() {
        assert_eq!(HotspotConfiguration::open("").validate(), Err(HotspotError::InvalidSsid));
        assert_eq!(HotspotConfiguration::open(&ssid_of(1)).validate(), Ok(()));
        assert_eq!(HotspotConfiguration::open(&ssid_of(32)).validate(), Ok(()));
        assert_eq!(HotspotConfiguration::open(&ssid_of(33)).validate(), Err(HotspotError::InvalidSsid));
        // Two two-byte characters are 4 octets, so a 16-character name of them is 32 octets and still valid.
        assert_eq!(HotspotConfiguration::open(&"é".repeat(16)).validate(), Ok(()));
        assert_eq!(HotspotConfiguration::open(&"é".repeat(17)).validate(), Err(HotspotError::InvalidSsid));
    }

    #[test]
    fn wpa_passphrase_is_eight_to_sixty_three_characters_or_sixty_four_hex_digits() {
        let check = |p: &str| HotspotConfiguration::with_passphrase("Cafe", p, false).validate();
        assert_eq!(check("1234567"), Err(HotspotError::InvalidWpaPassphrase));
        assert_eq!(check("12345678"), Ok(()));
        assert_eq!(check(&"x".repeat(63)), Ok(()));
        assert_eq!(check(&"x".repeat(64)), Err(HotspotError::InvalidWpaPassphrase));
        assert_eq!(check(&"0123456789abcdef".repeat(4)), Ok(()));
        // 64 characters that are not all hex is a long passphrase, which is too long.
        assert_eq!(check(&format!("{}g", "0".repeat(63))), Err(HotspotError::InvalidWpaPassphrase));
    }

    #[test]
    fn wep_key_is_five_or_thirteen_ascii_or_ten_or_twenty_six_hex() {
        let check = |k: &str| HotspotConfiguration::with_passphrase("Old", k, true).validate();
        assert_eq!(check("abcde"), Ok(()));
        assert_eq!(check("abcdef"), Err(HotspotError::InvalidWepPassphrase));
        assert_eq!(check("abcdefghijklm"), Ok(()));
        assert_eq!(check("0123456789"), Ok(()));
        assert_eq!(check("0123456789abcdefABCDEF0123"), Ok(()));
        // Ten characters that are not hex is neither a 5 nor a 13 character key.
        assert_eq!(check("abcdefghij"), Err(HotspotError::InvalidWepPassphrase));
    }

    #[test]
    fn apply_checks_the_configuration_then_the_users_answer() {
        let mut m = HotspotManager::new();
        assert_eq!(m.apply(HotspotConfiguration::open("Cafe"), false), Err(HotspotError::UserDenied));
        assert!(m.configured_ssids().is_empty(), "a refused configuration is not kept");
        assert_eq!(m.apply(HotspotConfiguration::with_passphrase("Cafe", "short", false), true), Err(HotspotError::InvalidWpaPassphrase));
        assert_eq!(m.apply(HotspotConfiguration::open("Cafe"), true), Ok(()));
        assert_eq!(m.configured_ssids(), vec!["Cafe".to_string()]);
    }

    #[test]
    fn a_second_configuration_for_the_same_name_replaces_the_first() {
        let mut m = HotspotManager::new();
        m.apply(HotspotConfiguration::open("Cafe"), true).unwrap();
        m.apply(HotspotConfiguration::open("Office"), true).unwrap();
        m.apply(HotspotConfiguration::with_passphrase("Cafe", "12345678", false), true).unwrap();
        assert_eq!(m.configured_ssids(), vec!["Cafe".to_string(), "Office".to_string()]);
        assert_eq!(m.configuration("Cafe").unwrap().security(), HotspotSecurity::WpaPersonal);
        assert!(m.configuration("Unknown").is_none());
    }

    #[test]
    fn remove_only_takes_out_a_configuration_the_manager_holds() {
        let mut m = HotspotManager::new();
        m.apply(HotspotConfiguration::open("Cafe"), true).unwrap();
        assert!(m.remove_configuration_for_ssid("Cafe"));
        assert!(!m.remove_configuration_for_ssid("Cafe"), "removing twice reports nothing removed");
        assert!(!m.remove_configuration_for_ssid("Someone else's network"));
    }

    #[test]
    fn debug_output_never_shows_the_passphrase() {
        let c = HotspotConfiguration::with_passphrase("Cafe", "supersecret", false);
        let text = format!("{c:?}");
        assert!(!text.contains("supersecret"));
        assert!(text.contains("<redacted>"));
        assert!(text.contains("Cafe"));
    }

    #[test]
    fn errors_have_readable_text() {
        assert_eq!(HotspotError::UserDenied.to_string(), "The user refused the network change");
        assert_eq!(HotspotError::InvalidWepPassphrase.to_string(), "The WEP password is not valid");
    }
}

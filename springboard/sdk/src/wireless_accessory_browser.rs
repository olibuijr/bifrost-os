//! Wi-Fi setup for unconfigured accessories: an app searches for accessories that are not yet on a network, and asks
//! for one to be given a network name and passphrase.
//!
//! This is a pure model with no network access. The system calls `WirelessBrowser::found` with each accessory it
//! hears from. `configure` checks the credentials and returns a `ConfigRequest`; joining the accessory to the network
//! is the Wi-Fi service's job, and nothing sends the request yet. Configuring needs the grant `may_configure`, which
//! the system sets from the app's entitlement. The model does not check that entitlement itself.

use std::fmt;

/// Longest network name in bytes (802.11 allows 32 octets).
pub const MAX_SSID_BYTES: usize = 32;
/// Shortest passphrase the WPA2 personal rules accept, in bytes.
pub const MIN_PASSPHRASE_BYTES: usize = 8;
/// Longest passphrase the WPA2 personal rules accept, in bytes.
pub const MAX_PASSPHRASE_BYTES: usize = 63;
/// A passphrase of exactly this many hex digits is a raw 256-bit key, not a passphrase.
pub const HEX_KEY_DIGITS: usize = 64;

/// Whether the browser is looking for unconfigured accessories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseState {
    Idle,
    Searching,
}

/// An accessory that is not yet on a Wi-Fi network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnconfiguredAccessory {
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    /// The network name the accessory advertises for setup.
    pub ssid: String,
    /// The primary MAC address, six bytes. It identifies the accessory.
    pub mac_address: [u8; 6],
}

/// Why a configuration request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The app does not hold the grant to configure Wi-Fi accessories.
    NotEntitled,
    /// No accessory with this MAC address was found by the current or the last search.
    NotFound,
    /// The network name is empty or longer than `MAX_SSID_BYTES`.
    BadSsid,
    /// The passphrase is neither 8 to 63 printable ASCII characters nor 64 hex digits.
    BadPassphrase,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotEntitled => write!(f, "The app may not configure Wi-Fi accessories"),
            ConfigError::NotFound => write!(f, "The accessory was not found"),
            ConfigError::BadSsid => write!(f, "The network name must be 1 to 32 bytes"),
            ConfigError::BadPassphrase => {
                write!(f, "The passphrase must be 8 to 63 printable characters or 64 hex digits")
            }
        }
    }
}

/// A checked request to join an accessory to a network. The passphrase is kept out of `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigRequest {
    pub mac_address: [u8; 6],
    pub ssid: String,
    pub passphrase: String,
}

impl fmt::Debug for ConfigRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigRequest")
            .field("mac_address", &self.mac_address)
            .field("ssid", &self.ssid)
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

/// The browser: its search state, the accessories it has found, and the configuration grant.
#[derive(Debug)]
pub struct WirelessBrowser {
    may_configure: bool,
    state: BrowseState,
    /// Lower-cased name filter for the current search; `None` accepts every accessory.
    name_filter: Option<String>,
    found: Vec<UnconfiguredAccessory>,
}

impl WirelessBrowser {
    /// A browser that is not searching. `may_configure` is the app's grant to configure Wi-Fi accessories.
    pub fn new(may_configure: bool) -> Self {
        WirelessBrowser {
            may_configure,
            state: BrowseState::Idle,
            name_filter: None,
            found: Vec::new(),
        }
    }

    /// `startSearchingForUnconfiguredAccessories`: begins a search and clears the previous results. `name_filter`
    /// keeps only accessories whose name contains the text, ignoring case.
    pub fn start_searching(&mut self, name_filter: Option<&str>) {
        self.state = BrowseState::Searching;
        self.name_filter = name_filter.map(|f| f.to_lowercase());
        self.found.clear();
    }

    /// `stopSearchingForUnconfiguredAccessories`: ends the search. The results stay readable.
    pub fn stop_searching(&mut self) {
        self.state = BrowseState::Idle;
    }

    /// The current search state.
    pub fn state(&self) -> BrowseState {
        self.state
    }

    /// Called by the system when it hears from an unconfigured accessory. Returns true when the accessory was added.
    /// It is refused while no search runs, when its name fails the filter, and when its MAC address is already kept.
    pub fn found(&mut self, accessory: UnconfiguredAccessory) -> bool {
        if self.state != BrowseState::Searching {
            return false;
        }
        if let Some(filter) = &self.name_filter {
            if !accessory.name.to_lowercase().contains(filter.as_str()) {
                return false;
            }
        }
        if self.found.iter().any(|a| a.mac_address == accessory.mac_address) {
            return false;
        }
        self.found.push(accessory);
        true
    }

    /// `unconfiguredAccessories`: the accessories found by the current or the last search, in discovery order.
    pub fn unconfigured_accessories(&self) -> &[UnconfiguredAccessory] {
        &self.found
    }

    /// `configureAccessory`: checks the grant, the accessory and the credentials, then returns the request to join
    /// the accessory to the network. Nothing is sent here.
    pub fn configure(&self, mac_address: [u8; 6], ssid: &str, passphrase: &str) -> Result<ConfigRequest, ConfigError> {
        if !self.may_configure {
            return Err(ConfigError::NotEntitled);
        }
        if !self.found.iter().any(|a| a.mac_address == mac_address) {
            return Err(ConfigError::NotFound);
        }
        if ssid.is_empty() || ssid.len() > MAX_SSID_BYTES {
            return Err(ConfigError::BadSsid);
        }
        if !valid_passphrase(passphrase) {
            return Err(ConfigError::BadPassphrase);
        }
        Ok(ConfigRequest {
            mac_address,
            ssid: ssid.to_string(),
            passphrase: passphrase.to_string(),
        })
    }
}

/// True for 64 hex digits (a raw key) or 8 to 63 printable ASCII bytes (a passphrase).
fn valid_passphrase(passphrase: &str) -> bool {
    if passphrase.len() == HEX_KEY_DIGITS {
        return passphrase.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let len = passphrase.len();
    (MIN_PASSPHRASE_BYTES..=MAX_PASSPHRASE_BYTES).contains(&len)
        && passphrase.bytes().all(|b| (0x20..=0x7E).contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC_A: [u8; 6] = [0, 1, 2, 3, 4, 5];
    const MAC_B: [u8; 6] = [0, 1, 2, 3, 4, 6];

    fn accessory(name: &str, mac: [u8; 6]) -> UnconfiguredAccessory {
        UnconfiguredAccessory {
            name: name.to_string(),
            manufacturer: "Maker".to_string(),
            model: "M1".to_string(),
            ssid: "SETUP-NET".to_string(),
            mac_address: mac,
        }
    }

    #[test]
    fn nothing_is_found_while_the_browser_is_idle() {
        let mut browser = WirelessBrowser::new(true);
        assert_eq!(browser.state(), BrowseState::Idle);
        assert!(!browser.found(accessory("Lamp", MAC_A)));
        assert!(browser.unconfigured_accessories().is_empty());

        browser.start_searching(None);
        assert!(browser.found(accessory("Lamp", MAC_A)));
        browser.stop_searching();
        assert!(!browser.found(accessory("Plug", MAC_B)));
        assert_eq!(browser.unconfigured_accessories().len(), 1);
    }

    #[test]
    fn the_name_filter_keeps_only_matching_accessories() {
        let mut browser = WirelessBrowser::new(true);
        browser.start_searching(Some("CAM"));
        assert!(browser.found(accessory("Garden Camera", MAC_A)));
        assert!(!browser.found(accessory("Lamp", MAC_B)));
        assert_eq!(browser.unconfigured_accessories()[0].name, "Garden Camera");
    }

    #[test]
    fn the_same_mac_address_is_kept_once() {
        let mut browser = WirelessBrowser::new(true);
        browser.start_searching(None);
        assert!(browser.found(accessory("Lamp", MAC_A)));
        assert!(!browser.found(accessory("Lamp again", MAC_A)));
        assert_eq!(browser.unconfigured_accessories().len(), 1);
        assert_eq!(browser.unconfigured_accessories()[0].name, "Lamp");
    }

    #[test]
    fn configuring_without_the_grant_is_refused_before_anything_else() {
        let mut browser = WirelessBrowser::new(false);
        browser.start_searching(None);
        browser.found(accessory("Lamp", MAC_A));
        assert_eq!(browser.configure(MAC_A, "home", "password1"), Err(ConfigError::NotEntitled));
    }

    #[test]
    fn configuring_an_unknown_accessory_is_not_found() {
        let mut browser = WirelessBrowser::new(true);
        browser.start_searching(None);
        browser.found(accessory("Lamp", MAC_A));
        assert_eq!(browser.configure(MAC_B, "home", "password1"), Err(ConfigError::NotFound));
    }

    #[test]
    fn the_network_name_must_be_one_to_32_bytes() {
        let mut browser = WirelessBrowser::new(true);
        browser.start_searching(None);
        browser.found(accessory("Lamp", MAC_A));
        assert_eq!(browser.configure(MAC_A, "", "password1"), Err(ConfigError::BadSsid));
        let too_long = "a".repeat(33);
        assert_eq!(browser.configure(MAC_A, &too_long, "password1"), Err(ConfigError::BadSsid));
        let exactly_32 = "a".repeat(32);
        assert!(browser.configure(MAC_A, &exactly_32, "password1").is_ok());
    }

    #[test]
    fn the_passphrase_follows_the_wpa2_personal_rules() {
        let mut browser = WirelessBrowser::new(true);
        browser.start_searching(None);
        browser.found(accessory("Lamp", MAC_A));
        let check = |p: &str| browser.configure(MAC_A, "home", p);

        assert_eq!(check("short7!"), Err(ConfigError::BadPassphrase));
        assert!(check("password").is_ok());
        assert!(check(&"a".repeat(63)).is_ok());
        assert_eq!(check(&"a".repeat(64)), Err(ConfigError::BadPassphrase));
        assert!(check(&"0123456789abcdef".repeat(4)).is_ok());
        assert_eq!(check(&format!("g{}", "a".repeat(63))), Err(ConfigError::BadPassphrase));
    }

    #[test]
    fn a_valid_request_keeps_the_fields_and_hides_the_passphrase_in_debug_output() {
        let mut browser = WirelessBrowser::new(true);
        browser.start_searching(None);
        browser.found(accessory("Lamp", MAC_A));
        let request = browser.configure(MAC_A, "home", "password1").unwrap();
        assert_eq!(request.mac_address, MAC_A);
        assert_eq!(request.ssid, "home");
        assert_eq!(request.passphrase, "password1");
        let shown = format!("{:?}", request);
        assert!(shown.contains("<redacted>"));
        assert!(!shown.contains("password1"));
    }
}

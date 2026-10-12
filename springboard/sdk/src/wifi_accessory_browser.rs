//! Wi-Fi accessory browser for apps: scan for unconfigured accessories that need a network, keep the ones that match
//! the app's criteria, and start the configuration of one of them.
//!
//! This is the discovery stage in front of [`crate::accessory_hotspot`]: the browser finds an accessory with no network
//! yet, and the join hands it credentials. Pure model with no radio: the transport reports each accessory it hears with
//! [`AccessoryBrowser::report`] and the outcome of a configuration with [`AccessoryBrowser::finish_configure`]. An app
//! without the Wi-Fi accessory configuration entitlement gets no search and no configuration.
//!
//! Rules, in the order they apply:
//! - A search runs only for an entitled app. Starting a search clears the list found by the previous search.
//! - An accessory is kept only while a search runs and it matches the criteria. A repeat of a known MAC address replaces
//!   the record, so the list has one row per accessory.
//! - Stopping the search keeps the accessories already found.
//! - One configuration runs at a time, and only for an accessory in the list. A configured accessory leaves the list.
//!   A failed one stays, so the user can try again.

/// An accessory that has not been set up for a network yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnconfiguredAccessory {
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    /// The Wi-Fi network the accessory broadcasts while it waits for setup.
    pub ssid: String,
    /// The primary MAC address. It identifies the accessory and is stored as lowercase `aa:bb:cc:dd:ee:ff`.
    pub mac_address: String,
}

/// Which accessories a search keeps. Each field that is set must equal the accessory's field exactly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Criteria {
    pub name: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

fn field_matches(want: &Option<String>, have: &str) -> bool {
    match want {
        Some(w) => w == have,
        None => true,
    }
}

impl Criteria {
    pub fn matches(&self, accessory: &UnconfiguredAccessory) -> bool {
        field_matches(&self.name, &accessory.name)
            && field_matches(&self.manufacturer, &accessory.manufacturer)
            && field_matches(&self.model, &accessory.model)
    }
}

/// Normalises a MAC address to lowercase, colon-separated hex. `None` unless it is six hex pairs.
pub fn normalise_mac(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 || parts.iter().any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_hexdigit())) {
        return None;
    }
    Some(parts.join(":"))
}

/// Why a browser call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseError {
    /// The app does not hold the Wi-Fi accessory configuration entitlement.
    NotEntitled,
    /// No discovered accessory has this MAC address.
    UnknownAccessory,
    /// Another configuration is still running.
    Busy,
    /// No configuration is running, so there is nothing to finish.
    NothingToFinish,
}

/// The browser an app holds. It is created once per app with the entitlement the app was granted.
#[derive(Clone, Debug, Default)]
pub struct AccessoryBrowser {
    entitled: bool,
    search: Option<Criteria>,
    found: Vec<UnconfiguredAccessory>,
    configuring: Option<String>,
}

impl AccessoryBrowser {
    /// A browser with nothing running. `entitled` is the app's Wi-Fi accessory configuration entitlement.
    pub fn new(entitled: bool) -> Self {
        AccessoryBrowser { entitled, ..Default::default() }
    }

    pub fn is_entitled(&self) -> bool {
        self.entitled
    }

    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    /// The MAC address of the accessory being configured, if one is running.
    pub fn configuring(&self) -> Option<&str> {
        self.configuring.as_deref()
    }

    /// The accessories found so far, in the order they were first reported.
    pub fn unconfigured_accessories(&self) -> &[UnconfiguredAccessory] {
        &self.found
    }

    /// Starts a search for accessories that match `criteria`. A search already running is replaced.
    pub fn start_searching(&mut self, criteria: Criteria) -> Result<(), BrowseError> {
        if !self.entitled {
            return Err(BrowseError::NotEntitled);
        }
        self.search = Some(criteria);
        self.found.clear();
        Ok(())
    }

    /// Stops the search. The accessories already found stay in the list.
    pub fn stop_searching(&mut self) {
        self.search = None;
    }

    /// The transport reports an accessory it heard. It is kept only while a search runs, its MAC address is valid and
    /// it matches the criteria. Returns `false` and changes nothing when the report is ignored.
    pub fn report(&mut self, mut accessory: UnconfiguredAccessory) -> bool {
        let Some(criteria) = &self.search else {
            return false;
        };
        let Some(mac) = normalise_mac(&accessory.mac_address) else {
            return false;
        };
        if !criteria.matches(&accessory) {
            return false;
        }
        accessory.mac_address = mac.clone();
        match self.found.iter().position(|a| a.mac_address == mac) {
            Some(i) => self.found[i] = accessory,
            None => self.found.push(accessory),
        }
        true
    }

    /// Starts configuring the discovered accessory with `mac`. Refused while another configuration runs.
    pub fn configure(&mut self, mac: &str) -> Result<(), BrowseError> {
        if !self.entitled {
            return Err(BrowseError::NotEntitled);
        }
        if self.configuring.is_some() {
            return Err(BrowseError::Busy);
        }
        let mac = normalise_mac(mac).ok_or(BrowseError::UnknownAccessory)?;
        if !self.found.iter().any(|a| a.mac_address == mac) {
            return Err(BrowseError::UnknownAccessory);
        }
        self.configuring = Some(mac);
        Ok(())
    }

    /// Records the outcome the transport reports for the running configuration. A configured accessory leaves the list.
    pub fn finish_configure(&mut self, configured: bool) -> Result<(), BrowseError> {
        let mac = self.configuring.take().ok_or(BrowseError::NothingToFinish)?;
        if configured {
            self.found.retain(|a| a.mac_address != mac);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accessory(name: &str, mac: &str) -> UnconfiguredAccessory {
        UnconfiguredAccessory {
            name: name.into(),
            manufacturer: "Acme".into(),
            model: "L1".into(),
            ssid: "Setup-L1".into(),
            mac_address: mac.into(),
        }
    }

    const MAC_1: &str = "aa:bb:cc:00:11:22";
    const MAC_2: &str = "aa:bb:cc:00:11:33";

    #[test]
    fn an_app_without_the_entitlement_cannot_search_or_configure() {
        let mut b = AccessoryBrowser::new(false);
        assert_eq!(b.start_searching(Criteria::default()), Err(BrowseError::NotEntitled));
        assert!(!b.is_searching());
        assert_eq!(b.configure(MAC_1), Err(BrowseError::NotEntitled));
        assert!(!b.report(accessory("Lamp", MAC_1)));
    }

    #[test]
    fn a_search_keeps_only_matching_accessories_and_one_row_per_mac() {
        let mut b = AccessoryBrowser::new(true);
        let criteria = Criteria { name: Some("Lamp".into()), ..Default::default() };
        assert_eq!(b.start_searching(criteria), Ok(()));
        assert!(b.report(accessory("Lamp", "AA:BB:CC:00:11:22")));
        assert!(!b.report(accessory("Fan", MAC_2)));
        assert!(b.report(accessory("Lamp", MAC_1)));
        let found = b.unconfigured_accessories();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Lamp");
        assert_eq!(found[0].mac_address, MAC_1);
    }

    #[test]
    fn reports_are_ignored_outside_a_search_and_stopping_keeps_the_list() {
        let mut b = AccessoryBrowser::new(true);
        assert!(!b.report(accessory("Lamp", MAC_1)));
        assert!(b.unconfigured_accessories().is_empty());
        b.start_searching(Criteria::default()).unwrap();
        assert!(b.report(accessory("Lamp", MAC_1)));
        b.stop_searching();
        assert!(!b.is_searching());
        assert!(!b.report(accessory("Lamp", MAC_2)));
        assert_eq!(b.unconfigured_accessories().len(), 1);
    }

    #[test]
    fn a_new_search_clears_what_the_previous_one_found() {
        let mut b = AccessoryBrowser::new(true);
        b.start_searching(Criteria::default()).unwrap();
        b.report(accessory("Lamp", MAC_1));
        b.start_searching(Criteria::default()).unwrap();
        assert!(b.unconfigured_accessories().is_empty());
    }

    #[test]
    fn one_configuration_runs_at_a_time_and_a_configured_accessory_leaves_the_list() {
        let mut b = AccessoryBrowser::new(true);
        b.start_searching(Criteria::default()).unwrap();
        b.report(accessory("Lamp", MAC_1));
        b.report(accessory("Fan", MAC_2));
        assert_eq!(b.configure(MAC_1), Ok(()));
        assert_eq!(b.configuring(), Some(MAC_1));
        assert_eq!(b.configure(MAC_2), Err(BrowseError::Busy));
        assert_eq!(b.finish_configure(true), Ok(()));
        assert_eq!(b.configuring(), None);
        assert_eq!(b.unconfigured_accessories().len(), 1);
        assert_eq!(b.unconfigured_accessories()[0].mac_address, MAC_2);
        assert_eq!(b.configure(MAC_1), Err(BrowseError::UnknownAccessory));
        assert_eq!(b.finish_configure(true), Err(BrowseError::NothingToFinish));
    }

    #[test]
    fn a_failed_configuration_keeps_the_accessory_for_a_retry() {
        let mut b = AccessoryBrowser::new(true);
        b.start_searching(Criteria::default()).unwrap();
        b.report(accessory("Lamp", MAC_1));
        b.configure(MAC_1).unwrap();
        assert_eq!(b.finish_configure(false), Ok(()));
        assert_eq!(b.unconfigured_accessories().len(), 1);
        assert_eq!(b.configure(MAC_1), Ok(()));
    }

    #[test]
    fn mac_addresses_are_normalised_or_refused() {
        assert_eq!(normalise_mac(" AA:BB:CC:00:11:22 "), Some(MAC_1.to_string()));
        assert_eq!(normalise_mac("aa:bb:cc:dd:ee"), None);
        assert_eq!(normalise_mac("aa:bb:cc:dd:ee:gg"), None);
        assert_eq!(normalise_mac("aabbcc001122"), None);
        let mut b = AccessoryBrowser::new(true);
        b.start_searching(Criteria::default()).unwrap();
        assert!(!b.report(accessory("Lamp", "aa:bb:cc:dd:ee")));
    }
}

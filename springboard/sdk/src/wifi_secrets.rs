//! Wi-Fi network secrets for apps, kept per network name (SSID) and keychain domain: the shared password, the
//! 802.1X username and password, and the identity label chosen for an enterprise network.
//!
//! This is the pure model of the store. It holds the values in memory and makes no service call, so the shell
//! can back it with the keychain later without changing the app-facing names. Rules: the SSID is 1 to 32 bytes;
//! a secret (password, username, 802.1X password) is 1 to 63 bytes; the domain must be `User` or `System`. A
//! store that breaks a rule answers [`Error::Param`]; a find or delete with nothing stored answers
//! [`Error::ItemNotFound`]. Setting a value that exists replaces it, as an update does.

use std::collections::BTreeMap;

/// Longest network name an access point may advertise, in bytes.
pub const MAX_SSID_BYTES: usize = 32;
/// Longest secret accepted, in bytes (the longest WPA passphrase).
pub const MAX_SECRET_BYTES: usize = 63;

/// Keychain domain a Wi-Fi secret is stored in. `None` names no domain and is always rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Domain {
    None,
    /// Secrets of the signed-in account (the default for apps).
    User,
    /// Secrets shared by every account on the device.
    System,
}

/// Why a Wi-Fi secret call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Nothing is stored for the network in that domain.
    ItemNotFound,
    /// A name, secret or domain breaks the rules in the module comment.
    Param,
}

/// An 802.1X username and password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EapCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entry {
    password: Option<String>,
    eap: Option<EapCredentials>,
    identity: Option<String>,
}

impl Entry {
    fn is_empty(&self) -> bool {
        self.password.is_none() && self.eap.is_none() && self.identity.is_none()
    }
}

/// The Wi-Fi secret store. Each network has up to three slots: password, 802.1X credentials and identity label.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiSecrets {
    items: BTreeMap<(Domain, String), Entry>,
}

fn key(domain: Domain, ssid: &str) -> Result<(Domain, String), Error> {
    if domain == Domain::None || ssid.is_empty() || ssid.len() > MAX_SSID_BYTES {
        return Err(Error::Param);
    }
    Ok((domain, ssid.to_string()))
}

fn check_secret(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > MAX_SECRET_BYTES {
        Err(Error::Param)
    } else {
        Ok(())
    }
}

impl WifiSecrets {
    pub fn new() -> WifiSecrets {
        WifiSecrets::default()
    }

    /// Stores the shared password for a network, replacing any earlier one.
    pub fn set_password(&mut self, domain: Domain, ssid: &str, password: &str) -> Result<(), Error> {
        let k = key(domain, ssid)?;
        check_secret(password)?;
        self.items.entry(k).or_default().password = Some(password.to_string());
        Ok(())
    }

    /// The shared password for a network.
    pub fn find_password(&self, domain: Domain, ssid: &str) -> Result<&str, Error> {
        let k = key(domain, ssid)?;
        self.items.get(&k).and_then(|e| e.password.as_deref()).ok_or(Error::ItemNotFound)
    }

    /// Removes the shared password; the 802.1X slots of the network are kept.
    pub fn delete_password(&mut self, domain: Domain, ssid: &str) -> Result<(), Error> {
        let k = key(domain, ssid)?;
        let entry = self.items.get_mut(&k).ok_or(Error::ItemNotFound)?;
        if entry.password.take().is_none() {
            return Err(Error::ItemNotFound);
        }
        self.prune(&k);
        Ok(())
    }

    /// Stores the 802.1X username and password for a network, replacing any earlier pair.
    pub fn set_eap_username_password(
        &mut self,
        domain: Domain,
        ssid: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Error> {
        let k = key(domain, ssid)?;
        check_secret(username)?;
        check_secret(password)?;
        let creds = EapCredentials { username: username.to_string(), password: password.to_string() };
        self.items.entry(k).or_default().eap = Some(creds);
        Ok(())
    }

    /// The 802.1X username and password for a network.
    pub fn find_eap_username_password(&self, domain: Domain, ssid: &str) -> Result<&EapCredentials, Error> {
        let k = key(domain, ssid)?;
        self.items.get(&k).and_then(|e| e.eap.as_ref()).ok_or(Error::ItemNotFound)
    }

    /// Removes the 802.1X username and password; the password and identity slots are kept.
    pub fn delete_eap_username_password(&mut self, domain: Domain, ssid: &str) -> Result<(), Error> {
        let k = key(domain, ssid)?;
        let entry = self.items.get_mut(&k).ok_or(Error::ItemNotFound)?;
        if entry.eap.take().is_none() {
            return Err(Error::ItemNotFound);
        }
        self.prune(&k);
        Ok(())
    }

    /// Associates an identity label (the name the user picked for a certificate or account) with a network.
    pub fn set_eap_identity(&mut self, domain: Domain, ssid: &str, identity: &str) -> Result<(), Error> {
        let k = key(domain, ssid)?;
        check_secret(identity)?;
        self.items.entry(k).or_default().identity = Some(identity.to_string());
        Ok(())
    }

    /// The identity label associated with a network.
    pub fn find_eap_identity(&self, domain: Domain, ssid: &str) -> Result<&str, Error> {
        let k = key(domain, ssid)?;
        self.items.get(&k).and_then(|e| e.identity.as_deref()).ok_or(Error::ItemNotFound)
    }

    /// Every identity label stored in a domain, as `(network name, label)` pairs sorted by network name.
    /// `Domain::None` lists nothing.
    pub fn eap_identities(&self, domain: Domain) -> Vec<(&str, &str)> {
        if domain == Domain::None {
            return Vec::new();
        }
        self.items
            .iter()
            .filter(|((d, _), _)| *d == domain)
            .filter_map(|((_, ssid), e)| e.identity.as_deref().map(|id| (ssid.as_str(), id)))
            .collect()
    }

    fn prune(&mut self, k: &(Domain, String)) {
        if self.items.get(k).is_some_and(|e| e.is_empty()) {
            self.items.remove(k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_is_stored_and_replaced() {
        let mut s = WifiSecrets::new();
        s.set_password(Domain::User, "Home", "first-pass").unwrap();
        s.set_password(Domain::User, "Home", "second-pass").unwrap();
        assert_eq!(s.find_password(Domain::User, "Home"), Ok("second-pass"));
    }

    #[test]
    fn domains_do_not_share_items() {
        let mut s = WifiSecrets::new();
        s.set_password(Domain::User, "Cafe", "coffee-123").unwrap();
        assert_eq!(s.find_password(Domain::System, "Cafe"), Err(Error::ItemNotFound));
        assert_eq!(s.find_password(Domain::User, "Cafe"), Ok("coffee-123"));
    }

    #[test]
    fn ssid_length_is_one_to_thirty_two_bytes() {
        let mut s = WifiSecrets::new();
        let longest = "n".repeat(32);
        let too_long = "n".repeat(33);
        assert_eq!(s.set_password(Domain::User, "", "pw"), Err(Error::Param));
        assert_eq!(s.set_password(Domain::User, &too_long, "pw"), Err(Error::Param));
        assert_eq!(s.set_password(Domain::User, &longest, "pw"), Ok(()));
    }

    #[test]
    fn secret_length_is_one_to_sixty_three_bytes() {
        let mut s = WifiSecrets::new();
        assert_eq!(s.set_password(Domain::User, "Net", ""), Err(Error::Param));
        assert_eq!(s.set_password(Domain::User, "Net", &"p".repeat(64)), Err(Error::Param));
        assert_eq!(s.set_password(Domain::User, "Net", &"p".repeat(63)), Ok(()));
    }

    #[test]
    fn domain_none_is_rejected_everywhere() {
        let mut s = WifiSecrets::new();
        assert_eq!(s.set_password(Domain::None, "Net", "pw"), Err(Error::Param));
        assert_eq!(s.find_password(Domain::None, "Net"), Err(Error::Param));
        assert!(s.eap_identities(Domain::None).is_empty());
    }

    #[test]
    fn delete_removes_only_its_slot_and_reports_missing() {
        let mut s = WifiSecrets::new();
        s.set_password(Domain::User, "Office", "office-pw").unwrap();
        s.set_eap_username_password(Domain::User, "Office", "ola", "eap-pw").unwrap();
        s.delete_password(Domain::User, "Office").unwrap();
        assert_eq!(s.find_password(Domain::User, "Office"), Err(Error::ItemNotFound));
        assert_eq!(s.find_eap_username_password(Domain::User, "Office").unwrap().username, "ola");
        assert_eq!(s.delete_password(Domain::User, "Office"), Err(Error::ItemNotFound));
        s.delete_eap_username_password(Domain::User, "Office").unwrap();
        assert_eq!(s.find_eap_username_password(Domain::User, "Office"), Err(Error::ItemNotFound));
    }

    #[test]
    fn identity_list_is_per_domain_and_sorted_by_network() {
        let mut s = WifiSecrets::new();
        s.set_eap_identity(Domain::User, "Work", "work-cert").unwrap();
        s.set_eap_identity(Domain::User, "Campus", "student-id").unwrap();
        s.set_password(Domain::User, "Home", "home-pw").unwrap();
        s.set_eap_identity(Domain::System, "Work", "shared-cert").unwrap();
        assert_eq!(s.eap_identities(Domain::User), vec![("Campus", "student-id"), ("Work", "work-cert")]);
        assert_eq!(s.eap_identities(Domain::System), vec![("Work", "shared-cert")]);
        assert_eq!(s.find_eap_identity(Domain::User, "Home"), Err(Error::ItemNotFound));
    }
}

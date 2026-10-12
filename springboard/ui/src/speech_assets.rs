//! Speech model assets: which locales have a recognition model installed, which are downloading, and which locales
//! the app has reserved. Pure state with no I/O, shaped like the download queue. The transfer runs on the URL session's
//! download task; the shell reports it with `progress`, `finish` and `fail`.
//!
//! Lifecycle per locale: `NotInstalled` -> `Requested` -> `Downloading` -> `Installed`. A failed transfer goes to
//! `Failed(reason)` and may be requested again. An installed model leaves with `remove`. Reservations cap how many
//! locales one app holds at once, which limits storage and network use; the cap is set by the caller.

use std::collections::{BTreeMap, BTreeSet};

/// Longest accepted locale tag, in bytes.
pub const MAX_LOCALE_LEN: usize = 35;

/// Where one locale's model stands. Byte counts are the model's size as the transfer reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallState {
    NotInstalled,
    Requested { total: u64 },
    Downloading { received: u64, total: u64 },
    Installed { bytes: u64 },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InventoryError {
    /// The tag is empty, too long, or holds a character other than a letter, a digit or `-`.
    InvalidLocale(String),
    /// A request, or a removal, while the locale's transfer is still running.
    InProgress,
    AlreadyInstalled,
    NotInstalled,
    /// A progress, finish or failure report for a locale with no transfer running.
    NotInProgress,
    /// The transfer reported finish before every byte arrived.
    Incomplete { received: u64, total: u64 },
    /// The app already holds the most reservations it may hold.
    ReservationLimit(usize),
}

/// Model assets for one app: install states by locale, and the locales it has reserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInventory {
    states: BTreeMap<String, InstallState>,
    reserved: BTreeSet<String>,
    max_reservations: usize,
}

/// Checks a locale tag and returns it in lowercase, so `en-GB` and `en-gb` are one key.
fn key(locale: &str) -> Result<String, InventoryError> {
    let ok = !locale.is_empty()
        && locale.len() <= MAX_LOCALE_LEN
        && !locale.starts_with('-')
        && !locale.ends_with('-')
        && locale.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(locale.to_ascii_lowercase())
    } else {
        Err(InventoryError::InvalidLocale(locale.to_string()))
    }
}

impl AssetInventory {
    pub fn new(max_reservations: usize) -> Self {
        AssetInventory { states: BTreeMap::new(), reserved: BTreeSet::new(), max_reservations }
    }

    /// The install state of a locale. An unknown or invalid tag reads as `NotInstalled`.
    pub fn status(&self, locale: &str) -> InstallState {
        match key(locale) {
            Ok(k) => self.states.get(&k).cloned().unwrap_or(InstallState::NotInstalled),
            Err(_) => InstallState::NotInstalled,
        }
    }

    /// Starts a download of `total` bytes for the locale.
    pub fn request(&mut self, locale: &str, total: u64) -> Result<(), InventoryError> {
        let k = key(locale)?;
        match self.states.get(&k) {
            Some(InstallState::Installed { .. }) => return Err(InventoryError::AlreadyInstalled),
            Some(InstallState::Requested { .. }) | Some(InstallState::Downloading { .. }) => {
                return Err(InventoryError::InProgress)
            }
            _ => {}
        }
        self.states.insert(k, InstallState::Requested { total });
        Ok(())
    }

    /// Reports bytes received so far. A count above the total is clamped to it.
    pub fn progress(&mut self, locale: &str, received: u64) -> Result<(), InventoryError> {
        let k = key(locale)?;
        let total = match self.states.get(&k) {
            Some(InstallState::Requested { total }) | Some(InstallState::Downloading { total, .. }) => *total,
            _ => return Err(InventoryError::NotInProgress),
        };
        self.states.insert(k, InstallState::Downloading { received: received.min(total), total });
        Ok(())
    }

    /// Completes a download. Every byte must have arrived; otherwise the locale stays in progress.
    pub fn finish(&mut self, locale: &str) -> Result<(), InventoryError> {
        let k = key(locale)?;
        let (received, total) = match self.states.get(&k) {
            Some(InstallState::Requested { total }) => (0, *total),
            Some(InstallState::Downloading { received, total }) => (*received, *total),
            _ => return Err(InventoryError::NotInProgress),
        };
        if received < total {
            return Err(InventoryError::Incomplete { received, total });
        }
        self.states.insert(k, InstallState::Installed { bytes: total });
        Ok(())
    }

    /// Records a failed transfer with its reason. The locale can be requested again afterwards.
    pub fn fail(&mut self, locale: &str, reason: &str) -> Result<(), InventoryError> {
        let k = key(locale)?;
        match self.states.get(&k) {
            Some(InstallState::Requested { .. }) | Some(InstallState::Downloading { .. }) => {}
            _ => return Err(InventoryError::NotInProgress),
        }
        self.states.insert(k, InstallState::Failed(reason.to_string()));
        Ok(())
    }

    /// Removes an installed model, or clears a failed entry. A running transfer cannot be removed.
    pub fn remove(&mut self, locale: &str) -> Result<(), InventoryError> {
        let k = key(locale)?;
        let removable = match self.states.get(&k) {
            Some(InstallState::Installed { .. }) | Some(InstallState::Failed(_)) => true,
            Some(InstallState::Requested { .. }) | Some(InstallState::Downloading { .. }) => {
                return Err(InventoryError::InProgress)
            }
            _ => return Err(InventoryError::NotInstalled),
        };
        if removable {
            self.states.remove(&k);
        }
        Ok(())
    }

    /// Locales with an installed model, in sorted order.
    pub fn installed_locales(&self) -> Vec<String> {
        self.states
            .iter()
            .filter(|(_, s)| matches!(s, InstallState::Installed { .. }))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Total bytes of all installed models.
    pub fn installed_bytes(&self) -> u64 {
        self.states
            .values()
            .map(|s| match s {
                InstallState::Installed { bytes } => *bytes,
                _ => 0,
            })
            .sum()
    }

    /// Reserves a locale for the app. Returns `Ok(false)` when the locale was already reserved, and an error when
    /// the app is at its limit.
    pub fn reserve(&mut self, locale: &str) -> Result<bool, InventoryError> {
        let k = key(locale)?;
        if self.reserved.contains(&k) {
            return Ok(false);
        }
        if self.reserved.len() >= self.max_reservations {
            return Err(InventoryError::ReservationLimit(self.max_reservations));
        }
        self.reserved.insert(k);
        Ok(true)
    }

    /// Releases a reservation. Returns false when the locale was not reserved.
    pub fn release(&mut self, locale: &str) -> bool {
        match key(locale) {
            Ok(k) => self.reserved.remove(&k),
            Err(_) => false,
        }
    }

    /// The app's reserved locales, in sorted order.
    pub fn reserved_locales(&self) -> Vec<String> {
        self.reserved.iter().cloned().collect()
    }

    pub fn max_reservations(&self) -> usize {
        self.max_reservations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_runs_from_request_to_installed() {
        let mut inv = AssetInventory::new(2);
        inv.request("en-US", 100).unwrap();
        assert_eq!(inv.status("en-US"), InstallState::Requested { total: 100 });
        inv.progress("en-US", 60).unwrap();
        assert_eq!(inv.status("en-US"), InstallState::Downloading { received: 60, total: 100 });
        inv.progress("en-US", 100).unwrap();
        inv.finish("en-US").unwrap();
        assert_eq!(inv.status("en-US"), InstallState::Installed { bytes: 100 });
        assert_eq!(inv.installed_locales(), vec!["en-us".to_string()]);
        assert_eq!(inv.installed_bytes(), 100);
    }

    #[test]
    fn progress_above_the_total_is_clamped() {
        let mut inv = AssetInventory::new(2);
        inv.request("is-IS", 100).unwrap();
        inv.progress("is-IS", 150).unwrap();
        assert_eq!(inv.status("is-IS"), InstallState::Downloading { received: 100, total: 100 });
        inv.finish("is-IS").unwrap();
        assert_eq!(inv.status("is-IS"), InstallState::Installed { bytes: 100 });
    }

    #[test]
    fn finishing_early_is_refused_and_the_download_keeps_running() {
        let mut inv = AssetInventory::new(2);
        inv.request("de-DE", 100).unwrap();
        inv.progress("de-DE", 40).unwrap();
        assert_eq!(inv.finish("de-DE"), Err(InventoryError::Incomplete { received: 40, total: 100 }));
        assert_eq!(inv.status("de-DE"), InstallState::Downloading { received: 40, total: 100 });
        assert_eq!(inv.request("de-DE", 100), Err(InventoryError::InProgress));
    }

    #[test]
    fn locale_tags_are_checked_and_case_folded() {
        let mut inv = AssetInventory::new(2);
        for bad in ["", "en US", "en_US", "-en", "en-", "en/../x"] {
            assert_eq!(inv.request(bad, 1), Err(InventoryError::InvalidLocale(bad.to_string())));
        }
        inv.request("en-GB", 10).unwrap();
        assert_eq!(inv.status("EN-gb"), InstallState::Requested { total: 10 });
    }

    #[test]
    fn a_failed_download_can_be_requested_again() {
        let mut inv = AssetInventory::new(2);
        inv.request("fr-FR", 50).unwrap();
        inv.fail("fr-FR", "network dropped").unwrap();
        assert_eq!(inv.status("fr-FR"), InstallState::Failed("network dropped".to_string()));
        assert_eq!(inv.progress("fr-FR", 1), Err(InventoryError::NotInProgress));
        inv.request("fr-FR", 50).unwrap();
        assert_eq!(inv.status("fr-FR"), InstallState::Requested { total: 50 });
    }

    #[test]
    fn an_installed_model_is_removed_but_a_running_one_is_not() {
        let mut inv = AssetInventory::new(2);
        inv.request("sv-SE", 30).unwrap();
        assert_eq!(inv.remove("sv-SE"), Err(InventoryError::InProgress));
        inv.progress("sv-SE", 30).unwrap();
        inv.finish("sv-SE").unwrap();
        assert_eq!(inv.request("sv-SE", 30), Err(InventoryError::AlreadyInstalled));
        inv.remove("sv-SE").unwrap();
        assert_eq!(inv.status("sv-SE"), InstallState::NotInstalled);
        assert_eq!(inv.remove("sv-SE"), Err(InventoryError::NotInstalled));
        assert_eq!(inv.installed_bytes(), 0);
    }

    #[test]
    fn reservations_are_capped_and_released() {
        let mut inv = AssetInventory::new(2);
        assert_eq!(inv.reserve("en-US"), Ok(true));
        assert_eq!(inv.reserve("en-us"), Ok(false));
        assert_eq!(inv.reserve("is-IS"), Ok(true));
        assert_eq!(inv.reserve("de-DE"), Err(InventoryError::ReservationLimit(2)));
        assert_eq!(inv.reserved_locales(), vec!["en-us".to_string(), "is-is".to_string()]);
        assert!(inv.release("en-US"));
        assert!(!inv.release("en-US"));
        assert_eq!(inv.reserve("de-DE"), Ok(true));
        assert_eq!(inv.max_reservations(), 2);
    }
}

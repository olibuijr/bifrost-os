//! Bluetooth accessory picker for apps: an app asks for the nearby Bluetooth devices, the user chooses one, and the
//! app receives the chosen address.
//!
//! An app describes what it wants in a [`PickerRequest`]: a name filter and the kinds of device it accepts. The shell
//! builds a picker from the request, calls `BluetoothPicker::offer` with the devices its discovery found, and sends
//! the user's choice back through `select` or `cancel`. The app receives only the chosen address
//! ([`Outcome::address`]). The picker never pairs a device: pairing and bonding stay in the shell's Bluetooth service.
//! The model has no radio access; the request crosses the shell boundary once the shell wires the app side.

use std::fmt;

use bifrost_core::bt_address::{BdAddr, DeviceKind};

/// A nearby device the user may choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub address: BdAddr,
    pub name: String,
    pub kind: DeviceKind,
    pub paired: bool,
}

/// Why a picker call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerError {
    /// The address is not among the devices the picker shows (or the name filter hides it).
    NotShown,
    /// The user already chose a device or cancelled, so the picker takes no more input.
    Finished,
}

impl fmt::Display for PickerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PickerError::NotShown => write!(f, "The device is not in the list"),
            PickerError::Finished => write!(f, "The picker has already finished"),
        }
    }
}

/// How the picker ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The user chose this device.
    Selected(Candidate),
    /// The user dismissed the picker.
    Cancelled,
}

impl Outcome {
    /// The address the app receives: the chosen device's address, or `None` when the user cancelled.
    pub fn address(&self) -> Option<BdAddr> {
        match self {
            Outcome::Selected(candidate) => Some(candidate.address),
            Outcome::Cancelled => None,
        }
    }
}

/// What an app asks the picker to show. An empty `kinds` accepts every kind of device; a device must pass both the
/// name filter and the kind list to be shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickerRequest {
    /// Keeps only the devices whose name contains the text, ignoring case. `None` keeps every name.
    pub name_filter: Option<String>,
    /// The kinds of device the app accepts. Empty accepts every kind.
    pub kinds: Vec<DeviceKind>,
}

/// One picker session: the request it serves, the devices shown, and the outcome once the user has answered.
#[derive(Debug)]
pub struct BluetoothPicker {
    /// Lower-cased name filter; `None` shows every device.
    name_filter: Option<String>,
    kinds: Vec<DeviceKind>,
    shown: Vec<Candidate>,
    outcome: Option<Outcome>,
}

impl BluetoothPicker {
    /// A new picker. `name_filter` keeps only the devices whose name contains the text, ignoring case. `None` keeps
    /// every device.
    pub fn new(name_filter: Option<&str>) -> Self {
        Self::for_request(&PickerRequest {
            name_filter: name_filter.map(str::to_string),
            kinds: Vec::new(),
        })
    }

    /// A picker that serves an app's request.
    pub fn for_request(request: &PickerRequest) -> Self {
        BluetoothPicker {
            name_filter: request.name_filter.as_ref().map(|f| f.to_lowercase()),
            kinds: request.kinds.clone(),
            shown: Vec::new(),
            outcome: None,
        }
    }

    fn matches(&self, candidate: &Candidate) -> bool {
        let named = match &self.name_filter {
            None => true,
            Some(filter) => candidate.name.to_lowercase().contains(filter.as_str()),
        };
        named && (self.kinds.is_empty() || self.kinds.contains(&candidate.kind))
    }

    /// Replaces the shown devices with the candidates that pass the name filter, in the order given. A second
    /// candidate with an address already kept is dropped. Returns how many devices are now shown.
    pub fn offer(&mut self, candidates: &[Candidate]) -> Result<usize, PickerError> {
        if self.outcome.is_some() {
            return Err(PickerError::Finished);
        }
        let mut shown: Vec<Candidate> = Vec::new();
        for candidate in candidates {
            if !self.matches(candidate) {
                continue;
            }
            if shown.iter().any(|kept| kept.address == candidate.address) {
                continue;
            }
            shown.push(candidate.clone());
        }
        self.shown = shown;
        Ok(self.shown.len())
    }

    /// The user chose the device at `address`. Only a shown device can be chosen; a refused call leaves the picker
    /// open for another choice.
    pub fn select(&mut self, address: BdAddr) -> Result<(), PickerError> {
        if self.outcome.is_some() {
            return Err(PickerError::Finished);
        }
        let chosen = match self.shown.iter().find(|c| c.address == address) {
            Some(candidate) => candidate.clone(),
            None => return Err(PickerError::NotShown),
        };
        self.outcome = Some(Outcome::Selected(chosen));
        Ok(())
    }

    /// The user dismissed the picker.
    pub fn cancel(&mut self) -> Result<(), PickerError> {
        if self.outcome.is_some() {
            return Err(PickerError::Finished);
        }
        self.outcome = Some(Outcome::Cancelled);
        Ok(())
    }

    /// The devices currently shown, in the order they were offered.
    pub fn shown(&self) -> &[Candidate] {
        &self.shown
    }

    /// The outcome, once the user has chosen or cancelled.
    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    /// True once `select` or `cancel` has succeeded.
    pub fn is_finished(&self) -> bool {
        self.outcome.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(addr: &str, name: &str, kind: DeviceKind, paired: bool) -> Candidate {
        Candidate { address: addr.parse().unwrap(), name: name.to_string(), kind, paired }
    }

    fn sample() -> Vec<Candidate> {
        vec![
            candidate("00:11:22:33:44:01", "Trackpad", DeviceKind::Pointer, false),
            candidate("00:11:22:33:44:02", "Keyboard K2", DeviceKind::Keyboard, false),
            candidate("00:11:22:33:44:03", "Headset HS", DeviceKind::Headset, true),
        ]
    }

    #[test]
    fn no_filter_shows_every_device_in_offered_order() {
        let mut picker = BluetoothPicker::new(None);
        assert_eq!(picker.offer(&sample()), Ok(3));
        let names: Vec<&str> = picker.shown().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Trackpad", "Keyboard K2", "Headset HS"]);
    }

    #[test]
    fn name_filter_matches_a_substring_ignoring_case() {
        let mut picker = BluetoothPicker::new(Some("KEY"));
        assert_eq!(picker.offer(&sample()), Ok(1));
        assert_eq!(picker.shown()[0].name, "Keyboard K2");

        let mut none = BluetoothPicker::new(Some("x"));
        assert_eq!(none.offer(&sample()), Ok(0));
    }

    #[test]
    fn a_repeated_address_is_shown_once_with_its_first_name() {
        let mut picker = BluetoothPicker::new(None);
        let mut list = sample();
        list.push(candidate("00:11:22:33:44:01", "Trackpad renamed", DeviceKind::Pointer, true));
        assert_eq!(picker.offer(&list), Ok(3));
        assert_eq!(picker.shown()[0].name, "Trackpad");
    }

    #[test]
    fn selecting_a_shown_device_finishes_with_that_device() {
        let mut picker = BluetoothPicker::new(None);
        picker.offer(&sample()).unwrap();
        assert_eq!(picker.select("00:11:22:33:44:03".parse().unwrap()), Ok(()));
        assert!(picker.is_finished());
        match picker.outcome() {
            Some(Outcome::Selected(c)) => assert_eq!(c.name, "Headset HS"),
            other => panic!("expected a selection, got {:?}", other),
        }
    }

    #[test]
    fn a_device_hidden_by_the_filter_cannot_be_selected_and_the_picker_stays_open() {
        let mut picker = BluetoothPicker::new(Some("key"));
        picker.offer(&sample()).unwrap();
        assert_eq!(picker.select("00:11:22:33:44:01".parse().unwrap()), Err(PickerError::NotShown));
        assert!(!picker.is_finished());
        assert_eq!(picker.select("00:11:22:33:44:02".parse().unwrap()), Ok(()));
    }

    #[test]
    fn after_a_choice_or_a_cancel_no_further_input_is_taken() {
        let mut picker = BluetoothPicker::new(None);
        picker.offer(&sample()).unwrap();
        picker.cancel().unwrap();
        assert_eq!(picker.outcome(), Some(&Outcome::Cancelled));
        assert_eq!(picker.cancel(), Err(PickerError::Finished));
        assert_eq!(picker.select("00:11:22:33:44:01".parse().unwrap()), Err(PickerError::Finished));
        assert_eq!(picker.offer(&sample()), Err(PickerError::Finished));
    }
}

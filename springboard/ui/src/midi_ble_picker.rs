//! Bluetooth MIDI picker and advertiser: the list of nearby BLE MIDI peripherals a central shows to the user, and the
//! advertisement a local device sends so it appears in another device's list.
//!
//! Pure model: scan results come in as advertisement bytes and the transport performs the connection. Nothing is
//! scanned or advertised here. The advertisement is built with the `advertising` module, and the MIDI service UUID
//! comes from the `midi_ble` module. A peripheral is listed only when its advertisement carries that service UUID.

use std::collections::BTreeMap;

use crate::advertising::{self, ad_type, flag, AdvertisingData, AdvertisingError, LEGACY_PAYLOAD_MAX};
use crate::midi_ble::midi_service;

/// The label shown for a peripheral whose advertisement carries no name.
pub const UNNAMED_LABEL: &str = "Unnamed MIDI device";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Disconnected,
    Connecting,
    Connected,
}

impl LinkState {
    /// Sort rank: connected peripherals first, then connecting, then the rest.
    fn rank(self) -> u8 {
        match self {
            LinkState::Connected => 0,
            LinkState::Connecting => 1,
            LinkState::Disconnected => 2,
        }
    }
}

/// One nearby BLE MIDI peripheral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: u32,
    /// The advertised name. Empty when the advertisement carries none.
    pub name: String,
    /// Signal strength of the latest advertisement, in dBm.
    pub rssi: i16,
    pub state: LinkState,
    /// Milliseconds of the latest advertisement.
    pub last_seen_ms: u64,
}

impl Candidate {
    /// The name to show, or `UNNAMED_LABEL` when there is none.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            UNNAMED_LABEL
        } else {
            &self.name
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerError {
    NotFound(u32),
}

/// The list of nearby MIDI peripherals, keyed by the transport's peripheral identifier.
#[derive(Clone, Debug, Default)]
pub struct MidiPicker {
    candidates: BTreeMap<u32, Candidate>,
    selected: Option<u32>,
}

impl MidiPicker {
    pub fn new() -> MidiPicker {
        MidiPicker::default()
    }

    /// True when the advertisement carries the MIDI service UUID.
    pub fn is_midi_advertisement(payload: &[u8]) -> bool {
        advertising::service_uuids(payload).contains(&midi_service())
    }

    /// Records an advertisement from peripheral `id`. Only MIDI advertisements are kept. A later advertisement
    /// refreshes the signal strength and time, and it renames the peripheral only when it carries a name.
    /// Returns false, and records nothing, for an advertisement without the MIDI service.
    pub fn observe(&mut self, id: u32, payload: &[u8], rssi: i16, now_ms: u64) -> bool {
        if !Self::is_midi_advertisement(payload) {
            return false;
        }
        let name = advertising::local_name(payload);
        let entry = self.candidates.entry(id).or_insert_with(|| Candidate {
            id,
            name: String::new(),
            rssi,
            state: LinkState::Disconnected,
            last_seen_ms: now_ms,
        });
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            entry.name = n;
        }
        entry.rssi = rssi;
        entry.last_seen_ms = now_ms;
        true
    }

    /// Applies the scan response of a listed peripheral. The response carries the complete name when the
    /// advertisement only had a shortened one. Returns true when it renamed the peripheral. A response for an
    /// unknown peripheral is ignored, and so is one without a name.
    pub fn observe_scan_response(&mut self, id: u32, payload: &[u8]) -> bool {
        let Some(entry) = self.candidates.get_mut(&id) else {
            return false;
        };
        match advertising::local_name(payload).filter(|n| !n.is_empty()) {
            Some(name) => {
                entry.name = name;
                true
            }
            None => false,
        }
    }

    pub fn get(&self, id: u32) -> Option<&Candidate> {
        self.candidates.get(&id)
    }

    /// Peripherals in display order: connected first, then connecting, then the rest. Within a group the order is by
    /// name, ignoring case, and then by identifier.
    pub fn rows(&self) -> Vec<&Candidate> {
        let mut rows: Vec<&Candidate> = self.candidates.values().collect();
        rows.sort_by(|a, b| {
            a.state
                .rank()
                .cmp(&b.state.rank())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then(a.id.cmp(&b.id))
        });
        rows
    }

    pub fn selected(&self) -> Option<u32> {
        self.selected
    }

    /// Selects a listed peripheral.
    pub fn select(&mut self, id: u32) -> Result<(), PickerError> {
        if !self.candidates.contains_key(&id) {
            return Err(PickerError::NotFound(id));
        }
        self.selected = Some(id);
        Ok(())
    }

    /// Sets the link state the transport reports for a listed peripheral.
    pub fn set_state(&mut self, id: u32, state: LinkState) -> Result<(), PickerError> {
        let entry = self.candidates.get_mut(&id).ok_or(PickerError::NotFound(id))?;
        entry.state = state;
        Ok(())
    }

    /// Drops a peripheral from the list. Its selection is cleared too.
    pub fn forget(&mut self, id: u32) -> Result<(), PickerError> {
        self.candidates.remove(&id).ok_or(PickerError::NotFound(id))?;
        if self.selected == Some(id) {
            self.selected = None;
        }
        Ok(())
    }

    /// Removes peripherals that have not advertised for longer than `max_age_ms`. A connected or connecting
    /// peripheral is kept whatever its age. Returns how many were removed.
    pub fn prune(&mut self, now_ms: u64, max_age_ms: u64) -> usize {
        let stale: Vec<u32> = self
            .candidates
            .values()
            .filter(|c| c.state == LinkState::Disconnected)
            .filter(|c| now_ms.saturating_sub(c.last_seen_ms) > max_age_ms)
            .map(|c| c.id)
            .collect();
        for id in &stale {
            self.candidates.remove(id);
            if self.selected == Some(*id) {
                self.selected = None;
            }
        }
        stale.len()
    }
}

/// The advertisement a local device sends to appear as a BLE MIDI peripheral: discoverable flags, the MIDI service
/// UUID and the device name. The name is shortened when the complete name does not fit the legacy payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiAdvertiser {
    name: String,
    advertising: bool,
    centrals: u32,
}

impl MidiAdvertiser {
    pub fn new(name: &str) -> MidiAdvertiser {
        MidiAdvertiser { name: name.to_string(), advertising: false, centrals: 0 }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_advertising(&self) -> bool {
        self.advertising
    }

    pub fn set_advertising(&mut self, on: bool) {
        self.advertising = on;
    }

    pub fn connected_centrals(&self) -> u32 {
        self.centrals
    }

    pub fn central_connected(&mut self) {
        self.centrals += 1;
    }

    /// Records a central leaving. The count never goes below zero.
    pub fn central_disconnected(&mut self) {
        self.centrals = self.centrals.saturating_sub(1);
    }

    /// The advertisement bytes, within the legacy payload limit.
    pub fn payload(&self) -> Result<AdvertisingData, AdvertisingError> {
        let mut ad = AdvertisingData::new();
        ad.flags(flag::LE_GENERAL_DISCOVERABLE | flag::BR_EDR_NOT_SUPPORTED)?;
        ad.service_uuids(&[midi_service()], true)?;
        if self.name.is_empty() {
            return Ok(ad);
        }
        // The name gets whatever room is left after its two header bytes.
        let room = LEGACY_PAYLOAD_MAX.saturating_sub(ad.as_bytes().len() + 2);
        if self.name.len() <= room {
            ad.push(ad_type::COMPLETE_NAME, self.name.as_bytes())?;
        } else if room > 0 {
            let mut cut = room;
            while !self.name.is_char_boundary(cut) {
                cut -= 1;
            }
            ad.push(ad_type::SHORTENED_NAME, &self.name.as_bytes()[..cut])?;
        }
        Ok(ad)
    }

    /// The scan response: the complete name, sent when it fits one legacy payload on its own. A name the
    /// advertisement had to shorten is then sent whole here. A longer name has no room in a legacy payload, so the
    /// response is empty and the advertisement's shortened name stands.
    pub fn scan_response(&self) -> Result<AdvertisingData, AdvertisingError> {
        let mut ad = AdvertisingData::new();
        if !self.name.is_empty() && self.name.len() <= LEGACY_PAYLOAD_MAX - 2 {
            ad.push(ad_type::COMPLETE_NAME, self.name.as_bytes())?;
        }
        Ok(ad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi_payload(name: &str) -> Vec<u8> {
        let mut adv = MidiAdvertiser::new(name);
        adv.set_advertising(true);
        adv.payload().unwrap().as_bytes().to_vec()
    }

    fn plain_payload() -> Vec<u8> {
        let mut ad = AdvertisingData::new();
        ad.flags(flag::LE_GENERAL_DISCOVERABLE).unwrap();
        ad.local_name("Speaker").unwrap();
        ad.as_bytes().to_vec()
    }

    #[test]
    fn advertisement_carries_flags_service_and_name_within_the_legacy_limit() {
        let bytes = midi_payload("Pad");
        // Flags 3 bytes, 128-bit service list 18 bytes, complete name 5 bytes: 26 bytes.
        assert_eq!(bytes.len(), 26);
        assert_eq!(&bytes[..3], &[2, 1, 6][..]);
        assert_eq!(advertising::local_name(&bytes), Some("Pad".to_string()));
        assert!(MidiPicker::is_midi_advertisement(&bytes));
    }

    #[test]
    fn long_name_is_shortened_to_the_room_left_and_stays_on_a_character_boundary() {
        // 31 bytes total: 21 used by flags and service, then a 2-byte header and 8 bytes of name.
        let bytes = midi_payload("Keyboard Pro Mk2");
        assert_eq!(bytes.len(), 31);
        assert_eq!(advertising::local_name(&bytes), Some("Keyboard".to_string()));
        // "é" takes two bytes. Byte 8 falls inside it, so the name is cut back to seven bytes.
        let accented = midi_payload("aaaaaaaé");
        assert_eq!(advertising::local_name(&accented), Some("aaaaaaa".to_string()));
        assert!(accented.len() <= LEGACY_PAYLOAD_MAX);
    }

    #[test]
    fn empty_name_is_listed_under_the_unnamed_label() {
        let mut picker = MidiPicker::new();
        assert!(picker.observe(7, &midi_payload(""), -50, 100));
        assert_eq!(picker.get(7).unwrap().label(), UNNAMED_LABEL);
    }

    #[test]
    fn only_midi_advertisements_are_listed() {
        let mut picker = MidiPicker::new();
        assert!(!picker.observe(1, &plain_payload(), -40, 0));
        assert!(picker.rows().is_empty());
        assert!(picker.observe(2, &midi_payload("Keys"), -40, 0));
        assert_eq!(picker.rows().len(), 1);
    }

    #[test]
    fn rows_list_connected_first_then_by_name_ignoring_case() {
        let mut picker = MidiPicker::new();
        picker.observe(1, &midi_payload("beta"), -60, 1000);
        picker.observe(2, &midi_payload("Alpha"), -70, 1000);
        picker.observe(3, &midi_payload("Gamma"), -80, 1000);
        picker.set_state(3, LinkState::Connected).unwrap();
        let ids: Vec<u32> = picker.rows().iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![3, 2, 1]);
    }

    #[test]
    fn a_later_advertisement_refreshes_signal_and_time_and_keeps_a_known_name() {
        let mut picker = MidiPicker::new();
        picker.observe(5, &midi_payload("Pad"), -70, 1000);
        picker.observe(5, &midi_payload(""), -55, 4000);
        let c = picker.get(5).unwrap();
        assert_eq!(c.rssi, -55);
        assert_eq!(c.last_seen_ms, 4000);
        assert_eq!(c.name, "Pad");
    }

    #[test]
    fn prune_removes_old_idle_peripherals_and_keeps_connected_ones() {
        let mut picker = MidiPicker::new();
        picker.observe(1, &midi_payload("Old"), -60, 1000); // age 9000 at now 10000
        picker.observe(2, &midi_payload("Recent"), -60, 6000); // age 4000
        picker.observe(3, &midi_payload("Linked"), -60, 0); // old, but connected
        picker.set_state(3, LinkState::Connected).unwrap();
        assert_eq!(picker.prune(10_000, 5_000), 1);
        assert!(picker.get(1).is_none());
        assert!(picker.get(2).is_some());
        assert!(picker.get(3).is_some());
    }

    #[test]
    fn selection_needs_a_listed_peripheral_and_clears_when_it_is_removed() {
        let mut picker = MidiPicker::new();
        assert_eq!(picker.select(9), Err(PickerError::NotFound(9)));
        picker.observe(2, &midi_payload("Pad"), -60, 0);
        picker.select(2).unwrap();
        assert_eq!(picker.selected(), Some(2));
        picker.forget(2).unwrap();
        assert_eq!(picker.selected(), None);
        assert_eq!(picker.forget(2), Err(PickerError::NotFound(2)));
    }

    #[test]
    fn advertiser_counts_centrals_without_going_below_zero() {
        let mut adv = MidiAdvertiser::new("Pad");
        assert!(!adv.is_advertising());
        adv.central_connected();
        adv.central_connected();
        adv.central_disconnected();
        adv.central_disconnected();
        adv.central_disconnected();
        assert_eq!(adv.connected_centrals(), 0);
    }
}

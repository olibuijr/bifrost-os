//! Bluetooth service browser: choosing one service on a device from its list (std only).
//!
//! The browser keeps an optional allow list of service class UUIDs. With no entries it accepts any service. With
//! entries it shows only the services whose UUID is listed, and a choice must come from that filtered list. The
//! caller runs the service query on the radio and passes the records in; this module runs no query, opens no
//! socket and draws no panel. It holds the title, prompt and description text a panel would show, and the option bits.

use crate::bt_address::BdAddr;
use crate::sdp_element::Uuid;

/// One service a device offers: its class UUID and the name it advertises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub class_uuid: Uuid,
    pub name: String,
}

/// The service the user chose, with the device it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub device: BdAddr,
    pub service: Service,
}

/// Why a choice was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseError {
    /// The index is past the end of the filtered list.
    NoSuchService,
}

/// The state of one service browser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceBrowser {
    allowed: Vec<Uuid>,
    title: String,
    prompt: String,
    description: String,
    options: u32,
    result: Option<Selection>,
}

impl ServiceBrowser {
    /// A browser with no allow list, so every service is shown and accepted.
    pub fn new() -> Self {
        ServiceBrowser { prompt: "Select".to_string(), ..ServiceBrowser::default() }
    }

    /// Adds one UUID to the allow list. A UUID already listed is not added twice.
    pub fn add_allowed_uuid(&mut self, uuid: Uuid) {
        if !self.allowed.contains(&uuid) {
            self.allowed.push(uuid);
        }
    }

    /// Adds several UUIDs to the allow list, in order.
    pub fn add_allowed_uuids(&mut self, uuids: &[Uuid]) {
        for u in uuids {
            self.add_allowed_uuid(*u);
        }
    }

    /// Empties the allow list, so the browser accepts any service again.
    pub fn clear_allowed_uuids(&mut self) {
        self.allowed.clear();
    }

    /// The UUIDs on the allow list.
    pub fn allowed_uuids(&self) -> &[Uuid] {
        &self.allowed
    }

    /// Whether a service with this UUID may be shown and chosen.
    pub fn accepts(&self, uuid: Uuid) -> bool {
        self.allowed.is_empty() || self.allowed.contains(&uuid)
    }

    /// The services a panel would list: all of them with no allow list, otherwise the listed ones in their order.
    pub fn visible(&self, services: &[Service]) -> Vec<Service> {
        services.iter().filter(|s| self.accepts(s.class_uuid)).cloned().collect()
    }

    /// Records the user's choice: `index` picks from the visible list. Replaces any earlier choice.
    pub fn choose(&mut self, device: BdAddr, services: &[Service], index: usize) -> Result<(), BrowseError> {
        let shown = self.visible(services);
        let service = shown.get(index).cloned().ok_or(BrowseError::NoSuchService)?;
        self.result = Some(Selection { device, service });
        Ok(())
    }

    /// The last choice, or `None` when the user has not chosen (or the panel was cancelled).
    pub fn results(&self) -> Option<&Selection> {
        self.result.as_ref()
    }

    /// Forgets the choice, as cancelling the panel does.
    pub fn cancel(&mut self) {
        self.result = None;
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn set_prompt(&mut self, prompt: &str) {
        self.prompt = prompt.to_string();
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn set_description(&mut self, text: &str) {
        self.description = text.to_string();
    }

    /// The option bits that control the panel's behaviour. The bit meanings belong to the caller.
    pub fn options(&self) -> u32 {
        self.options
    }

    pub fn set_options(&mut self, bits: u32) {
        self.options = bits;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(u16_id: u16, name: &str) -> Service {
        Service { class_uuid: Uuid::U16(u16_id), name: name.to_string() }
    }

    fn device() -> BdAddr {
        BdAddr([0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
    }

    #[test]
    fn with_no_allow_list_every_service_is_visible_and_choosable() {
        let list = vec![svc(0x110B, "Audio Sink"), svc(0x1101, "Serial")];
        let mut b = ServiceBrowser::new();
        assert_eq!(b.visible(&list).len(), 2);
        b.choose(device(), &list, 1).unwrap();
        assert_eq!(b.results().unwrap().service.name, "Serial");
        assert_eq!(b.results().unwrap().device, device());
    }

    #[test]
    fn allow_list_filters_the_visible_services_and_keeps_their_order() {
        let list = vec![svc(0x1101, "Serial"), svc(0x110B, "Audio Sink"), svc(0x1108, "Headset")];
        let mut b = ServiceBrowser::new();
        b.add_allowed_uuids(&[Uuid::U16(0x1108), Uuid::U16(0x1101)]);
        let shown = b.visible(&list);
        assert_eq!(shown.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Serial", "Headset"]);
        assert!(!b.accepts(Uuid::U16(0x110B)));
    }

    #[test]
    fn a_choice_indexes_the_filtered_list_not_the_full_list() {
        let list = vec![svc(0x1101, "Serial"), svc(0x110B, "Audio Sink"), svc(0x1108, "Headset")];
        let mut b = ServiceBrowser::new();
        b.add_allowed_uuid(Uuid::U16(0x1108));
        // Index 0 of the filtered list is Headset, even though it is index 2 in the full list.
        b.choose(device(), &list, 0).unwrap();
        assert_eq!(b.results().unwrap().service.name, "Headset");
        assert_eq!(b.choose(device(), &list, 1), Err(BrowseError::NoSuchService));
    }

    #[test]
    fn duplicate_uuids_are_listed_once_and_clearing_restores_every_service() {
        let mut b = ServiceBrowser::new();
        b.add_allowed_uuid(Uuid::U16(0x1101));
        b.add_allowed_uuid(Uuid::U16(0x1101));
        assert_eq!(b.allowed_uuids().len(), 1);
        b.clear_allowed_uuids();
        assert!(b.accepts(Uuid::U16(0x110B)));
        assert!(b.allowed_uuids().is_empty());
    }

    #[test]
    fn cancel_clears_the_choice_and_text_and_options_keep_their_values() {
        let list = vec![svc(0x1101, "Serial")];
        let mut b = ServiceBrowser::new();
        assert_eq!(b.prompt(), "Select");
        b.set_title("Choose a service");
        b.set_description("Pick a Bluetooth service");
        b.set_options(0b101);
        b.choose(device(), &list, 0).unwrap();
        b.cancel();
        assert!(b.results().is_none());
        assert_eq!(b.title(), "Choose a service");
        assert_eq!(b.description(), "Pick a Bluetooth service");
        assert_eq!(b.options(), 0b101);
    }
}

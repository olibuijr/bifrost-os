//! Local push: which push provider runs on which Wi-Fi network, and a bounded inbox for the messages it delivers (std
//! only).
//!
//! Local push is the push path for a network that has no route to the wider internet. A push manager names the Wi-Fi
//! networks it runs on by SSID. When the machine is on one of those networks, the provider named by the manager keeps
//! a connection to a local server and hands messages to the app. Off those networks the manager stays idle.
//!
//! This is a pure model. It opens no connection, starts no provider and reads no Wi-Fi state: the caller tells the
//! registry which SSID is current, and the caller saves the registry in its own settings.
//!
//! Rules of this model:
//! - An SSID is 1 to 32 bytes, the length of a Wi-Fi network name. A manager holds each SSID at most once.
//! - A manager is active only when it is enabled and the current SSID is in its list. No current SSID means no
//!   manager is active.
//! - A refused change leaves the manager or the registry as it was.
//! - An inbox keeps the messages not yet taken, in arrival order, and drops the oldest when it is full. It also remembers
//!   the ids of the last `capacity` messages it accepted, so a message resent during a reconnect is not queued twice.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Longest Wi-Fi network name, in bytes.
pub const MAX_SSID_BYTES: usize = 32;

/// Why a push setting or message is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushError {
    /// The manager id, the description or the provider bundle identifier is empty.
    EmptyField(&'static str),
    /// An SSID is empty or longer than 32 bytes.
    InvalidSsid(String),
    /// The same SSID is listed twice.
    DuplicateSsid(String),
    /// A manager with this id is already in the registry.
    DuplicateManager(String),
    /// A message id is empty.
    EmptyMessageId,
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushError::EmptyField(field) => write!(f, "the {field} is empty"),
            PushError::InvalidSsid(ssid) => write!(f, "{ssid:?} is not a valid network name"),
            PushError::DuplicateSsid(ssid) => write!(f, "the network {ssid:?} is listed twice"),
            PushError::DuplicateManager(id) => write!(f, "a push manager with the id {id:?} already exists"),
            PushError::EmptyMessageId => write!(f, "the message id is empty"),
        }
    }
}

impl std::error::Error for PushError {}

/// One push manager: the provider it starts, the networks it runs on and whether it is switched on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushManager {
    id: String,
    description: String,
    provider_bundle_id: String,
    /// Vendor-specific settings the provider reads when it starts, in key order.
    pub provider_configuration: BTreeMap<String, String>,
    enabled: bool,
    match_ssids: Vec<String>,
}

impl PushManager {
    /// A manager that is switched off and matches no network.
    pub fn new(id: &str, description: &str, provider_bundle_id: &str) -> Result<Self, PushError> {
        let id = id.trim();
        let description = description.trim();
        let provider_bundle_id = provider_bundle_id.trim();
        if id.is_empty() {
            return Err(PushError::EmptyField("manager id"));
        }
        if description.is_empty() {
            return Err(PushError::EmptyField("description"));
        }
        if provider_bundle_id.is_empty() {
            return Err(PushError::EmptyField("provider bundle identifier"));
        }
        Ok(PushManager {
            id: id.to_string(),
            description: description.to_string(),
            provider_bundle_id: provider_bundle_id.to_string(),
            provider_configuration: BTreeMap::new(),
            enabled: false,
            match_ssids: Vec::new(),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn provider_bundle_id(&self) -> &str {
        &self.provider_bundle_id
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The networks this manager runs on, in the order they were set.
    pub fn match_ssids(&self) -> &[String] {
        &self.match_ssids
    }

    /// Replaces the list of networks. Each name must be 1 to 32 bytes and appear once. A refused list changes nothing.
    pub fn set_match_ssids(&mut self, ssids: &[&str]) -> Result<(), PushError> {
        let mut checked: Vec<String> = Vec::with_capacity(ssids.len());
        for ssid in ssids {
            if ssid.is_empty() || ssid.len() > MAX_SSID_BYTES {
                return Err(PushError::InvalidSsid(ssid.to_string()));
            }
            if checked.iter().any(|s| s.as_str() == *ssid) {
                return Err(PushError::DuplicateSsid(ssid.to_string()));
            }
            checked.push(ssid.to_string());
        }
        self.match_ssids = checked;
        Ok(())
    }

    /// True when the manager is enabled and `current_ssid` is one of its networks.
    pub fn is_active_on(&self, current_ssid: Option<&str>) -> bool {
        match current_ssid {
            Some(ssid) => self.enabled && self.match_ssids.iter().any(|s| s == ssid),
            None => false,
        }
    }
}

/// The push managers of one machine, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushRegistry {
    managers: Vec<PushManager>,
}

impl PushRegistry {
    pub fn new() -> Self {
        PushRegistry::default()
    }

    /// Adds a manager. A manager whose id is already in the registry is refused and nothing changes.
    pub fn add(&mut self, manager: PushManager) -> Result<(), PushError> {
        if self.managers.iter().any(|m| m.id == manager.id) {
            return Err(PushError::DuplicateManager(manager.id));
        }
        self.managers.push(manager);
        Ok(())
    }

    /// Removes the manager with `id`. Returns false when there is none.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.managers.len();
        self.managers.retain(|m| m.id != id);
        self.managers.len() != before
    }

    pub fn get(&self, id: &str) -> Option<&PushManager> {
        self.managers.iter().find(|m| m.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut PushManager> {
        self.managers.iter_mut().find(|m| m.id == id)
    }

    /// The managers that are active on `current_ssid`, in the order they were added.
    pub fn active_on(&self, current_ssid: Option<&str>) -> Vec<&PushManager> {
        self.managers.iter().filter(|m| m.is_active_on(current_ssid)).collect()
    }
}

/// One message the provider delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushMessage {
    pub id: String,
    pub payload: Vec<u8>,
}

/// What the inbox did with a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted {
    /// The message is waiting to be taken.
    Queued,
    /// The message id was seen recently, so the message was dropped.
    Duplicate,
}

/// A bounded inbox of pushed messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushInbox {
    capacity: usize,
    queue: VecDeque<PushMessage>,
    recent_ids: VecDeque<String>,
    dropped: u64,
}

impl PushInbox {
    /// An empty inbox that holds up to `capacity` messages. A capacity of zero is raised to one.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        PushInbox { capacity, queue: VecDeque::new(), recent_ids: VecDeque::new(), dropped: 0 }
    }

    /// Accepts one message. A message whose id is among the last `capacity` accepted ids is a duplicate. When the
    /// inbox is full, the oldest waiting message is dropped to make room.
    pub fn accept(&mut self, id: &str, payload: &[u8]) -> Result<Accepted, PushError> {
        if id.is_empty() {
            return Err(PushError::EmptyMessageId);
        }
        if self.recent_ids.iter().any(|seen| seen == id) {
            return Ok(Accepted::Duplicate);
        }
        self.recent_ids.push_back(id.to_string());
        if self.recent_ids.len() > self.capacity {
            self.recent_ids.pop_front();
        }
        if self.queue.len() == self.capacity {
            self.queue.pop_front();
            self.dropped += 1;
        }
        self.queue.push_back(PushMessage { id: id.to_string(), payload: payload.to_vec() });
        Ok(Accepted::Queued)
    }

    /// Takes the oldest waiting message.
    pub fn take(&mut self) -> Option<PushMessage> {
        self.queue.pop_front()
    }

    /// The number of messages waiting to be taken.
    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// The number of waiting messages dropped because the inbox was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(id: &str) -> PushManager {
        PushManager::new(id, "Calls", "com.example.voip-provider").unwrap()
    }

    #[test]
    fn a_name_must_be_one_to_thirty_two_bytes_and_listed_once() {
        let mut m = manager("calls");
        assert_eq!(m.set_match_ssids(&[""]), Err(PushError::InvalidSsid(String::new())));
        let long = "x".repeat(33);
        assert_eq!(m.set_match_ssids(&[long.as_str()]), Err(PushError::InvalidSsid(long.clone())));
        assert_eq!(m.set_match_ssids(&["ward", "ward"]), Err(PushError::DuplicateSsid("ward".into())));
        // The 32-byte name is the longest accepted.
        m.set_match_ssids(&["ward-2", &long[..32]]).unwrap();
        assert_eq!(m.match_ssids().len(), 2);
    }

    #[test]
    fn a_refused_list_keeps_the_previous_networks() {
        let mut m = manager("calls");
        m.set_match_ssids(&["ward-1"]).unwrap();
        assert!(m.set_match_ssids(&["ward-2", "ward-2"]).is_err());
        assert_eq!(m.match_ssids(), &["ward-1".to_string()]);
    }

    #[test]
    fn a_manager_is_active_only_when_enabled_on_a_listed_network() {
        let mut m = manager("calls");
        m.set_match_ssids(&["ward-1"]).unwrap();
        assert!(!m.is_active_on(Some("ward-1")), "a disabled manager is idle");
        m.set_enabled(true);
        assert!(m.is_active_on(Some("ward-1")));
        assert!(!m.is_active_on(Some("ward-2")));
        assert!(!m.is_active_on(None), "no current network means no active manager");
    }

    #[test]
    fn the_registry_lists_active_managers_in_the_order_they_were_added() {
        let mut reg = PushRegistry::new();
        let mut a = manager("a");
        a.set_match_ssids(&["ward"]).unwrap();
        a.set_enabled(true);
        let mut b = manager("b");
        b.set_match_ssids(&["ward"]).unwrap();
        let mut c = manager("c");
        c.set_match_ssids(&["ward"]).unwrap();
        c.set_enabled(true);
        reg.add(c).unwrap();
        reg.add(a).unwrap();
        reg.add(b).unwrap();
        let ids: Vec<String> = reg.active_on(Some("ward")).iter().map(|m| m.id().to_string()).collect();
        assert_eq!(ids, ["c", "a"]);
        assert!(reg.active_on(Some("home")).is_empty());
    }

    #[test]
    fn a_registry_keeps_one_manager_per_id() {
        let mut reg = PushRegistry::new();
        reg.add(manager("calls")).unwrap();
        assert_eq!(reg.add(manager("calls")), Err(PushError::DuplicateManager("calls".into())));
        assert!(reg.remove("calls"));
        assert!(!reg.remove("calls"));
        assert!(reg.get("calls").is_none());
    }

    #[test]
    fn empty_identifiers_are_refused() {
        assert_eq!(PushManager::new(" ", "x", "y").unwrap_err(), PushError::EmptyField("manager id"));
        assert_eq!(PushManager::new("a", "", "y").unwrap_err(), PushError::EmptyField("description"));
        assert_eq!(
            PushManager::new("a", "x", "").unwrap_err(),
            PushError::EmptyField("provider bundle identifier")
        );
    }

    #[test]
    fn a_repeated_message_id_is_dropped_while_it_is_remembered() {
        let mut inbox = PushInbox::new(2);
        assert_eq!(inbox.accept("m1", b"hello").unwrap(), Accepted::Queued);
        assert_eq!(inbox.accept("m1", b"hello").unwrap(), Accepted::Duplicate);
        assert_eq!(inbox.waiting(), 1);
        assert_eq!(inbox.accept("", b"x"), Err(PushError::EmptyMessageId));
    }

    #[test]
    fn a_full_inbox_drops_the_oldest_waiting_message() {
        let mut inbox = PushInbox::new(2);
        inbox.accept("a", b"1").unwrap();
        inbox.accept("b", b"2").unwrap();
        inbox.accept("c", b"3").unwrap();
        assert_eq!(inbox.dropped(), 1);
        assert_eq!(inbox.waiting(), 2);
        assert_eq!(inbox.take().unwrap().id, "b");
        assert_eq!(inbox.take().unwrap().payload, b"3".to_vec());
        assert!(inbox.take().is_none());
    }

    #[test]
    fn an_id_that_has_left_the_memory_is_queued_again() {
        let mut inbox = PushInbox::new(1);
        inbox.accept("a", b"1").unwrap();
        inbox.accept("b", b"2").unwrap();
        // The memory holds the last one id, so "a" is no longer a duplicate.
        assert_eq!(inbox.accept("a", b"1").unwrap(), Accepted::Queued);
        assert_eq!(inbox.dropped(), 2);
    }
}

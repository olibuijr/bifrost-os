//! Network audio browser: the receivers on the local network that an app can send audio to. The system layer runs
//! discovery and hands each announcement to `NetworkAudioBrowser::announce`. The browser keeps one entry per
//! receiver identifier, reports when a receiver appears or stops announcing, and lists the rest as route picker
//! rows through the `route_picker` module, so the app's authorization of each route applies. This is the pure
//! model. No socket is opened and no announcement is sent here.

use std::collections::BTreeMap;

use crate::route_auth::RouteAuth;
use crate::route_picker::{self, Destination};

/// How long a receiver stays listed after its last announcement, in milliseconds, unless the caller chooses another
/// time-to-live.
pub const DEFAULT_TTL_MS: u64 = 30_000;

/// One receiver as the discovery source describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receiver {
    /// Stable identifier across announcements, for example the receiver's advertised instance name.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// The address the receiver announced from.
    pub host: String,
    /// The port the receiver listens on. Zero is refused.
    pub port: u16,
    /// Milliseconds of the latest announcement.
    pub last_seen_ms: u64,
}

/// Failures of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseError {
    /// The identifier is empty or only spaces.
    EmptyId,
    /// The name is empty or only spaces.
    EmptyName,
    /// The host is empty or only spaces.
    EmptyHost,
    /// The port is zero.
    ZeroPort,
    /// The time-to-live is zero.
    InvalidTtl,
}

/// A change the browser reports, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseEvent {
    /// A receiver was announced for the first time, or again after it expired.
    Found(Receiver),
    /// A receiver stopped announcing for longer than the time-to-live. Carries its last description.
    Lost(Receiver),
}

/// Tracks the receivers seen on the network.
#[derive(Debug)]
pub struct NetworkAudioBrowser {
    ttl_ms: u64,
    browsing: bool,
    receivers: BTreeMap<String, Receiver>,
}

impl NetworkAudioBrowser {
    /// A stopped browser with the given time-to-live.
    pub fn new(ttl_ms: u64) -> Result<Self, BrowseError> {
        if ttl_ms == 0 {
            return Err(BrowseError::InvalidTtl);
        }
        Ok(NetworkAudioBrowser { ttl_ms, browsing: false, receivers: BTreeMap::new() })
    }

    /// A stopped browser with `DEFAULT_TTL_MS`.
    pub fn with_default_ttl() -> Self {
        NetworkAudioBrowser { ttl_ms: DEFAULT_TTL_MS, browsing: false, receivers: BTreeMap::new() }
    }

    pub fn start(&mut self) {
        self.browsing = true;
    }

    /// Stops browsing. Known receivers stay listed, and announcements and expiry are ignored until `start`.
    pub fn stop(&mut self) {
        self.browsing = false;
    }

    pub fn is_browsing(&self) -> bool {
        self.browsing
    }

    /// Records an announcement. A new receiver returns `Found`. A known receiver is refreshed in place with no event,
    /// so its name, host and port follow the latest announcement. While the browser is stopped the announcement is
    /// checked and then ignored, and nothing is returned.
    pub fn announce(&mut self, receiver: Receiver) -> Result<Option<BrowseEvent>, BrowseError> {
        let checked = check(receiver)?;
        if !self.browsing {
            return Ok(None);
        }
        if let Some(known) = self.receivers.get_mut(&checked.id) {
            *known = checked;
            return Ok(None);
        }
        self.receivers.insert(checked.id.clone(), checked.clone());
        Ok(Some(BrowseEvent::Found(checked)))
    }

    /// Removes every receiver whose last announcement is more than the time-to-live before `now_ms`. Returns the
    /// `Lost` events in identifier order. A receiver announced exactly one time-to-live ago is still listed.
    pub fn expire(&mut self, now_ms: u64) -> Vec<BrowseEvent> {
        if !self.browsing {
            return Vec::new();
        }
        let ttl = self.ttl_ms;
        let stale: Vec<String> = self
            .receivers
            .iter()
            .filter(|(_, r)| now_ms.saturating_sub(r.last_seen_ms) > ttl)
            .map(|(id, _)| id.clone())
            .collect();
        stale
            .into_iter()
            .filter_map(|id| self.receivers.remove(&id))
            .map(BrowseEvent::Lost)
            .collect()
    }

    /// Every listed receiver, in identifier order.
    pub fn receivers(&self) -> impl Iterator<Item = &Receiver> {
        self.receivers.values()
    }

    pub fn receiver(&self, id: &str) -> Option<&Receiver> {
        self.receivers.get(id)
    }

    /// The route picker rows for the listed receivers. Each row's identifier is the receiver's identifier, and its
    /// active state comes from `auth`. Rows are sorted by name ignoring case, as the route picker sorts them.
    pub fn destinations(&self, auth: &RouteAuth) -> Vec<Destination> {
        let found: Vec<(&str, &str)> = self.receivers.values().map(|r| (r.id.as_str(), r.name.as_str())).collect();
        route_picker::destinations(&[], &found, auth)
    }
}

/// Checks the fields of an announcement and trims the text fields.
fn check(receiver: Receiver) -> Result<Receiver, BrowseError> {
    let id = receiver.id.trim();
    let name = receiver.name.trim();
    let host = receiver.host.trim();
    if id.is_empty() {
        return Err(BrowseError::EmptyId);
    }
    if name.is_empty() {
        return Err(BrowseError::EmptyName);
    }
    if host.is_empty() {
        return Err(BrowseError::EmptyHost);
    }
    if receiver.port == 0 {
        return Err(BrowseError::ZeroPort);
    }
    Ok(Receiver {
        id: id.to_string(),
        name: name.to_string(),
        host: host.to_string(),
        port: receiver.port,
        last_seen_ms: receiver.last_seen_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx(id: &str, name: &str, at: u64) -> Receiver {
        Receiver { id: id.into(), name: name.into(), host: "192.168.1.20".into(), port: 7000, last_seen_ms: at }
    }

    fn browsing(ttl: u64) -> NetworkAudioBrowser {
        let mut b = NetworkAudioBrowser::new(ttl).unwrap();
        b.start();
        b
    }

    #[test]
    fn a_zero_time_to_live_is_refused() {
        assert_eq!(NetworkAudioBrowser::new(0).err(), Some(BrowseError::InvalidTtl));
        assert_eq!(NetworkAudioBrowser::with_default_ttl().receivers().count(), 0);
    }

    #[test]
    fn a_new_receiver_is_found_once_and_refreshed_silently() {
        let mut b = browsing(1_000);
        assert_eq!(b.announce(rx("r1", "Kitchen", 0)), Ok(Some(BrowseEvent::Found(rx("r1", "Kitchen", 0)))));
        assert_eq!(b.announce(rx("r1", "Kitchen", 500)), Ok(None));
        assert_eq!(b.receiver("r1").unwrap().last_seen_ms, 500);
    }

    #[test]
    fn a_renamed_receiver_keeps_its_identifier_and_takes_the_new_name() {
        let mut b = browsing(1_000);
        b.announce(rx("r1", "Kitchen", 0)).unwrap();
        b.announce(rx("r1", "Garden", 10)).unwrap();
        assert_eq!(b.receiver("r1").unwrap().name, "Garden");
        assert_eq!(b.receivers().count(), 1);
    }

    #[test]
    fn invalid_fields_are_refused_and_text_is_trimmed() {
        let mut b = browsing(1_000);
        assert_eq!(b.announce(rx(" ", "Kitchen", 0)).err(), Some(BrowseError::EmptyId));
        assert_eq!(b.announce(rx("r1", "", 0)).err(), Some(BrowseError::EmptyName));
        let mut no_host = rx("r1", "Kitchen", 0);
        no_host.host = "   ".into();
        assert_eq!(b.announce(no_host).err(), Some(BrowseError::EmptyHost));
        let mut no_port = rx("r1", "Kitchen", 0);
        no_port.port = 0;
        assert_eq!(b.announce(no_port).err(), Some(BrowseError::ZeroPort));
        let padded = b.announce(rx("  r2 ", "  Attic  ", 0)).unwrap();
        assert_eq!(padded, Some(BrowseEvent::Found(rx("r2", "Attic", 0))));
    }

    #[test]
    fn a_stopped_browser_ignores_announcements_and_expiry() {
        let mut b = NetworkAudioBrowser::new(100).unwrap();
        assert_eq!(b.announce(rx("r1", "Kitchen", 0)), Ok(None));
        assert_eq!(b.receivers().count(), 0);
        b.start();
        b.announce(rx("r1", "Kitchen", 0)).unwrap();
        b.stop();
        assert!(b.expire(10_000).is_empty(), "no loss is reported while stopped");
        assert!(b.receiver("r1").is_some(), "known receivers stay listed after stop");
    }

    #[test]
    fn receivers_that_stop_announcing_are_lost_after_the_time_to_live() {
        let mut b = browsing(1_000);
        b.announce(rx("a", "Attic", 0)).unwrap();
        b.announce(rx("b", "Bedroom", 900)).unwrap();
        // At 1000 ms, "a" is exactly one time-to-live old and stays. At 1001 ms it is lost.
        assert!(b.expire(1_000).is_empty());
        assert_eq!(b.expire(1_001), vec![BrowseEvent::Lost(rx("a", "Attic", 0))]);
        assert!(b.receiver("a").is_none());
        assert!(b.receiver("b").is_some());
    }

    #[test]
    fn a_lost_receiver_is_found_again_when_it_announces() {
        let mut b = browsing(100);
        b.announce(rx("a", "Attic", 0)).unwrap();
        b.expire(500);
        assert_eq!(b.announce(rx("a", "Attic", 600)), Ok(Some(BrowseEvent::Found(rx("a", "Attic", 600)))));
    }

    #[test]
    fn destinations_list_receivers_sorted_by_name_with_their_active_state() {
        let mut b = browsing(1_000);
        b.announce(rx("r1", "kitchen", 0)).unwrap();
        b.announce(rx("r2", "Attic", 0)).unwrap();
        let mut auth = RouteAuth::new();
        auth.authorize("r1");
        auth.set_active("r1", true).unwrap();
        let rows = b.destinations(&auth);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Attic", "kitchen"]);
        assert!(!rows[0].active);
        assert!(rows[1].active);
        assert!(rows.iter().all(|r| !r.local));
    }
}

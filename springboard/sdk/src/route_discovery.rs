//! Receiver discovery for route pickers, the app-facing side of a discovery extension.
//!
//! A discovery extension reports the media receivers it sees on the local network to the shell, and the shell lists
//! them in the route picker beside the local outputs. [`discovery_point`] declares the extension point that an app's
//! namespace offers for this. [`ReceiverFeed`] checks each report, keeps the receivers that are still announcing, and
//! lists them as picker rows with their authorization state. The feed sends no announcement and opens no socket: the
//! shell's browse service calls [`ReceiverFeed::report`] and [`ReceiverFeed::expire`] with what it received.

use std::fmt;

use bifrost_ui::extension::{Point, Scope};
use bifrost_ui::network_audio_browser::{BrowseError, NetworkAudioBrowser};
use bifrost_ui::route_auth::RouteAuth;

pub use bifrost_ui::network_audio_browser::{BrowseEvent as ReceiverEvent, Receiver};
pub use bifrost_ui::route_picker::Destination;

/// The identifier suffix of the discovery point that an app declares under its own bundle identifier.
pub const DISCOVERY_SUFFIX: &str = "route-discovery";

/// Why a discovery call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The owner is not a reverse-DNS bundle identifier such as `com.example.app`, or the discovery identifier built
    /// from it is too long.
    InvalidOwner,
    /// The time-to-live is zero.
    ZeroTimeToLive,
    /// The receiver report failed a check. The value names the field.
    Receiver(BrowseError),
}

impl From<BrowseError> for DiscoveryError {
    fn from(error: BrowseError) -> Self {
        match error {
            BrowseError::InvalidTtl => DiscoveryError::ZeroTimeToLive,
            other => DiscoveryError::Receiver(other),
        }
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            DiscoveryError::InvalidOwner => "owner must be a bundle identifier such as com.example.app",
            DiscoveryError::ZeroTimeToLive | DiscoveryError::Receiver(BrowseError::InvalidTtl) => {
                "the receiver time-to-live must be more than zero milliseconds"
            }
            DiscoveryError::Receiver(BrowseError::EmptyId) => "receiver identifier is empty",
            DiscoveryError::Receiver(BrowseError::EmptyName) => "receiver name is empty",
            DiscoveryError::Receiver(BrowseError::EmptyHost) => "receiver host is empty",
            DiscoveryError::Receiver(BrowseError::ZeroPort) => "receiver port is zero",
        };
        f.write_str(text)
    }
}

/// The discovery extension point for the app whose bundle identifier is `owner`. Any package with a valid bundle
/// identifier may bind a discovery extension to it, because the shell still asks the user before a receiver can be
/// used.
pub fn discovery_point(owner: &str) -> Result<Point, DiscoveryError> {
    let identifier = format!("{owner}.{DISCOVERY_SUFFIX}");
    Point::new(owner, &identifier, Scope::Any).map_err(|_| DiscoveryError::InvalidOwner)
}

/// The receivers that a discovery extension reported, kept while they keep announcing.
#[derive(Debug)]
pub struct ReceiverFeed {
    browser: NetworkAudioBrowser,
}

impl ReceiverFeed {
    /// A stopped feed. A receiver stays listed for `ttl_ms` milliseconds after its last report.
    pub fn new(ttl_ms: u64) -> Result<Self, DiscoveryError> {
        Ok(ReceiverFeed { browser: NetworkAudioBrowser::new(ttl_ms)? })
    }

    /// A stopped feed with the default time-to-live of thirty seconds.
    pub fn with_default_ttl() -> Self {
        ReceiverFeed { browser: NetworkAudioBrowser::with_default_ttl() }
    }

    /// Starts accepting reports.
    pub fn start(&mut self) {
        self.browser.start();
    }

    /// Stops accepting reports. Receivers already listed stay listed.
    pub fn stop(&mut self) {
        self.browser.stop();
    }

    pub fn is_running(&self) -> bool {
        self.browser.is_browsing()
    }

    /// Checks a report and records it. A receiver that was not listed returns `Found`. A refresh of a listed receiver,
    /// and any report while the feed is stopped, returns nothing.
    pub fn report(&mut self, receiver: Receiver) -> Result<Option<ReceiverEvent>, DiscoveryError> {
        Ok(self.browser.announce(receiver)?)
    }

    /// Drops the receivers whose last report is more than the time-to-live before `now_ms`, and returns their `Lost`
    /// events in identifier order.
    pub fn expire(&mut self, now_ms: u64) -> Vec<ReceiverEvent> {
        self.browser.expire(now_ms)
    }

    /// Every listed receiver, in identifier order.
    pub fn receivers(&self) -> impl Iterator<Item = &Receiver> {
        self.browser.receivers()
    }

    /// The route picker rows for the listed receivers, sorted by name ignoring case, with their active state from `auth`.
    pub fn destinations(&self, auth: &RouteAuth) -> Vec<Destination> {
        self.browser.destinations(auth)
    }
}
